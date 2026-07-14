//! # 类型检查 —— HIR → TYPE HIR (TyHIR)
//!
//! 这一趟遍历 [`crate::hir`]（HIR），产出 [`crate::tyhir`]（TYPE HIR）。
//! 在此期间完成：
//!
//! 1. **类型推导**：为每一个表达式算出它的 [`crate::hir::HirType`]，
//!    并以 `TyHirExpr.ty` 的形式携带下去，下游代码生成可直接使用。
//! 2. **字段名解析**：把 HIR 中遗留的字段名字符串，依据「对象类型」
//!    解析为对应字段的 [`crate::hir::DefId`]
//!    （[`crate::tyhir::TyHirExprKind::FieldAccess`] /
//!    [`crate::tyhir::TyHirExprKind::StructLiteral`]）。
//! 3. **类型校验**：检查二元操作数、赋值、返回、调用参数等的类型一致性，
//!    不一致时通过 [`crate::error::fatal`] 抛出 [`crate::error::MplangError`]，
//!    由其入口 [`check`](TypeChecker::check) 处的 [`crate::error::into_result`]
//!    收拢为 `Result`，从而给出清晰、可定位的中文报错。
//!
//! 类型检查是「信任边界」：通过之后，TyHIR 一定类型良好，
//! 代码生成阶段可以放心地照着生成机器码。

use std::collections::HashMap;

use crate::ast::{BinOp, GenericParam, GenericParamKind, Literal};
use crate::error::{MplangError, fatal, into_result};
use crate::hir::{self, ArrayLen, ConstArg, DefId, HirType, ParamKind, TypeOrConst};

/// 结构体字段信息（用于按字段名解析出字段 [`DefId`]）。
#[derive(Clone)]
struct StructFieldInfo {
    name: String,
    def_id: DefId,
    ty: HirType,
}

/// 函数签名信息（用于调用校验）。`generics` 为空表示非泛型函数；否则 `param_types` /
/// `ret_ty` 中可能含有 [`HirType::Var`]（类型参数占位符），调用时需代入具体实参。
#[derive(Clone)]
struct FuncSig {
    generics: Vec<GenericParam>,
    param_types: Vec<HirType>,
    ret_ty: HirType,
    is_variadic: bool,
}

/// 方法表键：接收者类型 + 方法名。用于在类型检查阶段依据 `.` 左侧表达式的类型
/// 解析出对应的方法 [`DefId`](crate::hir::DefId)。
#[derive(Clone, PartialEq, Eq, Hash)]
struct MethodKey(HirType, String);

pub struct TypeChecker {
    /// 结构体定义 ID → 其字段列表。
    struct_fields: HashMap<DefId, Vec<StructFieldInfo>>,
    /// 结构体定义 ID → 其泛型参数声明（用于泛型结构体实例化时在类型检查阶段推断实参）。
    struct_generics: HashMap<DefId, Vec<GenericParam>>,
    /// 函数定义 ID → 签名。
    func_sigs: HashMap<DefId, FuncSig>,
    /// 函数定义 ID → 泛型参数声明（非泛型函数为空）。
    fn_generics: HashMap<DefId, Vec<GenericParam>>,
    /// 方法表：(接收者类型, 方法名) → 方法定义 ID。
    method_table: HashMap<MethodKey, DefId>,
    /// 所有「值」定义（参数 / let / 全局变量）的 ID → 类型。
    def_types: HashMap<DefId, HirType>,
    /// 所有定义的 ID → 名字（仅用于报错信息）。
    names: HashMap<DefId, String>,
    /// 枚举定义 ID → 变体信息（用于变体构造与 match 类型检查）。
    enum_variants: HashMap<DefId, Vec<EnumVariantInfo>>,
    /// match 表达式类型检查的临时缓存：check_enum_match/check_int_match 将已类型检查的
    /// 分支列表存入此处，由 check_expr 中的 HirExpr::Match 分支取走。
    typed_arms: Option<Vec<crate::tyhir::TyHirMatchArm>>,
    /// trait 定义 ID → (方法名, 返回类型) 列表。用于 DynamicMethodCall 的返回类型推断。
    #[allow(dead_code)]
    trait_methods: HashMap<DefId, Vec<(String, HirType)>>,
    /// 期望的变体构造结果类型（由 let 绑定等上下文设置，用于 `None` 等无参泛型变体的类型推断）。
    variant_expected_type: Option<HirType>,
}

/// 枚举变体的元信息（类型检查阶段使用）。
#[derive(Clone)]
struct EnumVariantInfo {
    name: String,
    tag: u32,
    /// 载荷字段（名称 → (DefId, 类型)）。
    fields: Vec<(String, DefId, HirType)>,
}

impl TypeChecker {
    /// 预扫描整棵 HIR 树，收集结构体字段、函数签名与全局/参数的类型。
    pub fn new(_hir: &hir::HirCompilationUnit) -> Self {
        TypeChecker {
            struct_fields: HashMap::new(),
            struct_generics: HashMap::new(),
            func_sigs: HashMap::new(),
            fn_generics: HashMap::new(),
            method_table: HashMap::new(),
            def_types: HashMap::new(),
            names: HashMap::new(),
            enum_variants: HashMap::new(),
            typed_arms: None,
            trait_methods: HashMap::new(),
            variant_expected_type: None,
        }
    }

    fn collect_module(&mut self, m: &hir::HirModule) {
        self.names.insert(m.def_id, m.name.clone());
        for item in &m.items {
            self.collect_item(item);
        }
    }

    fn collect_item(&mut self, item: &hir::HirItem) {
        match item {
            hir::HirItem::Module(m) => self.collect_module(m),
            hir::HirItem::Struct {
                def_id,
                name,
                generics,
                fields,
                ..
            } => {
                self.names.insert(*def_id, name.clone());
                self.struct_generics.insert(*def_id, generics.clone());
                let infos = fields
                    .iter()
                    .map(|f| {
                        self.names.insert(f.def_id, f.name.clone());
                        StructFieldInfo {
                            name: f.name.clone(),
                            def_id: f.def_id,
                            ty: f.ty.clone(),
                        }
                    })
                    .collect();
                self.struct_fields.insert(*def_id, infos);
            }
            hir::HirItem::Enum {
                def_id,
                name,
                generics,
                variants,
                ..
            } => {
                self.names.insert(*def_id, name.clone());
                self.struct_generics.insert(*def_id, generics.clone());
                let infos = variants
                    .iter()
                    .map(|v| EnumVariantInfo {
                        name: v.name.clone(),
                        tag: v.tag,
                        fields: v
                            .fields
                            .iter()
                            .map(|f| (f.name.clone(), f.def_id, f.ty.clone()))
                            .collect(),
                    })
                    .collect();
                self.enum_variants.insert(*def_id, infos);
            }
            hir::HirItem::ExternFn {
                def_id,
                name,
                param_types,
                return_ty,
                is_variadic,
                ..
            } => {
                self.names.insert(*def_id, name.clone());
                self.func_sigs.insert(
                    *def_id,
                    FuncSig {
                        generics: Vec::new(),
                        param_types: param_types.clone(),
                        ret_ty: return_ty.clone(),
                        is_variadic: *is_variadic,
                    },
                );
            }
            hir::HirItem::Fn {
                def_id,
                name,
                generics,
                params,
                return_ty,
                impl_receiver,
                ..
            } => {
                self.names.insert(*def_id, name.clone());
                self.fn_generics.insert(*def_id, generics.clone());
                // 函数签名只统计「值参数」：类型参数 / 常量参数不是调用方提供的实参，
                // 其（实参）来自涡轮鱼或在被调用处的类型上下文里推断。
                let param_types: Vec<HirType> = params
                    .iter()
                    .filter(|p| p.kind == ParamKind::Value)
                    .map(|p| p.ty.clone())
                    .collect();
                self.func_sigs.insert(
                    *def_id,
                    FuncSig {
                        generics: generics.clone(),
                        param_types,
                        ret_ty: return_ty.clone(),
                        is_variadic: false,
                    },
                );
                // `impl` 方法：登记 (接收者类型, 方法名) → 定义 ID，供 `.` 调用解析。
                if let Some(recv) = impl_receiver {
                    // 对于泛型 impl（如 `impl<T> Vec<T>`），接收者类型可能是 `Generic(def, [Var(..)])`。
                    // 对于 enum 的 impl（如 `impl<T> Option<T>`），接收者类型是 `Enum(def, [Var(..)])`。
                    // 统一用 `Named(def)` 作为键。
                    let key_ty = match &recv {
                        HirType::Generic(d, _, _) | HirType::Enum(d, _, _) => HirType::Named(*d),
                        _ => recv.clone(),
                    };
                    let key = MethodKey(key_ty, name.clone());
                    if self.method_table.contains_key(&key) {
                        fatal(MplangError::lowering(format!(
                            "类型 {:?} 重复为方法 `{}` 提供实现（一个类型上同一方法只能实现一次）",
                            recv, name
                        )));
                    }
                    self.method_table.insert(key, *def_id);
                }
            }
            hir::HirItem::Static {
                def_id, name, ty, ..
            } => {
                self.names.insert(*def_id, name.clone());
                self.def_types.insert(*def_id, ty.clone());
            }
        }
    }

