use std::collections::HashMap;

use cranelift::prelude::*;
use cranelift_codegen::ir::{BlockArg, ExtFuncData, MemFlagsData, StackSlotData, StackSlotKind};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;

use super::LocalVar;
use crate::ast::{BinOp, Literal};
use crate::compiler::{Compiler, FuncInfo};
use crate::hir::{DefId, HirType};
use crate::tyhir;

impl<T: Module> Compiler<T> {
    pub(crate) fn translate_expr(
        &mut self,
        builder: &mut FunctionBuilder,
        expr: &tyhir::TyHirExpr,
        var_map: &HashMap<DefId, LocalVar>,
        expected_ty: &HirType,
    ) -> Value {
        let ptr_ty = self.ptr_type();

        match &expr.kind {
            tyhir::TyHirExprKind::Literal(l) => match l {
                Literal::Int(n) => {
                    self.assert_ty(&HirType::Int, expected_ty, "int literal");
                    builder.ins().iconst(types::I64, *n)
                }
                Literal::String(s) => {
                    self.assert_ty(&HirType::char_ptr(), expected_ty, "string literal");
                    let data_id = self.get_or_create_string_literal(s);
                    let global_val = self.module.declare_data_in_func(data_id, builder.func);
                    builder.ins().symbol_value(ptr_ty, global_val)
                }
            },

            tyhir::TyHirExprKind::Path(def_id) => {
                if let Some(info) = var_map.get(def_id) {
                    self.assert_ty(&info.ty, expected_ty, &format!("identifier {:?}", def_id));
                    // 被取地址的标量/字符/指针变量经栈槽读写。
                    if let Some(slot) = info.slot {
                        let ptr = builder.ins().stack_addr(self.ptr_type(), slot, 0);
                        return builder.ins().load(
                            self.var_type_to_cranelift(&info.ty),
                            MemFlagsData::new(),
                            ptr,
                            0,
                        );
                    }
                    return builder.use_var(info.var.unwrap());
                }
                if let Some((data_id, ty)) = self.data_map.get(def_id) {
                    self.assert_ty(ty, expected_ty, &format!("identifier {:?}", def_id));
                    let global_val = self.module.declare_data_in_func(*data_id, builder.func);
                    let addr = builder.ins().symbol_value(ptr_ty, global_val);
                    return match ty {
                        // 指针、结构体与数组以“地址”作为值直接返回。
                        HirType::Pointer(_) | HirType::Named(_) | HirType::Array(_, _) => addr,
                        HirType::Unit => builder.ins().iconst(types::I64, 0),
                        HirType::Int | HirType::Char => builder.ins().load(
                            self.var_type_to_cranelift(ty),
                            MemFlagsData::new(),
                            addr,
                            0,
                        ),
                        // 以下变体只存在于泛型模板中，单态化后不会到达此处。
                        HirType::Var(_) | HirType::Generic(_, _, _) => {
                            unreachable!("单态化后不应出现泛型类型占位符")
                        }
                        HirType::Enum(_, _, _) | HirType::TraitObject(_) => addr,
                        HirType::Variant(_, _) => unreachable!("Variant 不应到达此路径"),
                    };
                }
                panic!("undefined variable {:?}", def_id);
            }

            tyhir::TyHirExprKind::Binary { op, lhs, rhs } => match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                    self.assert_ty(&HirType::Int, expected_ty, "arithmetic result");
                    let l = self.translate_expr(builder, lhs, var_map, &HirType::Int);
                    let r = self.translate_expr(builder, rhs, var_map, &HirType::Int);
                    match op {
                        BinOp::Add => builder.ins().iadd(l, r),
                        BinOp::Sub => builder.ins().isub(l, r),
                        BinOp::Mul => builder.ins().imul(l, r),
                        BinOp::Div => builder.ins().sdiv(l, r),
                        _ => unreachable!(),
                    }
                }
                BinOp::Less
                | BinOp::LessEqual
                | BinOp::Equal
                | BinOp::NotEqual
                | BinOp::Greater
                | BinOp::GreaterEqual => {
                    self.assert_ty(&HirType::Int, expected_ty, "comparison result");
                    // 比较操作数可能是 int 或指针，按各自真实类型求值（不固定为 int）。
                    let l = self.translate_expr(builder, lhs, var_map, &lhs.ty);
                    let r = self.translate_expr(builder, rhs, var_map, &rhs.ty);
                    let cc = match op {
                        BinOp::Less => IntCC::SignedLessThan,
                        BinOp::LessEqual => IntCC::SignedLessThanOrEqual,
                        BinOp::Equal => IntCC::Equal,
                        BinOp::NotEqual => IntCC::NotEqual,
                        BinOp::Greater => IntCC::SignedGreaterThan,
                        BinOp::GreaterEqual => IntCC::SignedGreaterThanOrEqual,
                        _ => unreachable!(),
                    };
                    self.emit_cmp(builder, l, r, cc)
                }
            },

