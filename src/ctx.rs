use std::fmt::Debug;
use std::fmt;
use llvm_ir::{Module, Function, Name, BasicBlock, TypeRef, Type, Constant, constant, Operand};
use std::collections::HashMap;
use llvm_ir::types::NamedStructDef;
use crate::value::{Value, add_u64_i64};
use crate::layout::{Layout, AggrLayout};
use crate::memory::{Memory};
use crate::native::NativeFun;
use crate::symbols::{SymbolTable, Symbol};


pub struct Ctx<'ctx> {
    pub modules: &'ctx [Module],
    pub functions: HashMap<Symbol<'ctx>, CompiledFunction<'ctx>>,
    pub native: HashMap<Symbol<'ctx>, &'ctx dyn NativeFun>,
    pub ptr_bits: u64,
    pub page_size: u64,
    symbols: SymbolTable<'ctx>,
}


#[derive(Copy, Clone, Debug, Default)]
pub struct EvalCtx {
    pub module: Option<usize>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct ThreadCtx {
    pub threadid: usize,
}

#[derive(Debug)]
pub struct CompiledFunction<'ctx> {
    pub src: &'ctx Function,
    pub blocks: HashMap<&'ctx Name, &'ctx BasicBlock>,
    pub ectx: EvalCtx,
}

impl<'ctx> CompiledFunction<'ctx> {
    fn new(ectx: &EvalCtx, fun: &'ctx Function) -> Self {
        CompiledFunction {
            src: &fun,
            blocks: fun.basic_blocks.iter().map(|block| (&block.name, block)).collect(),
            ectx: ectx.clone(),
        }
    }
}


impl<'ctx> Ctx<'ctx> {
    pub fn new(modules: &'ctx [Module], native: &'ctx [Box<dyn NativeFun>], memory: &mut Memory<'ctx>) -> Ctx<'ctx> {
        let functions =
            modules.iter()
                .enumerate()
                .flat_map(|(mi, m)| m.functions.iter().map(move |f| (mi, f)))
                .map(|(mi, function)| {
                    (Symbol::new(function.linkage, mi, &function.name),
                     CompiledFunction::new(&EvalCtx { module: Some(mi) }, function))
                })
                .collect();
        let native = native.iter().map(|f| (Symbol::External(f.name()), &**f)).collect();

        let mut ctx = Ctx {
            modules,
            functions,
            native,
            ptr_bits: 64,
            page_size: 4096,
            symbols: SymbolTable::new(),
        };
        ctx.initialize_globals(memory);
        ctx
    }
    pub fn named_struct_def(&self, x: &str) -> Option<&'ctx NamedStructDef> {
        self.modules.iter().find_map(|m| m.types.named_struct_def(x))
    }
    pub fn struct_of(&self, fields: Vec<TypeRef>, is_packed: bool) -> TypeRef {
        self.modules[0].types.struct_of(fields, is_packed)
    }
    pub fn array_of(&self, elem: TypeRef, num_elements: usize) -> TypeRef {
        self.modules[0].types.array_of(elem, num_elements)
    }
    pub fn int(&self, bits: u64) -> TypeRef {
        self.modules[0].types.int(bits as u32)
    }
    pub fn pointer_to(&self, ty: TypeRef) -> TypeRef {
        self.modules[0].types.pointer_to(ty)
    }
    pub fn value_from_address(&self, x: u64) -> Value {
        match self.ptr_bits {
            32 => assert!(x <= u32::MAX as u64),
            64 => assert!(x <= u64::MAX),
            _ => todo!(),
        }
        Value::from(x)
    }
    pub fn aggregate_zero(&self, ty: &TypeRef) -> Value {
        match &**ty {
            Type::ArrayType { element_type, num_elements } => {
                let zero = self.aggregate_zero(element_type);
                Value::aggregate((0..*num_elements).map(|_| zero.clone()), false)
            }
            Type::StructType { element_types, is_packed } => {
                Value::aggregate(element_types.iter().map(|ty| self.aggregate_zero(ty)), *is_packed)
            }
            Type::IntegerType { bits } => {
                Value::new(*bits as u64, 0)
            }
            Type::PointerType { pointee_type, addr_space } => {
                Value::new(self.ptr_bits, 0)
            }
            _ => todo!("{:?}", ty)
        }
    }
    pub fn int_from_bytes(&self, bytes: &[u8], bits: u64) -> Value {
        assert_eq!(((bits + 7) / 8) as usize, bytes.len());
        let mut array = [0u8; 16];
        array[..bytes.len()].copy_from_slice(bytes);
        Value::new(bits, u128::from_le_bytes(array))
    }
    pub fn layout_of_ptr(&self) -> Layout {
        Layout::from_bits(self.ptr_bits, self.ptr_bits)
    }
    pub fn layout(&self, typ: &TypeRef) -> Layout {
        match &**typ {
            Type::IntegerType { bits } => Layout::of_int(*bits as u64),
            Type::PointerType { .. } => self.layout_of_ptr(),
            Type::StructType { element_types, is_packed } => {
                AggrLayout::new(*is_packed, element_types.iter().map(|t| self.layout(t))).layout()
            }
            Type::VectorType { element_type, num_elements }
            | Type::ArrayType { element_type, num_elements } => {
                let layout = self.layout(&*element_type).pad_to_align();
                Layout::from_bits(layout.bits() * (*num_elements as u64), layout.bit_align())
            }
            Type::NamedStructType { name } => {
                self.layout(self.type_of_struct(name))
            }
            _ => todo!("{:?}", typ),
        }
    }
    pub fn type_of_struct(&self, name: &str) -> &TypeRef {
        match self.named_struct_def(name)
            .unwrap_or_else(|| panic!("Unknown struct {}", name)) {
            NamedStructDef::Opaque => panic!("Cannot layout opaque {}", name),
            NamedStructDef::Defined(ty) => {
                ty
            }
        }
    }