    // ───────────────── 入口 ─────────────────

    pub fn check(
        &mut self,
        hir: &hir::HirCompilationUnit,
    ) -> Result<crate::tyhir::TyHirCompilationUnit, MplangError> {
        into_result(|| self.check_into(hir))
    }

    fn check_into(&mut self, hir: &hir::HirCompilationUnit) -> crate::tyhir::TyHirCompilationUnit {
        // 预扫描（收集结构体字段 / 函数签名 / 方法表）放到「受 `into_result`
        // 收拢的类型检查过程内」，以便其中的受控错误（`fatal`）能被正确转换为 `Result`，
        // 而非以裸 panic 逃逸（此前 `new` 中直接调用 collect 会导致重复方法等错误裸奔）。
        self.collect_module(&hir.root_module);
        let root = self.check_module(&hir.root_module);
        log::debug!("类型检查完成");
        crate::tyhir::TyHirCompilationUnit {
            root_module: root,
            vtables: hir.vtables.clone(),
        }
    }

    fn check_module(&mut self, m: &hir::HirModule) -> crate::tyhir::TyHirModule {
        let items = m.items.iter().map(|it| self.check_item(it)).collect();
        crate::tyhir::TyHirModule {
            def_id: m.def_id,
            name: m.name.clone(),
            visibility: m.visibility.clone(),
            attributes: m.attributes.clone(),
            items,
        }
    }

    fn check_item(&mut self, item: &hir::HirItem) -> crate::tyhir::TyHirItem {
        match item {
            hir::HirItem::Module(m) => crate::tyhir::TyHirItem::Module(self.check_module(m)),

            hir::HirItem::ExternFn {
                def_id,
                visibility,
                attributes,
                link_name,
                name,
                param_types,
                return_ty,
                is_variadic,
            } => crate::tyhir::TyHirItem::ExternFn {
                def_id: *def_id,
                visibility: visibility.clone(),
                attributes: attributes.clone(),
                link_name: link_name.clone(),
                name: name.clone(),
                param_types: param_types.clone(),
                return_ty: return_ty.clone(),
                is_variadic: *is_variadic,
            },

            hir::HirItem::Fn {
                def_id,
                visibility,
                attributes,
                name,
                generics,
                params,
                return_ty,
                body,
                impl_receiver,
                ..
            } => {
                // 登记参数类型（参数可作为值被引用）。
                for p in params {
                    self.def_types.insert(p.def_id, p.ty.clone());
                    self.names.insert(p.def_id, p.name.clone());
                }
                let body_ty = self.check_body(body, return_ty);
                crate::tyhir::TyHirItem::Fn {
                    def_id: *def_id,
                    visibility: visibility.clone(),
                    attributes: attributes.clone(),
                    name: name.clone(),
                    generics: generics.clone(),
                    params: params
                        .iter()
                        .map(|p| crate::tyhir::TyHirParam {
                            def_id: p.def_id,
                            name: p.name.clone(),
                            ty: p.ty.clone(),
                            kind: p.kind,
                        })
                        .collect(),
                    return_ty: return_ty.clone(),
                    body: body_ty,
                    impl_receiver: impl_receiver.clone(),
                }
            }

            hir::HirItem::Struct {
                def_id,
                visibility,
                attributes,
                name,
                generics,
                fields,
                ..
            } => crate::tyhir::TyHirItem::Struct {
                def_id: *def_id,
                visibility: visibility.clone(),
                attributes: attributes.clone(),
                name: name.clone(),
                generics: generics.clone(),
                fields: fields
                    .iter()
                    .map(|f| crate::tyhir::TyHirField {
                        def_id: f.def_id,
                        name: f.name.clone(),
                        ty: f.ty.clone(),
                        visibility: f.visibility.clone(),
                    })
                    .collect(),
            },

            hir::HirItem::Static {
                def_id,
                visibility,
                attributes,
                name,
                ty,
                init,
                is_const,
            } => {
                let init_ty = self.check_expr(init);
                self.assert_ty(&init_ty.ty, ty, &format!("全局变量 `{}` 初始化", name));
                crate::tyhir::TyHirItem::Static {
                    def_id: *def_id,
                    visibility: visibility.clone(),
                    attributes: attributes.clone(),
                    name: name.clone(),
                    ty: ty.clone(),
                    init: init_ty,
                    is_const: *is_const,
                }
            }

            hir::HirItem::Enum {
                def_id,
                visibility,
                attributes,
                name,
                generics,
                variants,
            } => {
                // 枚举的类型检查在 Step 2 中实现；此处仅传递结构。
                crate::tyhir::TyHirItem::Enum {
                    def_id: *def_id,
                    visibility: visibility.clone(),
                    attributes: attributes.clone(),
                    name: name.clone(),
                    generics: generics.clone(),
                    variants: variants
                        .iter()
                        .map(|v| crate::tyhir::TyHirEnumVariant {
                            def_id: v.def_id,
                            name: v.name.clone(),
                            tag: v.tag,
                            fields: v
                                .fields
                                .iter()
                                .map(|f| crate::tyhir::TyHirField {
                                    def_id: f.def_id,
                                    name: f.name.clone(),
                                    ty: f.ty.clone(),
                                    visibility: f.visibility.clone(),
                                })
                                .collect(),
                        })
                        .collect(),
                }
            }
        }
    }

    fn check_body(&mut self, body: &hir::HirBody, ret_ty: &HirType) -> crate::tyhir::TyHirBody {
        crate::tyhir::TyHirBody {
            stmts: body
                .stmts
                .iter()
                .map(|s| self.check_stmt(s, ret_ty))
                .collect(),
        }
    }

