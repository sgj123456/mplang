//! # Lowering —— AST → HIR
//!
//! 把解析得到的 [`crate::ast::CompilationUnit`] 降低（lower）为
//! [`crate::hir::HirCompilationUnit`]。这是「HIR 层」的核心构造过程，
//! 主要做两件事：
//!
//! 1. **定义 ID 分配**：为每一个定义（函数、外部函数、结构体、结构体字段、
//!    参数、局部变量、全局变量、模块）分配一个唯一的 [`crate::hir::DefId`]，
//!    并把「全路径名 → DefId」登记到 [`Lowerer::path_to_def`]。
//! 2. **名字解析（值 / 函数）**：把 AST 表达式里的 `Path`
//!    （如 `add`、`add::add`）解析为对应的 [`crate::hir::DefId`]，
//!    输出 HIR 中的 [`crate::hir::HirExpr::Path`] /
//!    [`crate::hir::HirExpr::Call`]。
//!
//! 字段名（`a.b` 的 `b`、结构体字面量的字段）在这一阶段**仍保留字符串**，
//! 因为只有在类型检查（得到 TYPE HIR）时才能根据对象的类型把字段名
//! 解析为字段的 [`crate::hir::DefId`]。
//!
//! 为了支持跨文件模块（`mod x;` + `use x::y;`）与前向引用，名字登记分两趟：
//! 先趟 A 把所有定义登记进 `path_to_def`，再趟 B 真正降低并登记 `use` 别名。

use std::collections::{HashMap, HashSet};
use std::path::{Path as FilePath, PathBuf};

use crate::ast::{
    self, CompilationUnit, Expr, GenericParam, Literal, Meta, Stmt, TopLevelDecl, Type,
    attr_string_value,
};
use crate::error::{MplangError, fatal, into_result};
use crate::hir::{self, ArrayLen, DefId, HirType, ParamKind, TypeOrConst, Visibility};
use crate::symbol::SymbolPath;

pub struct Lowerer {
    next_id: u32,

    /// 全限定路径 → 定义 ID（顶层 + 模块限定）。
    path_to_def: HashMap<SymbolPath, DefId>,

    /// 最后一段名字 → 定义 ID（平铺索引）。
    /// 用于「按最后一段名字回退」：当 `Point` 在 `path_to_def` 中以 `[模块, Point]`
    /// 登记时，单段 `Point` 也能通过此索引解析到对应 DefId。冲突时保留最后插入者
    /// （与现有多段回退的语义一致，且本语言无跨模块裸名作用域，故可接受）。
    last_seg_index: HashMap<String, DefId>,

    /// 当前正在降低的泛型定义（函数 / 结构体 / impl 方法）的泛型参数列表。
    /// 用于 `lower_type` 把裸类型参数名解析为 [`HirType::Var`] / [`HirType::Generic`]，
    /// 并把常量参数名解析为数组长度的常量下标。不处于泛型上下文时为空。
    cur_generics: Vec<GenericParam>,

    /// `use` 别名：最后一段名字 → 定义 ID（在降低趟 B 中登记）。
    aliases: HashMap<String, DefId>,

    /// 已加载模块名集合（防止循环加载）。
    loaded_set: HashSet<String>,
    /// 已加载模块的源码缓存（避免重复解析）。
    loaded_cache: HashMap<String, CompilationUnit>,

    /// `trait` 名 → 其方法契约。键用 trait 的「裸名」（最后一段）。
    /// 每个方法保留原始 AST 参数与（可选的）默认实现体，供「默认方法合成」使用。
    traits: HashMap<String, Vec<TraitMethodInfo>>,

    /// 变体构造器 DefId → (枚举 DefId, 变体名)。
    /// 由 register_decls 在 Enum 分支填充，供 lower_expr 在 Call→Variant 改写时查询。
    variant_defs: HashMap<DefId, (DefId, String)>,

    /// 所有已注册的枚举 DefId 集合。用于在 lower_type 中区分 struct 和 enum 类型。
    enum_defs: HashSet<DefId>,

    /// trait 裸名 → trait 的 DefId。用于 lower_type 中识别 `*Trait` 语法。
    trait_def_map: HashMap<String, DefId>,

    /// (trait_def, impl_type_def) → 按 trait 方法声明顺序排列的具体方法 DefId 列表。
    /// 在 lower_decls 的 Impl 分支填充，最终写入 HirCompilationUnit。
    vtables: HashMap<(DefId, DefId), Vec<DefId>>,

    /// 输入文件所在目录，用于查找 `mod x;` 对应的 `x.mp`。
    base_dir: Option<PathBuf>,
}

/// `trait` 中声明的一个方法（编译期契约信息）。
#[derive(Clone)]
struct TraitMethodInfo {
    name: String,
    params: Vec<ast::Param>,
    return_ty: ast::Type,
    /// `None` = 必须由实现方提供；`Some(body)` = 默认实现（可被重写）。
    default_body: Option<Vec<ast::Stmt>>,
    is_static: bool,
}

impl Lowerer {
    pub fn new(input_path: Option<&FilePath>) -> Self {
        let base_dir = input_path.and_then(|p| p.parent().map(|d| d.to_path_buf()));
        Lowerer {
            next_id: 0,
            path_to_def: HashMap::new(),
            last_seg_index: HashMap::new(),
            aliases: HashMap::new(),
            loaded_set: HashSet::new(),
            loaded_cache: HashMap::new(),
            traits: HashMap::new(),
            variant_defs: HashMap::new(),
            enum_defs: HashSet::new(),
            trait_def_map: HashMap::new(),
            vtables: HashMap::new(),
            cur_generics: Vec::new(),
            base_dir,
        }
    }

    fn alloc(&mut self) -> DefId {
        let id = DefId(self.next_id);
        self.next_id += 1;
        id
    }

    /// 登记一个定义：同时写入全限定路径表与「最后一段名字」平铺索引。
    fn register_def(&mut self, prefix: &[String], name: &str) -> DefId {
        let id = self.alloc();
        let mut segs = prefix.to_vec();
        segs.push(name.to_string());
        self.path_to_def.insert(self.sym(&segs), id);
        self.last_seg_index.insert(name.to_string(), id);
        id
    }

    fn sym(&self, segments: &[String]) -> SymbolPath {
        SymbolPath {
            segments: segments.to_vec(),
        }
    }

    /// 把 AST 降低为 HIR。
    pub fn lower(
        &mut self,
        unit: &CompilationUnit,
    ) -> Result<hir::HirCompilationUnit, MplangError> {
        into_result(|| self.lower_into(unit))
    }

    fn lower_into(&mut self, unit: &CompilationUnit) -> hir::HirCompilationUnit {
        // 趟 A：登记所有定义的全路径名（递归进入模块）。
        self.register_decls(&unit.declarations, &[]);

        // 趟 B：真正降低（此时 path_to_def 已完整，可安全解析所有名字）。
        let root_id = self.alloc();
        let items = self.lower_decls(&unit.declarations, &[]);

        hir::HirCompilationUnit {
            root_module: hir::HirModule {
                def_id: root_id,
                name: "crate".to_string(),
                visibility: Visibility::Public,
                attributes: Vec::new(),
                items,
            },
            vtables: std::mem::take(&mut self.vtables),
        }
    }

