//! # HIR —— High-Level Intermediate Representation
//!
//! HIR 是 AST 之后的第一层中间表示。相比 AST，HIR 的所有「名称」都已经被
//! 解析为 [`DefId`]（定义 ID），因而不再携带 `Path`/字符串形式的名字
//! （局部变量、函数、全局、结构体字段、结构体本身都已用 [`DefId`] 表示）。
//!
//! 但 HIR 仍然*没有*类型信息：字段访问 [`HirExpr::FieldAccess`] 中的
//! `field` 仍是字段名字符串，结构体字面量 [`HirExpr::StructLiteral`] 中的
//! 字段也是名字字符串 —— 这些只有在「类型检查」阶段（产出 TYPE HIR）才能
//! 根据对象的类型解析为 [`DefId`]。这样分层符合编译器惯例：先解析「值/函数」
//! 的名字，再依据类型解析「字段」的名字。

use crate::ast::{BinOp, GenericParam, Literal, Meta};

/// 定义 ID：编译器内部用来无歧义地引用每一个定义
/// （函数、外部函数、结构体、结构体字段、局部变量、全局变量……）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(pub u32);

/// HIR 中的可见性（当前语言没有可见性语法，全部按 [`Visibility::Public`] 处理）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Crate,
    Private,
}

/// 常量泛型实参：整数字面量，或某个常量泛型参数的下标（在单态化时按实例化实参映射为具体整数）。
///
/// 例如 `make_pair<T, const N: int>() -> Pair<T, N>` 的返回类型里，`N` 被记为 [`ConstArg::Param`]
/// （下标指 `make_pair` 自身的第 1 号常量参数）；单态化 `make_pair<int, 2>` 时该 `Param` 被解算为
/// [`ConstArg::Literal`]`(2)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstArg {
    Literal(i64),
    Param(usize),
}

impl ConstArg {
    /// 把一个常量实参转换为数组长度：整数字面量 → 已知长度；常量参数引用 → 仍记为参数下标。
    pub fn as_array_len(self, param_idx: usize) -> ArrayLen {
        match self {
            ConstArg::Literal(v) => ArrayLen::Known(v as usize),
            ConstArg::Param(_) => ArrayLen::Const(param_idx),
        }
    }
}

/// 数组长度：编译期已知长度，或某个常量泛型参数的下标（单态化时映射为具体整数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrayLen {
    Known(usize),
    /// 来自常量泛型参数（`const N: int`）的下标。
    Const(usize),
}

impl ArrayLen {
    /// 取编译期已知长度；常量泛型长度在单态化前不可用，调用方需保证已解算。
    pub fn known(&self) -> usize {
        match self {
            ArrayLen::Known(n) => *n,
            ArrayLen::Const(_) => {
                panic!("内部错误：数组长度仍依赖常量泛型参数，未在最終单态化")
            }
        }
    }
}

/// 构造「已知的」数组长度。
pub fn known_len(n: usize) -> ArrayLen {
    ArrayLen::Known(n)
}

/// HIR 中的类型。与 AST 的 `Type` 不同，`Named` 携带的是已解析的 [`DefId`]
/// （指向对应的 `Struct` 定义），而非源文件路径。
///
/// 该枚举是 **HIR 与 TYPE HIR 共用的唯一类型真相来源**（TyHIR 的
/// `TyHirExpr.ty`、参数/返回/字段类型都直接复用 `HirType`），
/// 因此新增指针/字符类型时无需在 tyhir 里再定义一套类型表示。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HirType {
    Int,
    /// UTF-8 字符（代码生成层映射为 `i8`）。
    Char,
    /// 指针类型，指向 `Box` 内的元素类型（如 `*int`、`char*`）。
    Pointer(Box<HirType>),
    Unit,
    Named(DefId),
    /// 定长数组类型 `[T; N]`。`N` 为数组长度（见 [`ArrayLen`]），`Box` 内为元素类型。
    /// 在代码生成层映射为一个指向连续存储的指针值（与结构体 `Named` 同约定）。
    Array(Box<HirType>, ArrayLen),
    /// 类型参数占位符。`usize` 是该泛型定义在其自身参数表里的下标（类型参数）。
    /// 仅存在于泛型「模板」之中，单态化时被替换为具体类型。
    Var(usize),
    /// 泛型应用类型：路径（已解析为结构体 [`DefId`]）加上类型实参与常量实参。
    /// 例如 `List<T>` / `Pair<int, 3>`。常量实参可为 [`ConstArg::Param`]（引用外层常量参数）
    /// 或 [`ConstArg::Literal`]（具体整数），单态化后 `Param` 被解算为 `Literal` 并最终变为 [`Named`]。
    Generic(DefId, Vec<HirType>, Vec<ConstArg>),
}