    fn initialize_globals(&mut self, memory: &mut Memory<'ctx>) {
        let func_layout = Layout::from_bytes(256, 1);
        for (name, _) in self.native.iter() {
            self.symbols.add_symbol(memory, *name, func_layout);
        }
        let mut inits = vec![];
        for (mi, module) in self.modules.iter().enumerate() {
            for f in module.functions.iter() {
                let loc = self.symbols.add_symbol(memory, Symbol::new(f.linkage, mi, &f.name), func_layout);
            }
            for g in module.global_vars.iter() {
                let symbol = Symbol::new(g.linkage, mi, str_of_name(&g.name));
                let mut layout = self.layout(&self.target_of(&g.ty));
                layout = Layout::from_bits(layout.bits(), layout.bit_align().max(8 * g.alignment as u64));
                let loc = self.symbols.add_symbol(memory, symbol, layout);
                inits.push((mi, symbol, layout, g, loc));
            }
        }
        for (mi, symbol, layout, g, loc) in inits {
            let (ty, value) =
                self.get_constant(EvalCtx { module: Some(mi) }, g.initializer.as_ref().unwrap());
            assert_eq!(ty, self.target_of(&g.ty), "{} == {}", ty, g.ty);
            memory.store(&loc, &value, None);
        }
        for (mi, module) in self.modules.iter().enumerate() {
            for g in module.global_vars.iter() {
                let symbol = Symbol::new(g.linkage, mi, str_of_name(&g.name));
            }
        }
    }