    fn check_stmt(&mut self, stmt: &hir::HirStmt, ret_ty: &HirType) -> crate::tyhir::TyHirStmt {
        match stmt {
            hir::HirStmt::Let {
                def_id,
                name,
                ty,
                init,
            } => {
                // 若有类型标注，将其设为期望的变体构造结果类型（用于 `None` 等无参泛型变体的类型推断）。
                if let Some(expected_ty) = ty {
                    self.variant_expected_type = Some(expected_ty.clone());
                }
                let init_ty = self.check_expr(init);
                self.variant_expected_type = None;
                // 检测 TraitCast 模式：let v: *Trait = &struct_expr;
                let init_ty = if let Some(HirType::TraitObject(trait_def)) = ty {
                    if let crate::tyhir::TyHirExprKind::AddressOf(inner) = &init_ty.kind {
                        if matches!(inner.ty, HirType::Named(_)) {
                            crate::tyhir::TyHirExpr {
                                ty: HirType::TraitObject(*trait_def),
                                kind: crate::tyhir::TyHirExprKind::TraitCast {
                                    value: inner.clone(),
                                    trait_def: *trait_def,
                                },
                            }
                        } else {
                            init_ty
                        }
                    } else {
                        init_ty
                    }
                } else {
                    init_ty
                };
                let binding_ty = match ty {
                    Some(t) => {
                        self.assert_ty(&init_ty.ty, t, &format!("let `{}` 类型标注", name));
                        t.clone()
                    }
                    None => init_ty.ty.clone(),
                };
                self.def_types.insert(*def_id, binding_ty.clone());
                crate::tyhir::TyHirStmt::Let {
                    def_id: *def_id,
                    name: name.clone(),
                    ty: binding_ty,
                    init: init_ty,
                }
            }

            hir::HirStmt::Assign { target, value } => {
                if !self.is_lvalue(target) {
                    fatal(MplangError::type_error("赋值目标必须是可修改的左值"));
                }
                let target_ty = self.check_expr(target);
                let value_ty = self.check_expr(value);
                self.assert_ty(&value_ty.ty, &target_ty.ty, "赋值（右值类型需与左值一致）");
                crate::tyhir::TyHirStmt::Assign {
                    target: target_ty,
                    value: value_ty,
                }
            }

            hir::HirStmt::Expr(e) => crate::tyhir::TyHirStmt::Expr(self.check_expr(e)),

            hir::HirStmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond_ty = self.check_expr(cond);
                self.assert_ty(&cond_ty.ty, &HirType::Int, "if 条件");
                crate::tyhir::TyHirStmt::If {
                    cond: cond_ty,
                    then_branch: self.check_body(then_branch, ret_ty),
                    else_branch: else_branch.as_ref().map(|b| self.check_body(b, ret_ty)),
                }
            }

            hir::HirStmt::While { cond, body } => {
                let cond_ty = self.check_expr(cond);
                self.assert_ty(&cond_ty.ty, &HirType::Int, "while 条件");
                crate::tyhir::TyHirStmt::While {
                    cond: cond_ty,
                    body: self.check_body(body, ret_ty),
                }
            }

            hir::HirStmt::Return(e) => crate::tyhir::TyHirStmt::Return(match e {
                Some(ex) => {
                    let te = self.check_expr(ex);
                    self.assert_ty(&te.ty, ret_ty, "return");
                    Some(te)
                }
                None => {
                    self.assert_ty(&HirType::Unit, ret_ty, "return");
                    None
                }
            }),
        }
    }

    fn check_expr(&mut self, expr: &hir::HirExpr) -> crate::tyhir::TyHirExpr {
        match expr {
            hir::HirExpr::Literal(l) => {
                let ty = match l {
                    Literal::Int(_) => HirType::Int,
                    // 字符串字面量是 `char*`（指向 char 的指针）。
                    Literal::String(_) => HirType::char_ptr(),
                };
                crate::tyhir::TyHirExpr {
                    ty,
                    kind: crate::tyhir::TyHirExprKind::Literal(l.clone()),
                }
            }

            hir::HirExpr::Path(def_id) => {
                // 若该 DefId 是某个枚举的单元变体构造器（如 `None`），当作无参 Variant 处理。
                if let Some((enum_def, variant)) = self.find_variant_by_def_id(*def_id) {
                    return self.check_expr(&hir::HirExpr::Variant {
                        def_id: enum_def,
                        variant,
                        args: Vec::new(),
                        turbofish: Vec::new(),
                    });
                }
                let ty = if let Some(t) = self.def_types.get(def_id) {
                    t.clone()
                } else if self.func_sigs.contains_key(def_id) {
                    fatal(MplangError::type_error(format!(
                        "函数 `{}` 不能当作值使用",
                        self.name_of(def_id)
                    )));
                } else {
                    fatal(MplangError::other(format!(
                        "未找到定义 `{}` 的类型（内部错误）",
                        self.name_of(def_id)
                    )));
                };
                crate::tyhir::TyHirExpr {
                    ty,
                    kind: crate::tyhir::TyHirExprKind::Path(*def_id),
                }
            }

            hir::HirExpr::Binary { op, lhs, rhs } => {
                let l = self.check_expr(lhs);
                let r = self.check_expr(rhs);
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                        self.assert_ty(&l.ty, &HirType::Int, "算术运算左操作数");
                        self.assert_ty(&r.ty, &HirType::Int, "算术运算右操作数");
                    }
                    BinOp::Less
                    | BinOp::LessEqual
                    | BinOp::Equal
                    | BinOp::NotEqual
                    | BinOp::Greater
                    | BinOp::GreaterEqual => {
                        // 整数或同是指针均可比较（指针比较按地址）。
                        let int_cmp = matches!(l.ty, HirType::Int) && matches!(r.ty, HirType::Int);
                        let ptr_cmp = matches!(l.ty, HirType::Pointer(_))
                            && matches!(r.ty, HirType::Pointer(_));
                        if !(int_cmp || ptr_cmp) {
                            fatal(MplangError::type_error(
                                "比较运算的两个操作数必须同为 int 或同为指针",
                            ));
                        }
                    }
                }
                crate::tyhir::TyHirExpr {
                    ty: HirType::Int,
                    kind: crate::tyhir::TyHirExprKind::Binary {
                        op: *op,
                        lhs: Box::new(l),
                        rhs: Box::new(r),
                    },
                }
            }

            hir::HirExpr::MethodCall { object, name, args } => {
                // 先算出接收者类型
                let obj = self.check_expr(object);

                // 若接收者是 trait 对象，产生动态方法调用
                if let HirType::TraitObject(trait_def) = &obj.ty {
                    // 对 trait 对象的方法调用：动态分发
                    // v1 简化：方法索引固定为 0，返回类型固定为 Int
                    let typed_args: Vec<crate::tyhir::TyHirExpr> =
                        args.iter().map(|a| self.check_expr(a)).collect();
                    let _ = name;
                    return crate::tyhir::TyHirExpr {
                        ty: HirType::Int,
                        kind: crate::tyhir::TyHirExprKind::DynamicMethodCall {
                            trait_def: *trait_def,
                            method_index: 0,
                            receiver: Box::new(obj),
                            args: typed_args,
                        },
                    };
                }

                let lookup_ty = match &obj.ty {
                    HirType::Generic(d, _, _) | HirType::Enum(d, _, _) => HirType::Named(*d),
                    _ => obj.ty.clone(),
                };
                let callee = self
                    .method_table
                    .get(&MethodKey(lookup_ty, name.clone()))
                    .cloned()
                    .unwrap_or_else(|| {
                        fatal(MplangError::type_error(format!(
                            "类型 {:?} 没有方法 `{}`",
                            obj.ty, name
                        )))
                    });
                let sig = self.func_sigs.get(&callee).cloned().unwrap_or_else(|| {
                    fatal(MplangError::type_error(format!(
                        "调用了未定义的方法 `{}`",
                        name
                    )))
                });
                // 调用约定：接收者 `self` 是方法的第 0 个参数，用户实参从 1 开始对应。
                let expected_user = sig.param_types.len() - 1;
                if args.len() != expected_user {
                    fatal(MplangError::type_error(format!(
                        "方法 `{}` 需要 {} 个参数，实际给了 {}",
                        name,
                        expected_user,
                        args.len()
                    )));
                }
                // 对接收者类型做结构级比较（支持 Generic/Enum(def, targs) vs Generic/Enum(def, [Var(..)]) 匹配）
                match (&obj.ty, &sig.param_types[0]) {
                    (HirType::Generic(d1, _, _), HirType::Generic(d2, _, _)) if d1 == d2 => {}
                    (HirType::Enum(d1, _, _), HirType::Enum(d2, _, _)) if d1 == d2 => {}
                    (HirType::Named(d1), HirType::Named(d2)) if d1 == d2 => {}
                    _ => self.assert_ty(
                        &obj.ty,
                        &sig.param_types[0],
                        &format!("方法 `{}` 的接收者类型", name),
                    ),
                }

                // 接收者作为首个实参，紧随其后为用户实参。
                // 若方法来自泛型 impl，需要把 self 的泛型实参代入 param_types 和 ret_ty。
                let (subst_params, subst_ret_ty) =
                    if let HirType::Generic(_def_id, targs, cargs) = &obj.ty {
                        let subst_params: Vec<HirType> = sig
                            .param_types
                            .iter()
                            .map(|pt| Self::subst_type(pt, &sig.generics, targs, cargs))
                            .collect();
                        let subst_ret = Self::subst_type(&sig.ret_ty, &sig.generics, targs, cargs);
                        (subst_params, subst_ret)
                    } else if let HirType::Enum(_def_id, targs, cargs) = &obj.ty {
                        let subst_params: Vec<HirType> = sig
                            .param_types
                            .iter()
                            .map(|pt| Self::subst_type(pt, &sig.generics, targs, cargs))
                            .collect();
                        let subst_ret = Self::subst_type(&sig.ret_ty, &sig.generics, targs, cargs);
                        (subst_params, subst_ret)
                    } else {
                        (sig.param_types.clone(), sig.ret_ty.clone())
                    };

                let mut typed_args: Vec<crate::tyhir::TyHirExpr> =
                    Vec::with_capacity(args.len() + 1);
                typed_args.push(obj);
                for (i, arg) in args.iter().enumerate() {
                    let te = self.check_expr(arg);
                    self.assert_ty(
                        &te.ty,
                        &subst_params[i + 1],
                        &format!("方法 `{}` 第 {} 个参数", name, i),
                    );
                    typed_args.push(te);
                }

                crate::tyhir::TyHirExpr {
                    ty: subst_ret_ty,
                    kind: crate::tyhir::TyHirExprKind::Call {
                        callee,
                        args: typed_args,
                        turbofish: Vec::new(),
                    },
                }
            }

            hir::HirExpr::Call {
                callee,
                args,
                turbofish,
            } => {
                let sig = self.func_sigs.get(callee).cloned().unwrap_or_else(|| {
                    fatal(MplangError::type_error(format!(
                        "调用了未定义的函数 `{}`",
                        self.name_of(callee)
                    )))
                });
                if sig.is_variadic {
                    if args.len() < sig.param_types.len() {
                        fatal(MplangError::type_error(format!(
                            "可变参数函数 `{}` 至少需要 {} 个参数，实际给了 {}",
                            self.name_of(callee),
                            sig.param_types.len(),
                            args.len()
                        )));
                    }
                } else if args.len() != sig.param_types.len() {
                    fatal(MplangError::type_error(format!(
                        "函数 `{}` 需要 {} 个参数，实际给了 {}",
                        self.name_of(callee),
                        sig.param_types.len(),
                        args.len()
                    )));
                }

                // 先逐个检查实参，得到带类型的实参列表；再统一代入泛型形参 / 返回类型。
                let typed_args: Vec<crate::tyhir::TyHirExpr> =
                    args.iter().map(|a| self.check_expr(a)).collect();

                // 泛型函数：把 `param_types` / `ret_ty` 中的 `Var` 占位符按实参代入（拒绝鸭子类型）。
                if sig.generics.is_empty() {
                    for (i, te) in typed_args.iter().enumerate() {
                        if i < sig.param_types.len() {
                            self.assert_ty(
                                &te.ty,
                                &sig.param_types[i],
                                &format!("函数 `{}` 第 {} 个参数", self.name_of(callee), i),
                            );
                        }
                    }
                } else {
                    let (subst_params, subst_ret) = self.instantiate_fn_sig(
                        &sig.generics,
                        &sig.param_types,
                        &sig.ret_ty,
                        turbofish,
                        &typed_args,
                    );
                    for (i, te) in typed_args.iter().enumerate() {
                        if i < subst_params.len() {
                            self.assert_ty(
                                &te.ty,
                                &subst_params[i],
                                &format!("函数 `{}` 第 {} 个参数", self.name_of(callee), i),
                            );
                        }
                    }
                    let call_ty = subst_ret;
                    return crate::tyhir::TyHirExpr {
                        ty: call_ty,
                        kind: crate::tyhir::TyHirExprKind::Call {
                            callee: *callee,
                            args: typed_args,
                            turbofish: turbofish.clone(),
                        },
                    };
                }

                crate::tyhir::TyHirExpr {
                    ty: sig.ret_ty.clone(),
                    kind: crate::tyhir::TyHirExprKind::Call {
                        callee: *callee,
                        args: typed_args,
                        turbofish: turbofish.clone(),
                    },
                }
            }

            hir::HirExpr::FieldAccess { object, field } => {
                let obj = self.check_expr(object);
                // 接收者可能是普通结构体 `Named`，也可能是一个泛型「应用类型」`Generic`。
                let (struct_def, sub) = match &obj.ty {
                    HirType::Named(d) => (*d, None),
                    HirType::Generic(d, targs, cargs) => (*d, Some((targs.clone(), cargs.clone()))),
                    other => fatal(MplangError::type_error(format!(
                        "对非结构体类型 {:?} 进行字段访问 `.{}`",
                        other, field
                    ))),
                };
                let info = self.find_field(struct_def, field).unwrap_or_else(|| {
                    fatal(MplangError::type_error(format!(
                        "结构体 `{}` 没有字段 `{}`",
                        self.name_of(&struct_def),
                        field
                    )))
                });
                // 字段类型若引用了类型参数（Var），用泛型实参代入得到具体类型。
                let field_ty = match &sub {
                    Some((targs, cargs)) => {
                        let gs = self
                            .struct_generics
                            .get(&struct_def)
                            .cloned()
                            .unwrap_or_default();
                        Self::subst_type(&info.ty, &gs, targs, cargs)
                    }
                    None => info.ty.clone(),
                };
                crate::tyhir::TyHirExpr {
                    ty: field_ty,
                    kind: crate::tyhir::TyHirExprKind::FieldAccess {
                        object: Box::new(obj),
                        field: info.def_id,
                    },
                }
            }

            hir::HirExpr::StructLiteral {
                def_id,
                fields,
                turbofish,
            } => {
                let infos = self.struct_fields.get(def_id).cloned().unwrap_or_else(|| {
                    fatal(MplangError::type_error(format!(
                        "未定义的结构体 `{}`",
                        self.name_of(def_id)
                    )))
                });
                // 先逐个检查字段初始化表达式，拿到其类型（供泛型实参推断）。
                let checked: Vec<(&str, crate::tyhir::TyHirExpr)> = fields
                    .iter()
                    .map(|(fname, fe)| (fname.as_str(), self.check_expr(fe)))
                    .collect();

                // 泛型结构体：计算实参（类型 / 常量），产生「应用类型」作为整体类型。
                if let Some(generics) = self.struct_generics.get(def_id)
                    && !generics.is_empty()
                {
                    let (targs, cargs) =
                        self.infer_struct_args(def_id, generics, turbofish, &infos, &checked);
                    let mut resolved: Vec<(DefId, crate::tyhir::TyHirExpr)> =
                        Vec::with_capacity(fields.len());
                    for ((fname, te), info) in checked.iter().zip(infos.iter()) {
                        let expect = Self::subst_type(&info.ty, generics, &targs, &cargs);
                        self.assert_ty(
                            &te.ty,
                            &expect,
                            &format!("结构体 `{}` 字段 `{}`", self.name_of(def_id), fname),
                        );
                        resolved.push((info.def_id, te.clone()));
                    }
                    let ty = HirType::Generic(*def_id, targs, cargs);
                    return crate::tyhir::TyHirExpr {
                        ty,
                        kind: crate::tyhir::TyHirExprKind::StructLiteral {
                            def_id: *def_id,
                            fields: resolved,
                            turbofish: turbofish.clone(),
                        },
                    };
                }

                let mut resolved: Vec<(DefId, crate::tyhir::TyHirExpr)> =
                    Vec::with_capacity(fields.len());
                for (info, (_fname, te)) in infos.iter().zip(checked.iter()) {
                    self.assert_ty(
                        &te.ty,
                        &info.ty,
                        &format!("结构体 `{}` 字段 `{}`", self.name_of(def_id), _fname),
                    );
                    resolved.push((info.def_id, te.clone()));
                }

                crate::tyhir::TyHirExpr {
                    ty: HirType::Named(*def_id),
                    kind: crate::tyhir::TyHirExprKind::StructLiteral {
                        def_id: *def_id,
                        fields: resolved,
                        turbofish: turbofish.clone(),
                    },
                }
            }

            hir::HirExpr::ArrayLiteral { elements, repeat } => {
                // 推断元素类型：所有元素须类型一致（重复形式仅检查模板元素）。
                let mut elem_ty: Option<HirType> = None;
                let mut checked: Vec<crate::tyhir::TyHirExpr> = Vec::with_capacity(elements.len());
                for e in elements {
                    let te = self.check_expr(e);
                    match &elem_ty {
                        Some(et) => self.assert_ty(&te.ty, et, "数组字面量各元素类型须一致"),
                        None => elem_ty = Some(te.ty.clone()),
                    }
                    checked.push(te);
                }
                let elem_ty = elem_ty.unwrap_or_else(|| {
                    fatal(MplangError::type_error("数组字面量至少要有一个元素"))
                });
                let len = match repeat {
                    Some(n) => *n,
                    None => elements.len(),
                };
                let array_ty = HirType::Array(Box::new(elem_ty), ArrayLen::Known(len));
                crate::tyhir::TyHirExpr {
                    ty: array_ty,
                    kind: crate::tyhir::TyHirExprKind::ArrayLiteral {
                        elements: checked,
                        repeat: *repeat,
                    },
                }
            }

            hir::HirExpr::Index { array, index } => {
                let arr = self.check_expr(array);
                let elem_ty = match &arr.ty {
                    HirType::Array(e, _) => (**e).clone(),
                    other => fatal(MplangError::type_error(format!(
                        "对非数组类型 {:?} 进行下标访问 `[..]`",
                        other
                    ))),
                };
                let idx = self.check_expr(index);
                self.assert_ty(&idx.ty, &HirType::Int, "数组下标（须为 int）");
                crate::tyhir::TyHirExpr {
                    ty: elem_ty,
                    kind: crate::tyhir::TyHirExprKind::Index {
                        array: Box::new(arr),
                        index: Box::new(idx),
                    },
                }
            }

            hir::HirExpr::AddressOf(inner) => {
                let te = self.check_expr(inner);
                // 取地址的操作数必须是可寻址的左值。
                if !self.is_lvalue(inner) {
                    fatal(MplangError::type_error(
                        "& 的操作数必须是可寻址的左值（变量、全局或结构体字段）",
                    ));
                }
                let pointee = te.ty.clone();
                crate::tyhir::TyHirExpr {
                    ty: HirType::Pointer(Box::new(pointee)),
                    kind: crate::tyhir::TyHirExprKind::AddressOf(Box::new(te)),
                }
            }

            hir::HirExpr::Deref(inner) => {
                let te = self.check_expr(inner);
                let pointee = match &te.ty {
                    HirType::Pointer(p) => (**p).clone(),
                    other => fatal(MplangError::type_error(format!(
                        "解引用 * 需要指针类型，实际为 {:?}",
                        other
                    ))),
                };
                crate::tyhir::TyHirExpr {
                    ty: pointee,
                    kind: crate::tyhir::TyHirExprKind::Deref(Box::new(te)),
                }
            }

            hir::HirExpr::Variant {
                def_id,
                variant,
                args,
                turbofish,
            } => {
                let infos = self.enum_variants.get(def_id).cloned().unwrap_or_else(|| {
                    fatal(MplangError::type_error(format!(
                        "未定义的枚举 `{}`",
                        self.name_of(def_id)
                    )))
                });
                let var_info = infos
                    .iter()
                    .find(|v| v.name == *variant)
                    .unwrap_or_else(|| {
                        fatal(MplangError::type_error(format!(
                            "枚举 `{}` 没有变体 `{}`",
                            self.name_of(def_id),
                            variant
                        )))
                    });
                if args.len() != var_info.fields.len() {
                    fatal(MplangError::type_error(format!(
                        "变体 `{}::{}` 需要 {} 个参数，实际给了 {}",
                        self.name_of(def_id),
                        variant,
                        var_info.fields.len(),
                        args.len()
                    )));
                }
                // 泛型枚举：按字段实参推断类型参数；若无法推断则尝试涡轮鱼
                let generics = self
                    .struct_generics
                    .get(def_id)
                    .cloned()
                    .unwrap_or_default();
                let mut targs: Vec<Option<HirType>> = generics.iter().map(|_| None).collect();
                let mut typed_args = Vec::with_capacity(args.len());
                for (_i, (arg, (_, _, fty))) in args.iter().zip(var_info.fields.iter()).enumerate()
                {
                    let ta = self.check_expr(arg);
                    // 用 unify_ty 来推断 Var 占位符
                    self.unify_ty(fty, &ta.ty, &generics, &mut targs);
                    typed_args.push(ta);
                }
                // 尝试从 variant_expected_type 推断未填充的类型参数
                for (i, t) in targs.iter_mut().enumerate() {
                    if t.is_some() {
                        continue;
                    }
                    if let Some(HirType::Enum(_, expected_targs, _)) = &self.variant_expected_type
                        && let Some(et) = expected_targs.get(i)
                    {
                        *t = Some(et.clone());
                    }
                }
                // 将推断结果转为具体实参列表
                let concrete_targs: Vec<HirType> = targs
                    .into_iter()
                    .map(|o| {
                        o.unwrap_or_else(|| {
                            fatal(MplangError::type_error(format!(
                                "无法推断枚举 `{}` 的泛型参数",
                                self.name_of(def_id)
                            )))
                        })
                    })
                    .collect();
                crate::tyhir::TyHirExpr {
                    ty: HirType::Enum(*def_id, concrete_targs, Vec::new()),
                    kind: crate::tyhir::TyHirExprKind::Variant {
                        def_id: *def_id,
                        variant: variant.clone(),
                        args: typed_args,
                        turbofish: turbofish.clone(),
                    },
                }
            }

            hir::HirExpr::Match { scrutinee, arms } => {
                let scrutinee_ty = self.check_expr(scrutinee);
                let match_ty = match &scrutinee_ty.ty {
                    HirType::Enum(enum_def, targs, cargs) => {
                        self.check_enum_match(&scrutinee_ty, enum_def, targs, cargs, arms)
                    }
                    HirType::Int => self.check_int_match(&scrutinee_ty, arms),
                    other => fatal(MplangError::type_error(format!(
                        "match 表达式需要枚举或 int 类型的被匹配值，实际为 {:?}",
                        other
                    ))),
                };
                crate::tyhir::TyHirExpr {
                    ty: match_ty,
                    kind: crate::tyhir::TyHirExprKind::Match {
                        scrutinee: Box::new(scrutinee_ty),
                        arms: self.typed_arms.take().unwrap(),
                    },
                }
            }
        }
    }

    // ───────────────── match 表达式类型检查 ─────────────────

    /// 对枚举类型的 scrutinee 进行 match 类型检查。返回 match 表达式的结果类型。
    fn check_enum_match(
        &mut self,
        _scrutinee_ty: &crate::tyhir::TyHirExpr,
        enum_def: &DefId,
        targs: &[HirType],
        cargs: &[ConstArg],
        arms: &[hir::HirMatchArm],
    ) -> HirType {
        let infos = self
            .enum_variants
            .get(enum_def)
            .cloned()
            .unwrap_or_else(|| {
                fatal(MplangError::type_error(format!(
                    "未定义的枚举 `{}`",
                    self.name_of(enum_def)
                )))
            });
        let generics = self
            .struct_generics
            .get(enum_def)
            .cloned()
            .unwrap_or_default();

        let mut covered_variants: Vec<String> = Vec::new();
        let mut result_ty: Option<HirType> = None;
        let mut typed_arms = Vec::with_capacity(arms.len());
        let unit_ret = HirType::Unit;

        for arm in arms {
            let saved_def_types: HashMap<DefId, HirType> = self.def_types.clone();

            match &arm.pattern {
                hir::HirPattern::Wildcard | hir::HirPattern::Ident(_) => {}
                hir::HirPattern::Variant {
                    enum_def: arm_enum_def,
                    variant,
                    bindings,
                } => {
                    if arm_enum_def != enum_def {
                        fatal(MplangError::type_error(format!(
                            "match 分支的变体 `{}` 不属于枚举 `{}`",
                            variant,
                            self.name_of(enum_def)
                        )));
                    }
                    let var_info = infos
                        .iter()
                        .find(|v| v.name == *variant)
                        .unwrap_or_else(|| {
                            fatal(MplangError::type_error(format!(
                                "枚举 `{}` 没有变体 `{}`",
                                self.name_of(enum_def),
                                variant
                            )))
                        });
                    if bindings.len() != var_info.fields.len() {
                        fatal(MplangError::type_error(format!(
                            "变体 `{}` 需要 {} 个绑定，实际给了 {}",
                            variant,
                            var_info.fields.len(),
                            bindings.len()
                        )));
                    }
                    for ((bdef, _bty), (_fname, _fdef, fty)) in
                        bindings.iter().zip(var_info.fields.iter())
                    {
                        let field_ty = Self::subst_type(fty, &generics, targs, cargs);
                        self.def_types.insert(*bdef, field_ty);
                    }
                    covered_variants.push(variant.clone());
                }
                hir::HirPattern::Literal(_) => {
                    fatal(MplangError::type_error("枚举 match 中不能使用字面量模式"))
                }
            }

            let arm_body = self.check_body(&arm.body, &unit_ret);
            let arm_result_ty = arm_body
                .stmts
                .last()
                .map(|s| self.stmt_result_ty(s))
                .unwrap_or(HirType::Unit);

            match &result_ty {
                Some(expected) => {
                    self.assert_ty(&arm_result_ty, expected, "match 各分支结果类型必须一致");
                }
                None => {
                    result_ty = Some(arm_result_ty);
                }
            }

            self.def_types = saved_def_types;

            typed_arms.push(crate::tyhir::TyHirMatchArm {
                pattern: self.lower_pattern_to_tyhir(&arm.pattern),
                body: arm_body,
            });
        }

        let has_catch_all = arms.iter().any(|a| {
            matches!(
                a.pattern,
                hir::HirPattern::Wildcard | hir::HirPattern::Ident(_)
            )
        });
        if !has_catch_all {
            for v in &infos {
                if !covered_variants.contains(&v.name) {
                    fatal(MplangError::type_error(format!(
                        "match 表达式未覆盖枚举 `{}` 的变体 `{}`",
                        self.name_of(enum_def),
                        v.name
                    )));
                }
            }
        }

        self.typed_arms = Some(typed_arms);
        result_ty.unwrap_or(HirType::Unit)
    }

    /// 对 int 类型的 scrutinee 进行 match 类型检查。
    fn check_int_match(
        &mut self,
        scrutinee_ty: &crate::tyhir::TyHirExpr,
        arms: &[hir::HirMatchArm],
    ) -> HirType {
        let mut result_ty: Option<HirType> = None;
        let mut typed_arms = Vec::with_capacity(arms.len());
        let unit_ret = HirType::Unit;
        let _ = scrutinee_ty;

        for arm in arms {
            let saved_def_types: HashMap<DefId, HirType> = self.def_types.clone();

            match &arm.pattern {
                hir::HirPattern::Literal(_) => {}
                hir::HirPattern::Wildcard => {}
                hir::HirPattern::Ident(bdef) => {
                    self.def_types.insert(*bdef, HirType::Int);
                }
                hir::HirPattern::Variant { .. } => {
                    fatal(MplangError::type_error("int 类型 match 中不能使用变体模式"))
                }
            }

            let arm_body = self.check_body(&arm.body, &unit_ret);
            let arm_result_ty = arm_body
                .stmts
                .last()
                .map(|s| self.stmt_result_ty(s))
                .unwrap_or(HirType::Unit);

            match &result_ty {
                Some(expected) => {
                    self.assert_ty(&arm_result_ty, expected, "match 各分支结果类型必须一致");
                }
                None => {
                    result_ty = Some(arm_result_ty);
                }
            }

            self.def_types = saved_def_types;

            typed_arms.push(crate::tyhir::TyHirMatchArm {
                pattern: self.lower_pattern_to_tyhir(&arm.pattern),
                body: arm_body,
            });
        }

        let has_catch_all = arms.iter().any(|a| {
            matches!(
                a.pattern,
                hir::HirPattern::Wildcard | hir::HirPattern::Ident(_)
            )
        });
        if !has_catch_all {
            fatal(MplangError::type_error(
                "int 类型 match 表达式必须包含通配符 `_` 或标识符分支",
            ));
        }

        self.typed_arms = Some(typed_arms);
        result_ty.unwrap_or(HirType::Unit)
    }

    /// 从 HirPattern 转换为 TyHirPattern（直接克隆，二者结构相同）。
    fn lower_pattern_to_tyhir(&self, pattern: &hir::HirPattern) -> crate::tyhir::TyHirPattern {
        match pattern {
            hir::HirPattern::Variant {
                enum_def,
                variant,
                bindings,
            } => crate::tyhir::TyHirPattern::Variant {
                enum_def: *enum_def,
                variant: variant.clone(),
                bindings: bindings.clone(),
            },
            hir::HirPattern::Literal(l) => crate::tyhir::TyHirPattern::Literal(l.clone()),
            hir::HirPattern::Wildcard => crate::tyhir::TyHirPattern::Wildcard,
            hir::HirPattern::Ident(d) => crate::tyhir::TyHirPattern::Ident(*d),
        }
    }

    /// 取语句的「结果类型」——如果是 Expr 语句，取其表达式的类型；否则为 Unit。
    fn stmt_result_ty(&self, stmt: &crate::tyhir::TyHirStmt) -> HirType {
        match stmt {
            crate::tyhir::TyHirStmt::Expr(e) => e.ty.clone(),
            _ => HirType::Unit,
        }
    }

    /// 判断一个 HIR 表达式是否为「左值」（拥有确定存储、可对其取地址）。
    /// `Path`（变量/全局）、`FieldAccess`（结构体字段）、`AddressOf`、`Deref`
    /// 都是左值；字面量、算术结果、函数调用返回值等右值则不是。
    fn is_lvalue(&self, e: &hir::HirExpr) -> bool {
        match e {
            hir::HirExpr::Path(_) => true,
            hir::HirExpr::FieldAccess { object, .. } => self.is_lvalue(object),
            hir::HirExpr::Index { array, .. } => self.is_lvalue(array),
            hir::HirExpr::AddressOf(_) => true,
            hir::HirExpr::Deref(_) => true,
            hir::HirExpr::Variant { .. } => false,
            hir::HirExpr::Match { .. } => false,
            _ => false,
        }
    }

    // ───────────────── 辅助 ─────────────────

    fn find_field(&self, struct_def: DefId, field: &str) -> Option<&StructFieldInfo> {
        self.struct_fields
            .get(&struct_def)
            .and_then(|fs| fs.iter().find(|f| f.name == field))
    }

    /// 根据变体构造器 DefId 查找所属的枚举 DefId 与变体名。
    fn find_variant_by_def_id(&self, def_id: DefId) -> Option<(DefId, String)> {
        let name = self.names.get(&def_id)?;
        for (enum_def, variants) in &self.enum_variants {
            for v in variants {
                if v.name == *name {
                    return Some((*enum_def, v.name.clone()));
                }
            }
        }
        None
    }

    fn name_of(&self, def_id: &DefId) -> String {
        self.names
            .get(def_id)
            .cloned()
            .unwrap_or_else(|| format!("#{:?}", def_id))
    }

    fn assert_ty(&self, actual: &HirType, expected: &HirType, ctx: &str) {
        if actual != expected {
            fatal(MplangError::type_error(format!(
                "{}：期望 {:?}，实际 {:?}",
                ctx, expected, actual
            )));
        }
    }

    /// 用一个泛型参数声明表与「具体实参」代入类型。`type_args`/`const_args` 分别按
    /// 类型参数 / 常量参数的出现顺序存放（与 [`HirType::Generic`] 的存储一致）。
    /// `Var(i)` 中的 `i` 是「全部泛型参数」里的下标，需先映射到类型参数序列里的位置再取实参。
    /// 以关联函数形式提供（无 `self`），供单态化趟通过 [`TypeChecker::subst_type`] 复用。
    pub(crate) fn subst_type(
        ty: &HirType,
        generics: &[GenericParam],
        type_args: &[HirType],
        const_args: &[ConstArg],
    ) -> HirType {
        match ty {
            HirType::Var(i) => {
                let p = generics[0..*i]
                    .iter()
                    .filter(|g| matches!(g.kind, GenericParamKind::Type))
                    .count();
                type_args[p].clone()
            }
            HirType::Generic(d, ta, ca) => HirType::Generic(
                *d,
                ta.iter()
                    .map(|t| Self::subst_type(t, generics, type_args, const_args))
                    .collect(),
                // 常量实参中的 `Param` 引用按「外层上下文」解算：若外层已给出具体整数则化为
                // `Literal`，否则保留为 `Param`（仍指向外层某常量参数）。不做跨层递归。
                ca.iter()
                    .map(|a| Self::resolve_const(a, generics, const_args))
                    .collect(),
            ),
            HirType::Pointer(e) => HirType::Pointer(Box::new(Self::subst_type(
                e, generics, type_args, const_args,
            ))),
            HirType::Array(e, l) => {
                let nl = match l {
                    ArrayLen::Known(n) => ArrayLen::Known(*n),
                    ArrayLen::Const(i) => {
                        Self::resolve_const(&ConstArg::Param(*i), generics, const_args)
                            .as_array_len(*i)
                    }
                };
                HirType::Array(
                    Box::new(Self::subst_type(e, generics, type_args, const_args)),
                    nl,
                )
            }
            HirType::Enum(d, ta, ca) => HirType::Enum(
                *d,
                ta.iter()
                    .map(|t| Self::subst_type(t, generics, type_args, const_args))
                    .collect(),
                ca.iter()
                    .map(|a| Self::resolve_const(a, generics, const_args))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// 把一个 [`ConstArg`] 在「外层泛型上下文」中解算一层：
    /// - `Literal(v)` 原样返回；
    /// - `Param(i)` 取外层 `const_args` 中对应位置的项（该项已是外层上下文的具体值或更外层的
    ///   `Param`，不再递归到当前模板，避免 `const_args` 自身含 `Param` 时陷入死循环）。
    fn resolve_const(
        ca: &ConstArg,
        generics: &[GenericParam],
        const_args: &[ConstArg],
    ) -> ConstArg {
        match ca {
            ConstArg::Literal(v) => ConstArg::Literal(*v),
            ConstArg::Param(i) => {
                let q = Self::const_slot(generics, *i);
                const_args[q]
            }
        }
    }

    /// 类型参数 `generics[k]`（种类为 Type）在「类型实参向量」中的下标。
    fn type_slot(generics: &[GenericParam], k: usize) -> usize {
        generics[0..k]
            .iter()
            .filter(|g| matches!(g.kind, GenericParamKind::Type))
            .count()
    }

    /// 常量参数 `generics[k]`（种类为 Const）在「常量实参向量」中的下标。
    fn const_slot(generics: &[GenericParam], k: usize) -> usize {
        generics[0..k]
            .iter()
            .filter(|g| matches!(g.kind, GenericParamKind::Const))
            .count()
    }

    /// 把「期望类型」中的 [`HirType::Var`] 占位符按与「实际类型」的对应关系填入 `targs`。
    /// 仅处理类型参数；具体类型不回填（由调用处的 [`assert_ty`] 负责校验）。
    fn unify_ty(
        &self,
        expected: &HirType,
        actual: &HirType,
        generics: &[GenericParam],
        targs: &mut [Option<HirType>],
    ) {
        match expected {
            HirType::Var(j) => {
                let slot = Self::type_slot(generics, *j);
                if targs[slot].is_none() {
                    targs[slot] = Some(actual.clone());
                } else if targs[slot].as_ref() != Some(actual) {
                    fatal(MplangError::type_error(
                        "泛型类型参数推断冲突（同一类型参数被推导为不同的具体类型）",
                    ));
                }
            }
            HirType::Pointer(e) => {
                if let HirType::Pointer(a) = actual {
                    self.unify_ty(e, a, generics, targs);
                }
            }
            HirType::Array(e1, _) => {
                if let HirType::Array(e2, _) = actual {
                    self.unify_ty(e1, e2, generics, targs);
                }
            }
            HirType::Generic(d1, t1, _) => {
                if let HirType::Generic(d2, t2, _) = actual
                    && d1 == d2
                {
                    for (x, y) in t1.iter().zip(t2.iter()) {
                        self.unify_ty(x, y, generics, targs);
                    }
                }
            }
            HirType::Enum(d1, t1, _) => {
                if let HirType::Enum(d2, t2, _) = actual
                    && d1 == d2
                {
                    for (x, y) in t1.iter().zip(t2.iter()) {
                        self.unify_ty(x, y, generics, targs);
                    }
                }
            }
            _ => {}
        }
    }

    /// 为一个泛型函数调用推断类型 / 常量实参，并把签名（`param_types` / `ret_ty`）
    /// 中的 [`HirType::Var`] 占位符代入为具体类型，返回「已代入的形参类型」与「已代入的返回类型」。
    /// 拒绝鸭子类型：类型参数必须能由涡轮鱼或位置实参类型唯一确定；常量参数只能由涡轮鱼提供。
    fn instantiate_fn_sig(
        &self,
        generics: &[GenericParam],
        param_types: &[HirType],
        ret_ty: &HirType,
        turbofish: &[TypeOrConst],
        args: &[crate::tyhir::TyHirExpr],
    ) -> (Vec<HirType>, HirType) {
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
        if turbofish.len() > generics.len() {
            fatal(MplangError::type_error(format!(
                "泛型实参过多（期望 {} 个）",
                generics.len()
            )));
        }
        for (k, a) in turbofish.iter().enumerate() {
            match (&generics[k].kind, a) {
                (GenericParamKind::Type, TypeOrConst::Type(t)) => {
                    targs[Self::type_slot(generics, k)] = Some(t.clone());
                }
                (GenericParamKind::Const, TypeOrConst::Const(v)) => {
                    cargs[Self::const_slot(generics, k)] = Some(ConstArg::Literal(*v));
                }
                (GenericParamKind::Const, TypeOrConst::ConstParam(_)) => fatal(
                    MplangError::type_error("常量参数名引用无法在调用处解析，请改用整数字面量"),
                ),
                _ => fatal(MplangError::type_error(
                    "泛型实参种类与声明不符（类型参数需要类型，常量参数需要整型）",
                )),
            }
        }

        // 2) 从位置实参类型推断类型参数。
        for (pt, arg) in param_types.iter().zip(args.iter()) {
            self.unify_ty(pt, &arg.ty, generics, &mut targs);
        }

        // 3) 校验全部实参已就位。
        for (i, g) in generics.iter().enumerate() {
            match g.kind {
                GenericParamKind::Type => {
                    if targs[Self::type_slot(generics, i)].is_none() {
                        fatal(MplangError::type_error(format!(
                            "无法推断类型参数 `{}`，请用涡轮鱼 `::<...>` 显式指定",
                            g.name
                        )));
                    }
                }
                GenericParamKind::Const => {
                    if cargs[Self::const_slot(generics, i)].is_none() {
                        fatal(MplangError::type_error(format!(
                            "常量参数 `{}` 必须由涡轮鱼 `::<...>` 显式指定",
                            g.name
                        )));
                    }
                }
            }
        }

        let tvec: Vec<HirType> = targs.into_iter().map(|o| o.unwrap()).collect();
        let cvec: Vec<ConstArg> = cargs.into_iter().map(|o| o.unwrap()).collect();
        let subst_params = param_types
            .iter()
            .map(|t| Self::subst_type(t, generics, &tvec, &cvec))
            .collect();
        let subst_ret = Self::subst_type(ret_ty, generics, &tvec, &cvec);
        (subst_params, subst_ret)
    }

    /// 为泛型结构体字面量推断类型 / 常量实参。
    /// - 优先使用涡轮鱼 `::<...>`（按参数声明顺序与种类对应）。
    /// - 其余类型参数：扫描字段，若某字段的声明类型正好等于 `Var(j)`，则取该字段初始化表达式的类型。
    /// - 常量参数无法从字段推断，必须由涡轮鱼提供。
    fn infer_struct_args(
        &self,
        struct_def: &DefId,
        generics: &[GenericParam],
        turbofish: &[TypeOrConst],
        infos: &[StructFieldInfo],
        checked: &[(&str, crate::tyhir::TyHirExpr)],
    ) -> (Vec<HirType>, Vec<ConstArg>) {
        let n = generics.len();
        let n_type = generics
            .iter()
            .filter(|g| matches!(g.kind, GenericParamKind::Type))
            .count();
        let n_const = generics
            .iter()
            .filter(|g| matches!(g.kind, GenericParamKind::Const))
            .count();
        let mut type_args: Vec<Option<HirType>> = vec![None; n_type];
        let mut const_args: Vec<Option<ConstArg>> = vec![None; n_const];

        // 1) 涡轮鱼。
        if turbofish.len() > n {
            fatal(MplangError::type_error(format!(
                "结构体 `{}` 的涡轮鱼实参过多（期望 {} 个）",
                self.name_of(struct_def),
                n
            )));
        }
        for (ti, a) in turbofish.iter().enumerate() {
            match (&generics[ti].kind, a) {
                (GenericParamKind::Type, TypeOrConst::Type(t)) => {
                    type_args[Self::type_slot(generics, ti)] = Some(t.clone());
                }
                (GenericParamKind::Const, TypeOrConst::Const(v)) => {
                    const_args[Self::const_slot(generics, ti)] = Some(ConstArg::Literal(*v));
                }
                (GenericParamKind::Const, TypeOrConst::ConstParam(i)) => {
                    const_args[Self::const_slot(generics, ti)] = Some(ConstArg::Param(*i));
                }
                _ => fatal(MplangError::type_error(format!(
                    "结构体 `{}` 第 {} 个泛型实参与其声明种类不符",
                    self.name_of(struct_def),
                    ti
                ))),
            }
        }

        // 2) 字段初始化表达式推断类型参数。
        for (fname, te) in checked {
            if let Some(info) = infos.iter().find(|f| &f.name == fname)
                && let HirType::Var(j) = &info.ty
            {
                type_args[*j] = Some(te.ty.clone());
            }
        }

        // 3) 校验全部实参已就位。
        for (i, g) in generics.iter().enumerate() {
            match g.kind {
                GenericParamKind::Type => {
                    if type_args[Self::type_slot(generics, i)].is_none() {
                        fatal(MplangError::type_error(format!(
                            "无法推断结构体 `{}` 的类型参数 `{}`，请用涡轮鱼显式指定",
                            self.name_of(struct_def),
                            g.name
                        )));
                    }
                }
                GenericParamKind::Const => {
                    if const_args[Self::const_slot(generics, i)].is_none() {
                        fatal(MplangError::type_error(format!(
                            "结构体 `{}` 的常量参数 `{}` 必须由涡轮鱼显式指定",
                            self.name_of(struct_def),
                            g.name
                        )));
                    }
                }
            }
        }

        let targs: Vec<HirType> = type_args.into_iter().map(|o| o.unwrap()).collect();
        let cargs = const_args.into_iter().map(|o| o.unwrap()).collect();
        (targs, cargs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;
    use crate::error::{ErrorKind, MplangError};
    use crate::hir::*;
    use crate::lexer::Lexer;
    use crate::lowering::Lowerer;
    use crate::parser::Parser;
    use crate::tyhir::*;

    fn parse(src: &str) -> CompilationUnit {
        let toks = Lexer::new(src.chars().collect()).lex().unwrap();
        Parser::new(toks).parse().unwrap()
    }

    fn frontend(src: &str) -> std::result::Result<TyHirCompilationUnit, MplangError> {
        let hir = Lowerer::new(None).lower(&parse(src))?;
        TypeChecker::new(&hir).check(&hir)
    }

    fn check_ok(src: &str) -> TyHirCompilationUnit {
        frontend(src).unwrap()
    }

    fn frontend_err(src: &str) -> MplangError {
        frontend(src).unwrap_err()
    }

    fn find_fn<'a>(m: &'a TyHirModule, name: &str) -> &'a TyHirItem {
        for it in &m.items {
            if let TyHirItem::Fn { name: n, .. } = it {
                if n == name {
                    return it;
                }
            }
        }
        panic!("fn {} not found", name);
    }

    #[test]
    fn valid_program_typechecks() {
        let t = check_ok("fn main(){ let x:int = 1; let y:int = x + 2; }");
        if let TyHirItem::Fn { body, .. } = find_fn(&t.root_module, "main") {
            // 第二条语句 `let y:int = x + 2` 的初始化应是 int 类型的二元运算。
            if let TyHirStmt::Let { ty, init, .. } = &body.stmts[1] {
                assert_eq!(ty, &HirType::Int);
                assert_eq!(init.ty, HirType::Int);
                assert!(matches!(
                    init.kind,
                    TyHirExprKind::Binary { op: BinOp::Add, .. }
                ));
            } else {
                panic!("expected let y");
            }
        } else {
            panic!("no main");
        }
    }

    #[test]
    fn type_mismatch_string_into_int() {
        let e = frontend_err("fn main(){ let x:int = \"hi\"; }");
        assert_eq!(e.kind, ErrorKind::TypeCheck);
    }

    #[test]
    fn field_access_resolves_to_field_defid() {
        let t = check_ok(
            "struct Point { x:int } fn main(){ let p:Point = Point { x:1 }; let v:int = p.x; }",
        );
        if let TyHirItem::Fn { body, .. } = find_fn(&t.root_module, "main") {
            if let TyHirStmt::Let { init, .. } = &body.stmts[1] {
                assert!(matches!(init.kind, TyHirExprKind::FieldAccess { .. }));
                assert_eq!(init.ty, HirType::Int);
            } else {
                panic!("expected let v");
            }
        } else {
            panic!("no main");
        }
    }

    #[test]
    fn struct_literal_fields_resolved() {
        let t = check_ok(
            "struct Point { x:int, y:int } fn main(){ let p:Point = Point { x:1, y:2 }; }",
        );
        if let TyHirItem::Fn { body, .. } = find_fn(&t.root_module, "main") {
            if let TyHirStmt::Let { init, .. } = &body.stmts[0] {
                if let TyHirExprKind::StructLiteral { def_id, fields, .. } = &init.kind {
                    let _ = def_id;
                    assert_eq!(fields.len(), 2);
                    assert!(matches!(fields[0].0, DefId(_)));
                } else {
                    panic!("expected struct literal");
                }
            } else {
                panic!("expected let");
            }
        } else {
            panic!("no main");
        }
    }

    #[test]
    fn undefined_function_typechecks_as_lowering_error() {
        let e = frontend_err("fn main(){ let r:int = missing(1); }");
        assert_eq!(e.kind, ErrorKind::Lowering);
    }

    #[test]
    fn comparison_requires_int_operands() {
        // 比较运算要求两个操作数同为 int 或同为指针；int 与 char* 混用应报错。
        let e = frontend_err("fn main(){ let s:int = 1; let b:int = s < \"b\"; }");
        assert_eq!(e.kind, ErrorKind::TypeCheck);
    }

    /// `&x` 得到指向 `x` 的指针，类型应为 `Pointer(Int)`。
    #[test]
    fn address_of_yields_pointer() {
        let t = check_ok("fn main(){ let x:int = 1; let p:*int = &x; }");
        if let TyHirItem::Fn { body, .. } = find_fn(&t.root_module, "main") {
            if let TyHirStmt::Let { init, .. } = &body.stmts[1] {
                assert_eq!(init.ty, HirType::Pointer(Box::new(HirType::Int)));
                assert!(matches!(init.kind, TyHirExprKind::AddressOf(_)));
            } else {
                panic!("expected let p");
            }
        } else {
            panic!("no main");
        }
    }

    /// `*p` 解引用得到被指对象的类型（此处为 `int`）。
    #[test]
    fn deref_yields_pointee() {
        let t = check_ok("fn main(){ let x:int = 1; let p:*int = &x; let y:int = *p; }");
        if let TyHirItem::Fn { body, .. } = find_fn(&t.root_module, "main") {
            if let TyHirStmt::Let { init, .. } = &body.stmts[2] {
                assert_eq!(init.ty, HirType::Int);
                assert!(matches!(init.kind, TyHirExprKind::Deref(_)));
            } else {
                panic!("expected let y");
            }
        } else {
            panic!("no main");
        }
    }

    /// 对右值（非左值）取地址是类型错误。
    #[test]
    fn address_of_rvalue_is_type_error() {
        let e = frontend_err("fn main(){ let p:*int = &1; }");
        assert_eq!(e.kind, ErrorKind::TypeCheck);
    }

    /// 字符串字面量类型应解析为 `char*`（`Pointer(Char)`）。
    #[test]
    fn string_literal_is_char_ptr() {
        let t = check_ok("fn main(){ let s:*char = \"hi\"; }");
        if let TyHirItem::Fn { body, .. } = find_fn(&t.root_module, "main") {
            if let TyHirStmt::Let { init, .. } = &body.stmts[0] {
                assert_eq!(init.ty, HirType::char_ptr());
            } else {
                panic!("expected let s");
            }
        } else {
            panic!("no main");
        }
    }

    /// 两个 `char*` 指针相等比较（`==`）应当类型通过。
    #[test]
    fn pointer_equality_typechecks() {
        let _t = check_ok(
            "fn main(){ let x:int = 1; let a:*int = &x; let b:*int = &x; let c:int = a == b; }",
        );
    }

    /// 取地址符可用于结构体字段：`&pt.x` 得到指向该字段的指针。
    #[test]
    fn address_of_struct_field_yields_pointer() {
        let t = check_ok(
            "struct Point { x:int, y:int } fn main(){ let pt:Point = Point { x:1, y:2 }; let fx:*int = &pt.x; }",
        );
        if let TyHirItem::Fn { body, .. } = find_fn(&t.root_module, "main") {
            if let TyHirStmt::Let { init, .. } = &body.stmts[1] {
                assert_eq!(init.ty, HirType::Pointer(Box::new(HirType::Int)));
            } else {
                panic!("expected let fx");
            }
        } else {
            panic!("no main");
        }
    }

    /// 数组字面量可正确类型检查，下标读取结果类型为元素类型。
    #[test]
    fn array_literal_and_index_typecheck() {
        let _t = check_ok("fn main() { let a: [int; 4] = [1, 2, 3, 4]; let s: int = a[0]; }");
    }

    /// 列表长度与类型标注不符（[int;3] vs [1,2,3,4]）应报类型错误。
    #[test]
    fn array_length_mismatch_is_type_error() {
        let e = frontend_err("fn main() { let a: [int; 3] = [1, 2, 3, 4]; }");
        assert_eq!(e.kind, ErrorKind::TypeCheck);
    }

    /// 重复形式长度与类型标注不符（[int;4] vs [1;5]）应报类型错误。
    #[test]
    fn array_repeat_length_mismatch_is_type_error() {
        let e = frontend_err("fn main() { let a: [int; 4] = [1; 5]; }");
        assert_eq!(e.kind, ErrorKind::TypeCheck);
    }

    /// 对非数组类型下标访问应报类型错误。
    #[test]
    fn index_non_array_is_type_error() {
        let e = frontend_err("fn main() { let x: int = 1; let y: int = x[0]; }");
        assert_eq!(e.kind, ErrorKind::TypeCheck);
    }

    /// 通过对下标赋值（左值写入）可以正确类型检查。
    #[test]
    fn array_index_write_typechecks() {
        let _t = check_ok("fn main() { let a: [int; 3] = [1, 2, 3]; a[0] = 10; }");
    }

    /// 数组作为函数参数与返回值可正确类型检查。
    #[test]
    fn array_param_and_return_typecheck() {
        let _t = check_ok(
            "fn sum(a: [int; 3]) -> int { return a[0] + a[1] + a[2]; } \
             fn make() -> [int; 2] { return [1, 2]; } \
             fn main() { let m: [int; 2] = make(); let s: int = sum([1, 2, 3]); }",
        );
    }

    /// `obj.method()` 在类型检查阶段解析为对方法函数的 [`Call`]，且接收者作为首个实参。
    #[test]
    fn method_call_resolves_to_call_with_receiver() {
        let t = check_ok(
            "struct Point { x:int, y:int } \
             impl Point { fn sum() -> int { return self.x + self.y; } } \
             fn main() { let p:Point = Point { x:1, y:2 }; let s:int = p.sum(); }",
        );
        let sum_id = if let TyHirItem::Fn { def_id, .. } = find_fn(&t.root_module, "sum") {
            *def_id
        } else {
            panic!("no sum");
        };
        if let TyHirItem::Fn { body, .. } = find_fn(&t.root_module, "main") {
            if let TyHirStmt::Let { init, .. } = &body.stmts[1] {
                if let TyHirExprKind::Call { callee, args, .. } = &init.kind {
                    assert_eq!(*callee, sum_id);
                    // 首个实参是接收者 `p`（一个 Path），其后无用户实参。
                    assert_eq!(args.len(), 1);
                    assert!(matches!(args[0].kind, TyHirExprKind::Path(_)));
                } else {
                    panic!("expected call");
                }
            } else {
                panic!("expected let s");
            }
        } else {
            panic!("no main");
        }
    }

    /// 调用某类型上不存在的方法应给出类型错误。
    #[test]
    fn method_not_found_is_type_error() {
        let e = frontend_err(
            "struct Point { x:int } \
             fn main() { let p:Point = Point { x:1 }; let s:int = p.nope(); }",
        );
        assert_eq!(e.kind, ErrorKind::TypeCheck);
    }

    /// 为整数类型 `impl int` 上的方法调用可正确类型检查。
    #[test]
    fn int_method_call_typechecks() {
        let _t = check_ok(
            "impl int { fn double() -> int { return self + self; } } \
             fn main() { let d:int = 21.double(); }",
        );
    }

    /// trait 的默认方法（被合成为实现类型的函数后）可正确类型检查，
    /// 且其内部对同类其它方法 `self.other()` 的调用能解析到该类型的实现。
    #[test]
    fn trait_default_method_call_typechecks() {
        let _t = check_ok(
            "trait Show { fn show() -> int; fn pretty() -> int { return self.show() + 1; } } \
             struct Point { x:int } \
             impl Show for Point { fn show() -> int { return self.x; } } \
             fn main() { let p:Point = Point { x:3 }; let v:int = p.pretty(); }",
        );
    }

    /// 同一类型上通过 `impl` 与 `impl Trait for` 重复实现同名方法应抛一致性错误。
    #[test]
    fn duplicate_method_impl_is_error() {
        let e = frontend_err(
            "struct Point { x:int } \
             impl Point { fn show() -> int { return self.x; } } \
             trait Show { fn show() -> int; } \
             impl Show for Point { fn show() -> int { return self.x; } }",
        );
        assert_eq!(e.kind, ErrorKind::Lowering);
    }
}