/// 函数 / 方法参数种类。用于区分「值参数」「类型参数」「常量参数」，
/// 使类型检查与单态化能够识别泛型参数（类型/常量参数在函数体内没有独立存储，
/// 但常量参数以 `Int` 类型的局部 [`DefId`] 形式被引用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    Value,
    TypeParam(usize),
    ConstParam(usize),
}

/// 涡轮鱼 / 显式泛型实参（HIR 层）：类型实参、整型常量实参，或常量参数名引用。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeOrConst {
    Type(HirType),
    Const(i64),
    /// 常量参数名引用（如 `Pair::<T, N>` 中的 `N`），`usize` 为该常量参数在「全部泛型参数」中的下标。
    /// 单态化时按实例化实参解算为具体整数。
    ConstParam(usize),
}

impl HirType {
    /// `char*`：指向 `char` 的指针，源码中写作 `char*` 或 `*char`。
    pub fn char_ptr() -> HirType {
        HirType::Pointer(Box::new(HirType::Char))
    }
}

/// 整个程序的 HIR 根：一个隐含的 crate 根模块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirCompilationUnit {
    pub root_module: HirModule,
}

/// 模块：一组 item。顶层源码整体构成一个根模块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirModule {
    pub def_id: DefId,
    pub name: String,
    pub visibility: Visibility,
    /// 该模块上的注解（`mod x;` 上的 `#[...]`），原样保留供后续阶段解读。
    pub attributes: Vec<Meta>,
    pub items: Vec<HirItem>,
}