    pub fn get_constant(&self, ectx: EvalCtx, c: &'ctx Constant) -> (TypeRef, Value) {
        match c {
            Constant::Int { bits, value } => {
                (self.modules[0].types.int(*bits).clone(), Value::new(*bits as u64, *value as u128))
            }
            Constant::BitCast(bitcast) => {
                let (_, value) = self.get_constant(ectx, &*bitcast.operand);
                (bitcast.to_type.clone(), value)
            }
            Constant::IntToPtr(inttoptr) => {
                let (_, value) = self.get_constant(ectx, &*inttoptr.operand);
                (inttoptr.to_type.clone(), value)
            }
            Constant::PtrToInt(ptrtoint) => {
                let (_, value) = self.get_constant(ectx, &*ptrtoint.operand);
                (ptrtoint.to_type.clone(), value)
            }
            Constant::GlobalReference { name, ty } => {
                let name = match name {
                    Name::Name(name) => name,
                    Name::Number(_) => panic!(),
                };
                (self.pointer_to(ty.clone()), self.lookup(ectx, name).clone())
            }
            Constant::Undef(typ) => {
                (typ.clone(), self.aggregate_zero(typ))
            }
            Constant::Null(typ) => {
                (typ.clone(), self.aggregate_zero(typ))
            }
            Constant::Struct { name, values, is_packed, } => {
                let children = values.iter().map(|c| self.get_constant(ectx, c)).collect::<Vec<_>>();
                let ty = if let Some(name) = name {
                    self.type_of_struct(name).clone()
                } else {
                    self.struct_of(children.iter().map(|(ty, v)| ty.clone()).collect(), *is_packed)
                };
                (ty, Value::aggregate(children.iter().map(|(ty, v)| v.clone()), *is_packed))
            }
            Constant::Array { element_type, elements } => {
                (self.array_of(element_type.clone(), elements.len()),
                 Value::aggregate(elements.iter().map(|c| {
                     let (ty, v) = self.get_constant(ectx, c);
                     assert_eq!(ty, *element_type);
                     v
                 }), false))
            }
            Constant::GetElementPtr(constant::GetElementPtr { address, indices, in_bounds }) => {
                let (ty, address) = self.get_constant(ectx, address);
                let indices = indices.iter().map(|c| self.get_constant(ectx, c).1.as_i64()).collect::<Vec<_>>();
                let (ty, val) = self.offset_of(&ty, indices.iter().cloned());
                (self.pointer_to(ty.clone()), self.value_from_address(add_u64_i64(address.as_u64(), val)))
            }
            Constant::AggregateZero(typ) => {
                (typ.clone(), self.aggregate_zero(typ))
            }
            x => todo!("{:?}", x),
        }
    }
    pub fn target_of(&self, ty: &TypeRef) -> TypeRef {
        match &**ty {
            Type::PointerType { pointee_type, .. } => pointee_type.clone(),
            _ => todo!("{:?}", ty),
        }
    }
    pub fn type_of(&self, ectx: EvalCtx, oper: &'ctx Operand) -> TypeRef {
        match oper {
            Operand::LocalOperand { ty, .. } => {
                ty.clone()
            }
            Operand::ConstantOperand(c) => {
                self.get_constant(ectx, c).0
            }
            x => todo!("{:?}", x),
        }
    }
    pub fn offset_of(&self, ty: &'ctx TypeRef, vs: impl Iterator<Item=i64>) -> (TypeRef, i64) {
        let mut offset = 0i64;
        let mut ty = ty.clone();
        for i in vs {
            let (ty2, offset2) = self.field(&ty, i);
            offset += offset2;
            ty = ty2;
        }
        (ty, offset)
    }
    pub fn field(&self, ty: &'ctx Type, index: i64) -> (TypeRef, i64) {
        match ty {
            Type::PointerType { pointee_type, .. } => {
                (pointee_type.clone(), index * (self.layout(&*pointee_type).bytes() as i64))
            }
            Type::StructType { element_types, is_packed } => {
                let layout = AggrLayout::new(*is_packed,
                                             element_types
                                                 .iter()
                                                 .map(|t| self.layout(t)));
                let offset_bits = layout.bit_offset(index as usize);
                assert!(offset_bits % 8 == 0);
                (element_types[index as usize].clone(), (offset_bits / 8) as i64)
            }
            Type::ArrayType { element_type, num_elements } => {
                (element_type.clone(), index * (self.layout(&element_type).bytes() as i64))
            }
            Type::NamedStructType { name } => {
                self.field(self.type_of_struct(name), index)
            }
            ty => todo!("{:?}", ty),
        }
    }
    pub fn reverse_lookup(&self, address: &Value) -> Symbol<'ctx> {
        self.symbols.reverse_lookup(address)
    }
    pub fn lookup(&self, ectx: EvalCtx, name: &'ctx str) -> &Value {
        self.symbols.lookup(ectx, name)
    }
}


struct DebugModule<'ctx>(&'ctx [Module]);

impl<'ctx> Debug for DebugModule<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Module")
            .field("names", &self.0.iter().map(|x| &x.name).collect::<Vec<_>>())
            .finish()
    }
}

impl<'ctx> Debug for Ctx<'ctx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ctx")
            .field("module", &DebugModule(&self.modules))
            .field("functions", &self.functions)
            .finish()
    }
}

fn str_of_name(name: &Name) -> &str {
    match name {
        Name::Name(name) => &***name,
        Name::Number(_) => panic!(),
    }
}