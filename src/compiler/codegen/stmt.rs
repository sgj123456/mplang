use std::collections::{HashMap, HashSet};

use cranelift::prelude::*;
use cranelift_codegen::ir::{BlockArg, MemFlagsData};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;

use super::LocalVar;
use crate::compiler::Compiler;
use crate::hir::{DefId, HirType};
use crate::tyhir;

impl<T: Module> Compiler<T> {
    pub(crate) fn translate_stmts(
        &mut self,
        builder: &mut FunctionBuilder,
        stmts: &[tyhir::TyHirStmt],
        var_map: &mut HashMap<DefId, LocalVar>,
        current_ret_ty: &HirType,
        sret_ptr: Option<Value>,
        addr_taken: &HashSet<DefId>,
    ) -> Value {
        let ptr_ty = self.ptr_type();
        let mut last = builder.ins().iconst(ptr_ty, 0);

        for stmt in stmts {
            match stmt {
                tyhir::TyHirStmt::Let {
                    def_id, ty, init, ..
                } => {
                    let var_ty = ty.clone();
                    let raw_val = self.translate_expr(builder, init, var_map, &var_ty);
                    let val = self.copy_value(builder, raw_val, &var_ty);

                    // 被取地址的标量/字符/指针局部用栈槽存储；结构体 / 数组局部本就持有地址（指针值）。
                    let local = self.bind_local(builder, &var_ty, val, addr_taken.contains(def_id));
                    var_map.insert(*def_id, local);
                    last = raw_val;
                }

                tyhir::TyHirStmt::Assign { target, value } => {
                    match &target.kind {
                        tyhir::TyHirExprKind::Path(def_id) => {
                            if let Some(info) = var_map.get(def_id) {
                                let info = info.clone();
                                let raw_val =
                                    self.translate_expr(builder, value, var_map, &info.ty);
                                if let Some(slot) = info.slot {
                                    let dst = builder.ins().stack_addr(self.ptr_type(), slot, 0);
                                    builder.ins().store(MemFlagsData::new(), raw_val, dst, 0);
                                    last = raw_val;
                                } else if matches!(
                                    info.ty,
                                    HirType::Named(_) | HirType::Array(_, _)
                                ) {
                                    let dst_ptr = builder.use_var(info.var.unwrap());
                                    self.copy_value_to(builder, raw_val, &info.ty, dst_ptr);
                                    last = dst_ptr;
                                } else {
                                    let val = self.copy_value(builder, raw_val, &info.ty);
                                    builder.def_var(info.var.unwrap(), val);
                                    last = val;
                                }
                            } else {
                                // 全局变量（static/const）：取其符号地址并写入。
                                let (data_id, ty) = match self.data_map.get(def_id) {
                                    Some((d, t)) => (*d, t.clone()),
                                    None => panic!("undefined variable: {:?}", def_id),
                                };
                                let gv = self.module.declare_data_in_func(data_id, builder.func);
                                let dst = builder.ins().symbol_value(self.ptr_type(), gv);
                                let val = self.translate_expr(builder, value, var_map, &ty);
                                self.copy_value_to(builder, val, &ty, dst);
                                last = val;
                            }
                        }
                        // 字段 / 下标 / 解引用：统一取左值地址，再把右值写入该地址。
                        // 借 `translate_lvalue` 复用与「读」完全一致的地址计算，消除三处重复。
                        _ => {
                            let addr = self.translate_lvalue(builder, target, var_map);
                            let val = self.translate_expr(builder, value, var_map, &target.ty);
                            self.copy_value_to(builder, val, &target.ty, addr);
                            last = addr;
                        }
                    }
                }

                tyhir::TyHirStmt::Expr(expr) => {
                    last = self.translate_expr(builder, expr, var_map, &expr.ty);
                }

                tyhir::TyHirStmt::If {
                    cond,
                    then_branch,
                    else_branch,
                } => {
                    let saved_var_map = var_map.clone();

                    let cond_val = self.translate_expr(builder, cond, var_map, &HirType::Int);
                    let zero = builder.ins().iconst(types::I64, 0);
                    let cmp = builder.ins().icmp(IntCC::NotEqual, cond_val, zero);
                    let then_block = builder.create_block();
                    let else_block = builder.create_block();
                    let merge_block = builder.create_block();
                    builder.append_block_param(merge_block, ptr_ty);

                    builder.ins().brif(cmp, then_block, &[], else_block, &[]);

                    builder.switch_to_block(then_block);
                    let then_val = self.translate_stmts(
                        builder,
                        &then_branch.stmts,
                        var_map,
                        current_ret_ty,
                        sret_ptr,
                        addr_taken,
                    );
                    builder
                        .ins()
                        .jump(merge_block, &[BlockArg::Value(then_val)]);

                    *var_map = saved_var_map.clone();

                    builder.switch_to_block(else_block);
                    let else_val = if let Some(eb) = else_branch {
                        self.translate_stmts(
                            builder,
                            &eb.stmts,
                            var_map,
                            current_ret_ty,
                            sret_ptr,
                            addr_taken,
                        )
                    } else {
                        builder.ins().iconst(ptr_ty, 0)
                    };
                    builder
                        .ins()
                        .jump(merge_block, &[BlockArg::Value(else_val)]);

                    builder.switch_to_block(merge_block);
                    builder.seal_block(then_block);
                    builder.seal_block(else_block);
                    builder.seal_block(merge_block);

                    *var_map = saved_var_map;
                    last = builder.block_params(merge_block)[0];
                }

                tyhir::TyHirStmt::While { cond, body } => {
                    let saved_var_map = var_map.clone();

                    let header = builder.create_block();
                    let body_block = builder.create_block();
                    let exit = builder.create_block();

                    builder.ins().jump(header, &[]);
                    builder.switch_to_block(header);

                    let cond_val = self.translate_expr(builder, cond, var_map, &HirType::Int);
                    let zero = builder.ins().iconst(types::I64, 0);
                    let cmp = builder.ins().icmp(IntCC::NotEqual, cond_val, zero);
                    builder.ins().brif(cmp, body_block, &[], exit, &[]);

                    builder.switch_to_block(body_block);
                    self.translate_stmts(
                        builder,
                        &body.stmts,
                        var_map,
                        current_ret_ty,
                        sret_ptr,
                        addr_taken,
                    );
                    builder.ins().jump(header, &[]);

                    builder.seal_block(header);
                    builder.seal_block(body_block);

                    builder.switch_to_block(exit);
                    builder.seal_block(exit);

                    *var_map = saved_var_map;
                    last = builder.ins().iconst(ptr_ty, 0);
                }

                tyhir::TyHirStmt::Return(expr) => {
                    if let Some(sret) = sret_ptr {
                        if let Some(e) = expr {
                            let ret_ty = current_ret_ty.clone();
                            let val = self.translate_expr(builder, e, var_map, &ret_ty);
                            self.copy_value_to(builder, val, &ret_ty, sret);
                        }
                        builder.ins().return_(&[]);
                    } else {
                        let val = if let Some(e) = expr {
                            let ret_ty = current_ret_ty.clone();
                            self.translate_expr(builder, e, var_map, &ret_ty)
                        } else {
                            builder.ins().iconst(ptr_ty, 0)
                        };
                        builder.ins().return_(&[val]);
                    }

                    let dead = builder.create_block();
                    builder.switch_to_block(dead);
                    builder.seal_block(dead);
                    last = builder.ins().iconst(ptr_ty, 0);
                }
            }
        }
        last
    }
}
