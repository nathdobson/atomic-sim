use llvm_ir::{Module, Instruction, Terminator, Function, Name, BasicBlock, Operand, ConstantRef, Constant};
use llvm_ir::instruction::{Call, InlineAssembly, Phi, Alloca, Store, Load, Xor, SExt, BitCast, InsertValue};
use std::collections::{HashMap, BTreeMap};
use std::rc::Rc;
use std::fmt::{Debug, Formatter};
use std::fmt;
use std::borrow::BorrowMut;
use either::Either;
use std::cell::Cell;
use llvm_ir::instruction::Add;
use llvm_ir::instruction::ICmp;
use llvm_ir::constant::Constant::Undef;

const PIPELINE_SIZE: usize = 10;

pub struct Ctx<'ctx> {
    pub module: &'ctx Module,
    functions: HashMap<&'ctx str, CompiledFunction<'ctx>>,
}

#[derive(Debug)]
pub struct CompiledFunction<'ctx> {
    src: &'ctx Function,
    blocks: HashMap<&'ctx Name, &'ctx BasicBlock>,
}

pub struct Thread<'ctx> {
    threadid: usize,
    seq: usize,
    ctx: &'ctx Ctx<'ctx>,
    frames: Vec<Frame<'ctx>>,
    pub thunks: BTreeMap<usize, Rc<Thunk<'ctx>>>,
}

enum InstrPtr<'ctx> {
    Start,
    Next(&'ctx BasicBlock, usize),
}

pub struct Frame<'ctx> {
    fun: &'ctx CompiledFunction<'ctx>,
    origin: Option<&'ctx Name>,
    ip: InstrPtr<'ctx>,
    temps: HashMap<&'ctx Name, Rc<Thunk<'ctx>>>,
    allocs: Vec<Rc<Thunk<'ctx>>>,
    result: Option<&'ctx Name>,
}

