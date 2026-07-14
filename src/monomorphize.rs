//! # 单态化（Monomorphization）—— TYPE HIR → 具体的 TYPE HIR
//!
//! 前端的词法 / 语法 / 降级 / 类型检查已经完整支持泛型：类型检查产出的 TYPE HIR
//! 里仍可能包含「泛型占位符」[`HirType::Var`]（类型参数）、
//! [`HirType::Generic`]（泛型结构体应用类型）、[`ArrayLen::Const`]
//! （常量参数当数组长度）。后端（Cranelift 代码生成）要求所有类型都是具体的，
//! 因此本趟在「类型检查」与「代码生成」之间，把泛型定义按需展开为具体实例。
//!
//! 策略（与 Rust 一致，**拒绝鸭子类型**）：
//! - 每个被（直接或间接）调用的泛型函数、被实例化的泛型结构体，都会生成一份
//!   **具体实例**（新的 [`DefId`]），函数体内所有 `Var`/`Generic`/`ArrayLen::Const`
//!   被代入为真实类型 / 整数；常量参数 `N` 在实例化后作为整型字面量使用
//!   （可直接 `return N;`、赋给变量、传给普通函数）。
//! - 类型实参的推断来自调用处：优先用涡轮鱼 `::<...>`，其余从位置实参类型推断；
//!   常量实参只能由涡轮鱼显式指定（无法从运行时值推断）。
//! - 未被任何调用点引用的泛型定义不实例化（类比 Rust 的死代码不单态化）。

use std::collections::HashMap;

use crate::ast::{GenericParam, GenericParamKind, Literal};
use crate::error::{MplangError, fatal, into_result};
use crate::hir::{ConstArg, DefId, HirType, ParamKind, TypeOrConst};
use crate::tycheck::TypeChecker;
use crate::tyhir;

/// 一个泛型实例的「实参键」：类型实参与常量实参，按泛型参数声明顺序。
#[derive(Clone, PartialEq, Eq, Hash)]
struct MonoKey {
    targs: Vec<HirType>,
    cargs: Vec<i64>,
}

/// 泛型函数模板（单态化前）。
struct FnTemplate {
    generics: Vec<GenericParam>,
    params: Vec<tyhir::TyHirParam>,
    return_ty: HirType,
    body: tyhir::TyHirBody,
    impl_receiver: Option<HirType>,
}

/// 泛型结构体模板。
struct StructTemplate {
    generics: Vec<GenericParam>,
    fields: Vec<tyhir::TyHirField>,
}

/// 实例化的结构体 → 其模板与实参（用于 `impl<T>` 方法按接收者类型反查类型参数）。
struct InstanceInfo {
    targs: Vec<HirType>,
    cargs: Vec<ConstArg>,
}

struct Mono {
    fn_templates: HashMap<DefId, FnTemplate>,
    struct_templates: HashMap<DefId, StructTemplate>,
    /// 函数模板 DefId → (实参键 → 实例 DefId)。
    mono_fns: HashMap<DefId, HashMap<MonoKey, DefId>>,
    /// 结构体模板 DefId → (实参键 → 实例 DefId)。
    mono_structs: HashMap<DefId, HashMap<MonoKey, DefId>>,
    /// 实例结构体 DefId → 模板与实参（反查）。
    instance_structs: HashMap<DefId, InstanceInfo>,
    out_items: Vec<tyhir::TyHirItem>,
    next_id: u32,
}

/// 入口：消费一份可能含泛型占位符的 TYPE HIR，产出完全具体的 TYPE HIR。
pub fn monomorphize(
    unit: &tyhir::TyHirCompilationUnit,
) -> Result<tyhir::TyHirCompilationUnit, MplangError> {
    into_result(|| Mono::new(unit).run(unit))
}

impl Mono {
    fn new(unit: &tyhir::TyHirCompilationUnit) -> Self {
        // 计算不与已有 DefId 冲突的起始编号。
        let mut max_id = 0u32;
        collect_ids(&unit.root_module, &mut max_id);
        Mono {
            fn_templates: HashMap::new(),
            struct_templates: HashMap::new(),
            mono_fns: HashMap::new(),
            mono_structs: HashMap::new(),
            instance_structs: HashMap::new(),
            out_items: Vec::new(),
            next_id: max_id + 1,
        }
    }

    fn alloc(&mut self) -> DefId {
        let id = DefId(self.next_id);
        self.next_id += 1;
        id
    }

