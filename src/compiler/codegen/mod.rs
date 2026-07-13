use cranelift::prelude::*;
use cranelift_codegen::ir::{MemFlagsData, StackSlotData, StackSlotKind};
use cranelift_frontend::{FunctionBuilder, Variable};
use cranelift_module::{Linkage, Module};

use super::{Compiler, FuncInfo, StructFieldLayout};
use crate::compiler::globals::const_init;
use crate::hir::HirType;
use crate::tyhir;

/// 局部变量/参数的存储描述。
/// - `var`：SSA 变量（非地址被取的标量/结构体用 `use_var`/`def_var`）。
/// - `slot`：栈槽（被 `&` 取过地址的标量/字符/指针变量用，读写都经栈槽）。
/// - `ty`：变量类型。
#[derive(Clone)]
pub(crate) struct LocalVar {
    pub(crate) var: Option<Variable>,
    pub(crate) slot: Option<cranelift_codegen::ir::StackSlot>,
    pub(crate) ty: HirType,
}

mod addr_taken;
mod expr;
mod function;
mod stmt;

pub(crate) use addr_taken::collect_addr_taken;

impl<T: Module> Compiler<T> {
    /// 入口：消费 TYPE HIR（TyHIR），生成所有函数 / 数据。
    pub fn translate(&mut self, prog: &tyhir::TyHirCompilationUnit) {
        let root = &prog.root_module;

        // Pass 1：全局变量 / 常量。
        Self::walk_items(&root.items, &mut |item| {
            if let tyhir::TyHirItem::Static {
                def_id,
                name,
                ty,
                init,
                ..
            } = item
            {
                let (bytes, init_ty) = const_init(init);
                assert_eq!(&init_ty, ty, "global `{}` 字面量类型与声明不符", name);
                self.create_global_var(*def_id, name, bytes, ty);
            }
        });

        // Pass 2：结构体布局。
        Self::walk_items(&root.items, &mut |item| {
            if let tyhir::TyHirItem::Struct { def_id, fields, .. } = item {
                let mut layout: Vec<StructFieldLayout> = Vec::new();
                let mut offset: u32 = 0;
                for f in fields {
                    let align = self.var_type_align(&f.ty);
                    offset = (offset + align - 1) & !(align - 1);
                    layout.push(StructFieldLayout {
                        field_def_id: f.def_id,
                        field_type: f.ty.clone(),
                        offset,
                    });
                    offset += self.var_type_size(&f.ty);
                }
                self.struct_map.insert(*def_id, layout);
            }
        });

        // Pass 3：外部函数声明。
        Self::walk_items(&root.items, &mut |item| {
            if let tyhir::TyHirItem::ExternFn {
                def_id,
                link_name,
                name,
                param_types,
                return_ty,
                is_variadic,
                ..
            } = item
            {
                let sig = self.build_signature(param_types, return_ty, None);
                // `link_name` 来自 `#[link_name = "..."]`；缺省时回退为 mplang 函数名。
                let linkage_name = link_name.clone().unwrap_or_else(|| name.clone());
                let func_id = self
                    .module
                    .declare_function(linkage_name.as_str(), Linkage::Import, &sig)
                    .unwrap_or_else(|e| panic!("failed to declare extern fn '{}': {}", name, e));
                if self.func_map.contains_key(def_id) {
                    return;
                }
                self.func_map.insert(
                    *def_id,
                    FuncInfo {
                        func_id,
                        name: name.clone(),
                        param_types: param_types.clone(),
                        ret_ty: return_ty.clone(),
                        is_variadic: *is_variadic,
                    },
                );
            }
        });

        // Pass 4：函数。先统一声明签名（支持前向调用），再逐个生成函数体。
        let mut fn_items: Vec<tyhir::TyHirItem> = Vec::new();
        Self::walk_items(&root.items, &mut |item| {
            if let tyhir::TyHirItem::Fn { .. } = item {
                fn_items.push(item.clone());
            }
        });
        for item in &fn_items {
            self.declare_function(item);
        }
        for item in &fn_items {
            self.define_function(item);
        }
    }

    /// 把「某类型的值」绑定到 [`LocalVar`] 的存储槽位。
    ///
    /// 被 `&` 取过地址的标量/字符/指针用栈槽存储（读写都经栈槽）；
    /// 否则用 SSA 变量。结构体 / 数组本就持有地址（指针值），永远走 SSA 变量。
    ///
    /// 供「函数参数」与「`let` 局部」两处共用，避免重复实现取地址栈槽逻辑。
    pub(crate) fn bind_local(
        &mut self,
        builder: &mut FunctionBuilder,
        ty: &HirType,
        val: Value,
        addr_taken: bool,
    ) -> LocalVar {
        let addr_taken = addr_taken && !matches!(ty, HirType::Named(_) | HirType::Array(_, _));
        if addr_taken {
            let size = self.var_type_size(ty);
            let align = self.var_type_align(ty) as u8;
            let slot = builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                size,
                align,
            ));
            let dst = builder.ins().stack_addr(self.ptr_type(), slot, 0);
            builder.ins().store(MemFlagsData::new(), val, dst, 0);
            LocalVar {
                var: None,
                slot: Some(slot),
                ty: ty.clone(),
            }
        } else {
            let clif_ty = self.var_type_to_cranelift(ty);
            let var = builder.declare_var(clif_ty);
            builder.def_var(var, val);
            LocalVar {
                var: Some(var),
                slot: None,
                ty: ty.clone(),
            }
        }
    }

    /// 递归遍历模块中的所有顶层 item（含子模块）。
    fn walk_items(items: &[tyhir::TyHirItem], f: &mut dyn FnMut(&tyhir::TyHirItem)) {
        for item in items {
            f(item);
            if let tyhir::TyHirItem::Module(m) = item {
                Self::walk_items(&m.items, f);
            }
        }
    }
}