    // ───────────────── 趟 A：登记定义 ─────────────────

    fn register_decls(&mut self, decls: &[TopLevelDecl], prefix: &[String]) {
        for decl in decls {
            match decl {
                TopLevelDecl::ModDecl { name, .. } => {
                    let id = self.register_def(prefix, name);
                    let _ = id;

                    if let Some(sub) = self.get_module(name) {
                        let mut sub_prefix = prefix.to_vec();
                        sub_prefix.push(name.clone());
                        self.register_decls(&sub.declarations, &sub_prefix);
                    }
                }
                TopLevelDecl::ExternCrate {
                    name, attributes, ..
                } => {
                    let id = self.register_def(prefix, name);
                    let _ = id;

                    // 加载外部 crate 源文件（路径可由 `#[path = "..."]` 指定），
                    // 并把其中的定义登记到 `[name, ...]` 前缀下，使 `use name::item`
                    // 与 `name::item` 能够解析。
                    if let Some(sub) = self.load_crate_source(attributes, name) {
                        let mut sub_prefix = prefix.to_vec();
                        sub_prefix.push(name.clone());
                        self.register_decls(&sub.declarations, &sub_prefix);
                    }
                }
                TopLevelDecl::UseDecl { .. } => {
                    // 别名在趟 B 登记，这里跳过。
                }
                TopLevelDecl::ExternFnDef { name, .. }
                | TopLevelDecl::FnDef { name, .. }
                | TopLevelDecl::StructDef { name, .. }
                | TopLevelDecl::Static { name, .. }
                | TopLevelDecl::Const { name, .. } => {
                    self.register_def(prefix, name);
                }
                TopLevelDecl::Trait { name, methods, .. } => {
                    // trait 仅作编译期契约：登记其方法签名表（按裸名索引），
                    // 并为 trait 自身分配一个 DefId（供 `impl Trait for` 名字解析）。
                    self.register_def(prefix, name);
                    let mut segs = prefix.to_vec();
                    segs.push(name.clone());
                    let trait_def = self.path_to_def[&self.sym(&segs)];
                    self.trait_def_map.insert(name.clone(), trait_def);
                    let infos = methods
                        .iter()
                        .map(|m| TraitMethodInfo {
                            name: m.name.clone(),
                            params: m.params.clone(),
                            return_ty: m.return_ty.clone(),
                            default_body: m.default_body.clone(),
                            is_static: m.is_static,
                        })
                        .collect();
                    self.traits.insert(name.clone(), infos);
                }
                TopLevelDecl::Impl { .. } => {
                    // 方法的 DefId 在降低趟 B 中按需分配（含 trait 默认方法的合成）。
                }
                TopLevelDecl::Enum { name, variants, .. } => {
                    // 为枚举及其每个变体注册 DefId（变体构造器名与函数名/结构体名处于同一命名空间）。
                    self.register_def(prefix, name);
                    let mut enum_segs = prefix.to_vec();
                    enum_segs.push(name.clone());
                    let enum_def = self.path_to_def[&self.sym(&enum_segs)];
                    self.enum_defs.insert(enum_def);
                    for v in variants {
                        self.register_def(prefix, &v.name);
                        let mut v_segs = prefix.to_vec();
                        v_segs.push(v.name.clone());
                        let v_def = self.path_to_def[&self.sym(&v_segs)];
                        self.variant_defs.insert(v_def, (enum_def, v.name.clone()));
                    }
                }
            }
        }
    }

    // ───────────────── 趟 B：降低 ─────────────────

