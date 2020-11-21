use llvm_ir::{Module, Instruction, Terminator, Function, Name, BasicBlock, Operand, ConstantRef, Constant, TypeRef, Type, HasDebugLoc};
use llvm_ir::instruction::{Call, InlineAssembly, Phi, Alloca, Store, Load, Xor, SExt, BitCast, InsertValue, ZExt, AtomicRMW, Trunc, Select, PtrToInt, Sub, Or, And, IntToPtr, UDiv, SDiv, URem, SRem, Shl, LShr, AShr, ExtractValue, Mul, CmpXchg, Fence};
use std::collections::{HashMap, BTreeMap};
use std::rc::Rc;
use std::fmt::{Debug, Formatter, Display};
use std::{fmt, iter, mem, ops};
use std::borrow::BorrowMut;
use either::Either;
use std::cell::{Cell, RefCell, Ref};
use llvm_ir::instruction::Add;
use llvm_ir::instruction::ICmp;
use llvm_ir::constant::Constant::Undef;
use llvm_ir::types::NamedStructDef;
use llvm_ir::function::ParameterAttribute;
use std::mem::size_of;
use crate::value::Value;
use crate::memory::{Memory, Symbol};
use crate::ctx::{Ctx, CompiledFunction, EvalCtx};
use crate::native::NativeFunction;

const PIPELINE_SIZE: usize = 1;

pub struct Thread<'ctx> {
    threadid: usize,
    seq: usize,
    ctx: &'ctx Ctx<'ctx>,
    frames: Vec<Frame<'ctx>>,
    pub thunks: BTreeMap<usize, Rc<Thunk<'ctx>>>,
}

struct InstrPtr<'ctx> {
    block: &'ctx BasicBlock,
    index: usize,
    stuck: Option<Vec<Rc<Thunk<'ctx>>>>,
}

pub struct Frame<'ctx> {
    fun: &'ctx CompiledFunction<'ctx>,
    origin: Option<&'ctx Name>,
    ip: InstrPtr<'ctx>,
    temps: HashMap<&'ctx Name, Rc<Thunk<'ctx>>>,
    allocs: Vec<Rc<Thunk<'ctx>>>,
    result: Option<&'ctx Name>,
}