    fn run(mut self, unit: &tyhir::TyHirCompilationUnit) -> tyhir::TyHirCompilationUnit {
        // 趟 1：收集模板（泛型函数 / 所有结构体）。
        self.collect_templates(&unit.root_module);

        // 趟 2：展开顶层 item。泛型模板本身不输出，仅其被调用的实例会输出。
        // 实例在遍历过程中被收集到 `out_items`，最终与顶层非泛型 item 合并。
        let items = self.mono_items(&unit.root_module.items);

        let mut all_items = items;
        all_items.extend(self.out_items.iter().cloned());

        tyhir::TyHirCompilationUnit {
            root_module: tyhir::TyHirModule {
                def_id: unit.root_module.def_id,
                name: unit.root_module.name.clone(),
                visibility: unit.root_module.visibility.clone(),
                attributes: unit.root_module.attributes.clone(),
                items: all_items,
            },
            vtables: unit.vtables.clone(),
        }
    }

    fn collect_templates(&mut self, m: &tyhir::TyHirModule) {
        for item in &m.items {
            match item {
                tyhir::TyHirItem::Fn {
                    def_id,
                    generics,
                    params,
                    return_ty,
                    body,
                    impl_receiver,
                    ..
                } => {
                    if !generics.is_empty() {
                        self.fn_templates.insert(
                            *def_id,
                            FnTemplate {
                                generics: generics.clone(),
                                params: params.clone(),
                                return_ty: return_ty.clone(),
                                body: body.clone(),
                                impl_receiver: impl_receiver.clone(),
                            },
                        );
                    }
                }
                tyhir::TyHirItem::Struct {
                    def_id,
                    generics,
                    fields,
                    ..
                } => {
                    self.struct_templates.insert(
                        *def_id,
                        StructTemplate {
                            generics: generics.clone(),
                            fields: fields.clone(),
                        },
                    );
                }
                tyhir::TyHirItem::Module(sub) => self.collect_templates(sub),
                _ => {}
            }
        }
    }

    fn mono_items(&mut self, items: &[tyhir::TyHirItem]) -> Vec<tyhir::TyHirItem> {
        let mut out = Vec::new();
        for item in items {
            match item {
                tyhir::TyHirItem::Module(m) => {
                    out.push(tyhir::TyHirItem::Module(tyhir::TyHirModule {
                        def_id: m.def_id,
                        name: m.name.clone(),
                        visibility: m.visibility.clone(),
                        attributes: m.attributes.clone(),
                        items: self.mono_items(&m.items),
                    }));
                }
                // 泛型函数模板：不输出（只输出其实例）。
                tyhir::TyHirItem::Fn { generics, .. } if !generics.is_empty() => {}
                // 非泛型函数：展开函数体（其中的泛型调用会按需实例化）。
                tyhir::TyHirItem::Fn {
                    def_id,
                    visibility,
                    attributes,
                    name,
                    params,
                    return_ty,
                    body,
                    impl_receiver,
                    ..
                } => {
                    let params = params
                        .iter()
                        .map(|p| tyhir::TyHirParam {
                            def_id: p.def_id,
                            name: p.name.clone(),
                            ty: p.ty.clone(),
                            kind: p.kind,
                        })
                        .collect();
                    let body = self.mono_body(body, &[], &[], &[], &HashMap::new());
                    out.push(tyhir::TyHirItem::Fn {
                        def_id: *def_id,
                        visibility: visibility.clone(),
                        attributes: attributes.clone(),
                        name: name.clone(),
                        generics: Vec::new(),
                        params,
                        return_ty: return_ty.clone(),
                        body,
                        impl_receiver: impl_receiver.clone(),
                    });
                }
                // 泛型结构体模板：不输出（只输出其实例）。
                tyhir::TyHirItem::Struct { generics, .. } if !generics.is_empty() => {}
                // 非泛型结构体：原样输出。
                tyhir::TyHirItem::Struct {
                    def_id,
                    visibility,
                    attributes,
                    name,
                    generics,
                    fields,
                    ..
                } => {
                    out.push(tyhir::TyHirItem::Struct {
                        def_id: *def_id,
                        visibility: visibility.clone(),
                        attributes: attributes.clone(),
                        name: name.clone(),
                        generics: generics.clone(),
                        fields: fields.clone(),
                    });
                }
                tyhir::TyHirItem::Static {
                    def_id,
                    visibility,
                    attributes,
                    name,
                    ty,
                    init,
                    is_const,
                } => {
                    let init = self.mono_expr(init, &[], &[], &[], &HashMap::new());
                    out.push(tyhir::TyHirItem::Static {
                        def_id: *def_id,
                        visibility: visibility.clone(),
                        attributes: attributes.clone(),
                        name: name.clone(),
                        ty: ty.clone(),
                        init,
                        is_const: *is_const,
                    });
                }
                tyhir::TyHirItem::ExternFn { .. } => out.push(item.clone()),
                tyhir::TyHirItem::Enum { .. } => {
                    // 透传枚举；变体字段类型中的 Var 由 codegen 布局阶段处理（回退为 8 字节占位）。
                    out.push(item.clone());
                }
                tyhir::TyHirItem::VTable { .. } => {
                    out.push(item.clone());
                }
            }
        }
        out
    }