    fn lower_decls(&mut self, decls: &[TopLevelDecl], prefix: &[String]) -> Vec<hir::HirItem> {
        let mut items = Vec::new();
        for decl in decls {
            match decl {
                TopLevelDecl::Trait { .. } => {
                    // trait 仅作编译期契约，不产生任何 HIR item。
                }
                TopLevelDecl::ModDecl { name, attributes } => {
                    let mut segs = prefix.to_vec();
                    segs.push(name.clone());
                    let module = self.lower_module(name, &segs, attributes.clone());
                    items.push(hir::HirItem::Module(module));
                }
                TopLevelDecl::ExternCrate { name, attributes } => {
                    let mut segs = prefix.to_vec();
                    segs.push(name.clone());
                    // 与 `mod` 一样把外部 crate 降低为一个名为 `name` 的模块，
                    // 区别仅在于源文件由 `#[path]`（或默认 `<name>.mp`）指定。
                    let sub = self.load_crate_source(attributes, name);
                    let module_items = match &sub {
                        Some(unit) => self.lower_decls(&unit.declarations, &segs),
                        None => Vec::new(),
                    };
                    let id = self.path_to_def[&self.sym(&segs)];
                    items.push(hir::HirItem::Module(hir::HirModule {
                        def_id: id,
                        name: name.clone(),
                        visibility: Visibility::Public,
                        attributes: attributes.clone(),
                        items: module_items,
                    }));
                }
                TopLevelDecl::UseDecl { path, .. } => {
                    // 登记别名：把最后一段名字映射到目标定义。
                    let def = self.resolve_path(path, "use 声明");
                    self.aliases.insert(path.last().unwrap().to_string(), def);
                }
                TopLevelDecl::ExternFnDef {
                    attributes,
                    name,
                    param_types,
                    return_ty,
                    is_variadic,
                } => {
                    let mut segs = prefix.to_vec();
                    segs.push(name.clone());
                    let id = self.path_to_def[&self.sym(&segs)];
                    // 「通用注解读取」：从注解列表中按名取出 `link_name` 的值。
                    // 仅此一处与具体注解耦合，新增其它注解时不影响注解语法 / IR。
                    let link_name = crate::ast::attr_string_value(attributes, "link_name")
                        .map(|s| s.to_string());
                    items.push(hir::HirItem::ExternFn {
                        def_id: id,
                        visibility: Visibility::Public,
                        attributes: attributes.clone(),
                        link_name,
                        name: name.clone(),
                        param_types: param_types.iter().map(|t| self.lower_type(t)).collect(),
                        return_ty: self.lower_type(return_ty),
                        is_variadic: *is_variadic,
                    });
                }
                TopLevelDecl::FnDef {
                    attributes,
                    name,
                    generics,
                    params,
                    return_ty,
                    body,
                } => {
                    let mut segs = prefix.to_vec();
                    segs.push(name.clone());
                    let id = self.path_to_def[&self.sym(&segs)];

                    let mut locals: HashMap<String, DefId> = HashMap::new();
                    // 返回类型、参数类型、函数体都可能引用泛型参数，降低期间设好泛型上下文。
                    let saved = std::mem::replace(&mut self.cur_generics, generics.clone());
                    let params_hir =
                        self.lower_generic_fn(generics, params, return_ty, body, &mut locals);

                    items.push(hir::HirItem::Fn {
                        def_id: id,
                        visibility: Visibility::Public,
                        attributes: attributes.clone(),
                        name: name.clone(),
                        generics: generics.clone(),
                        params: params_hir,
                        return_ty: self.lower_type(return_ty),
                        body: self.lower_body(body, &mut locals),
                        impl_receiver: None,
                    });
                    self.cur_generics = saved;
                }
                TopLevelDecl::Impl {
                    trrait,
                    ty,
                    methods,
                    generics,
                    ..
                } => {
                    // 接收者类型可能引用 impl 的泛型参数，降低期间把泛型上下文设为 impl 自身。
                    let saved = std::mem::replace(&mut self.cur_generics, generics.clone());
                    let recv_ty = self.lower_type(ty);
                    self.cur_generics = saved;

                    // 若为 trait 实现：校验必填方法都已提供、签名匹配，并收集
                    // 未重写但带默认实现的方法（合成后一并降低）。
                    let mut to_lower = methods.clone();
                    let mut trait_def_for_vtable = None;
                    if let Some(tr) = trrait {
                        let tr_name = tr.last().unwrap_or("?").to_string();
                        let req = self.traits.get(&tr_name).cloned().unwrap_or_else(|| {
                            fatal(MplangError::lowering(format!("未找到 trait `{}`", tr_name)))
                        });
                        // 查找 trait 的 DefId
                        if let Some(trait_def) = self.trait_def_map.get(&tr_name) {
                            trait_def_for_vtable = Some(*trait_def);
                        }
                        let provided: std::collections::HashSet<String> =
                            methods.iter().map(|m| m.name.clone()).collect();

                        for m in methods {
                            // 实现的方法必须属于该 trait。
                            let sig = req
                                .iter()
                                .find(|t| t.name == m.name)
                                .unwrap_or_else(|| {
                                    fatal(MplangError::lowering(format!(
                                        "为类型 `{:?}` 实现 trait `{}` 时提供了不在 trait 中的方法 `{}`",
                                        ty, tr_name, m.name
                                    )))
                                });
                            // static 标志必须匹配。
                            if m.is_static != sig.is_static {
                                let kind_desc = if m.is_static { "静态" } else { "非静态" };
                                let sig_desc = if sig.is_static { "静态" } else { "非静态" };
                                fatal(MplangError::lowering(format!(
                                    "方法 `{}` 是{}方法，但 trait `{}` 中声明为{}方法",
                                    m.name, kind_desc, tr_name, sig_desc
                                )));
                            }
                            // 签名比对：显式参数（不含隐式 `self`）个数与类型，以及返回类型。
                            if m.params.len() != sig.params.len() {
                                fatal(MplangError::lowering(format!(
                                    "方法 `{}` 的参数个数（{}）与 trait `{}` 的声明（{}）不符",
                                    m.name,
                                    m.params.len(),
                                    tr_name,
                                    sig.params.len()
                                )));
                            }
                            for (i, p) in m.params.iter().enumerate() {
                                let expect = self.lower_type(&sig.params[i].ty);
                                let got = self.lower_type(&p.ty);
                                if got != expect {
                                    fatal(MplangError::lowering(format!(
                                        "方法 `{}` 第 {} 个参数类型与 trait `{}` 不符",
                                        m.name, i, tr_name
                                    )));
                                }
                            }
                            let expect_ret = self.lower_type(&sig.return_ty);
                            let got_ret = self.lower_type(&m.return_ty);
                            if got_ret != expect_ret {
                                fatal(MplangError::lowering(format!(
                                    "方法 `{}` 的返回类型与 trait `{}` 不符",
                                    m.name, tr_name
                                )));
                            }
                        }

                        // 必填方法（无默认实现）必须被提供。
                        for t in &req {
                            if t.default_body.is_none() && !provided.contains(&t.name) {
                                fatal(MplangError::lowering(format!(
                                    "类型 `{:?}` 实现 trait `{}` 时未提供必填方法 `{}`",
                                    ty, tr_name, t.name
                                )));
                            }
                            // 带默认实现且未被重写的方法：合成实现（用默认体）。
                            if t.default_body.is_some() && !provided.contains(&t.name) {
                                to_lower.push(ast::ImplMethod {
                                    name: t.name.clone(),
                                    generics: Vec::new(),
                                    params: t.params.clone(),
                                    return_ty: t.return_ty.clone(),
                                    body: t.default_body.clone().unwrap(),
                                    is_static: t.is_static,
                                });
                            }
                        }
                    }

                    // 收集方法 DefId（按 to_lower 的插入顺序，即 trait 方法声明顺序）。
                    let method_defs: Vec<(String, DefId)> = to_lower
                        .iter()
                        .map(|m| {
                            // 每个方法在循环开始处 self.alloc() 得到其 id
                            let id = self.alloc();
                            (m.name.clone(), id)
                        })
                        .collect();

                    let lowered_ids: HashMap<String, DefId> = method_defs.iter().cloned().collect();

                    // 收集 vtable 条目：按 trait 方法声明顺序排列
                    let impl_type_def = match &recv_ty {
                        HirType::Named(d) => Some(*d),
                        _ => None,
                    };
                    if let Some(trait_def) = trait_def_for_vtable
                        && let Some(impl_type_def) = impl_type_def
                    {
                        let tr_name = trrait.as_ref().unwrap().last().unwrap().to_string();
                        let req = self.traits.get(&tr_name).unwrap();
                        let vtable_methods: Vec<DefId> = req
                            .iter()
                            .map(|tm| {
                                lowered_ids.get(&tm.name).cloned().unwrap_or_else(|| {
                                    fatal(MplangError::lowering(format!(
                                        "内部错误：方法 `{}` 未在 vtable 中找到",
                                        tm.name
                                    )))
                                })
                            })
                            .collect();
                        self.vtables
                            .insert((trait_def, impl_type_def), vtable_methods);
                    }

                    // 现在用 method_defs 中的预分配 ID 来真正降低每个方法。
                    for (m, (mname, mid)) in to_lower.iter().zip(method_defs.iter()) {
                        // mname 应与 m.name 一致，mid 是预分配的 DefId
                        let _ = mname;
                        let combined: Vec<GenericParam> =
                            generics.iter().chain(m.generics.iter()).cloned().collect();
                        let saved = std::mem::replace(&mut self.cur_generics, combined.clone());

                        let mut params_hir: Vec<hir::HirParam> = Vec::new();
                        let mut locals: HashMap<String, DefId> = HashMap::new();

                        // 非静态方法：注入隐式 self 作为第一个参数。
                        if !m.is_static {
                            let self_id = self.alloc();
                            locals.insert("self".to_string(), self_id);
                            params_hir.push(hir::HirParam {
                                def_id: self_id,
                                name: "self".to_string(),
                                ty: recv_ty.clone(),
                                kind: ParamKind::Value,
                            });
                        }
                        for (i, gp) in combined.iter().enumerate() {
                            if gp.kind == crate::ast::GenericParamKind::Const {
                                let cid = self.alloc();
                                locals.insert(gp.name.clone(), cid);
                                params_hir.push(hir::HirParam {
                                    def_id: cid,
                                    name: gp.name.clone(),
                                    ty: HirType::Int,
                                    kind: ParamKind::ConstParam(i),
                                });
                            }
                        }
                        for p in &m.params {
                            let pid = self.alloc();
                            locals.insert(p.name.clone(), pid);
                            params_hir.push(hir::HirParam {
                                def_id: pid,
                                name: p.name.clone(),
                                ty: self.lower_type(&p.ty),
                                kind: ParamKind::Value,
                            });
                        }

                        let body_hir = self.lower_body(&m.body, &mut locals);

                        let return_ty_hir = self.lower_type(&m.return_ty);
                        self.cur_generics = saved;

                        items.push(hir::HirItem::Fn {
                            def_id: *mid,
                            visibility: Visibility::Public,
                            attributes: Vec::new(),
                            name: m.name.clone(),
                            generics: combined,
                            params: params_hir,
                            return_ty: return_ty_hir,
                            body: body_hir,
                            impl_receiver: if m.is_static {
                                None
                            } else {
                                Some(recv_ty.clone())
                            },
                        });

                        // 静态方法：同时注册为 `Type::method` 路径，使其可通过
                        // `Vec::new()` 或 `TypeName::method()` 语法调用。
                        if m.is_static {
                            // 从 AST ty 中提取类型名（如 "Vec"、"String"）。
                            let type_name = match ty {
                                crate::ast::Type::Named(p) | crate::ast::Type::Applied(p, _) => {
                                    p.last().unwrap_or("").to_string()
                                }
                                _ => String::new(),
                            };
                            if !type_name.is_empty() {
                                let mut static_segs = prefix.to_vec();
                                static_segs.push(type_name.clone());
                                static_segs.push(m.name.clone());
                                self.path_to_def.insert(self.sym(&static_segs), *mid);
                                // 也注册到无前缀路径（如 ["Vec", "new"]），使用户无需写
                                // `alloc::Vec::new()` 即可调用。
                                let bare_segs = vec![type_name.clone(), m.name.clone()];
                                self.path_to_def.insert(self.sym(&bare_segs), *mid);
                                self.last_seg_index.insert(m.name.clone(), *mid);
                            }
                        }
                    }
                }
                TopLevelDecl::StructDef {
                    attributes,
                    name,
                    generics,
                    fields,
                } => {
                    let mut segs = prefix.to_vec();
                    segs.push(name.clone());
                    let id = self.path_to_def[&self.sym(&segs)];
                    // 字段类型可能引用类型/常量参数，降低期间把泛型上下文设为该结构体自身。
                    let saved = std::mem::replace(&mut self.cur_generics, generics.clone());
                    let fields_hir = fields
                        .iter()
                        .map(|(fname, fty)| hir::HirField {
                            def_id: self.alloc(),
                            name: fname.clone(),
                            ty: self.lower_type(fty),
                            visibility: Visibility::Public,
                        })
                        .collect();
                    self.cur_generics = saved;
                    items.push(hir::HirItem::Struct {
                        def_id: id,
                        visibility: Visibility::Public,
                        attributes: attributes.clone(),
                        name: name.clone(),
                        generics: generics.clone(),
                        fields: fields_hir,
                    });
                }
                TopLevelDecl::Enum {
                    attributes,
                    name,
                    generics,
                    variants,
                } => {
                    let mut segs = prefix.to_vec();
                    segs.push(name.clone());
                    let id = self.path_to_def[&self.sym(&segs)];
                    let saved = std::mem::replace(&mut self.cur_generics, generics.clone());
                    let hir_variants: Vec<hir::HirEnumVariant> = variants
                        .iter()
                        .enumerate()
                        .map(|(tag, v)| {
                            let mut v_segs = prefix.to_vec();
                            v_segs.push(v.name.clone());
                            let v_def = self.path_to_def[&self.sym(&v_segs)];
                            let fields_hir = v
                                .fields
                                .iter()
                                .map(|(fname, fty)| hir::HirField {
                                    def_id: self.alloc(),
                                    name: fname.clone(),
                                    ty: self.lower_type(fty),
                                    visibility: Visibility::Public,
                                })
                                .collect();
                            hir::HirEnumVariant {
                                def_id: v_def,
                                name: v.name.clone(),
                                tag: tag as u32,
                                fields: fields_hir,
                            }
                        })
                        .collect();
                    self.cur_generics = saved;
                    items.push(hir::HirItem::Enum {
                        def_id: id,
                        visibility: Visibility::Public,
                        attributes: attributes.clone(),
                        name: name.clone(),
                        generics: generics.clone(),
                        variants: hir_variants,
                    });
                }
                TopLevelDecl::Static {
                    attributes,
                    name,
                    ty,
                    init,
                } => {
                    let mut segs = prefix.to_vec();
                    segs.push(name.clone());
                    let id = self.path_to_def[&self.sym(&segs)];
                    let mut locals = HashMap::new();
                    items.push(hir::HirItem::Static {
                        def_id: id,
                        visibility: Visibility::Public,
                        attributes: attributes.clone(),
                        name: name.clone(),
                        ty: self.lower_type(ty),
                        init: self.lower_expr(init, &mut locals),
                        is_const: false,
                    });
                }
                TopLevelDecl::Const {
                    attributes,
                    name,
                    ty,
                    init,
                } => {
                    let mut segs = prefix.to_vec();
                    segs.push(name.clone());
                    let id = self.path_to_def[&self.sym(&segs)];
                    let mut locals = HashMap::new();
                    items.push(hir::HirItem::Static {
                        def_id: id,
                        visibility: Visibility::Public,
                        attributes: attributes.clone(),
                        name: name.clone(),
                        ty: self.lower_type(ty),
                        init: self.lower_expr(init, &mut locals),
                        is_const: true,
                    });
                }
            }
        }
        items
    }