pub enum Node<'ctx> {
    Instr(&'ctx Instruction),
    Constant(&'ctx Constant),
    Value(Value),
    Metadata,
    Return(Option<Rc<Thunk<'ctx>>>, Vec<Rc<Thunk<'ctx>>>),
    Native(&'ctx NativeFunction),
}


pub struct Thunk<'ctx> {
    pub threadid: usize,
    pub seq: usize,
    pub deps: Vec<Rc<Thunk<'ctx>>>,
    pub node: Node<'ctx>,
    pub value: RefCell<Option<Value>>,
    pub ectx: EvalCtx,
}

#[derive(Debug)]
pub enum DecodeResult { Exit, Step, Stuck }

impl<'ctx> Thread<'ctx> {
    pub fn new(ctx: &'ctx Ctx<'ctx>, main: Symbol<'ctx>, threadid: usize, params: &[Value]) -> Self {
        let main = ctx.functions.get(&main).unwrap();
        let frames = vec![Frame {
            fun: main,
            origin: None,
            ip: InstrPtr {
                block: main.src.basic_blocks.first().unwrap(),
                index: 0,
                stuck: None,
            },
            temps: HashMap::new(),
            allocs: vec![],
            result: None,
        }];
        let thunks = BTreeMap::new();
        let mut thread = Thread { threadid, seq: 0, ctx, frames, thunks };
        for (name, value) in main.src.parameters.iter().zip(params.iter()) {
            thread.add_thunk(Some(&name.name),
                             Node::Value(value.clone()),
                             vec![]);
        }
        thread
    }
    fn top_mut(&mut self) -> &mut Frame<'ctx> {
        self.frames.last_mut().unwrap()
    }
    fn top(&self) -> &Frame<'ctx> {
        self.frames.last().unwrap()
    }
    pub fn decode(&mut self, memory: &Memory<'ctx>) -> bool {
        while self.thunks.len() < PIPELINE_SIZE {
            match self.decode_once(memory) {
                DecodeResult::Exit => return false,
                DecodeResult::Step => {}
                DecodeResult::Stuck => return true,
            }
        }
        true
    }
    pub fn decode_once(&mut self, memory: &Memory<'ctx>) -> DecodeResult {
        if self.frames.is_empty() {
            return DecodeResult::Exit;
        }
        let frame = self.frames.len() - 1;
        let index = self.frames[frame].ip.index;
        let block = self.frames[frame].ip.block;
        if let Some(deps) = self.frames[frame].ip.stuck.as_ref() {
            if deps.iter().any(|dep| dep.get().is_none()) {
                return DecodeResult::Stuck;
            }
        }
        if index < block.instrs.len() {
            self.decode_instr(memory, &block.instrs[index])
        } else {
            self.frames[frame].origin = Some(&block.name);
            self.decode_term(memory, &block.term);
            DecodeResult::Step
        }
    }
    fn decode_instr(&mut self, memory: &Memory<'ctx>, instr: &'ctx Instruction) -> DecodeResult {
        match instr {
            Instruction::Phi(phi) => {
                let (oper, _) =
                    phi.incoming_values.iter()
                        .find(|(_, name)| Some(name) == self.top().origin).unwrap();
                let deps = vec![self.get_temp(oper)];
                self.add_thunk(Some(&phi.dest), Node::Instr(instr), deps);
            }
            Instruction::Call(call) => {
                let frame = self.frames.len() - 1;
                if self.decode_call(memory, &call.function, &call.arguments, call.dest.as_ref()) {
                    self.frames[frame].ip.index += 1;
                    self.frames[frame].ip.stuck = None;
                }
                return DecodeResult::Step;
            }
            Instruction::Alloca(alloca) => {
                let thunk = self.add_thunk(Some(&alloca.dest), Node::Instr(instr), vec![]);
                self.top_mut().allocs.push(thunk);
            }
            Instruction::Store(store) => {
                let deps = vec![
                    self.get_temp(&store.address),
                    self.get_temp(&store.value)];
                self.add_thunk(None, Node::Instr(instr), deps);
            }
            Instruction::Load(load) => {
                let deps = vec![self.get_temp(&load.address)];
                self.add_thunk(Some(&load.dest), Node::Instr(instr), deps);
            }
            Instruction::Add(Add { dest, operand0, operand1, .. }) |
            Instruction::Sub(Sub { dest, operand0, operand1, .. }) |
            Instruction::Mul(Mul { dest, operand0, operand1, .. }) |
            Instruction::UDiv(UDiv { dest, operand0, operand1, .. }) |
            Instruction::SDiv(SDiv { dest, operand0, operand1, .. }) |
            Instruction::URem(URem { dest, operand0, operand1, .. }) |
            Instruction::SRem(SRem { dest, operand0, operand1, .. }) |
            Instruction::Xor(Xor { dest, operand0, operand1, .. }) |
            Instruction::And(And { dest, operand0, operand1, .. }) |
            Instruction::Or(Or { dest, operand0, operand1, .. }) |
            Instruction::Shl(Shl { dest, operand0, operand1, .. }) |
            Instruction::LShr(LShr { dest, operand0, operand1, .. }) |
            Instruction::AShr(AShr { dest, operand0, operand1, .. }) |
            Instruction::ICmp(ICmp { dest, operand0, operand1, .. })
            => {
                let deps = vec![self.get_temp(operand0), self.get_temp(operand1)];
                self.add_thunk(Some(dest), Node::Instr(instr), deps);
            }
            Instruction::SExt(SExt { dest, operand, .. }) |
            Instruction::ZExt(ZExt { dest, operand, .. }) |
            Instruction::Trunc(Trunc { dest, operand, .. }) |
            Instruction::PtrToInt(PtrToInt { dest, operand, .. }) |
            Instruction::IntToPtr(IntToPtr { dest, operand, .. }) |
            Instruction::BitCast(BitCast { dest, operand, .. })
            => {
                let deps = vec![self.get_temp(operand)];
                self.add_thunk(Some(dest), Node::Instr(instr), deps);
            }
            Instruction::GetElementPtr(get_element_ptr) => {
                let mut deps = vec![self.get_temp(&get_element_ptr.address)];
                for ind in get_element_ptr.indices.iter() {
                    deps.push(self.get_temp(ind));
                }
                self.add_thunk(Some(&get_element_ptr.dest), Node::Instr(instr), deps);
            }
            Instruction::InsertValue(InsertValue { aggregate, element, dest, .. }) => {
                let deps = vec![self.get_temp(aggregate), self.get_temp(element)];
                self.add_thunk(Some(dest), Node::Instr(instr), deps);
            }
            Instruction::AtomicRMW(AtomicRMW { address, value, dest, .. }) => {
                let deps = vec![self.get_temp(address), self.get_temp(value)];
                self.add_thunk(Some(dest), Node::Instr(instr), deps);
            }
            Instruction::Select(Select { condition, true_value, false_value, dest, .. }) => {
                let deps =
                    vec![self.get_temp(condition),
                         self.get_temp(true_value),
                         self.get_temp(false_value)];
                self.add_thunk(Some(dest), Node::Instr(instr), deps);
            }
            Instruction::ExtractValue(ExtractValue { aggregate, dest, .. }) => {
                let deps = vec![self.get_temp(aggregate)];
                self.add_thunk(Some(dest), Node::Instr(instr), deps);
            }
            Instruction::CmpXchg(CmpXchg { address, expected, replacement, dest, .. }) => {
                let deps =
                    vec![self.get_temp(address),
                         self.get_temp(expected),
                         self.get_temp(replacement)];
                self.add_thunk(Some(dest), Node::Instr(instr), deps);
            }
            Instruction::Fence(Fence { atomicity, .. }) => {
                //TODO
            }
            _ => todo!("{:?}", instr),
        }
        self.top_mut().ip.index += 1;
        self.top_mut().ip.stuck = None;
        DecodeResult::Step
    }
    fn jump(&mut self, dest: &'ctx Name) {
        let next = &self.top().fun.blocks.get(dest)
            .unwrap_or_else(|| panic!("Bad jump target {:?} in {:?}", dest, self.top().fun.src));
        self.top_mut().ip = InstrPtr { block: next, index: 0, stuck: None };
    }

    fn decode_term(&mut self, memory: &Memory<'ctx>, term: &'ctx Terminator) -> DecodeResult {
        match &term {
            Terminator::Br(br) => {
                self.jump(&br.dest);
                DecodeResult::Step
            }
            Terminator::Ret(ret) => {
                let result =
                    ret.return_operand.as_ref()
                        .map(|oper| { self.get_temp(oper) });
                let top = self.frames.pop().unwrap();
                let allocs = top.allocs;
                let mut deps = allocs.clone();
                deps.extend(result.iter().cloned());
                self.add_thunk(top.result, Node::Return(result, allocs), deps);
                DecodeResult::Step
            }
            Terminator::CondBr(condbr) => {
                if let Some(stuck) = self.top_mut().ip.stuck.as_ref() {
                    if let Some(result) = stuck[0].get() {
                        if result.unwrap_bool() {
                            self.jump(&condbr.true_dest);
                        } else {
                            self.jump(&condbr.false_dest);
                        }
                        DecodeResult::Step
                    } else {
                        DecodeResult::Stuck
                    }
                } else {
                    let cond = self.get_temp(&condbr.condition);
                    self.top_mut().ip.stuck = Some(vec![cond]);
                    DecodeResult::Step
                }
            }
            Terminator::Unreachable(_) => {
                panic!("Unreachable");
            }
            Terminator::Invoke(invoke) => {
                let next = &self.top().fun.blocks.get(&invoke.return_label)
                    .unwrap_or_else(|| panic!("Bad jump target {:?} in {:?}", invoke.return_label, self.top().fun.src));
                let frame = self.frames.len() - 1;
                if self.decode_call(memory, &invoke.function, &invoke.arguments, Some(&invoke.result)) {
                    self.frames[frame].ip = InstrPtr { block: next, index: 0, stuck: None };
                    DecodeResult::Step
                } else {
                    DecodeResult::Step
                }
            }
            Terminator::Switch(switch) => {
                if let Some(stuck) = self.top_mut().ip.stuck.as_ref() {
                    let val = stuck[0].get().unwrap();
                    let target =
                        stuck[1..].iter()
                            .zip(switch.dests.iter())
                            .find_map(|(pat, (_, target))| {
                                let pat = pat.get().unwrap();
                                (pat == val).then_some(target)
                            })
                            .unwrap_or(&switch.default_dest);
                    self.jump(target);
                    DecodeResult::Step
                } else {
                    let cond = self.get_temp(&switch.operand);
                    let deps =
                        iter::once(cond).chain(
                            switch.dests.iter().map(|(pat, _)|
                                self.add_thunk(None, Node::Constant(pat), vec![])))
                            .collect::<Vec<_>>();
                    self.top_mut().ip.stuck = Some(deps);
                    DecodeResult::Step
                }
            }
            term => todo!("{:?}", term)
        }
    }
    fn get_temp(&mut self, oper: &'ctx Operand) -> Rc<Thunk<'ctx>> {
        match oper {
            Operand::LocalOperand { name, .. } => {
                self.top().temps.get(name).unwrap_or_else(|| panic!("No variable named {:?} in \n{:#?}", name, self)).clone()
            }
            Operand::ConstantOperand(constant) => {
                self.add_thunk(None, Node::Constant(constant), vec![])
            }
            Operand::MetadataOperand => {
                self.add_thunk(None, Node::Metadata, vec![])
            }
        }
    }
    fn decode_call<'a>(&'a mut self,
                       memory: &'a Memory<'ctx>,
                       function: &'ctx Either<InlineAssembly, Operand>,
                       arguments: &'ctx Vec<(Operand, Vec<ParameterAttribute>)>,
                       dest: Option<&'ctx Name>,
    ) -> bool {
        if let Some(fun) = self.top_mut().ip.stuck.as_ref() {
            let fun = memory.reverse_lookup(fun[0].unwrap());
            if let Some(native) = self.ctx.native.get(&fun) {
                let deps =
                    arguments.iter()
                        .map(|(arg, _)| { self.get_temp(arg) })
                        .collect::<Vec<_>>();
                //println!("{:?}", deps);
                let result = self.add_thunk(dest, Node::Native(native), deps.clone());
            } else if let Some(compiled) = self.ctx.functions.get(&fun) {
                let temps =
                    compiled.src.parameters.iter().zip(arguments.iter())
                        .map(|(param, (arg, _))| {
                            (&param.name, self.get_temp(arg))
                        }).collect();
                self.frames.push(Frame {
                    fun: compiled,
                    origin: None,
                    ip: InstrPtr {
                        block: compiled.src.basic_blocks.first().unwrap(),
                        index: 0,
                        stuck: None,
                    },
                    temps,
                    allocs: vec![],
                    result: dest,
                });
            } else {
                panic!("Unknown function {:?}", fun);
            }
            return true;
        } else {
            let fun = match function {
                Either::Left(assembly) => todo!("{:?}", assembly),
                Either::Right(fun) => fun,
            };
            self.top_mut().ip.stuck = Some(vec![self.get_temp(fun)]);
            return false;
        }
    }
    fn add_thunk(&mut self, dest: Option<&'ctx Name>, node: Node<'ctx>, deps: Vec<Rc<Thunk<'ctx>>>) -> Rc<Thunk<'ctx>> {
        let seq = self.seq;
        self.seq += 1;
        let thunk = Rc::new(Thunk {
            threadid: self.threadid,
            seq,
            deps,
            node,
            value: RefCell::new(None),
            ectx: self.frames.last().map(|f| f.fun.ectx.clone()).unwrap_or(EvalCtx::default()),
        });
        self.thunks.insert(seq, thunk.clone());
        if let Some(dest) = dest {
            self.top_mut().temps.insert(dest, thunk.clone());
        }
        thunk
    }
}


