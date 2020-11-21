use crate::exec::Process;
use llvm_ir::{Operand, TypeRef, Type};
use crate::ctx::EvalCtx;
use crate::layout::AggrLayout;

impl<'ctx> Process<'ctx> {
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
                (pointee_type.clone(), index * (self.ctx.layout(&*pointee_type).bytes() as i64))
            }
            Type::StructType { element_types, is_packed } => {
                let layout = AggrLayout::new(*is_packed,
                                             element_types
                                                 .iter()
                                                 .map(|t| self.ctx.layout(t)));
                let offset_bits = layout.bit_offset(index as usize);
                assert!(offset_bits % 8 == 0);
                (element_types[index as usize].clone(), (offset_bits / 8) as i64)
            }
            Type::ArrayType { element_type, num_elements } => {
                (element_type.clone(), index * (self.ctx.layout(&element_type).bytes() as i64))
            }
            Type::NamedStructType { name } => {
                self.field(self.ctx.type_of_struct(name), index)
            }
            ty => todo!("{:?}", ty),
        }
    }
}