use std::collections::{HashMap, HashSet};

use cranelift::prelude::*;
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};

use super::LocalVar;
use super::collect_addr_taken;
use crate::compiler::{Compiler, FuncInfo};
use crate::hir::{DefId, HirType};
use crate::tyhir;

impl<T: Module> Compiler<T> {
    /// 声明函数签名（不含函数体），登记到 `func_map`。
    pub(crate) fn declare_function(&mut self, item: &tyhir::TyHirItem) {
        let (def_id, name, params, return_ty, impl_receiver) = match item {
            tyhir::TyHirItem::Fn {
                def_id,
                name,
                params,
                return_ty,
                impl_receiver,
                ..
            } => (def_id, name, params, return_ty, impl_receiver),
            _ => return,
        };
        if self.func_map.contains_key(def_id) {
            return;
        }
        // `impl` 方法的链接名加 DefId 后缀，避免不同接收者类型上的同名方法冲突。
        let link_name = if impl_receiver.is_some() {
            format!("{}_{}", name, def_id.0)
        } else {
            name.clone()
        };
        let param_types: Vec<HirType> = params.iter().map(|p| p.ty.clone()).collect();
        let sig = self.build_signature(&param_types, return_ty, None);
        let linkage = if name == "main" {
            Linkage::Export
        } else {
            Linkage::Local
        };
        let func_id = self
            .module
            .declare_function(&link_name, linkage, &sig)
            .expect("failed to declare function");
        self.func_map.insert(
            *def_id,
            FuncInfo {
                func_id,
                name: link_name,
                param_types,
                ret_ty: return_ty.clone(),
                is_variadic: false,
            },
        );
    }

    /// 生成已声明函数的函数体并定义。
    pub(crate) fn define_function(&mut self, item: &tyhir::TyHirItem) {
        let (def_id, name, params, _return_ty, body, _impl_receiver) = match item {
            tyhir::TyHirItem::Fn {
                def_id,
                name,
                params,
                return_ty,
                body,
                impl_receiver,
                ..
            } => (def_id, name, params, return_ty, body, impl_receiver),
            _ => return,
        };
        let info = self
            .func_map
            .get(def_id)
            .cloned()
            .unwrap_or_else(|| panic!("function '{}' not declared", name));
        let func_id = info.func_id;
        let sig = self.build_signature(&info.param_types, &info.ret_ty, None);

        // 收集被 `&` 取过地址的变量（含参数），这些变量需用栈槽存储。
        let mut addr_taken: HashSet<DefId> = HashSet::new();
        collect_addr_taken(&body.stmts, &mut addr_taken);

        let mut func_ctx = FunctionBuilderContext::new();
        let mut ctx = Context::new();
        ctx.func.signature = sig;
        ctx.func.name = cranelift_codegen::ir::UserFuncName::user(0, func_id.as_u32());

        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let mut var_map: HashMap<DefId, LocalVar> = HashMap::new();

            let block_params = builder.block_params(entry);
            let returns_struct = Self::needs_sret(&info.ret_ty);
            let sret_ptr = if returns_struct {
                Some(block_params[0])
            } else {
                None
            };
            let param_offset = if returns_struct { 1 } else { 0 };

            for (i, p) in params.iter().enumerate() {
                let val = builder.block_params(entry)[i + param_offset];
                // 被取地址的标量/字符/指针参数用栈槽存储；结构体 / 数组参数本就持有地址（指针值）。
                let local =
                    self.bind_local(&mut builder, &p.ty, val, addr_taken.contains(&p.def_id));
                var_map.insert(p.def_id, local);
            }

            let return_val = self.translate_stmts(
                &mut builder,
                &body.stmts,
                &mut var_map,
                &info.ret_ty,
                sret_ptr,
                &addr_taken,
            );

            if returns_struct {
                builder.ins().return_(&[]);
            } else {
                builder.ins().return_(&[return_val]);
            }
            builder.finalize();
        }

        log::debug!("{}", ctx.func.display());

        self.module
            .define_function(func_id, &mut ctx)
            .expect("failed to define function");
    }
}