            tyhir::TyHirExprKind::Call { callee, args, .. } => {
                let FuncInfo {
                    func_id: base_func_id,
                    param_types,
                    ret_ty,
                    is_variadic,
                    ..
                } = self
                    .func_map
                    .get(callee)
                    .cloned()
                    .unwrap_or_else(|| panic!("undefined function {:?}", callee));
                self.assert_ty(&ret_ty, expected_ty, &format!("call {:?} return", callee));

                let func_ref = if is_variadic {
                    let extra_arg_types: Vec<HirType> = args
                        .iter()
                        .skip(param_types.len())
                        .map(|a| a.ty.clone())
                        .collect();
                    let sig = self.build_signature(&param_types, &ret_ty, Some(&extra_arg_types));
                    let sig_ref = builder.import_signature(sig);

                    let ext_name = self.module.declare_func_in_func(base_func_id, builder.func);
                    let existing_ext = &builder.func.dfg.ext_funcs[ext_name];
                    let name_ref = existing_ext.name.clone();

                    let new_ext = ExtFuncData {
                        name: name_ref,
                        signature: sig_ref,
                        colocated: false,
                        patchable: false,
                    };
                    builder.func.dfg.ext_funcs.push(new_ext)
                } else {
                    if args.len() != param_types.len() {
                        panic!(
                            "function {:?} expects {} arguments, got {}",
                            callee,
                            param_types.len(),
                            args.len()
                        );
                    }
                    self.module.declare_func_in_func(base_func_id, builder.func)
                };

                let mut arg_vals: Vec<Value> = Vec::with_capacity(args.len());

                let sret_slot = if Self::needs_sret(&ret_ty) {
                    let size = self.var_type_size(&ret_ty);
                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        size,
                        0,
                    ));
                    let sret_ptr = builder.ins().stack_addr(self.ptr_type(), slot, 0);
                    arg_vals.push(sret_ptr);
                    Some((slot, sret_ptr))
                } else {
                    None
                };

                for (arg, pty) in args.iter().zip(param_types.iter()) {
                    let raw = self.translate_expr(builder, arg, var_map, pty);
                    arg_vals.push(self.copy_value(builder, raw, pty));
                }
                if is_variadic {
                    for arg in args.iter().skip(param_types.len()) {
                        let ty = &arg.ty;
                        let raw = self.translate_expr(builder, arg, var_map, ty);
                        arg_vals.push(self.copy_value(builder, raw, ty));
                    }
                }

                let call_inst = builder.ins().call(func_ref, &arg_vals);

                if let Some((_slot, sret_ptr)) = sret_slot {
                    sret_ptr
                } else if ret_ty == HirType::Unit {
                    builder.ins().iconst(types::I64, 0)
                } else {
                    builder.inst_results(call_inst)[0]
                }
            }

            tyhir::TyHirExprKind::FieldAccess {
                object,
                field: field_def_id,
            } => {
                let obj_ty = &object.ty;
                let struct_def = match obj_ty {
                    HirType::Named(d) => d,
                    other => panic!(
                        "field access on non-struct type {:?} for field {:?}",
                        other, field_def_id
                    ),
                };
                let field_ty = self.field_type(struct_def, field_def_id);
                self.assert_ty(
                    &field_ty,
                    expected_ty,
                    &format!("field access {:?}.{:?}", struct_def, field_def_id),
                );

                // 地址计算复用 `translate_lvalue`，避免与「取地址 / 写入」路径重复实现。
                let addr = self.translate_lvalue(builder, expr, var_map);
                self.read_value_at(builder, addr, &field_ty)
            }

            tyhir::TyHirExprKind::StructLiteral { def_id, fields, .. } => {
                let named_ty = HirType::Named(*def_id);
                self.assert_ty(
                    &named_ty,
                    expected_ty,
                    &format!("struct literal {:?}", def_id),
                );

                let total_size = self.struct_size(def_id);
                if total_size == 0 {
                    return builder.ins().iconst(ptr_ty, 0);
                }

                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    total_size,
                    0,
                ));
                let base_ptr = builder.ins().stack_addr(ptr_ty, slot, 0);

                for (field_def_id, field_expr) in fields {
                    let field_ty = self.field_type(def_id, field_def_id);
                    let raw_val = self.translate_expr(builder, field_expr, var_map, &field_ty);
                    let offset = self.field_offset(def_id, field_def_id);
                    let dst_addr = builder.ins().iadd_imm(base_ptr, offset as i64);
                    self.copy_value_to(builder, raw_val, &field_ty, dst_addr)
                }

                base_ptr
            }

            tyhir::TyHirExprKind::ArrayLiteral { elements, repeat } => {
                let arr_ty = &expr.ty;
                self.assert_ty(arr_ty, expected_ty, "array literal");
                let elem_ty = match arr_ty {
                    HirType::Array(e, _) => (**e).clone(),
                    _ => unreachable!("array literal type must be Array"),
                };
                let total_size = self.var_type_size(arr_ty);
                let base_ptr = if total_size == 0 {
                    builder.ins().iconst(self.ptr_type(), 0)
                } else {
                    let slot = builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        total_size,
                        0,
                    ));
                    builder.ins().stack_addr(self.ptr_type(), slot, 0)
                };
                let stride = self.var_type_size(&elem_ty) as i64;
                match repeat {
                    Some(n) => {
                        let v = self.translate_expr(builder, &elements[0], var_map, &elem_ty);
                        for i in 0..*n {
                            let dst = builder.ins().iadd_imm(base_ptr, stride * (i as i64));
                            self.copy_value_to(builder, v, &elem_ty, dst);
                        }
                    }
                    None => {
                        for (i, el) in elements.iter().enumerate() {
                            let v = self.translate_expr(builder, el, var_map, &elem_ty);
                            let dst = builder.ins().iadd_imm(base_ptr, stride * (i as i64));
                            self.copy_value_to(builder, v, &elem_ty, dst);
                        }
                    }
                }
                base_ptr
            }

            tyhir::TyHirExprKind::Index { .. } => {
                let elem_ty = &expr.ty;
                self.assert_ty(elem_ty, expected_ty, "array index result");
                // 地址计算复用 `translate_lvalue`，避免与「取地址 / 写入」路径重复实现。
                let addr = self.translate_lvalue(builder, expr, var_map);
                self.read_value_at(builder, addr, elem_ty)
            }

            tyhir::TyHirExprKind::Deref(inner) => {
                // 指针值本身就是地址；按承载类型 load 出来。
                let ptr = self.translate_expr(builder, inner, var_map, &inner.ty);
                let pointee = match &inner.ty {
                    HirType::Pointer(p) => &**p,
                    other => unreachable!("deref of non-pointer type {:?}", other),
                };
                self.assert_ty(pointee, expected_ty, "dereference result");
                let clif_ty = self.var_type_to_cranelift(pointee);
                builder.ins().load(clif_ty, MemFlagsData::new(), ptr, 0)
            }

            tyhir::TyHirExprKind::AddressOf(inner) => {
                // 取地址：返回操作数的存储地址。
                self.translate_lvalue(builder, inner, var_map)
            }

            tyhir::TyHirExprKind::Variant {
                def_id,
                variant,
                args,
                turbofish: _,
            } => {
                let layout = self.enum_map.get(def_id).expect("unknown enum").clone();
                let var_layout = layout
                    .variants
                    .iter()
                    .find(|v| v.name == *variant)
                    .cloned()
                    .unwrap_or_else(|| panic!("variant {} not in enum {:?}", variant, def_id));
                let size = layout.size;
                let align = layout.align as u8;
                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    size,
                    align,
                ));
                let base = builder.ins().stack_addr(self.ptr_type(), slot, 0);
                // 写 tag（i64，offset 0）
                let tag_val = builder.ins().iconst(types::I64, var_layout.tag as i64);
                builder.ins().store(MemFlagsData::new(), tag_val, base, 0);
                // 写载荷字段
                let payload_base = var_layout.payload_offset as i64;
                for (arg, field) in args.iter().zip(var_layout.fields.iter()) {
                    // 用 arg 的实际类型而非 field.field_type（后者可能含 Var 占位符）
                    let val = self.translate_expr(builder, arg, var_map, &arg.ty);
                    let val = self.copy_value(builder, val, &arg.ty);
                    let field_addr = builder
                        .ins()
                        .iadd_imm(base, payload_base + field.offset as i64);
                    builder.ins().store(MemFlagsData::new(), val, field_addr, 0);
                }
                // Enum 的值就是指向这个栈槽的指针
                base
            }
            tyhir::TyHirExprKind::Match { scrutinee, arms } => {
                let ptr_ty_local = self.ptr_type();
                let merge_block = builder.create_block();
                builder.append_block_param(merge_block, ptr_ty_local);

                // 为每个 arm 创建一个 block
                let n_arms = arms.len();
                let mut arm_blocks: Vec<Block> = Vec::with_capacity(n_arms);
                for _ in 0..n_arms {
                    arm_blocks.push(builder.create_block());
                }

                // 翻译 scrutinee
                let scrutinee_raw = self.translate_expr(builder, scrutinee, var_map, &scrutinee.ty);
                let is_enum_match = matches!(&scrutinee.ty, HirType::Enum(_, _, _));
                let scrutinee_val = if is_enum_match {
                    // 枚举值是指针：copy_value 保持地址
                    self.copy_value(builder, scrutinee_raw, &scrutinee.ty)
                } else {
                    // int 值：直接使用
                    scrutinee_raw
                };

                // 为每个臂生成「检查 → 跳转到对应 arm block」的代码链
                for (arm_idx, arm) in arms.iter().enumerate() {
                    let this_arm_block = arm_blocks[arm_idx];

                    match &arm.pattern {
                        tyhir::TyHirPattern::Wildcard | tyhir::TyHirPattern::Ident(_) => {
                            // 通配/标识符总是匹配：直接跳转到 arm block
                            builder.ins().jump(this_arm_block, &[]);
                            break; // 后续臂不可达
                        }
                        tyhir::TyHirPattern::Variant {
                            enum_def,
                            variant,
                            bindings: _,
                        } => {
                            let layout = self.enum_map.get(enum_def).expect("unknown enum");
                            let var_layout = layout
                                .variants
                                .iter()
                                .find(|v| v.name == *variant)
                                .cloned()
                                .unwrap_or_else(|| {
                                    panic!("variant {} not found in enum {:?}", variant, enum_def)
                                });

                            let tag_val = builder.ins().load(
                                types::I64,
                                MemFlagsData::new(),
                                scrutinee_val,
                                0,
                            );
                            let variant_tag =
                                builder.ins().iconst(types::I64, var_layout.tag as i64);
                            let cmp = builder.ins().icmp(IntCC::Equal, tag_val, variant_tag);

                            if arm_idx + 1 < n_arms {
                                let _next_arm_block = arm_blocks[arm_idx + 1];
                                let fallthrough = builder.create_block();
                                builder
                                    .ins()
                                    .brif(cmp, this_arm_block, &[], fallthrough, &[]);
                                builder.switch_to_block(fallthrough);
                                builder.seal_block(fallthrough);
                            } else {
                                let zero = builder.ins().iconst(ptr_ty_local, 0);
                                builder.ins().brif(
                                    cmp,
                                    this_arm_block,
                                    &[],
                                    merge_block,
                                    &[BlockArg::Value(zero)],
                                );
                            }
                        }
                        tyhir::TyHirPattern::Literal(lit) => {
                            // int 匹配：scrutinee_val 就是整数值，直接比较
                            let lit_val = match lit {
                                Literal::Int(v) => builder.ins().iconst(types::I64, *v),
                                Literal::String(_) => {
                                    panic!("字符串字面量不能用作 match 模式")
                                }
                            };
                            let cmp = builder.ins().icmp(IntCC::Equal, scrutinee_val, lit_val);

                            if arm_idx + 1 < n_arms {
                                let _next_arm_block = arm_blocks[arm_idx + 1];
                                let fallthrough = builder.create_block();
                                builder
                                    .ins()
                                    .brif(cmp, this_arm_block, &[], fallthrough, &[]);
                                builder.switch_to_block(fallthrough);
                                builder.seal_block(fallthrough);
                            } else {
                                let zero = builder.ins().iconst(ptr_ty_local, 0);
                                builder.ins().brif(
                                    cmp,
                                    this_arm_block,
                                    &[],
                                    merge_block,
                                    &[BlockArg::Value(zero)],
                                );
                            }
                        }
                    }
                }

                // 填充每个 arm block 的 body
                for (arm, arm_block) in arms.iter().zip(arm_blocks.iter()) {
                    builder.switch_to_block(*arm_block);
                    builder.seal_block(*arm_block);

                    let mut arm_var_map = var_map.clone();

                    // 处理模式绑定
                    match &arm.pattern {
                        tyhir::TyHirPattern::Variant {
                            enum_def,
                            variant,
                            bindings,
                        } => {
                            let layout = self.enum_map.get(enum_def).expect("unknown enum");
                            let var_layout = layout
                                .variants
                                .iter()
                                .find(|v| v.name == *variant)
                                .cloned()
                                .unwrap_or_else(|| {
                                    panic!("variant {} not found in enum {:?}", variant, enum_def)
                                });
                            for (field_idx, (bdef, _bty)) in bindings.iter().enumerate() {
                                let field_info = &var_layout.fields[field_idx];
                                let field_offset =
                                    (var_layout.payload_offset + field_info.offset) as i64;
                                let field_addr =
                                    builder.ins().iadd_imm(scrutinee_val, field_offset);
                                // 字段类型可能含 Var（泛型占位符），此时按 8 字节 i64 处理
                                let (field_clif_ty, bind_ty) = if matches!(
                                    field_info.field_type,
                                    HirType::Var(_) | HirType::Generic(_, _, _)
                                ) {
                                    (types::I64, HirType::Int)
                                } else {
                                    (
                                        self.var_type_to_cranelift(&field_info.field_type),
                                        field_info.field_type.clone(),
                                    )
                                };
                                let field_val = builder.ins().load(
                                    field_clif_ty,
                                    MemFlagsData::new(),
                                    field_addr,
                                    0,
                                );
                                let local = self.bind_local(builder, &bind_ty, field_val, false);
                                arm_var_map.insert(*bdef, local);
                            }
                        }
                        tyhir::TyHirPattern::Ident(bdef) => {
                            // Ident 绑定整个 scrutinee
                            let local =
                                self.bind_local(builder, &scrutinee.ty, scrutinee_val, false);
                            arm_var_map.insert(*bdef, local);
                        }
                        tyhir::TyHirPattern::Wildcard | tyhir::TyHirPattern::Literal(_) => {}
                    }

                    let body_val = self.translate_stmts(
                        builder,
                        &arm.body.stmts,
                        &mut arm_var_map,
                        &HirType::Unit,
                        None,
                        &Default::default(),
                    );
                    let jump_val = if expr.ty == HirType::Unit {
                        builder.ins().iconst(ptr_ty_local, 0)
                    } else {
                        body_val
                    };
                    builder
                        .ins()
                        .jump(merge_block, &[BlockArg::Value(jump_val)]);
                }

                // 切换到 merge 块并封口
                builder.switch_to_block(merge_block);
                builder.seal_block(merge_block);

                builder.block_params(merge_block)[0]
            }
            tyhir::TyHirExprKind::TraitCast { value, trait_def } => {
                // TraitCast 的 expr.ty 是 TraitObject(trait_def)，
                // 但这里我们需要返回一个 fat pointer 的地址（16 字节栈槽）。
                // 先翻译 value（应是 Named(struct_def) 类型的结构体指针）
                let data_ptr = self.translate_expr(builder, value, var_map, &value.ty);
                // 从 value.ty 获取 struct_def
                let struct_def = match &value.ty {
                    HirType::Named(d) => *d,
                    _ => unreachable!("TraitCast value must be a Named struct type"),
                };
                // 查找 vtable 的数据 ID
                let data_id = self
                    .vtable_map
                    .get(&(*trait_def, struct_def))
                    .unwrap_or_else(|| {
                        panic!(
                            "vtable not found for trait {:?} + struct {:?}",
                            trait_def, struct_def
                        )
                    });
                let vtable_addr = {
                    let gv = self.module.declare_data_in_func(*data_id, builder.func);
                    builder.ins().symbol_value(self.ptr_type(), gv)
                };
                // 创建 16 字节栈槽存放 fat pointer: [data_ptr, vtable_ptr]
                let ptr_bytes_i64 = self.ptr_type().bytes() as i64;
                let ptr_bytes_i32 = self.ptr_type().bytes() as i32;
                let size = (ptr_bytes_i64 * 2) as u32;
                let slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    size,
                    8,
                ));
                let base = builder.ins().stack_addr(self.ptr_type(), slot, 0);
                builder.ins().store(MemFlagsData::new(), data_ptr, base, 0);
                builder
                    .ins()
                    .store(MemFlagsData::new(), vtable_addr, base, ptr_bytes_i32);
                base
            }

            tyhir::TyHirExprKind::DynamicMethodCall {
                trait_def: _,
                method_index,
                receiver,
                args,
            } => {
                // receiver 的 expr.ty 是 TraitObject(trait_def)，其值是一个 fat pointer
                // 的地址（栈槽指针，包含 [data_ptr, vtable_ptr]）。
                let receiver_val = self.translate_expr(builder, receiver, var_map, &receiver.ty);
                let _ptr_bytes_i64 = self.ptr_type().bytes() as i64;
                let ptr_bytes_i32 = self.ptr_type().bytes() as i32;
                // 从 fat pointer 中加载 data_ptr (offset 0) 和 vtable_ptr (offset ptr_bytes)
                let data_ptr =
                    builder
                        .ins()
                        .load(self.ptr_type(), MemFlagsData::new(), receiver_val, 0);
                let vtable_ptr = builder.ins().load(
                    self.ptr_type(),
                    MemFlagsData::new(),
                    receiver_val,
                    ptr_bytes_i32,
                );
                // 从 vtable 加载方法函数指针：offset = method_index * ptr_bytes
                let method_fn_ptr = builder.ins().load(
                    self.ptr_type(),
                    MemFlagsData::new(),
                    vtable_ptr,
                    (*method_index as i32) * ptr_bytes_i32,
                );
                // 构建调用签名：self(data_ptr) + args
                let sig = self.module.make_signature();
                let mut sig = sig;
                sig.params
                    .push(cranelift_codegen::ir::AbiParam::new(self.ptr_type())); // self/data ptr
                for a in args {
                    sig.params.push(cranelift_codegen::ir::AbiParam::new(
                        self.var_type_to_cranelift(&a.ty),
                    ));
                }
                sig.returns
                    .push(cranelift_codegen::ir::AbiParam::new(types::I64));
                let sig_ref = builder.import_signature(sig);
                // 生成间接调用
                let mut arg_vals: Vec<Value> = Vec::with_capacity(1 + args.len());
                arg_vals.push(data_ptr);
                for a in args {
                    let raw = self.translate_expr(builder, a, var_map, &a.ty);
                    arg_vals.push(self.copy_value(builder, raw, &a.ty));
                }
                let call_inst = builder
                    .ins()
                    .call_indirect(sig_ref, method_fn_ptr, &arg_vals);
                self.assert_ty(&HirType::Int, expected_ty, "DynamicMethodCall result");
                builder.inst_results(call_inst)[0]
            }
        }
    }

    /// 计算一个左值表达式的**地址**（而非它的值），用于实现取地址符 `&`。
    /// 假定该表达式已被类型检查确认为左值（拥有确定存储）。
    pub(crate) fn translate_lvalue(
        &mut self,
        builder: &mut FunctionBuilder,
        expr: &tyhir::TyHirExpr,
        var_map: &HashMap<DefId, LocalVar>,
    ) -> Value {
        let ptr_ty = self.ptr_type();
        match &expr.kind {
            tyhir::TyHirExprKind::Path(def_id) => {
                if let Some(info) = var_map.get(def_id) {
                    // 结构体局部变量按其 sret 约定本就是地址。
                    if let Some(slot) = info.slot {
                        return builder.ins().stack_addr(ptr_ty, slot, 0);
                    }
                    return builder.use_var(info.var.unwrap());
                }
                if let Some((data_id, _)) = self.data_map.get(def_id) {
                    let gv = self.module.declare_data_in_func(*data_id, builder.func);
                    return builder.ins().symbol_value(ptr_ty, gv);
                }
                panic!("undefined variable {:?}", def_id);
            }
            tyhir::TyHirExprKind::FieldAccess { object, field } => {
                let base = self.translate_lvalue(builder, object, var_map);
                let struct_def = match &object.ty {
                    HirType::Named(d) => *d,
                    other => panic!("field access on non-struct type {:?}", other),
                };
                let offset = self.field_offset(&struct_def, field);
                builder.ins().iadd_imm(base, offset as i64)
            }
            // &(*p) 即 p 本身（指针值已是地址）。
            tyhir::TyHirExprKind::Deref(inner) => {
                self.translate_expr(builder, inner, var_map, &inner.ty)
            }
            tyhir::TyHirExprKind::Index { array, index } => {
                // 数组下标的地址：基地址 + 下标 × 元素步长。
                let base = self.translate_expr(builder, array, var_map, &array.ty);
                let idx = self.translate_expr(builder, index, var_map, &HirType::Int);
                let elem_ty = match &array.ty {
                    HirType::Array(e, _) => (**e).clone(),
                    _ => unreachable!("index of non-array in lvalue"),
                };
                let stride = self.var_type_size(&elem_ty) as i64;
                let stride_val = builder.ins().iconst(types::I64, stride);
                let off = builder.ins().imul(idx, stride_val);
                builder.ins().iadd(base, off)
            }
            tyhir::TyHirExprKind::AddressOf(inner) => {
                self.translate_lvalue(builder, inner, var_map)
            }
            _ => panic!("cannot take address of a non-lvalue expression"),
        }
    }

    /// 从「聚合地址」处按类型读出一个值：聚合（结构体 / 数组）直接返回地址，
    /// 标量则 load 后经 `copy_value` 规范化。
    fn read_value_at(&mut self, builder: &mut FunctionBuilder, addr: Value, ty: &HirType) -> Value {
        match ty {
            HirType::Named(_) | HirType::Array(_, _) => addr,
            _ => {
                let ir_ty = self.var_type_to_cranelift(ty);
                let loaded = builder.ins().load(ir_ty, MemFlagsData::new(), addr, 0);
                self.copy_value(builder, loaded, ty)
            }
        }
    }

    fn emit_cmp(&self, builder: &mut FunctionBuilder, lhs: Value, rhs: Value, cc: IntCC) -> Value {
        let ptr_ty = self.ptr_type();
        let cmp = builder.ins().icmp(cc, lhs, rhs);
        builder.ins().uextend(ptr_ty, cmp)
    }
}
