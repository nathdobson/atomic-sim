use crate::exec::Process;
use crate::ctx::EvalCtx;
use llvm_ir::{Constant, TypeRef, Name, constant};
use crate::value::{Value, add_u64_i64};

impl<'ctx> Process<'ctx> {
    pub fn get_constant(&self, ectx: EvalCtx, c: &'ctx Constant) -> (TypeRef, Value) {
        match c {
            Constant::Int { bits, value } => {
                (self.ctx.modules[0].types.int(*bits).clone(), Value::new(*bits as u64, *value as u128))
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
                (self.ctx.pointer_to(ty.clone()), self.memory.lookup(ectx, name).clone())
            }
            Constant::Undef(typ) => {
                (typ.clone(), self.ctx.aggregate_zero(typ))
            }
            Constant::Null(typ) => {
                (typ.clone(), self.ctx.aggregate_zero(typ))
            }
            Constant::Struct { name, values, is_packed, } => {
                let children = values.iter().map(|c| self.get_constant(ectx, c)).collect::<Vec<_>>();
                let ty = if let Some(name) = name {
                    self.ctx.type_of_struct(name).clone()
                } else {
                    self.ctx.struct_of(children.iter().map(|(ty, v)| ty.clone()).collect(), *is_packed)
                };
                (ty, Value::aggregate(children.iter().map(|(ty, v)| v.clone()), *is_packed))
            }
            Constant::Array { element_type, elements } => {
                (self.ctx.array_of(element_type.clone(), elements.len()),
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
                (self.ctx.pointer_to(ty.clone()), self.ctx.value_from_address(add_u64_i64(address.as_u64(), val)))
            }
            Constant::AggregateZero(typ) => {
                (typ.clone(), self.ctx.aggregate_zero(typ))
            }
            x => todo!("{:?}", x),
        }
    }
}