    // ───────────────── 函数实例化 ─────────────────

    /// 实例化（或取已存在的）泛型函数实例，返回其 [`DefId`]。
    fn instantiate_fn(&mut self, tmpl: DefId, targs: &[HirType], cargs: &[i64]) -> DefId {
        let key = MonoKey {
            targs: targs.to_vec(),
            cargs: cargs.to_vec(),
        };
        if let Some(existing) = self.mono_fns.get(&tmpl).and_then(|m| m.get(&key)).copied() {
            return existing;
        }
        // 取出模板数据（克隆后立刻释放不可变借用，避免与下方可变借用冲突）。
        let (generics, tparams, treturn, trecv, tbody) = {
            let t = self.fn_templates.get(&tmpl).expect("fn template");
            (
                t.generics.clone(),
                t.params.clone(),
                t.return_ty.clone(),
                t.impl_receiver.clone(),
                t.body.clone(),
            )
        };

        // 常量参数 def_id → 实际整数值（用于把 `N` 替换为整型字面量）。
        let mut const_map: HashMap<DefId, i64> = HashMap::new();
        for p in &tparams {
            if let ParamKind::ConstParam(i) = p.kind {
                const_map.insert(p.def_id, cargs[const_slot(&generics, i)]);
            }
        }

        let body = self.mono_body(&tbody, &generics, targs, cargs, &const_map);
        let new_id = self.alloc();
        let params: Vec<tyhir::TyHirParam> = tparams
            .iter()
            .filter(|p| !matches!(p.kind, ParamKind::ConstParam(_)))
            .map(|p| tyhir::TyHirParam {
                def_id: p.def_id,
                name: p.name.clone(),
                ty: self.mono_type(&p.ty, &generics, targs, cargs),
                kind: p.kind,
            })
            .collect();
        let return_ty = self.mono_type(&treturn, &generics, targs, cargs);
        let impl_receiver = trecv
            .as_ref()
            .map(|r| self.mono_type(r, &generics, targs, cargs));

        self.mono_fns.entry(tmpl).or_default().insert(key, new_id);

        self.out_items.push(tyhir::TyHirItem::Fn {
            def_id: new_id,
            visibility: crate::hir::Visibility::Public,
            attributes: Vec::new(),
            name: self.fn_name_hint(tmpl, targs, cargs),
            generics: Vec::new(),
            params,
            return_ty,
            body,
            impl_receiver,
        });
        new_id
    }

    fn fn_name_hint(&self, tmpl: DefId, targs: &[HirType], _cargs: &[i64]) -> String {
        // 仅用于链接名可读；实际链接名由 codegen 按 DefId 决定，名字可重复。
        // 包含类型实参哈希以确保不同实例有不同名字。
        let mut s = format!("__mono_{}", tmpl.0);
        for t in targs {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            t.hash(&mut h);
            s.push_str(&format!("_{}", h.finish()));
        }
        s
    }

    /// 实例化（或取已存在的）泛型结构体实例，返回其 [`DefId`]。
    /// 实例字段沿用模板的字段 [`DefId`]（因为 `struct_map` 查找以 (结构体 DefId, 字段 DefId)
    /// 为键，不同实例的结构体 DefId 不同，字段 DefId 复用不会冲突）。
    fn instantiate_struct(&mut self, tmpl: DefId, targs: &[HirType], cargs: &[i64]) -> DefId {
        let key = MonoKey {
            targs: targs.to_vec(),
            cargs: cargs.to_vec(),
        };
        if let Some(existing) = self
            .mono_structs
            .get(&tmpl)
            .and_then(|m| m.get(&key))
            .copied()
        {
            return existing;
        }
        let (generics, sfields) = {
            let s = self.struct_templates.get(&tmpl).expect("struct template");
            (s.generics.clone(), s.fields.clone())
        };
        let new_id = self.alloc();
        let fields: Vec<tyhir::TyHirField> = sfields
            .iter()
            .map(|f| tyhir::TyHirField {
                def_id: f.def_id,
                name: f.name.clone(),
                ty: self.mono_type(&f.ty, &generics, targs, cargs),
                visibility: f.visibility.clone(),
            })
            .collect();

        self.mono_structs
            .entry(tmpl)
            .or_default()
            .insert(key, new_id);
        self.instance_structs.insert(
            new_id,
            InstanceInfo {
                targs: targs.to_vec(),
                cargs: const_args_view(cargs),
            },
        );

        self.out_items.push(tyhir::TyHirItem::Struct {
            def_id: new_id,
            visibility: crate::hir::Visibility::Public,
            attributes: Vec::new(),
            name: self.struct_name_hint(tmpl),
            generics: Vec::new(),
            fields,
        });
        new_id
    }

    fn struct_name_hint(&self, tmpl: DefId) -> String {
        format!("__mono_s_{}", tmpl.0)
    }