    fn lower_module(
        &mut self,
        name: &str,
        prefix: &[String],
        attributes: Vec<Meta>,
    ) -> hir::HirModule {
        let id = self.path_to_def[&self.sym(prefix)];
        let sub = self.get_module(name);
        let items = match &sub {
            Some(unit) => self.lower_decls(&unit.declarations, prefix),
            None => Vec::new(),
        };
        hir::HirModule {
            def_id: id,
            name: name.to_string(),
            visibility: Visibility::Public,
            attributes,
            items,
        }
    }

    /// 降低一个泛型（或普通）函数的参数与常量参数。
    /// 类型参数不产生独立存储（仅作为 [`HirType::Var`] 占位符）；
    /// 常量参数以 `Int` 类型的局部 [`DefId`] 登记到 `locals`（供函数体内 `N` 引用）。
    /// 调用方需确保 `cur_generics` 已设为该函数的泛型参数列表。
    fn lower_generic_fn(
        &mut self,
        generics: &[GenericParam],
        params: &[ast::Param],
        _return_ty: &ast::Type,
        _body: &[Stmt],
        locals: &mut HashMap<String, DefId>,
    ) -> Vec<hir::HirParam> {
        let mut out = Vec::new();
        for (i, gp) in generics.iter().enumerate() {
            if gp.kind == crate::ast::GenericParamKind::Const {
                let cid = self.alloc();
                locals.insert(gp.name.clone(), cid);
                out.push(hir::HirParam {
                    def_id: cid,
                    name: gp.name.clone(),
                    ty: HirType::Int,
                    kind: ParamKind::ConstParam(i),
                });
            }
        }
        for p in params {
            let pid = self.alloc();
            locals.insert(p.name.clone(), pid);
            out.push(hir::HirParam {
                def_id: pid,
                name: p.name.clone(),
                ty: self.lower_type(&p.ty),
                kind: ParamKind::Value,
            });
        }
        out
    }