impl<'ctx> Thunk<'ctx> {
    pub fn unwrap(&self) -> &Value {
        self.get().unwrap_or_else(|| panic!("Unwrapped incomplete {:?}", self))
    }
    pub fn set(&self, value: Value) {
        *self.value.borrow_mut() = Some(value);
    }
    pub fn get(&self) -> Option<&Value> {
        let r = self.value.borrow();
        if r.is_some() {
            Some(Ref::leak(r).as_ref().unwrap())
        } else {
            None
        }
    }
}


struct DebugFlat<T: Debug>(T);

impl<T: Debug> Debug for DebugFlat<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &self.0)
    }
}

impl<'ctx> Debug for Thread<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stack")
            .field("frames", &self.frames)
            .field("thunks", &self.thunks)
            .finish()
    }
}

impl<'ctx> Debug for Frame<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let temps =
            self.temps.iter()
                .map(|(k, v)| (format!("{}", k), v))
                .collect::<HashMap<_, _>>();
        f.debug_struct("Frame")
            .field("fun", &self.fun.src.name)
            .field("ip", &self.ip)
            .field("temps", &temps)
            .field("origin", &DebugFlat(&self.origin))
            .finish()
    }
}

impl<'ctx> Debug for InstrPtr<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let debug = if let Some(instr) = self.block.instrs.get(self.index) {
            instr.get_debug_loc()
        } else {
            self.block.term.get_debug_loc()
        };
        let debug = if let Some(debug) = debug {
            format!("{}", debug)
        } else {
            format!("")
        };
        f.debug_struct("InstrPtr")
            .field("block", &self.block.name)
            .field("index", &self.index)
            .field("stuck", &self.stuck)
            .field("??", &debug)
            .finish()
    }
}