/// 顶层/模块内条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HirItem {
    /// 子模块（由 `mod x;` 及被加载的源文件构成）。
    Module(HirModule),

    /// 外部函数声明（`extern fn`）。
    ExternFn {
        def_id: DefId,
        visibility: Visibility,
        /// 该声明上的注解（如 `#[link_name = "..."]`），原样保留；
        /// 具体的 `link_name` 已在 lowering 阶段解析并落到下面的 `link_name` 字段。
        attributes: Vec<Meta>,
        /// 链接名（导入符号名）。由 `#[link_name = "..."]` 注解提供；为 `None`
        /// 时回退为 mplang 函数名（见代码生成阶段）。
        link_name: Option<String>,
        name: String,
        param_types: Vec<HirType>,
        return_ty: HirType,
        is_variadic: bool,
    },

    /// 函数定义。
    Fn {
        def_id: DefId,
        visibility: Visibility,
        /// 该声明上的注解（如 `#[export_name = "..."]`、`#[no_mangle]` 等未来注解）。
        attributes: Vec<Meta>,
        name: String,
        /// 泛型参数声明（类型参数 / 常量参数）。空表示非泛型函数。
        generics: Vec<GenericParam>,
        params: Vec<HirParam>,
        return_ty: HirType,
        body: HirBody,
        /// 若该函数属于某个 `impl` 块，则记录其接收者类型；自由函数此字段为 `None`。
        /// 接收者 `self` 在 lowering 阶段已被显式加入 `params`（作为首个参数）。
        impl_receiver: Option<HirType>,
    },

    /// 结构体定义。
    Struct {
        def_id: DefId,
        visibility: Visibility,
        /// 该声明上的注解（如 `#[repr(...)]` 等未来注解）。
        attributes: Vec<Meta>,
        name: String,
        /// 泛型参数声明（类型参数 / 常量参数）。空表示非泛型结构体。
        generics: Vec<GenericParam>,
        fields: Vec<HirField>,
    },

    /// 全局变量（`static`）或编译期常量（`const`）。
    Static {
        def_id: DefId,
        visibility: Visibility,
        /// 该声明上的注解（如 `#[link_section = "..."]` 等未来注解）。
        attributes: Vec<Meta>,
        name: String,
        ty: HirType,
        init: HirExpr,
        is_const: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirParam {
    pub def_id: DefId,
    pub name: String,
    pub ty: HirType,
    /// 参数种类：值参数 / 类型参数 / 常量参数。泛型模板里用于区分。
    pub kind: ParamKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirField {
    pub def_id: DefId,
    pub name: String,
    pub ty: HirType,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirBody {
    pub stmts: Vec<HirStmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HirStmt {
    Let {
        def_id: DefId,
        name: String,
        /// 源代码中显式标注的类型；若为 `None` 则类型由初始化表达式推断。
        ty: Option<HirType>,
        init: HirExpr,
    },

    Assign {
        target: HirExpr,
        value: HirExpr,
    },

    Expr(HirExpr),

    If {
        cond: HirExpr,
        then_branch: HirBody,
        else_branch: Option<HirBody>,
    },

    While {
        cond: HirExpr,
        body: HirBody,
    },

    Return(Option<HirExpr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HirExpr {
    Literal(Literal),

    /// 一个「值」名字：局部变量 / 参数 / 全局变量，已解析为 [`DefId`]。
    Path(DefId),

    Binary {
        op: BinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
    },

    /// 函数调用，被调用者已解析为 [`DefId`]。
    Call {
        callee: DefId,
        args: Vec<HirExpr>,
        /// 涡轮鱼实参（`::<...>`）。位置与泛型参数声明一一对应；空表示依赖推断。
        turbofish: Vec<TypeOrConst>,
    },

    /// 字段访问。`field` 在 HIR 阶段仍是字段名字符串，
    /// 在类型检查阶段会依据 `object` 的类型解析为字段的 [`DefId`]。
    FieldAccess {
        object: Box<HirExpr>,
        field: String,
    },

    /// 结构体字面量。`def_id` 是结构体定义的 [`DefId`]（已在 lowering 阶段解析）；
    /// 字段仍以名字字符串保存，在类型检查阶段解析为字段 [`DefId`]。
    StructLiteral {
        def_id: DefId,
        fields: Vec<(String, HirExpr)>,
        /// 涡轮鱼实参（`::<...>`）。位置与泛型参数声明一一对应；空表示依赖推断（字段初始化表达式）。
        turbofish: Vec<TypeOrConst>,
    },

    /// 数组字面量。
    /// - `elements`：元素表达式（重复形式 `[v; n]` 仅含模板元素 `v`）。
    /// - `repeat`：重复形式的重复次数 `n`；列表形式为 `None`。
    ///
    /// 元素类型与数组长度在类型检查阶段确定（产出 TYPE HIR）。
    ArrayLiteral {
        elements: Vec<HirExpr>,
        repeat: Option<usize>,
    },

    /// 数组下标访问 `a[i]`。
    Index {
        array: Box<HirExpr>,
        index: Box<HirExpr>,
    },

    /// 取地址符 `&e`：得到一个指向 `e` 的指针。
    AddressOf(Box<HirExpr>),

    /// 解引用 `*p`：读取（或作为赋值目标写入）指针 `p` 所指向的值。
    Deref(Box<HirExpr>),

    /// 方法调用 `obj.method(args)`。`object` 与 `args` 已降低为 HIR 表达式，但方法名
    /// 此时仍是字符串——需在类型检查阶段依据接收者 `object` 的类型解析为函数 [`DefId`]，
    /// 并改写为普通的 [`HirExpr::Call`]（接收者作为首个实参）。代码生成阶段不会见到此变体。
    MethodCall {
        object: Box<HirExpr>,
        name: String,
        args: Vec<HirExpr>,
    },
}
