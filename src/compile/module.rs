use std::collections::HashMap;
use std::rc::Rc;

use itertools::Itertools;
use llvm_ir::{ConstantRef, Module, Name};
use llvm_ir::module::{Linkage, ThreadLocalMode};

use crate::compile::class::{Class, ClassKind, TypeMap, VectorClass};
use crate::compile::expr::{CExpr, ExprCompiler};
use crate::compile::function::COperand;
use crate::compile::function::FuncCompiler;
use crate::compile::operation::{COperation, COperationName, OperCompiler};
use crate::layout::Layout;
use crate::ordering::Ordering;
use crate::process::Process;
use crate::symbols::{Symbol, SymbolDef};
use crate::thread::ThreadId;
use crate::util::lazy::Lazy;
use crate::value::Value;

pub struct Compiler {
    process: Process,
    oper_compiler: Rc<OperCompiler>,
}

#[derive(Copy, Clone, Eq, Ord, PartialEq, PartialOrd, Hash, Debug)]
pub struct ModuleId(pub usize);

pub struct ModuleCompiler {
    moduleid: ModuleId,
    src: Module,
    process: Process,
    expr_compiler: Rc<ExprCompiler>,
}

impl ModuleCompiler {
    pub fn type_map(&self) -> TypeMap {
        self.process.types.clone()
    }
    pub fn compile_const(&self, cons: &ConstantRef) -> CExpr {
        self.expr_compiler.compile_const(cons)
    }
}

impl Compiler {
    pub fn new(process: Process) -> Self {
        Compiler {
            process: process.clone(),
            oper_compiler: Rc::new(OperCompiler::new(process.clone())),
        }
    }
    pub fn compile_modules(&mut self, modules: Vec<Module>) {
        let type_map = self.process.types.clone();
        for module in modules.iter() {
            type_map.add_module(module);
        }
        let modules = modules.into_iter().enumerate().map(|(index, module)| {
            let moduleid = ModuleId(index);
            Rc::new(ModuleCompiler {
                moduleid,
                src: module,
                process: self.process.clone(),
                expr_compiler: Rc::new(ExprCompiler::new(moduleid,
                                                         self.process.clone(),
                                                         self.oper_compiler.clone())),
            })
        }).collect::<Vec<_>>();
        let func_layout = Layout::of_func();
        let mut func_inits = vec![];
        let mut global_inits = vec![];
        for module in modules.iter() {
            for func in module.src.functions.iter() {
                let symbol = Symbol::new(func.linkage, module.moduleid, &func.name);
                let loc =
                    self.process.add_symbol(
                        symbol.clone(),
                        ThreadLocalMode::NotThreadLocal,
                        func_layout);
                let loc = loc.global().unwrap();
                func_inits.push((module, loc, func));
            }
            for g in module.src.global_vars.iter() {
                let initializer =
                    if let Some(initializer) = &g.initializer {
                        initializer
                    } else {
                        continue;
                    };
                let name = str_of_name(&g.name);
                let symbol = Symbol::new(g.linkage, module.moduleid, name);
                let mut layout = module.type_map().get(&g.ty).element(0).class.layout();
                layout = layout.align_to_bits(8 * g.alignment as u64);
                let def =
                    self.process.add_symbol(
                        symbol.clone(),
                        g.thread_local_mode,
                        layout);
                global_inits.push((module, def, layout, g));
            }
        }
        for module in modules.iter() {
            for g in module.src.global_aliases.iter() {
                let name = str_of_name(&g.name);
                let symbol = Symbol::new(g.linkage, module.moduleid, name);
                let target = module.compile_const(&g.aliasee);
                self.process.symbols.add_alias(symbol, target.as_const().unwrap().1.as_u64());
            }
        }
        for (module, loc, func) in func_inits {
            let module_compiler = module.clone();
            let type_map = module.type_map().clone();
            let func = func.clone();
            let process = self.process.clone();
            let expr_compiler = module.expr_compiler.clone();
            let oper_compiler = self.oper_compiler.clone();
            self.process.definitions.add_func(loc,
                                              Rc::new(Lazy::new(
                                                  move || FuncCompiler::new(process,
                                                                            expr_compiler,
                                                                            oper_compiler,
                                                  ).compile_func(loc, &func)
                                              )));
        }
        for (module, def, layout, global) in global_inits {
            let value = if let Some(init) = &global.initializer {
                let init = module.compile_const(&init);
                let (c, value) = init.as_const().unwrap();
                assert_eq!(c.layout().bits(), layout.bits());
                assert!(c.layout().bit_align() <= layout.bit_align());
                value.clone()
            } else {
                Value::zero(layout.bits())
            };
            match def {
                SymbolDef::Global(addr) =>
                    self.process.memory.store_impl(
                        ThreadId(0),
                        addr,
                        &value,
                        Ordering::None),
                SymbolDef::ThreadLocal(key) => {
                    self.process.definitions.add_thread_local_init(key, layout, value);
                }
            }
        }
    }
}

impl COperand {
    pub fn class(&self) -> &Class {
        match self {
            COperand::Constant(c, _) => c,
            COperand::Local(c, _) => c,
            COperand::Inline => todo!(),
            COperand::Expr(e) => e.class()
        }
    }
}

fn str_of_name(name: &Name) -> &str {
    match name {
        Name::Name(name) => &***name,
        Name::Number(_) => panic!(),
    }
}
