use std::fmt::Debug;
use std::fmt;
use llvm_ir::{Module, Function, Name, BasicBlock, TypeRef, Type, Constant, constant};
use std::collections::HashMap;
use llvm_ir::types::NamedStructDef;
use crate::value::{Value, add_u64_i64};
use crate::layout::{Layout, AggrLayout};
use crate::memory::{Memory, Symbol};
use crate::native::NativeFunction;


pub struct Ctx<'ctx> {
    pub modules: &'ctx [Module],
    pub functions: HashMap<Symbol<'ctx>, CompiledFunction<'ctx>>,
    pub native: HashMap<Symbol<'ctx>, &'ctx NativeFunction>,
    pub ptr_bits: u64,
    pub page_size: u64,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct EvalCtx {
    pub module: Option<usize>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct ThreadCtx {
    pub ectx: EvalCtx,
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
    pub fn new(modules: &'ctx [Module], native: &'ctx [NativeFunction]) -> Ctx<'ctx> {
        let functions =
            modules.iter()
                .enumerate()
                .flat_map(|(mi, m)| m.functions.iter().map(move |f| (mi, f)))
                .map(|(mi, function)| {
                    (Symbol::new(function.linkage, mi, &function.name),
                     CompiledFunction::new(&EvalCtx { module: Some(mi) }, function))
                })
                .collect();
        let native = native.iter().map(|f| (Symbol::External(&f.name), f)).collect();
        Ctx { modules, functions, native, ptr_bits: 64, page_size: 4096 }
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
    // pub fn from_bytes(&self, bytes: &[u8], to_type: &TypeRef) -> Value {
    //     match &**to_type {
    //         Type::IntegerType { bits } => {
    //             self.int_from_bytes(bytes, *bits as u64)
    //         }
    //         Type::PointerType { pointee_type, addr_space } => {
    //             self.int_from_bytes(bytes, self.ptr_bits)
    //         }
    //         Type::ArrayType { element_type, num_elements } => {
    //             let len = self.layout(element_type).size();
    //             assert_eq!((len as usize) * *num_elements, bytes.len());
    //             Value::aggregate(
    //                 bytes
    //                     .chunks(len as usize)
    //                     .map(|b| self.from_bytes(b, element_type)),
    //                 false)
    //         }
    //         Type::StructType { element_types, is_packed } => todo!(),
    //         _ => todo!("{:?}", to_type),
    //     }
    // }
    // pub fn to_bytes(&self, value: &Value) -> (Layout, Vec<u8>) {
    //     match value {
    //         Value::Int { bits, value } => {
    //             (Layout::of_int(*bits), value.to_le_bytes()[0..(*bits as usize + 7) / 8].to_vec())
    //         }
    //         Value::Aggregate { children, is_packed } => {
    //             let mut result = vec![];
    //             let mut result_layout = Layout::from_size_align(0, 1);
    //             for child in children.iter() {
    //                 let (layout, value) = self.to_bytes(child);
    //                 let offset = result_layout.offset(*is_packed, layout);
    //                 result.resize(offset as usize, 0);
    //                 result.extend_from_slice(&value);
    //                 result_layout = result_layout.extend(*is_packed, layout);
    //             }
    //             (result_layout.pad_to_align(), result)
    //         }
    //     }
    // }
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

impl EvalCtx {

}