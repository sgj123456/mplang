//! # TYPE HIR —— 类型化的高层中间表示
//!
//! TYPE HIR（简称 TyHIR）是 HIR 经过「类型检查」之后得到的第二层 IR。
//! 与 HIR 相比，它的关键区别在于：
//!
//! 1. **每个表达式都带有类型**：[`TyHirExpr`] 用 `ty: HirType` 字段携带
//!    该表达式求值结果的类型，下游代码生成阶段可以直接使用，无需再次推断。
//! 2. **字段名被解析为 [`DefId`]**：[`TyHirExprKind::FieldAccess`] 的
//!    `field` 是字段定义的 [`DefId`]；[`TyHirExprKind::StructLiteral`] 的
//!    字段列表也是 `(字段 DefId, 表达式)`。
//! 3. **类型已经过校验**：类型检查阶段会拦截类型不匹配并给出清晰报错，
//!    所以代码生成阶段可以信任 TyHIR 中的类型信息。
//!
//! 换言之，TYPE HIR = HIR + 类型标注 + 字段解析结果。它是「信任边界」：
//! 经过 TyHIR 后的程序保证类型良好，后端只需照着它生成机器码即可。

use std::collections::HashMap;

use crate::ast::{BinOp, GenericParam, Literal, Meta};
use crate::hir::{DefId, HirType, ParamKind, TypeOrConst, Visibility};