    /// 查找单段类型参数名在其声明泛型参数表中的下标（仅类型参数）。
    fn type_param_index(&self, name: &str) -> Option<usize> {
        self.cur_generics
            .iter()
            .position(|g| g.kind == crate::ast::GenericParamKind::Type && g.name == name)
    }

    /// 查找常量参数名在其声明泛型参数表中的下标（仅常量参数）。
    fn const_param_index(&self, name: &str) -> usize {
        self.cur_generics
            .iter()
            .position(|g| g.kind == crate::ast::GenericParamKind::Const && g.name == name)
            .unwrap_or_else(|| {
                fatal(MplangError::lowering(format!(
                    "未定义的常量参数 `{}`（内部错误）",
                    name
                )))
            })
    }

    fn lower_body(&mut self, stmts: &[Stmt], locals: &mut HashMap<String, DefId>) -> hir::HirBody {
        hir::HirBody {
            stmts: stmts.iter().map(|s| self.lower_stmt(s, locals)).collect(),
        }
    }

    fn lower_stmt(&mut self, stmt: &Stmt, locals: &mut HashMap<String, DefId>) -> hir::HirStmt {
        match stmt {
            Stmt::Let { name, ty, init } => {
                let id = self.alloc();
                locals.insert(name.clone(), id);
                hir::HirStmt::Let {
                    def_id: id,
                    name: name.clone(),
                    ty: ty.as_ref().map(|t| self.lower_type(t)),
                    init: self.lower_expr(init, locals),
                }
            }
            Stmt::Static { .. } | Stmt::Const { .. } => {
                // 仅顶层可以声明 static/const；函数体内出现属于语言不支持的写法。
                // 早期在此静默丢弃绑定（仅保留初始化表达式），会导致变量名后续无法解析，
                // 产生令人困惑的报错。这里直接给出明确的错误信息。
                fatal(MplangError::lowering(
                    "static/const 声明不允许出现在函数体内，请将它们移到顶层。",
                ))
            }
            Stmt::Assign { target, value } => hir::HirStmt::Assign {
                target: self.lower_expr(target, locals),
                value: self.lower_expr(value, locals),
            },
            Stmt::Expr(expr) => hir::HirStmt::Expr(self.lower_expr(expr, locals)),
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => hir::HirStmt::If {
                cond: self.lower_expr(cond, locals),
                then_branch: self.lower_body(then_branch, locals),
                else_branch: else_branch.as_ref().map(|b| self.lower_body(b, locals)),
            },
            Stmt::While { cond, body } => hir::HirStmt::While {
                cond: self.lower_expr(cond, locals),
                body: self.lower_body(body, locals),
            },
            Stmt::Return(expr) => {
                hir::HirStmt::Return(expr.as_ref().map(|e| self.lower_expr(e, locals)))
            }
        }
    }

    fn lower_expr(&mut self, expr: &Expr, locals: &mut HashMap<String, DefId>) -> hir::HirExpr {
        match expr {
            Expr::Literal(l) => hir::HirExpr::Literal(l.clone()),
            Expr::Ident(path) => {
                // 先查局部作用域（参数 / let）。
                if let Some(name) = path.last()
                    && let Some(id) = locals.get(name)
                {
                    return hir::HirExpr::Path(*id);
                }
                let def = self.resolve_path(path, "标识符");
                // 若标识符是变体构造器（如 `None`），改写为 Variant（无参）。
                if let Some((enum_def, variant)) = self.variant_defs.get(&def).cloned() {
                    hir::HirExpr::Variant {
                        def_id: enum_def,
                        variant,
                        args: Vec::new(),
                        turbofish: Vec::new(),
                    }
                } else {
                    hir::HirExpr::Path(def)
                }
            }
            Expr::Paren(inner) => self.lower_expr(inner, locals),
            Expr::Binary { op, lhs, rhs } => hir::HirExpr::Binary {
                op: *op,
                lhs: Box::new(self.lower_expr(lhs, locals)),
                rhs: Box::new(self.lower_expr(rhs, locals)),
            },
            Expr::Call {
                callee,
                args,
                turbofish,
            } => {
                let def = self.resolve_path(callee, "函数调用");
                let args_hir = args.iter().map(|a| self.lower_expr(a, locals)).collect();
                // 若 callee 是已注册的变体构造器，改写为 Variant 表达式。
                if let Some((enum_def, variant)) = self.variant_defs.get(&def).cloned() {
                    hir::HirExpr::Variant {
                        def_id: enum_def,
                        variant,
                        args: args_hir,
                        turbofish: self.lower_turbofish(turbofish),
                    }
                } else {
                    hir::HirExpr::Call {
                        callee: def,
                        args: args_hir,
                        turbofish: self.lower_turbofish(turbofish),
                    }
                }
            }
            Expr::FieldAccess { object, field } => hir::HirExpr::FieldAccess {
                object: Box::new(self.lower_expr(object, locals)),
                field: field.clone(),
            },
            Expr::StructLiteral {
                name,
                fields,
                turbofish,
            } => {
                let def = self.resolve_path(name, "结构体字面量");
                let fields_hir = fields
                    .iter()
                    .map(|(fname, fe)| (fname.clone(), self.lower_expr(fe, locals)))
                    .collect();
                hir::HirExpr::StructLiteral {
                    def_id: def,
                    fields: fields_hir,
                    turbofish: self.lower_turbofish(turbofish),
                }
            }
            Expr::AddrOf(inner) => {
                hir::HirExpr::AddressOf(Box::new(self.lower_expr(inner, locals)))
            }
            Expr::Deref(inner) => hir::HirExpr::Deref(Box::new(self.lower_expr(inner, locals))),

            Expr::MethodCall { object, name, args } => {
                let obj = self.lower_expr(object, locals);
                let args_hir = args.iter().map(|a| self.lower_expr(a, locals)).collect();
                hir::HirExpr::MethodCall {
                    object: Box::new(obj),
                    name: name.clone(),
                    args: args_hir,
                }
            }

            Expr::ArrayLiteral { elements, repeat } => {
                let repeat_len = match repeat {
                    Some(e) => {
                        if let Expr::Literal(Literal::Int(n)) = e.as_ref() {
                            Some(*n as usize)
                        } else {
                            crate::error::fatal(MplangError::lowering(
                                "数组字面量重复形式 [v; n] 的 n 必须是整型字面量",
                            ))
                        }
                    }
                    None => None,
                };
                let elements_hir = elements
                    .iter()
                    .map(|e| self.lower_expr(e, locals))
                    .collect();
                hir::HirExpr::ArrayLiteral {
                    elements: elements_hir,
                    repeat: repeat_len,
                }
            }

            Expr::Index { array, index } => hir::HirExpr::Index {
                array: Box::new(self.lower_expr(array, locals)),
                index: Box::new(self.lower_expr(index, locals)),
            },

            Expr::StaticCall {
                ty_path,
                method,
                args,
                turbofish,
            } => {
                // 把 `Type::method(args)` 构造为 `Call { callee: Path([..., "Type", "method"]), args }`。
                // 路径前缀来自 ty_path 的各段 + method 名。
                let mut callee_segs = ty_path.segments.clone();
                callee_segs.push(method.clone());
                let callee_path = crate::ast::Path::new(callee_segs);
                let def = self.resolve_path(&callee_path, "静态方法调用");
                let args_hir = args.iter().map(|a| self.lower_expr(a, locals)).collect();
                hir::HirExpr::Call {
                    callee: def,
                    args: args_hir,
                    turbofish: self.lower_turbofish(turbofish),
                }
            }

            Expr::Match { scrutinee, arms } => {
                let scrutinee_hir = self.lower_expr(scrutinee, locals);
                let mut arms_hir = Vec::with_capacity(arms.len());
                for arm in arms {
                    let saved = locals.clone();
                    let pattern_hir = self.lower_pattern(&arm.pattern, locals);
                    let body_hir = self.lower_body(&arm.body, locals);
                    arms_hir.push(hir::HirMatchArm {
                        pattern: pattern_hir,
                        body: body_hir,
                    });
                    // 恢复 locals，避免模式绑定泄漏到 match 表达式之后的作用域
                    *locals = saved;
                }
                hir::HirExpr::Match {
                    scrutinee: Box::new(scrutinee_hir),
                    arms: arms_hir,
                }
            }
        }
    }

