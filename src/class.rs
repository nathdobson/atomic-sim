use crate::layout::{Layout, AggrLayout, Packing};
use std::collections::HashMap;
use std::lazy::OnceCell;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::fmt::{Debug, Formatter};
use std::fmt;
use std::collections::hash_map::Entry;
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};
use llvm_ir::types::{FPType, Types, NamedStructDef};
use llvm_ir::constant::Constant::Int;
use llvm_ir::{TypeRef, Type, Module};
use crate::by_address::ByAddress;
use crate::lazy::{Lazy, RecursiveError};
use itertools::Itertools;

#[derive(Clone, Eq, PartialOrd, PartialEq, Ord, Hash, Debug)]
pub struct Element {
    pub class: Class,
    pub bit_offset: i64,
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct VoidClass;

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct IntegerClass { bits: u64 }

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct PointerClass {
    pub pointee: ClassWeak
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub struct FPClass {
    pub fptype: FPType
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct FuncClass {
    pub result: Class,
    pub params: Vec<Class>,
    pub is_var_arg: bool,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct VectorClass {
    pub element: Class,
    pub len: u64,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct ArrayClass {
    pub element: Class,
    pub len: u64,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct StructClass {
    pub fields: Vec<Element>,
    pub layout: Layout,
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub enum ClassKind {
    VoidClass(VoidClass),
    IntegerClass(IntegerClass),
    PointerClass(PointerClass),
    FPClass(FPClass),
    FuncClass(FuncClass),
    VectorClass(VectorClass),
    ArrayClass(ArrayClass),
    StructClass(StructClass),
}

struct ClassInner {
    id: usize,
    reentrant_debug: RefCell<()>,
    typ: TypeRef,
    kind: ClassKind,
    layout: Layout,
}

#[derive(Clone, Eq, Ord, PartialOrd, PartialEq, Hash)]
pub struct ClassWeak(ByAddress<Weak<Lazy<ClassInner>>>);

#[derive(Clone, Eq, Ord, PartialOrd, PartialEq, Hash)]
pub struct Class(ByAddress<Rc<Lazy<ClassInner>>>);

pub struct TypeMapInner {
    types: Types,
    table: HashMap<TypeRef, Class>,
    ptr_bits: u64,
    next_id: usize,
}

#[derive(Clone)]
pub struct TypeMap(Rc<RefCell<TypeMapInner>>);

impl ClassWeak {
    pub fn upgrade(&self) -> Class {
        Class(self.0.upgrade().unwrap())
    }
}

impl TypeMap {
    pub fn new(ptr_bits: u64) -> Self {
        TypeMap(Rc::new(RefCell::new(TypeMapInner {
            types: Types::blank_for_testing(),
            table: HashMap::new(),
            ptr_bits,
            next_id: 0,
        })))
    }
    pub fn add_module(&self, module: &Module) {
        for typename in module.types.all_struct_names() {
            match module.types.named_struct_def(typename).unwrap() {
                NamedStructDef::Opaque => todo!(),
                NamedStructDef::Defined(typ) => {
                    let c = self.get(typ);
                    self.0.borrow_mut().table.insert(module.types.named_struct(typename).unwrap(), c);
                }
            }
        }
    }
    fn get_inner(&self, typ: TypeRef) -> ClassInner {
        let (kind, layout) = match &*typ {
            Type::VoidType | Type::MetadataType =>
                (ClassKind::VoidClass(VoidClass),
                 Layout::from_bytes(0, 1)),
            Type::IntegerType { bits } =>
                (ClassKind::IntegerClass(IntegerClass { bits: *bits as u64 }),
                 Layout::of_int(*bits as u64), ),
            Type::PointerType { pointee_type, addr_space } =>
                (ClassKind::PointerClass(
                    PointerClass {
                        pointee: self.get(pointee_type).downgrade()
                    }),
                 Layout::of_int(self.0.borrow().ptr_bits)),
            Type::FPType(fptype) =>
                (ClassKind::FPClass(FPClass { fptype: *fptype }),
                 match *fptype {
                     FPType::Half => Layout::of_int(16),
                     FPType::Single => Layout::of_int(32),
                     FPType::Double => Layout::of_int(64),
                     FPType::FP128 => Layout::of_int(128),
                     FPType::X86_FP80 => Layout::from_bits(80, 128),
                     FPType::PPC_FP128 => Layout::of_int(128),
                 }),
            Type::FuncType { result_type, param_types, is_var_arg } =>
                (ClassKind::FuncClass(
                    FuncClass {
                        result: self.get(result_type),
                        params: param_types.iter().map(|ty| self.get(ty)).collect(),
                        is_var_arg: *is_var_arg,
                    }),
                 Layout::of_func()),
            Type::VectorType { element_type, num_elements } => {
                let element = self.get(element_type);
                let layout = element.layout().repeat(Packing::Bit, *num_elements as u64);
                (ClassKind::VectorClass(VectorClass { element, len: *num_elements as u64 }),
                 layout)
            }
            Type::ArrayType { element_type, num_elements } => {
                let element = self.get(element_type);
                let layout = element.layout().repeat(Packing::None, *num_elements as u64);
                (ClassKind::VectorClass(VectorClass { element, len: *num_elements as u64 }),
                 layout)
            }
            Type::StructType { element_types, is_packed } => {
                let mut fields = Vec::with_capacity(element_types.len());
                let mut layout = Layout::of_int(0);
                for ty in element_types.iter() {
                    let class = self.get(ty);
                    let (layout2, offset) =
                        layout.extend(Packing::from(*is_packed), class.layout());
                    fields.push(Element { class: class.clone(), bit_offset: offset });
                    layout = layout2;
                }
                layout = layout.pad_to_align();
                (ClassKind::StructClass(StructClass { fields, layout }),
                 layout)
            }
            _ => todo!("{:?}", typ),
        };
        let id = self.0.borrow_mut().next_id;
        self.0.borrow_mut().next_id += 1;
        ClassInner {
            id,
            reentrant_debug: RefCell::new(()),
            typ,
            kind,
            layout,
        }
    }
    pub fn get(&self, typ: &TypeRef) -> Class {
        let mut this = self.0.borrow_mut();
        if let Some(result) = this.table.get(typ) {
            return result.clone();
        }
        let typ2 = typ.clone();
        let self2 = self.clone();
        let result = Class(ByAddress(Rc::new(Lazy::new(move || self2.get_inner(typ2)))));
        this.table.insert(typ.clone(), result.clone());
        result
    }
    pub fn void(&self) -> Class {
        let ty = self.0.borrow().types.void();
        self.get(&ty)
    }
    pub fn bool(&self) -> Class {
        self.int(1)
    }
    pub fn int(&self, bits: u64) -> Class {
        let ty = self.0.borrow().types.int(bits as u32);
        self.get(&ty)
    }
    pub fn ptr(&self, pointee: Class) -> Class {
        let pointee_type = pointee.typ().clone();
        let ty = self.0.borrow().types.pointer_to(pointee_type);
        self.get(&ty)
    }
    pub fn size(&self) -> Class {
        let bits = self.0.borrow().ptr_bits;
        self.int(bits)
    }
    pub fn float(&self, fptype: FPType) -> Class {
        let ty = self.0.borrow().types.fp(fptype);
        self.get(&ty)
    }
    pub fn func(&self, result: Class, params: Vec<Class>, is_var_arg: bool) -> Class {
        let result_type = result.typ().clone();
        let param_types = params.iter().map(|c| c.typ().clone()).collect();
        let ty = self.0.borrow().types.func_type(result_type, param_types, is_var_arg);
        self.get(&ty)
    }
    pub fn vector(&self, element: Class, len: u64) -> Class {
        let element_type = element.typ().clone();
        let ty = self.0.borrow().types.vector_of(element_type, len as usize);
        self.get(&ty)
    }
    pub fn array(&self, element: Class, len: u64) -> Class {
        let element_type = element.typ().clone();
        let ty = self.0.borrow().types.array_of(element_type, len as usize);
        self.get(&ty)
    }
    pub fn struc(&self, elements: Vec<Class>, is_packed: bool) -> Class {
        let element_types = elements.iter().map(|e| e.typ().clone()).collect();
        let ty = self.0.borrow().types.struct_of(element_types, is_packed);
        self.get(&ty)
    }
}


impl Class {
    pub fn length(&self) -> u64 { todo!() }
    pub fn element_rec(&self, indices: &mut dyn Iterator<Item=i64>) -> Element {
        let mut bit_offset = 0;
        let mut class = self.clone();
        for index in indices {
            let elem = class.element(index);
            class = elem.class;
            bit_offset += elem.bit_offset;
        }
        Element { class, bit_offset }
    }
    pub fn element(&self, index: i64) -> Element {
        match self.kind() {
            ClassKind::VoidClass(_) => panic!(),
            ClassKind::IntegerClass(_) => panic!(),
            ClassKind::PointerClass(pointer) => {
                let pointee = pointer.pointee.upgrade();
                let bit_offset = pointee.layout().pad_to_align().bits() as i64 * index;
                Element { class: pointee, bit_offset }
            }
            ClassKind::FPClass(_) => panic!(),
            ClassKind::FuncClass(_) => panic!(),
            ClassKind::VectorClass(vector) => {
                assert_eq!(vector.element.layout().bit_align() % 8, 0);
                Element {
                    class: vector.element.clone(),
                    bit_offset: vector.element.layout().bits() as i64 * index,
                }
            }
            ClassKind::ArrayClass(array) =>
                Element {
                    class: array.element.clone(),
                    bit_offset: array.element.layout().pad_to_align().bits() as i64 * index,
                },
            ClassKind::StructClass(struc) => {
                struc.fields[index as usize].clone()
            }
        }
    }
    pub fn layout(&self) -> Layout {
        self.0.0.force().unwrap().layout
    }
    pub fn kind(&self) -> &ClassKind {
        &self.0.0.force().unwrap().kind
    }
    pub fn typ(&self) -> &TypeRef {
        &self.0.0.force().unwrap().typ
    }
    pub fn downgrade(&self) -> ClassWeak {
        ClassWeak(ByAddress::downgrade(&self.0))
    }
    pub fn as_vector(&self) -> Option<&VectorClass> {
        match self.kind() {
            ClassKind::VectorClass(v) => Some(v),
            _ => None
        }
    }
    pub fn target(&self) -> Class {
        self.element(-1).class
    }
}

impl Debug for ClassWeak {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0.0.upgrade())
    }
}

impl Debug for Class {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.0.0.force() {
            Ok(v) => write!(f, "{:?}", v),
            Err(e) => write!(f, "{:?}", e),
        }
    }
}

impl Debug for ClassInner {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut str = f.debug_struct("Class");
        str.field("id", &self.id);
        if let Ok(guard) = self.reentrant_debug.try_borrow_mut() {
            str.field("layout", &self.layout);
            str.field("kind", &self.kind);
        }
        str.finish()
    }
}

impl Debug for ClassKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ClassKind::VoidClass(c) =>
                write!(f, "{:?}", c),
            ClassKind::IntegerClass(c) =>
                write!(f, "{:?}", c),
            ClassKind::PointerClass(c) =>
                write!(f, "{:?}", c),
            ClassKind::FPClass(c) =>
                write!(f, "{:?}", c),
            ClassKind::FuncClass(c) =>
                write!(f, "{:?}", c),
            ClassKind::VectorClass(c) =>
                write!(f, "{:?}", c),
            ClassKind::ArrayClass(c) =>
                write!(f, "{:?}", c),
            ClassKind::StructClass(c) =>
                write!(f, "{:?}", c),
        }
    }
}

impl Debug for VoidClass {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "void")
    }
}

impl Debug for IntegerClass {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "i{}", self.bits)
    }
}

impl Debug for PointerClass {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Pointer({:?})", self.pointee)
    }
}

impl Debug for FPClass {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "f{}", self.fptype)
    }
}