    // ───────────────── 表达式 / 语句遍历（含类型代入） ─────────────────

    /// 单态化函数体。`generics`/`targs`/`cargs` 描述当前所在的泛型实例化上下文
    /// （非泛型函数这些为空）；`const_map` 把常量参数 def_id 映射到具体整数。
    fn mono_body(
        &mut self,
        body: &tyhir::TyHirBody,
        generics: &[GenericParam],
        targs: &[HirType],
        cargs: &[i64],
        const_map: &HashMap<DefId, i64>,
    ) -> tyhir::TyHirBody {
        tyhir::TyHirBody {
            stmts: body
                .stmts
                .iter()
                .map(|s| self.mono_stmt(s, generics, targs, cargs, const_map))
                .collect(),
        }
    }

    fn mono_stmt(
        &mut self,
        stmt: &tyhir::TyHirStmt,
        generics: &[GenericParam],
        targs: &[HirType],
        cargs: &[i64],
        const_map: &HashMap<DefId, i64>,
    ) -> tyhir::TyHirStmt {
        match stmt {
            tyhir::TyHirStmt::Let {
                def_id,
                name,
                ty,
                init,
            } => tyhir::TyHirStmt::Let {
                def_id: *def_id,
                name: name.clone(),
                ty: self.mono_type(ty, generics, targs, cargs),
                init: self.mono_expr(init, generics, targs, cargs, const_map),
            },
            tyhir::TyHirStmt::Assign { target, value } => tyhir::TyHirStmt::Assign {
                target: self.mono_expr(target, generics, targs, cargs, const_map),
                value: self.mono_expr(value, generics, targs, cargs, const_map),
            },
            tyhir::TyHirStmt::Expr(e) => {
                tyhir::TyHirStmt::Expr(self.mono_expr(e, generics, targs, cargs, const_map))
            }
            tyhir::TyHirStmt::If {
                cond,
                then_branch,
                else_branch,
            } => tyhir::TyHirStmt::If {
                cond: self.mono_expr(cond, generics, targs, cargs, const_map),
                then_branch: self.mono_body(then_branch, generics, targs, cargs, const_map),
                else_branch: else_branch
                    .as_ref()
                    .map(|b| self.mono_body(b, generics, targs, cargs, const_map)),
            },
            tyhir::TyHirStmt::While { cond, body } => tyhir::TyHirStmt::While {
                cond: self.mono_expr(cond, generics, targs, cargs, const_map),
                body: self.mono_body(body, generics, targs, cargs, const_map),
            },
            tyhir::TyHirStmt::Return(e) => tyhir::TyHirStmt::Return(
                e.as_ref()
                    .map(|x| self.mono_expr(x, generics, targs, cargs, const_map)),
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mono_expr(
        &mut self,
        expr: &tyhir::TyHirExpr,
        generics: &[GenericParam],
        targs: &[HirType],
        cargs: &[i64],
        const_map: &HashMap<DefId, i64>,
    ) -> tyhir::TyHirExpr {
        let ty = self.mono_type(&expr.ty, generics, targs, cargs);
        let kind = match &expr.kind {
            tyhir::TyHirExprKind::Literal(l) => tyhir::TyHirExprKind::Literal(l.clone()),

            tyhir::TyHirExprKind::Path(def_id) => {
                // 常量参数 `N` 在实例中退化为整型字面量，可直接使用（传给普通函数/变量亦可）。
                if let Some(v) = const_map.get(def_id) {
                    return tyhir::TyHirExpr {
                        ty: HirType::Int,
                        kind: tyhir::TyHirExprKind::Literal(Literal::Int(*v)),
                    };
                }
                tyhir::TyHirExprKind::Path(*def_id)
            }

            tyhir::TyHirExprKind::Binary { op, lhs, rhs } => tyhir::TyHirExprKind::Binary {
                op: *op,
                lhs: Box::new(self.mono_expr(lhs, generics, targs, cargs, const_map)),
                rhs: Box::new(self.mono_expr(rhs, generics, targs, cargs, const_map)),
            },

            tyhir::TyHirExprKind::Call {
                callee,
                args,
                turbofish,
            } => {
                // `mono_call` 已产出含返回类型的完整表达式，直接返回。
                return self.mono_call(
                    *callee, args, turbofish, &ty, generics, targs, cargs, const_map,
                );
            }

            tyhir::TyHirExprKind::FieldAccess { object, field } => {
                tyhir::TyHirExprKind::FieldAccess {
                    object: Box::new(self.mono_expr(object, generics, targs, cargs, const_map)),
                    field: *field,
                }
            }

            tyhir::TyHirExprKind::StructLiteral {
                def_id,
                fields,
                turbofish: _,
            } => {
                // 先单态化各字段初始化表达式。`ty` 经 `mono_type` 后，泛型结构体已被实例化为
                // `Named(实例)`；非泛型结构体仍为 `Named(模板)`。结构体字面量的 `def_id` 取
                // 自 `ty`（若已是实例则用之，否则沿用模板）。
                let fields = fields
                    .iter()
                    .map(|(fdef, fe)| {
                        (*fdef, self.mono_expr(fe, generics, targs, cargs, const_map))
                    })
                    .collect();
                let lit_def = match &ty {
                    HirType::Named(d) => *d,
                    HirType::Generic(gdef, gtargs, gcargs) => {
                        self.instantiate_struct(*gdef, gtargs, &const_args_to_i64(gcargs))
                    }
                    _ => *def_id,
                };
                return tyhir::TyHirExpr {
                    ty,
                    kind: tyhir::TyHirExprKind::StructLiteral {
                        def_id: lit_def,
                        fields,
                        turbofish: Vec::new(),
                    },
                };
            }

            tyhir::TyHirExprKind::ArrayLiteral { elements, repeat } => {
                tyhir::TyHirExprKind::ArrayLiteral {
                    elements: elements
                        .iter()
                        .map(|e| self.mono_expr(e, generics, targs, cargs, const_map))
                        .collect(),
                    repeat: *repeat,
                }
            }

            tyhir::TyHirExprKind::Index { array, index } => tyhir::TyHirExprKind::Index {
                array: Box::new(self.mono_expr(array, generics, targs, cargs, const_map)),
                index: Box::new(self.mono_expr(index, generics, targs, cargs, const_map)),
            },

            tyhir::TyHirExprKind::AddressOf(inner) => tyhir::TyHirExprKind::AddressOf(Box::new(
                self.mono_expr(inner, generics, targs, cargs, const_map),
            )),

            tyhir::TyHirExprKind::Deref(inner) => tyhir::TyHirExprKind::Deref(Box::new(
                self.mono_expr(inner, generics, targs, cargs, const_map),
            )),

            tyhir::TyHirExprKind::Variant {
                def_id,
                variant,
                args,
                turbofish,
            } => tyhir::TyHirExprKind::Variant {
                def_id: *def_id,
                variant: variant.clone(),
                args: args
                    .iter()
                    .map(|a| self.mono_expr(a, generics, targs, cargs, const_map))
                    .collect(),
                turbofish: turbofish.clone(),
            },

            tyhir::TyHirExprKind::Match { scrutinee, arms } => tyhir::TyHirExprKind::Match {
                scrutinee: Box::new(self.mono_expr(scrutinee, generics, targs, cargs, const_map)),
                arms: arms
                    .iter()
                    .map(|a| tyhir::TyHirMatchArm {
                        pattern: a.pattern.clone(),
                        body: self.mono_body(&a.body, generics, targs, cargs, const_map),
                    })
                    .collect(),
            },

            tyhir::TyHirExprKind::TraitCast { value, trait_def } => {
                tyhir::TyHirExprKind::TraitCast {
                    value: Box::new(self.mono_expr(value, generics, targs, cargs, const_map)),
                    trait_def: *trait_def,
                }
            }

            tyhir::TyHirExprKind::DynamicMethodCall {
                trait_def,
                method_index,
                receiver,
                args,
            } => tyhir::TyHirExprKind::DynamicMethodCall {
                trait_def: *trait_def,
                method_index: *method_index,
                receiver: Box::new(self.mono_expr(receiver, generics, targs, cargs, const_map)),
                args: args
                    .iter()
                    .map(|a| self.mono_expr(a, generics, targs, cargs, const_map))
                    .collect(),
            },
        };
        tyhir::TyHirExpr { ty, kind }
    }

    /// 单态化一个类型：先 [`TypeChecker::subst_type`] 代入泛型实参，再把「已完全具体化的」
    /// 泛型应用类型（`Generic`）解算为其结构体实例的 [`HirType::Named`]。
    /// 对于仍含未代入 `Var` 的类型（处在泛型模板内部时），保留原样交给外层实例化。
    fn mono_type(
        &mut self,
        ty: &HirType,
        generics: &[GenericParam],
        targs: &[HirType],
        cargs: &[i64],
    ) -> HirType {
        let subst = TypeChecker::subst_type(ty, generics, targs, &const_args_view(cargs));
        if let HirType::Generic(gdef, gtargs, gcargs) = &subst {
            // 仅当全部实参都已具体（无残留 Var / Param）时才实例化；否则保留 Generic 供外层处理。
            let all_concrete = gtargs.iter().all(|t| !matches!(t, HirType::Var(_)))
                && gcargs.iter().all(|c| matches!(c, ConstArg::Literal(_)));
            if all_concrete {
                let new_def = self.instantiate_struct(*gdef, gtargs, &const_args_to_i64(gcargs));
                return HirType::Named(new_def);
            }
        }
        subst
    }

    /// 单态化一个函数调用：若被调用者是泛型函数，推断实参并实例化，
    /// 把 `callee` 改写为实例 DefId。返回完整的 `TyHirExpr`（含代入后的返回类型）。
    #[allow(clippy::too_many_arguments)]
    fn mono_call(
        &mut self,
        callee: DefId,
        args: &[tyhir::TyHirExpr],
        turbofish: &[TypeOrConst],
        ret_ty: &HirType,
        generics: &[GenericParam],
        targs: &[HirType],
        cargs: &[i64],
        const_map: &HashMap<DefId, i64>,
    ) -> tyhir::TyHirExpr {
        // 先递归单态化实参（其中可能实例化其它泛型）。
        let mono_args: Vec<tyhir::TyHirExpr> = args
            .iter()
            .map(|a| self.mono_expr(a, generics, targs, cargs, const_map))
            .collect();

        if self.fn_templates.contains_key(&callee) {
            // 泛型函数：推断类型 / 常量实参（先取下模板的泛型声明，避免与后续可变借用冲突）。
            let tg = self.fn_templates.get(&callee).unwrap().generics.clone();
            let tparams = self.fn_templates.get(&callee).unwrap().params.clone();
            let trecv = self
                .fn_templates
                .get(&callee)
                .unwrap()
                .impl_receiver
                .clone();
            let (itargs, icargs) =
                self.infer_call_args(&tg, &tparams, &trecv, &mono_args, turbofish);
            // 涡轮鱼里的常量参数名引用（`ConstParam`）需按外层实例的具体 `cargs` 解算为整型，
            // 才能用于实例键与 `const_map`。
            let icargs_i64: Vec<i64> = icargs
                .iter()
                .map(|c| match c {
                    ConstArg::Literal(v) => *v,
                    ConstArg::Param(i) => cargs[const_slot(generics, *i)],
                })
                .collect();
            let new_def = self.instantiate_fn(callee, &itargs, &icargs_i64);
            let rty = self.mono_type(ret_ty, &tg, &itargs, &icargs_i64);
            return tyhir::TyHirExpr {
                ty: rty,
                kind: tyhir::TyHirExprKind::Call {
                    callee: new_def,
                    args: mono_args,
                    turbofish: Vec::new(),
                },
            };
        }

        tyhir::TyHirExpr {
            ty: self.mono_type(ret_ty, generics, targs, cargs),
            kind: tyhir::TyHirExprKind::Call {
                callee,
                args: mono_args,
                turbofish: Vec::new(),
            },
        }
    }

    /// 从涡轮鱼 + 位置实参（含接收者）推断泛型函数的类型 / 常量实参。
    fn infer_call_args(
        &self,
        generics: &[GenericParam],
        params: &[tyhir::TyHirParam],
        impl_receiver: &Option<HirType>,
        args: &[tyhir::TyHirExpr],
        turbofish: &[TypeOrConst],
    ) -> (Vec<HirType>, Vec<ConstArg>) {
        let n_type = generics
            .iter()
            .filter(|g| matches!(g.kind, GenericParamKind::Type))
            .count();
        let n_const = generics
            .iter()
            .filter(|g| matches!(g.kind, GenericParamKind::Const))
            .count();
        let mut targs: Vec<Option<HirType>> = vec![None; n_type];
        let mut cargs: Vec<Option<ConstArg>> = vec![None; n_const];

        // 1) 涡轮鱼（按泛型参数声明顺序）。
        for (k, a) in turbofish.iter().enumerate() {
            if k >= generics.len() {
                fatal(MplangError::type_error(format!(
                    "泛型实参过多（期望 {} 个）",
                    generics.len()
                )));
            }
            match (&generics[k].kind, a) {
                (GenericParamKind::Type, TypeOrConst::Type(t)) => {
                    targs[type_slot(generics, k)] = Some(t.clone());
                }
                (GenericParamKind::Const, TypeOrConst::Const(v)) => {
                    cargs[const_slot(generics, k)] = Some(ConstArg::Literal(*v));
                }
                (GenericParamKind::Const, TypeOrConst::ConstParam(i)) => {
                    cargs[const_slot(generics, k)] = Some(ConstArg::Param(*i));
                }
                _ => fatal(MplangError::type_error(
                    "泛型实参种类与声明不符（类型参数需要类型，常量参数需要整型）",
                )),
            }
        }

        // 2) 从位置实参类型推断（跳过常量参数占位）。
        let mut arg_idx = 0;
        for p in params {
            if matches!(p.kind, ParamKind::ConstParam(_)) {
                continue;
            }
            if arg_idx < args.len() {
                self.unify(&p.ty, &args[arg_idx].ty, generics, &mut targs);
            }
            arg_idx += 1;
        }

        // 3) 方法接收者类型推断（`impl<T>` 等方法）。
        if let Some(recv_ty) = impl_receiver
            && !args.is_empty()
        {
            self.unify(recv_ty, &args[0].ty, generics, &mut targs);
        }

        // 4) 校验全部实参已就位。
        for (i, g) in generics.iter().enumerate() {
            match g.kind {
                GenericParamKind::Type => {
                    if targs[type_slot(generics, i)].is_none() {
                        fatal(MplangError::type_error(format!(
                            "无法推断类型参数 `{}`，请用涡轮鱼 `::<...>` 显式指定",
                            g.name
                        )));
                    }
                }
                GenericParamKind::Const => {
                    if cargs[const_slot(generics, i)].is_none() {
                        fatal(MplangError::type_error(format!(
                            "常量参数 `{}` 必须由涡轮鱼 `::<...>` 显式指定",
                            g.name
                        )));
                    }
                }
            }
        }

        (
            targs.into_iter().map(|o| o.unwrap()).collect(),
            cargs.into_iter().map(|o| o.unwrap()).collect(),
        )
    }
}

/// 把 `cargs: &[i64]`（具体整型常量实参）包装为 [`subst_type`] 所需的 `&[ConstArg]`。
fn const_args_view(cargs: &[i64]) -> Vec<ConstArg> {
    cargs.iter().map(|v| ConstArg::Literal(*v)).collect()
}

/// 把 [`ConstArg`] 序列（单态化后均为 `Literal`）展开为具体 `i64` 向量。
fn const_args_to_i64(cargs: &[ConstArg]) -> Vec<i64> {
    cargs
        .iter()
        .map(|c| match c {
            ConstArg::Literal(v) => *v,
            ConstArg::Param(_) => {
                panic!("内部错误：单态化后不应残留常量参数引用")
            }
        })
        .collect()
}

/// 类型参数 `generics[k]`（种类为 Type）在 `targs` 向量中的下标。
fn type_slot(generics: &[GenericParam], k: usize) -> usize {
    generics[0..k]
        .iter()
        .filter(|g| matches!(g.kind, GenericParamKind::Type))
        .count()
}

/// 常量参数 `generics[k]`（种类为 Const）在 `cargs` 向量中的下标。
fn const_slot(generics: &[GenericParam], k: usize) -> usize {
    generics[0..k]
        .iter()
        .filter(|g| matches!(g.kind, GenericParamKind::Const))
        .count()
}

/// 把「期望类型」中的 `Var` 占位符按与「实际类型」的对应关系填入 `targs`。
/// 同时处理 `Generic` 期望与实例化后 `Named` 实际（通过 `instance_structs` 反查实参），
/// 从而支持 `impl<T>` 等方法按接收者类型推断类型参数。
impl Mono {
    /// 把「期望类型」中的 `Var` 占位符按与「实际类型」的对应关系填入 `targs`。
    /// 同时处理 `Generic` 期望与实例化后 `Named` 实际（通过 `instance_structs` 反查实参），
    /// 从而支持 `impl<T>` 等方法按接收者类型推断类型参数。
    fn unify(
        &self,
        expected: &HirType,
        actual: &HirType,
        generics: &[GenericParam],
        targs: &mut [Option<HirType>],
    ) {
        match expected {
            HirType::Var(k) => {
                let slot = type_slot(generics, *k);
                if targs[slot].is_none() {
                    targs[slot] = Some(actual.clone());
                } else if targs[slot].as_ref() != Some(actual) {
                    fatal(MplangError::type_error(
                        "泛型类型参数推断冲突（同一类型参数被推导为不同的具体类型）",
                    ));
                }
            }
            HirType::Generic(_, eta, _) => {
                // 实际可能是实例化后的 `Named`（反查其实参）或同为 `Generic`。
                let (ata, aca) = if let HirType::Named(ad) = actual {
                    if let Some(info) = self.instance_structs.get(ad) {
                        (info.targs.clone(), info.cargs.clone())
                    } else {
                        return;
                    }
                } else if let HirType::Generic(_, at, ac) = actual {
                    (at.clone(), ac.clone())
                } else {
                    return;
                };
                for (e, a) in eta.iter().zip(ata.iter()) {
                    self.unify(e, a, generics, targs);
                }
                let _ = aca;
            }
            HirType::Array(e1, _) => {
                if let HirType::Array(e2, _) = actual {
                    self.unify(e1, e2, generics, targs);
                }
            }
            HirType::Pointer(e1) => {
                if let HirType::Pointer(e2) = actual {
                    self.unify(e1, e2, generics, targs);
                }
            }
            HirType::Enum(d1, t1, _) => {
                if let HirType::Enum(d2, t2, _) = actual
                    && d1 == d2
                {
                    for (x, y) in t1.iter().zip(t2.iter()) {
                        self.unify(x, y, generics, targs);
                    }
                }
            }
            _ => {}
        }
    }
}

/// 收集单元内所有出现过的 [`DefId`]，用于确定新分配的起始编号。
fn collect_ids(m: &tyhir::TyHirModule, max: &mut u32) {
    if m.def_id.0 > *max {
        *max = m.def_id.0;
    }
    for item in &m.items {
        match item {
            tyhir::TyHirItem::Module(sub) => collect_ids(sub, max),
            tyhir::TyHirItem::Fn {
                def_id,
                params,
                body,
                ..
            } => {
                if def_id.0 > *max {
                    *max = def_id.0;
                }
                for p in params {
                    if p.def_id.0 > *max {
                        *max = p.def_id.0;
                    }
                }
                collect_ids_body(body, max);
            }
            tyhir::TyHirItem::Struct { def_id, fields, .. } => {
                if def_id.0 > *max {
                    *max = def_id.0;
                }
                for f in fields {
                    if f.def_id.0 > *max {
                        *max = f.def_id.0;
                    }
                }
            }
            tyhir::TyHirItem::Static { def_id, init, .. } => {
                if def_id.0 > *max {
                    *max = def_id.0;
                }
                collect_ids_expr(init, max);
            }
            _ => {}
        }
    }
}

fn collect_ids_body(b: &tyhir::TyHirBody, max: &mut u32) {
    for s in &b.stmts {
        match s {
            tyhir::TyHirStmt::Let { def_id, init, .. } => {
                if def_id.0 > *max {
                    *max = def_id.0;
                }
                collect_ids_expr(init, max);
            }
            tyhir::TyHirStmt::Assign { target, value } => {
                collect_ids_expr(target, max);
                collect_ids_expr(value, max);
            }
            tyhir::TyHirStmt::Expr(e) => collect_ids_expr(e, max),
            tyhir::TyHirStmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                collect_ids_expr(cond, max);
                collect_ids_body(then_branch, max);
                if let Some(b) = else_branch {
                    collect_ids_body(b, max);
                }
            }
            tyhir::TyHirStmt::While { cond, body } => {
                collect_ids_expr(cond, max);
                collect_ids_body(body, max);
            }
            tyhir::TyHirStmt::Return(e) => {
                if let Some(x) = e {
                    collect_ids_expr(x, max);
                }
            }
        }
    }
}

fn collect_ids_pattern(p: &tyhir::TyHirPattern, max: &mut u32) {
    match p {
        tyhir::TyHirPattern::Variant { bindings, .. } => {
            for (bdef, _) in bindings {
                if bdef.0 > *max {
                    *max = bdef.0;
                }
            }
        }
        tyhir::TyHirPattern::Ident(d) => {
            if d.0 > *max {
                *max = d.0;
            }
        }
        tyhir::TyHirPattern::Wildcard | tyhir::TyHirPattern::Literal(_) => {}
    }
}

fn collect_ids_expr(e: &tyhir::TyHirExpr, max: &mut u32) {
    match &e.kind {
        tyhir::TyHirExprKind::Path(d) => {
            if d.0 > *max {
                *max = d.0;
            }
        }
        tyhir::TyHirExprKind::Binary { lhs, rhs, .. } => {
            collect_ids_expr(lhs, max);
            collect_ids_expr(rhs, max);
        }
        tyhir::TyHirExprKind::Call { callee, args, .. } => {
            if callee.0 > *max {
                *max = callee.0;
            }
            for a in args {
                collect_ids_expr(a, max);
            }
        }
        tyhir::TyHirExprKind::FieldAccess { object, .. } => collect_ids_expr(object, max),
        tyhir::TyHirExprKind::StructLiteral { def_id, fields, .. } => {
            if def_id.0 > *max {
                *max = def_id.0;
            }
            for (_, fe) in fields {
                collect_ids_expr(fe, max);
            }
        }
        tyhir::TyHirExprKind::ArrayLiteral { elements, .. } => {
            for el in elements {
                collect_ids_expr(el, max);
            }
        }
        tyhir::TyHirExprKind::Index { array, index } => {
            collect_ids_expr(array, max);
            collect_ids_expr(index, max);
        }
        tyhir::TyHirExprKind::AddressOf(inner) | tyhir::TyHirExprKind::Deref(inner) => {
            collect_ids_expr(inner, max)
        }
        tyhir::TyHirExprKind::Variant { args, .. } => {
            for a in args {
                collect_ids_expr(a, max);
            }
        }
        tyhir::TyHirExprKind::Match { scrutinee, arms } => {
            collect_ids_expr(scrutinee, max);
            for a in arms {
                collect_ids_pattern(&a.pattern, max);
                collect_ids_body(&a.body, max);
            }
        }
        tyhir::TyHirExprKind::TraitCast { value, .. } => {
            collect_ids_expr(value, max);
        }
        tyhir::TyHirExprKind::DynamicMethodCall { receiver, args, .. } => {
            collect_ids_expr(receiver, max);
            for a in args {
                collect_ids_expr(a, max);
            }
        }
        _ => {}
    }
}
