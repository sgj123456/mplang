use super::Compiler;
use crate::hir::HirType;
use cranelift::prelude::*;
use cranelift_codegen::ir::{MemFlagsData, StackSlotData, StackSlotKind};
use cranelift_module::Module;

impl<T: Module> Compiler<T> {
    /// 按值复制：对于 struct / array 分配新栈槽并拷贝；标量直接返回。
    pub fn copy_value(&mut self, builder: &mut FunctionBuilder, val: Value, ty: &HirType) -> Value {
        match ty {
            HirType::Named(_) | HirType::Array(_, _) => {
                let size = self.var_type_size(ty);
                if size == 0 {
                    return val;
                }
                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    size,
                    0,
                ));
                let ptr = builder.ins().stack_addr(self.ptr_type(), slot, 0);
                self.copy_value_to(builder, val, ty, ptr);
                ptr
            }
            _ => val,
        }
    }

    pub fn copy_value_to(
        &mut self,
        builder: &mut FunctionBuilder,
        val: Value,
        ty: &HirType,
        dst: Value,
    ) {
        match ty {
            HirType::Int | HirType::Char | HirType::Unit | HirType::Pointer(_) => {
                builder.ins().store(MemFlagsData::new(), val, dst, 0);
            }
            HirType::Array(elem, len) => {
                // 逐元素拷贝：聚合（Named/Array）元素递归、标量元素 load+store。
                let stride = self.var_type_size(elem) as i64;
                for i in 0..len.known() {
                    let off = stride * (i as i64);
                    let src_addr = builder.ins().iadd_imm(val, off);
                    let dst_addr = builder.ins().iadd_imm(dst, off);
                    match **elem {
                        HirType::Named(_) | HirType::Array(_, _) => {
                            self.copy_value_to(builder, src_addr, elem, dst_addr);
                        }
                        _ => {
                            let ir_ty = self.var_type_to_cranelift(elem);
                            let v = builder.ins().load(ir_ty, MemFlagsData::new(), src_addr, 0);
                            builder.ins().store(MemFlagsData::new(), v, dst_addr, 0);
                        }
                    }
                }
            }
            HirType::Named(struct_def) => {
                let layout = self.struct_map.get(struct_def).unwrap().clone();
                for field in &layout {
                    let src_addr = builder.ins().iadd_imm(val, field.offset as i64);
                    let dst_addr = builder.ins().iadd_imm(dst, field.offset as i64);
                    if matches!(field.field_type, HirType::Named(_) | HirType::Array(_, _)) {
                        self.copy_value_to(builder, src_addr, &field.field_type, dst_addr);
                    } else {
                        let ir_ty = self.var_type_to_cranelift(&field.field_type);
                        let v = builder.ins().load(ir_ty, MemFlagsData::new(), src_addr, 0);
                        builder.ins().store(MemFlagsData::new(), v, dst_addr, 0);
                    }
                }
            }
            // 以下变体只存在于泛型模板中，单态化后不会到达此处。
            HirType::Var(_) | HirType::Generic(_, _, _) => {
                unreachable!("单态化后不应出现泛型类型占位符")
            }
        }
    }
}