    fn lower_pattern(
        &mut self,
        pattern: &ast::Pattern,
        locals: &mut HashMap<String, DefId>,
    ) -> hir::HirPattern {
        match pattern {
            ast::Pattern::Variant {
                enum_def,
                variant,
                bindings,
            } => {
                // 先解析变体构造器名（`Some`）得到其 DefId，再反查枚举 DefId
                let variant_def = self.resolve_path(enum_def, "变体模式");
                let (enum_def_id, _) =
                    self.variant_defs
                        .get(&variant_def)
                        .cloned()
                        .unwrap_or_else(|| {
                            fatal(MplangError::lowering(format!(
                                "`{}` 不是枚举变体",
                                enum_def
                            )))
                        });
                let mut hir_bindings = Vec::with_capacity(bindings.len());
                for (bname, bty) in bindings {
                    let id = self.alloc();
                    locals.insert(bname.clone(), id);
                    let hir_ty = bty.as_ref().map(|t| self.lower_type(t));
                    hir_bindings.push((id, hir_ty));
                }
                hir::HirPattern::Variant {
                    enum_def: enum_def_id,
                    variant: variant.clone(),
                    bindings: hir_bindings,
                }
            }
            ast::Pattern::Literal(l) => hir::HirPattern::Literal(l.clone()),
            ast::Pattern::Wildcard => hir::HirPattern::Wildcard,
            ast::Pattern::Ident(name) => {
                // 检查该名字是否对应一个单元变体（如 `None`）
                if let Some(&v_def) = self.last_seg_index.get(name)
                    && let Some((enum_def, variant_name)) = self.variant_defs.get(&v_def).cloned()
                {
                    // 是变体构造器 → 单元变体模式
                    return hir::HirPattern::Variant {
                        enum_def,
                        variant: variant_name,
                        bindings: Vec::new(),
                    };
                }
                let id = self.alloc();
                locals.insert(name.clone(), id);
                hir::HirPattern::Ident(id)
            }
        }
    }

    fn lower_type(&mut self, ty: &Type) -> HirType {
        match ty {
            Type::Int => HirType::Int,
            Type::Char => HirType::Char,
            Type::Pointer(inner) => {
                let inner_hir = self.lower_type(inner);
                // 若 inner 是 `Named(def)` 且 def 是已注册的 trait，则返回 TraitObject
                if let HirType::Named(def) = &inner_hir
                    && self.trait_def_map.values().any(|t| t == def)
                {
                    return HirType::TraitObject(*def);
                }
                HirType::Pointer(Box::new(inner_hir))
            }
            Type::Unit => HirType::Unit,
            Type::Array(inner, len) => {
                let e = self.lower_type(inner);
                let l = match len {
                    crate::ast::ArrayLen::Known(n) => ArrayLen::Known(*n),
                    crate::ast::ArrayLen::Param(name) => {
                        ArrayLen::Const(self.const_param_index(name))
                    }
                };
                HirType::Array(Box::new(e), l)
            }
            Type::Named(path) => {
                // 单段的裸名若在泛型上下文中是某个类型参数，则解析为其 Var 占位符。
                if path.segments.len() == 1
                    && let Some(idx) = self.type_param_index(&path.segments[0])
                {
                    return HirType::Var(idx);
                }
                let def = self.resolve_path(path, "类型");
                if self.enum_defs.contains(&def) {
                    HirType::Enum(def, Vec::new(), Vec::new())
                } else {
                    HirType::Named(def)
                }
            }
            Type::Applied(path, args) => {
                let def = self.resolve_path(path, "类型");
                let mut targs = Vec::new();
                let mut cargs = Vec::new();
                for a in args {
                    match a {
                        crate::ast::TypeOrConst::Type(t) => {
                            // 单段裸名若在当前泛型上下文中是某个「常量参数」，则记为对该参数的引用
                            // （如 `Pair<T, N>` 中的 `N`），而非一个类型名。
                            if let crate::ast::Type::Named(p) = t
                                && p.segments.len() == 1
                                && let Some(idx) = self.cur_generics.iter().position(|g| {
                                    g.kind == crate::ast::GenericParamKind::Const
                                        && g.name == p.segments[0]
                                })
                            {
                                cargs.push(crate::hir::ConstArg::Param(idx));
                            } else {
                                targs.push(self.lower_type(t));
                            }
                        }
                        crate::ast::TypeOrConst::Const(v) => {
                            cargs.push(crate::hir::ConstArg::Literal(*v))
                        }
                    }
                }
                if self.enum_defs.contains(&def) {
                    HirType::Enum(def, targs, cargs)
                } else {
                    HirType::Generic(def, targs, cargs)
                }
            }
            Type::Enum(path, args) => {
                let def = self.resolve_path(path, "枚举类型");
                let mut targs = Vec::new();
                let mut cargs = Vec::new();
                for a in args {
                    match a {
                        crate::ast::TypeOrConst::Type(t) => {
                            if let crate::ast::Type::Named(p) = t
                                && p.segments.len() == 1
                                && let Some(idx) = self.cur_generics.iter().position(|g| {
                                    g.kind == crate::ast::GenericParamKind::Const
                                        && g.name == p.segments[0]
                                })
                            {
                                cargs.push(crate::hir::ConstArg::Param(idx));
                            } else {
                                targs.push(self.lower_type(t));
                            }
                        }
                        crate::ast::TypeOrConst::Const(v) => {
                            cargs.push(crate::hir::ConstArg::Literal(*v))
                        }
                    }
                }
                HirType::Enum(def, targs, cargs)
            }
        }
    }

