use super::Compiler;
use crate::ast::Literal;
use crate::hir::{DefId, HirType};
use crate::tyhir;
use cranelift_module::{Linkage, Module};

impl<T: Module> Compiler<T> {
    pub fn get_or_create_string_literal(&mut self, s: &str) -> cranelift_module::DataId {
        if let Some(&id) = self.string_pool.get(s) {
            return id;
        }

        let name = format!("__mplang_str_lit_{}", self.str_counter);
        self.str_counter += 1;

        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);

        let data_id = self
            .module
            .declare_data(&name, Linkage::Export, false, false)
            .expect("failed to declare string literal");
        let mut ctx = cranelift_module::DataDescription::new();
        ctx.define(bytes.into_boxed_slice());
        self.module
            .define_data(data_id, &ctx)
            .expect("failed to define string literal");

        self.string_pool.insert(s.to_string(), data_id);
        data_id
    }

    /// 声明一个全局变量 / 常量，并以定义 ID 为键登记。
    pub fn create_global_var<V: Into<Box<[u8]>>>(
        &mut self,
        def_id: DefId,
        _name: &str,
        init_value: V,
        ty: &HirType,
    ) -> cranelift_module::DataId {
        if let Some((data_id, existing_ty)) = self.data_map.get(&def_id) {
            self.assert_ty(existing_ty, ty, "global variable type mismatch");
            return *data_id;
        }

        let data_name = format!("__mplang_global_{}", def_id.0);
        let data_id = self
            .module
            .declare_data(&data_name, Linkage::Local, true, false)
            .expect("failed to declare global variable");

        let mut data_ctx = cranelift_module::DataDescription::new();
        data_ctx.define(init_value.into());

        self.module
            .define_data(data_id, &data_ctx)
            .expect("failed to define global variable");

        self.data_map.insert(def_id, (data_id, ty.clone()));
        data_id
    }
}

/// 计算全局变量 / 常量的初始化字节（仅支持字面量）。
pub(crate) fn const_init(init: &tyhir::TyHirExpr) -> (Vec<u8>, HirType) {
    match &init.kind {
        tyhir::TyHirExprKind::Literal(Literal::Int(n)) => (n.to_le_bytes().to_vec(), HirType::Int),
        tyhir::TyHirExprKind::Literal(Literal::String(s)) => {
            let mut bytes = s.as_bytes().to_vec();
            bytes.push(0);
            (bytes, HirType::char_ptr())
        }
        tyhir::TyHirExprKind::ArrayLiteral { elements, repeat } => {
            // 编译期常量数组：仅支持 int / char 元素（指针元素需要重定位，暂不支持）。
            let (elem_ty, elem_owned, len) = match &init.ty {
                HirType::Array(e, n) => (e.as_ref(), (*e).clone(), n.known()),
                _ => unreachable!("array literal type must be Array"),
            };
            let elem_size: u32 = match elem_ty {
                HirType::Int | HirType::Unit => 8,
                HirType::Char => 1,
                other => panic!(
                    "不支持的全局数组元素类型 {:?}：编译期常量数组仅支持 int / char 元素",
                    other
                ),
            };
            let total = (elem_size as usize) * len;
            let mut bytes = vec![0u8; total];
            let elems: Vec<&tyhir::TyHirExpr> = match repeat {
                Some(_) => std::iter::repeat_n(&elements[0], len).collect(),
                None => elements.iter().collect(),
            };
            let mut off = 0usize;
            for v in elems {
                match &v.kind {
                    tyhir::TyHirExprKind::Literal(Literal::Int(n)) => {
                        let b = n.to_le_bytes();
                        bytes[off..(elem_size as usize + off)]
                            .copy_from_slice(&b[..(elem_size as usize)]);
                    }
                    _ => panic!("全局数组的编译期常量元素必须是 int 字面量"),
                }
                off += elem_size as usize;
            }
            (
                bytes,
                HirType::Array(elem_owned, crate::hir::ArrayLen::Known(len)),
            )
        }
        _ => panic!("global variable initializer must be a literal"),
    }
}