#[derive(Debug)]
pub enum Node<'ctx> {
    Store(&'ctx Store),
    Load(&'ctx Load),
    Phi(&'ctx Phi),
    Alloca(&'ctx Alloca),
    Apply(&'ctx Instruction),
    Constant(&'ctx Constant),
    Metadata,
    Return(Option<Rc<Thunk<'ctx>>>),
    Intrinsic(&'ctx str),
}

#[derive(Debug, Copy, Clone)]
pub struct Value { bits: u64, value: u128 }

pub struct Thunk<'ctx> {
    pub threadid: usize,
    pub seq: usize,
    pub deps: Vec<Rc<Thunk<'ctx>>>,
    pub node: Node<'ctx>,
    pub value: Cell<Option<Value>>,
}

#[derive(Debug)]
pub enum DecodeResult { Exit, Step, Stuck }

impl<'ctx> Thread<'ctx> {
    pub fn new(ctx: &'ctx Ctx<'ctx>, main: &str, threadid: usize) -> Self {
        let main = ctx.functions.get(main).unwrap();
        let frames = vec![Frame {
            fun: main,
            origin: None,
            ip: InstrPtr::Start,
            temps: HashMap::new(),
            allocs: vec![],
            result: None,
        }];
        let thunks = BTreeMap::new();

        Thread { threadid, seq: 0, ctx, frames, thunks }
    }
    fn top_mut(&mut self) -> &mut Frame<'ctx> {
        self.frames.last_mut().unwrap()
    }
    fn top(&self) -> &Frame<'ctx> {
        self.frames.last().unwrap()
    }
    pub fn decode(&mut self) -> bool {
        while self.thunks.len() < PIPELINE_SIZE {
            match self.decode_once() {
                DecodeResult::Exit => return false,
                DecodeResult::Step => {}
                DecodeResult::Stuck => return true,
            }
        }
        true
    }
    pub fn decode_once(&mut self) -> DecodeResult {
        if self.frames.is_empty() {
            return DecodeResult::Exit;
        }
        let frame = self.frames.len() - 1;
        let (block, ip) = match self.frames[frame].ip {
            InstrPtr::Start => {
                self.frames[frame].ip = InstrPtr::Next(self.frames[frame].fun.src.basic_blocks.first().unwrap(), 0);
                return DecodeResult::Step;
            }
            InstrPtr::Next(block, ip) => (block, ip)
        };
        if ip < block.instrs.len() {
            let result = self.decode_instr(&block.instrs[ip]);
            match result {
                DecodeResult::Exit => {
                    DecodeResult::Exit
                }
                DecodeResult::Step => {
                    self.frames[frame].ip = InstrPtr::Next(block, ip + 1);
                    DecodeResult::Step
                }
                DecodeResult::Stuck => unreachable!()
            }
        } else {
            self.frames[frame].origin = Some(&block.name);
            self.decode_term(&block.term)
        }
    }
    fn decode_instr(&mut self, instr: &'ctx Instruction) -> DecodeResult {
        println!("decoding {:?}", instr);
        match instr {
            Instruction::Phi(phi) => {
                let (oper, _) =
                    phi.incoming_values.iter()
                        .find(|(_, name)| Some(name) == self.top().origin).unwrap();
                let deps = vec![self.get_temp(oper)];
                self.add_thunk(Some(&phi.dest), Node::Phi(phi), deps);
            }
            Instruction::Call(call) => {
                match &call.function {
                    Either::Right(Operand::ConstantOperand(fun)) => {
                        match &**fun {
                            Constant::GlobalReference { name: Name::Name(fun), .. } => {
                                let fun = &***fun;
                                if fun == "llvm.dbg.declare"
                                    || fun == "_ZN4core5panic8Location6caller17h4a6732fb1c79b809E"
                                {
                                    let deps =
                                        call.arguments.iter()
                                            .map(|(arg, _)| { self.get_temp(arg) })
                                            .collect();
                                    self.add_thunk(call.dest.as_ref(), Node::Intrinsic(fun), deps);
                                } else if fun == "_ZN3std9panicking20rust_panic_with_hook17h4d446ca45c8e1faaE" {
                                    eprintln!("Panic!");
                                    return DecodeResult::Exit;
                                } else {
                                    let fun =
                                        self.ctx.functions.get(fun)
                                            .unwrap_or_else(|| panic!("Unknown function {:?}", fun));
                                    let temps =
                                        fun.src.parameters.iter().zip(call.arguments.iter())
                                            .map(|(param, (arg, _))| {
                                                (&param.name, self.get_temp(arg))
                                            }).collect();
                                    self.frames.push(Frame {
                                        fun,
                                        origin: None,
                                        ip: InstrPtr::Start,
                                        temps,
                                        allocs: vec![],
                                        result: call.dest.as_ref(),
                                    });
                                }
                            }
                            _ => todo!(),
                        }
                    }
                    _ => todo!()
                }
            }
            Instruction::Alloca(alloca) => {
                let thunk = self.add_thunk(Some(&alloca.dest), Node::Alloca(alloca), vec![]);
                self.top_mut().allocs.push(thunk);
            }
            Instruction::Store(store) => {
                let deps = vec![
                    self.get_temp(&store.address),
                    self.get_temp(&store.value)];
                self.add_thunk(None, Node::Store(store), deps);
            }
            Instruction::Load(load) => {
                let deps = vec![self.get_temp(&load.address)];
                self.add_thunk(Some(&load.dest), Node::Load(load), deps);
            }
            Instruction::Add(Add { dest, operand0, operand1, .. }) |
            Instruction::Xor(Xor { dest, operand0, operand1, .. }) |
            Instruction::ICmp(ICmp { dest, operand0, operand1, .. })
            => {
                let deps = vec![self.get_temp(operand0), self.get_temp(operand1)];
                self.add_thunk(Some(dest), Node::Apply(instr), deps);
            }
            Instruction::SExt(SExt { dest, operand, .. })
            => {
                let deps = vec![self.get_temp(operand)];
                self.add_thunk(Some(dest), Node::Apply(instr), deps);
            }
            Instruction::BitCast(BitCast { operand, dest, .. }) => {
                let deps = vec![self.get_temp(operand)];
                self.add_thunk(Some(dest), Node::Apply(instr), deps);
            }
            Instruction::GetElementPtr(get_element_ptr) => {
                let mut deps = vec![self.get_temp(&get_element_ptr.address)];
                for ind in get_element_ptr.indices.iter() {
                    deps.push(self.get_temp(ind));
                }
                self.add_thunk(Some(&get_element_ptr.dest), Node::Apply(instr), deps);
            }
            Instruction::InsertValue(InsertValue { aggregate, element, dest, .. }) => {
                let deps = vec![self.get_temp(aggregate), self.get_temp(element)];
                self.add_thunk(Some(dest), Node::Apply(instr), deps);
            }
            //Instruction::Undef(Undef {}) => {}
            instr => todo!("{:?}", instr),
        }
        DecodeResult::Step
    }
    fn jump(&mut self, dest: &'ctx Name) {
        let next = &self.top().fun.blocks[dest];
        self.top_mut().ip = InstrPtr::Next(next, 0);
    }
    fn decode_term(&mut self, term: &'ctx Terminator) -> DecodeResult {
        println!("Decoding term {:?}", term);
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
                let mut deps = top.allocs;
                deps.extend(result.iter().cloned());
                self.add_thunk(top.result, Node::Return(result), deps);
                DecodeResult::Step
            }
            Terminator::CondBr(condbr) => {
                if let Some(cond) = self.get_temp(&condbr.condition).get() {
                    if cond.as_bool() {
                        self.jump(&condbr.true_dest);
                    } else {
                        self.jump(&condbr.false_dest);
                    }
                    DecodeResult::Step
                } else {
                    return DecodeResult::Stuck;
                }
            }
            Terminator::Unreachable(unreachable) => {
                panic!("Unreachable");
                DecodeResult::Exit
            }
            term => todo!("{:?}", term)
        }
    }
    fn get_temp(&mut self, oper: &'ctx Operand) -> Rc<Thunk<'ctx>> {
        match oper {
            Operand::LocalOperand { name, .. } => {
                self.top().temps.get(name).unwrap_or_else(|| panic!("No variable named {:?}", name)).clone()
            }
            Operand::ConstantOperand(constant) => {
                self.add_thunk(None, Node::Constant(constant), vec![])
            }
            Operand::MetadataOperand => {
                self.add_thunk(None, Node::Metadata, vec![])
            }
        }
    }
    fn add_thunk(&mut self, dest: Option<&'ctx Name>, node: Node<'ctx>, deps: Vec<Rc<Thunk<'ctx>>>) -> Rc<Thunk<'ctx>> {
        let seq = self.seq;
        self.seq += 1;
        let thunk = Rc::new(Thunk {
            threadid: self.threadid,
            seq: seq,
            deps,
            node,
            value: Cell::new(None),
        });
        self.thunks.insert(seq, thunk.clone());
        if let Some(dest) = dest {
            self.top_mut().temps.insert(dest, thunk.clone());
        }
        thunk
    }
}

impl<'ctx> CompiledFunction<'ctx> {
    fn new(fun: &'ctx Function) -> Self {
        CompiledFunction {
            src: &fun,
            blocks: fun.basic_blocks.iter().map(|block| (&block.name, block)).collect(),
        }
    }
}

impl<'ctx> Ctx<'ctx> {
    pub fn new(module: &'ctx Module) -> Ctx<'ctx> {
        let functions = module.functions.iter()
            .map(|function| (function.name.as_str(), CompiledFunction::new(function)))
            .collect();
        Ctx { module, functions }
    }
}

impl<'ctx> Thunk<'ctx> {
    pub fn unwrap(&self) -> Value {
        self.value.get().unwrap_or_else(|| panic!("Unwrapped incomplete {:?}", self))
    }
    pub fn set(&self, value: Value) {
        self.value.set(Some(value));
    }
    pub fn get(&self) -> Option<Value> {
        self.value.get()
    }
}

impl Value {
    pub fn bits(self) -> u64 {
        self.bits
    }
    pub fn as_u64(self) -> u64 {
        self.value as u64
    }
    pub fn as_u128(self) -> u128 {
        self.value
    }
    pub fn as_usize(self) -> usize { self.value as usize }
    pub fn as_bool(self) -> bool {
        self.value != 0
    }
    pub fn as_bytes(self) -> [u8; 16] {
        self.value.to_le_bytes()
    }
    pub fn get_bool(self) -> bool {
        assert_eq!(self.bits, 1);
        self.as_bool()
    }
    pub fn new(bits: u64, value: u128) -> Self {
        Value { bits, value }
    }
    pub fn with_bytes(bits: u64, bytes: [u8; 16]) -> Self {
        Value { bits, value: u128::from_le_bytes(bytes) }
    }
}

impl From<()> for Value {
    fn from(value: ()) -> Self {
        Value { bits: 0, value: 0u128 }
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Value { bits: 64, value: value as u128 }
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Value { bits: 32, value: value as u128 }
    }
}

impl From<u128> for Value {
    fn from(value: u128) -> Self {
        Value { bits: 128, value: value }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value { bits: 1, value: if value { 1 } else { 0 } }
    }
}

struct DebugModule<'ctx>(&'ctx Module);

impl<'ctx> Debug for DebugModule<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Module")
            .field("name", &self.0.name)
            .finish()
    }
}

impl<'ctx> Debug for Ctx<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ctx")
            .field("module", &DebugModule(&self.module))
            .field("functions", &self.functions)
            .finish()
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
        f.debug_struct("Frame")
            .field("fun", &self.fun.src.name)
            .field("ip", &self.ip)
            .field("temps", &self.temps)
            .field("origin", &DebugFlat(&self.origin))
            .finish()
    }
}

impl<'ctx> Debug for InstrPtr<'ctx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            InstrPtr::Start => write!(f, "start"),
            InstrPtr::Next(block, ip) => write!(f, "{:?}:{}", block.name, ip),
        }
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
        f.debug_struct("Thunk")
            .field("threadid", &self.threadid)
            .field("seq", &self.seq)
            .field("value", &self.value)
            .field("node", &DebugNode(&self.node))
            .field("deps", &DebugDeps(self.deps.as_slice()))
            .finish()
    }
}