    /// 把 AST 涡轮鱼实参列表降低为 HIR 层（`TypeOrConst` 中的类型实参经 [`lower_type`]）。
    /// 单段裸名若在当前泛型上下文中是某个「常量参数」，则记为 [`TypeOrConst::ConstParam`]
    /// （如 `Pair::<T, N>` 中的 `N`），而非当作类型名去解析。
    fn lower_turbofish(&mut self, tf: &[crate::ast::TypeOrConst]) -> Vec<TypeOrConst> {
        tf.iter()
            .map(|a| match a {
                crate::ast::TypeOrConst::Type(t) => {
                    if let crate::ast::Type::Named(p) = t
                        && p.segments.len() == 1
                        && let Some(idx) = self.cur_generics.iter().position(|g| {
                            g.kind == crate::ast::GenericParamKind::Const && g.name == p.segments[0]
                        })
                    {
                        TypeOrConst::ConstParam(idx)
                    } else {
                        TypeOrConst::Type(self.lower_type(t))
                    }
                }
                crate::ast::TypeOrConst::Const(v) => TypeOrConst::Const(*v),
            })
            .collect()
    }

    // ───────────────── 名字解析 ─────────────────

    /// 把 AST 的 `Path` 解析为 [`DefId`]。
    fn resolve_path(&self, path: &crate::ast::Path, ctx: &str) -> DefId {
        let segs = &path.segments;

        // 1. `use` 别名（单段）。
        if segs.len() == 1
            && let Some(d) = self.aliases.get(&segs[0])
        {
            return *d;
        }

        // 2. 全限定路径精确匹配。
        let sp = self.sym(segs);
        if let Some(d) = self.path_to_def.get(&sp) {
            return *d;
        }

        // 3. 按最后一段名字回退（平铺定义，如 `add`，或子模块内定义的 `Point`）。
        //    `path_to_def` 以全限定路径为键，故单段 `Point` 需经「最后一段名字」索引
        //    才能命中 `[模块, Point]`。根模块下的单段名已由第 2 步精确命中，不受影响。
        let last = segs.last().unwrap().clone();
        if let Some(d) = self.last_seg_index.get(&last) {
            return *d;
        }

        crate::error::fatal(MplangError::lowering(format!(
            "无法解析名字 `{}`（{}）",
            path, ctx
        )));
    }

    // ───────────────── 模块 / crate 加载 ─────────────────