struct DebugNode<'a>(&'a Node<'a>);

impl<'a> Debug for DebugNode<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

struct DebugDeps<'a>(&'a [Rc<Thunk<'a>>]);

impl<'a> Debug for DebugDeps<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.iter().map(|thunk| thunk.seq)).finish()
    }
}


impl<'ctx> Debug for Thunk<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{} = {:?} [{:?}] ({:?})", self.threadid, self.seq, self.get(), self.deps.iter().map(|dep| dep.seq).collect::<Vec<_>>(), self.node)
        // f.debug_struct("Thunk")
        //     .field("threadid", &self.threadid)
        //     .field("seq", &self.seq)
        //     .field("value", &self.value)
        //     .field("node", &DebugNode(&self.node))
        //     .field("deps", &DebugDeps(self.deps.as_slice()))
        //     .finish()
    }
}

impl<'ctx> Debug for Node<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Node::Instr(apply) => write!(f, "{}", apply),
            Node::Constant(constant) => write!(f, "{}", constant),
            Node::Value(value) => write!(f, "Value{{{:?}}}", value),
            Node::Metadata => write!(f, "Metadata"),
            Node::Return(ret, allocs) => write!(f, "Return{{{:?}}}", ret),
            Node::Native(native) => write!(f, "{:?}", native),
        }
    }
}