/// 整个程序的 TyHIR 根。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyHirCompilationUnit {
    pub root_module: TyHirModule,
    /// vtable 表：(trait_def, impl_type_def) → 按 trait 方法声明顺序排列的具体方法 DefId 列表。
    /// codegen 阶段据此发射函数指针数组。
    pub vtables: HashMap<(DefId, DefId), Vec<DefId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyHirModule {
    pub def_id: DefId,
    pub name: String,
    pub visibility: Visibility,
    pub attributes: Vec<Meta>,
    pub items: Vec<TyHirItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyHirItem {
    Module(TyHirModule),

    ExternFn {
        def_id: DefId,
        visibility: Visibility,
        /// 原样保留的注解列表（同 HIR 的 `ExternFn`）。
        attributes: Vec<Meta>,
        /// 链接名（导入符号名），由 `#[link_name = "..."]` 提供；`None` 时回退为函数名。
        link_name: Option<String>,
        name: String,
        param_types: Vec<HirType>,
        return_ty: HirType,
        is_variadic: bool,
    },

    Fn {
        def_id: DefId,
        visibility: Visibility,
        attributes: Vec<Meta>,
        name: String,
        /// 泛型参数声明（类型参数 / 常量参数）。单态化阶段据其生成具体实例；
        /// 非泛型函数为空。后端不使用此字段。
        generics: Vec<GenericParam>,
        params: Vec<TyHirParam>,
        return_ty: HirType,
        body: TyHirBody,
        /// 若为 `impl` 块中的方法，记录接收者类型；自由函数为 `None`。
        impl_receiver: Option<HirType>,
    },

    Struct {
        def_id: DefId,
        visibility: Visibility,
        attributes: Vec<Meta>,
        name: String,
        /// 泛型参数声明（类型参数 / 常量参数）。单态化阶段据其生成具体实例；
        /// 非泛型结构体为空。后端不使用此字段。
        generics: Vec<GenericParam>,
        fields: Vec<TyHirField>,
    },

    Enum {
        def_id: DefId,
        visibility: Visibility,
        attributes: Vec<Meta>,
        name: String,
        generics: Vec<GenericParam>,
        variants: Vec<TyHirEnumVariant>,
    },

    Static {
        def_id: DefId,
        visibility: Visibility,
        attributes: Vec<Meta>,
        name: String,
        ty: HirType,
        init: TyHirExpr,
        is_const: bool,
    },

    /// vtable 表。`trait_def` 为 trait 定义 ID；`impl_type_def` 为实现方类型定义 ID；
    /// `methods` 为按 trait 方法声明顺序排列的具体方法 [`DefId`] 列表。
    /// codegen 阶段据此发射函数指针数组。
    VTable {
        trait_def: DefId,
        impl_type_def: DefId,
        methods: Vec<DefId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyHirParam {
    pub def_id: DefId,
    pub name: String,
    pub ty: HirType,
    /// 参数种类（值 / 类型参数 / 常量参数）。单态化据此识别常量参数，
    /// 实例化后常量参数退化为整型字面量。后端不使用此字段。
    pub kind: ParamKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyHirField {
    pub def_id: DefId,
    pub name: String,
    pub ty: HirType,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyHirEnumVariant {
    pub def_id: DefId,
    pub name: String,
    pub tag: u32,
    pub fields: Vec<TyHirField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyHirBody {
    pub stmts: Vec<TyHirStmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyHirStmt {
    Let {
        def_id: DefId,
        name: String,
        /// 绑定类型：显式标注或（缺失时）由初始化表达式推断得到。
        ty: HirType,
        init: TyHirExpr,
    },

    Assign {
        target: TyHirExpr,
        value: TyHirExpr,
    },

    Expr(TyHirExpr),

    If {
        /// 条件一定是 `int` 类型。
        cond: TyHirExpr,
        then_branch: TyHirBody,
        else_branch: Option<TyHirBody>,
    },

    While {
        cond: TyHirExpr,
        body: TyHirBody,
    },

    Return(Option<TyHirExpr>),
}

/// 带类型的表达式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyHirExpr {
    /// 该表达式求值结果的类型。
    pub ty: HirType,
    pub kind: TyHirExprKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyHirExprKind {
    Literal(Literal),

    /// 值名字，已解析为定义 ID。
    Path(DefId),

    Binary {
        op: BinOp,
        lhs: Box<TyHirExpr>,
        rhs: Box<TyHirExpr>,
    },

    Call {
        callee: DefId,
        args: Vec<TyHirExpr>,
        /// 涡轮鱼实参（透传 HIR，单态化阶段用于推断实例）。
        turbofish: Vec<TypeOrConst>,
    },

    FieldAccess {
        object: Box<TyHirExpr>,
        /// 已解析为对应结构体字段的 [`DefId`]。
        field: DefId,
    },

    StructLiteral {
        /// 结构体定义的 [`DefId`]。
        def_id: DefId,
        /// `(字段 DefId, 字段值表达式)`。
        fields: Vec<(DefId, TyHirExpr)>,
        /// 涡轮鱼实参（透传 HIR，单态化阶段用于推断实例）。
        turbofish: Vec<TypeOrConst>,
    },

    /// 数组字面量。整个表达式类型为 [`HirType::Array`](crate::hir::HirType::Array)，
    /// 携带已确定的元素类型与长度。
    ArrayLiteral {
        /// 元素表达式（重复形式 `[v; n]` 仅含模板元素 `v`）。
        elements: Vec<TyHirExpr>,
        /// 重复形式的重复次数 `n`；列表形式为 `None`。
        repeat: Option<usize>,
    },

    /// 数组下标访问 `a[i]`。表达式类型为数组元素类型。
    Index {
        array: Box<TyHirExpr>,
        index: Box<TyHirExpr>,
    },

    /// 取地址符 `&e`：结果是 `Pointer(te.ty)` 类型的指针值。
    AddressOf(Box<TyHirExpr>),

    /// 解引用 `*p`：读取（或作为赋值目标写入）`p` 所指向的值，类型为指针的承载类型。
    Deref(Box<TyHirExpr>),

    /// 枚举变体构造。`def_id` 为枚举定义 DefId；`variant` 为变体名；`args` 为载荷实参。
    /// 整个表达式类型为 `HirType::Enum(def_id, targs, cargs)`。
    Variant {
        def_id: DefId,
        variant: String,
        args: Vec<TyHirExpr>,
        turbofish: Vec<TypeOrConst>,
    },

    /// `match` 表达式。`scrutinee_ty` 为被匹配值的类型（`Enum` 或 `Int`）。
    Match {
        scrutinee: Box<TyHirExpr>,
        arms: Vec<TyHirMatchArm>,
    },

    /// 将 `value`（一个实现了 trait `trait_def` 的 struct 值）转换为 trait 对象。
    TraitCast {
        value: Box<TyHirExpr>,
        trait_def: DefId,
    },

    /// 对 trait 对象的动态方法调用。`method_index` 为该方法在 trait 方法声明中的序号；
    /// `receiver` 为 trait 对象值（fat pointer）；`args` 为除 self 外的其余实参。
    /// 返回类型在 `ty` 字段中。
    DynamicMethodCall {
        trait_def: DefId,
        method_index: usize,
        receiver: Box<TyHirExpr>,
        args: Vec<TyHirExpr>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TyHirMatchArm {
    pub pattern: TyHirPattern,
    pub body: TyHirBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyHirPattern {
    Variant {
        enum_def: DefId,
        variant: String,
        bindings: Vec<(DefId, Option<HirType>)>,
    },
    Literal(Literal),
    Wildcard,
    Ident(DefId),
}