    /// 读取并解析给定路径的源文件，返回 [`CompilationUnit`]。
    /// 仅负责「读文件 → 词法 → 语法」这一物理动作；缓存与循环防护由 [`load_cached`] 负责。
    fn load_source_at(&self, path: &FilePath) -> CompilationUnit {
        log::info!("加载源文件：{}", path.display());
        let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
            fatal(MplangError::io(format!(
                "无法加载源文件 `{}`：{}",
                path.display(),
                e
            )))
        });
        let chars: Vec<char> = content.chars().collect();
        let mut lexer = crate::lexer::Lexer::new(chars);
        let tokens = lexer.lex().unwrap_or_else(|e| fatal(e));
        let mut parser = crate::parser::Parser::new(tokens);
        parser.parse().unwrap_or_else(|e| fatal(e))
    }

    /// 统一的源文件加载入口：带缓存与循环加载防护（`loaded_set` 以「路径字符串」为键）。
    /// 加载失败（文件缺失 / 读取错误）会升格为编译错误。
    fn load_cached(&mut self, path: &FilePath) -> Option<CompilationUnit> {
        let key = path.to_string_lossy().to_string();
        if let Some(u) = self.loaded_cache.get(&key) {
            return Some(u.clone());
        }
        if self.loaded_set.contains(&key) {
            // 检测到循环加载（如 crate A 又加载回 A），返回空模块避免死循环。
            return None;
        }
        self.loaded_set.insert(key.clone());
        let unit = self.load_source_at(path);
        self.loaded_cache.insert(key, unit.clone());
        Some(unit)
    }

    /// 加载并返回名为 `name` 的模块源文件（`name.mp`，相对输入目录）。
    fn get_module(&mut self, name: &str) -> Option<CompilationUnit> {
        let base = self.base_dir.as_ref()?;
        let path = base.join(format!("{}.mp", name));
        self.load_cached(&path)
    }

    /// 解析 `extern crate` 的源文件路径：
    /// - 若声明了 `#[path = "X"]`：相对输入目录（`base`）拼接，或把 `X` 当作绝对路径；
    ///   若 `X` 指向一个目录，则取其下的 `<name>.mp`。
    /// - 否则在标准库目录 `library/` 中查找（从输入目录的各级父目录、再到
    ///   当前工作目录寻找 `<dir>/library/<name>.mp`），最后回退为默认
    ///   `<base>/<name>.mp`（与 `mod name;` 一致）。
    ///   这样把 `library/` 作为语言的标准库目录后，`extern crate std;` 这类写法
    ///   无需显式 `#[path]` 即可被找到。
    fn resolve_crate_path(&self, attributes: &[Meta], base: &FilePath, name: &str) -> PathBuf {
        let explicit = attr_string_value(attributes, "path");
        let candidate: PathBuf = match explicit {
            Some(p) => {
                let pb = FilePath::new(p);
                if pb.is_absolute() {
                    pb.to_path_buf()
                } else {
                    base.join(pb)
                }
            }
            None => {
                let file = format!("{}.mp", name);
                // 从 `base` 向上遍历，寻找 `<祖先>/library/<name>.mp`。
                let mut dir = base.to_path_buf();
                loop {
                    let cand = dir.join("library").join(&file);
                    if cand.exists() {
                        return cand;
                    }
                    match dir.parent() {
                        Some(p) => dir = p.to_path_buf(),
                        None => break,
                    }
                }
                // 当前工作目录下的 `<cwd>/library/<name>.mp`。
                if let Ok(cwd) = std::env::current_dir() {
                    let cand = cwd.join("library").join(&file);
                    if cand.exists() {
                        return cand;
                    }
                }
                base.join(&file)
            }
        };
        if candidate.is_dir() {
            candidate.join(format!("{}.mp", name))
        } else {
            candidate
        }
    }

    /// 加载并返回 `extern crate NAME;` 指定的外部 crate 源文件（带缓存 + 循环防护）。
    fn load_crate_source(&mut self, attributes: &[Meta], name: &str) -> Option<CompilationUnit> {
        let base = self.base_dir.as_ref()?;
        let path = self.resolve_crate_path(attributes, base, name);
        self.load_cached(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;
    use crate::error::ErrorKind;
    use crate::hir::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn parse(src: &str) -> CompilationUnit {
        let toks = Lexer::new(src.chars().collect()).lex().unwrap();
        Parser::new(toks).parse().unwrap()
    }

    fn lower(src: &str) -> HirCompilationUnit {
        Lowerer::new(None).lower(&parse(src)).unwrap()
    }

    fn find_fn<'a>(m: &'a HirModule, name: &str) -> &'a HirItem {
        for it in &m.items {
            if let HirItem::Fn { name: n, .. } = it {
                if n == name {
                    return it;
                }
            }
        }
        panic!("fn {} not found", name);
    }

    #[test]
    fn call_resolves_to_callee_defid() {
        let hir = lower(
            "fn helper(a:int,b:int)->int { return a+b; } \
             fn main(){ let r:int = helper(1,2); }",
        );
        let helper_id = if let HirItem::Fn { def_id, .. } = find_fn(&hir.root_module, "helper") {
            *def_id
        } else {
            panic!("no helper");
        };
        if let HirItem::Fn { body, .. } = find_fn(&hir.root_module, "main") {
            if let HirStmt::Let { init, .. } = &body.stmts[0] {
                if let HirExpr::Call { callee, .. } = init {
                    assert_eq!(*callee, helper_id);
                } else {
                    panic!("expected call");
                }
            } else {
                panic!("expected let");
            }
        } else {
            panic!("no main");
        }
    }

    #[test]
    fn field_access_keeps_name_string() {
        let hir = lower(
            "struct Point { x:int } fn main(){ let p:Point = Point { x:1 }; let v:int = p.x; }",
        );
        if let HirItem::Fn { body, .. } = find_fn(&hir.root_module, "main") {
            if let HirStmt::Let { init, .. } = &body.stmts[1] {
                if let HirExpr::FieldAccess { field, .. } = init {
                    assert_eq!(field, "x");
                } else {
                    panic!("expected field access");
                }
            } else {
                panic!("expected let v");
            }
        } else {
            panic!("no main");
        }
    }

    #[test]
    fn struct_literal_keeps_field_names() {
        let hir =
            lower("struct Point { x:int, y:int } fn main(){ let p:Point = Point { x:1, y:2 }; }");
        if let HirItem::Fn { body, .. } = find_fn(&hir.root_module, "main") {
            if let HirStmt::Let { init, .. } = &body.stmts[0] {
                if let HirExpr::StructLiteral { fields, .. } = init {
                    assert_eq!(fields.len(), 2);
                    assert_eq!(fields[0].0, "x");
                    assert_eq!(fields[1].0, "y");
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
    fn module_loading_resolves_cross_file_names() {
        let dir = std::env::temp_dir().join("mplang_ut_mod");
        let _ = std::fs::create_dir_all(&dir);
        let helper = dir.join("helper.mp");
        std::fs::write(&helper, "fn helper(a:int,b:int)->int { return a+b; }").unwrap();
        let main = dir.join("main.mp");
        std::fs::write(
            &main,
            "mod helper; use helper::helper; fn main(){ let r:int = helper(1,2); }",
        )
        .unwrap();

        let content = std::fs::read_to_string(&main).unwrap();
        let toks = Lexer::new(content.chars().collect()).lex().unwrap();
        let ast = Parser::new(toks).parse().unwrap();
        let hir = Lowerer::new(Some(&main)).lower(&ast).unwrap();

        // helper 的定义应位于名为 `helper` 的子模块中。
        let helper_def = {
            let mut found = None;
            for it in &hir.root_module.items {
                if let HirItem::Module(m) = it {
                    if m.name == "helper" {
                        if let Some(HirItem::Fn { def_id, .. }) = m.items.first() {
                            found = Some(*def_id);
                        }
                    }
                }
            }
            found.expect("helper module should contain helper fn")
        };

        // main 中对 helper 的调用应解析到该 DefId。
        if let HirItem::Fn { body, .. } = find_fn(&hir.root_module, "main") {
            if let HirStmt::Let { init, .. } = &body.stmts[0] {
                if let HirExpr::Call { callee, .. } = init {
                    assert_eq!(*callee, helper_def);
                } else {
                    panic!("expected call");
                }
            } else {
                panic!("expected let");
            }
        } else {
            panic!("no main");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undefined_name_errors_with_lowering_kind() {
        let e = Lowerer::new(None)
            .lower(&parse("fn main(){ let r:int = nope(1); }"))
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Lowering);
    }

    #[test]
    fn static_inside_function_is_lowering_error() {
        // 函数体内声明 static/const 不被支持，应给出明确的 Lowering 错误，
        // 而不是静默丢弃绑定导致后续出现难以理解的“未定义名字”报错。
        let e = Lowerer::new(None)
            .lower(&parse("fn main(){ static x:int = 1; }"))
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Lowering);
    }

    #[test]
    fn impl_method_gets_implicit_self_param() {
        let hir = lower("struct Point { x:int } impl Point { fn sum() -> int { return self.x; } }");
        if let HirItem::Fn {
            params,
            impl_receiver,
            ..
        } = find_fn(&hir.root_module, "sum")
        {
            // 接收者类型应作为方法的首个（隐式 `self`）参数类型。
            assert!(impl_receiver.is_some());
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "self");
            assert_eq!(impl_receiver, &Some(params[0].ty.clone()));
        } else {
            panic!("no sum");
        }
    }

    #[test]
    fn method_call_survives_lowering_as_method_call() {
        let hir = lower(
            "struct Point { x:int } impl Point { fn sum() -> int { return self.x; } } \
             fn main() { let p:Point = Point { x:1 }; let s:int = p.sum(); }",
        );
        if let HirItem::Fn { body, .. } = find_fn(&hir.root_module, "main") {
            if let HirStmt::Let { init, .. } = &body.stmts[1] {
                assert!(matches!(init, hir::HirExpr::MethodCall { name, .. } if name == "sum"));
            } else {
                panic!("expected let s");
            }
        } else {
            panic!("no main");
        }
    }

    #[test]
    fn trait_default_method_is_synthesized() {
        // `Show` 的 `pretty` 是带默认实现的方法；`impl Show for Point` 未重写它，
        // lowering 应自动合成一个挂在 Point 上、名为 `pretty` 的默认实现函数。
        let hir = lower(
            "trait Show { fn show() -> int; fn pretty() -> int { return self.show() + 1; } } \
             struct Point { x:int } \
             impl Show for Point { fn show() -> int { return self.x; } }",
        );
        // 最终应能找到一个名为 `pretty`、接收者为 Point 的方法（来自默认合成）。
        let mut found_pretty = false;
        for it in &hir.root_module.items {
            if let HirItem::Fn {
                name,
                impl_receiver,
                ..
            } = it
            {
                if name == "pretty" && matches!(impl_receiver, Some(HirType::Named(_))) {
                    found_pretty = true;
                }
            }
        }
        assert!(found_pretty, "默认方法 `pretty` 应被合成为 Point 的方法");
    }

    #[test]
    fn trait_missing_required_method_is_lowering_error() {
        let e = Lowerer::new(None)
            .lower(&parse(
                "trait Show { fn show() -> int; } struct Point { x:int } \
                 impl Show for Point { }",
            ))
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Lowering);
    }
}
