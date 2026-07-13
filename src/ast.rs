use std::fmt::Display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Less,
    LessEqual,
    Equal,
    NotEqual,
    Greater,
    GreaterEqual,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Literal {
    Int(i64),
    String(String),
}

/// 泛型参数的种类。
/// - `Type`：类型参数 `T`（可出现在类型位置）。
/// - `Const`：常量参数 `const N: int`（可出现在数组长度与需要 `int` 的值位置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericParamKind {
    Type,
    Const,
}

/// 一个泛型参数声明，如 `T` 或 `const N: int`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericParam {
    pub name: String,
    pub kind: GenericParamKind,
}

/// 涡轮鱼 / 显式泛型实参：要么是「类型实参」，要么是「整型常量实参」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeOrConst {
    Type(Type),
    Const(i64),
}

/// 数组长度：既可以是编译期已知的整数，也可以是一个常量泛型参数 `const N`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArrayLen {
    /// 编译期已知长度。
    Known(usize),
    /// 来自常量泛型参数（保存该常量参数在其声明中的名字，lowering 阶段解析为下标）。
    Param(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Int,
    Char,
    /// 指针类型 `*T`。`string` 在解析层被等价替换为 `Pointer(Char)`（即 C 的 `char*`）。
    Pointer(Box<Type>),
    Unit,
    Named(Path),
    /// 定长数组类型 `[T; N]`。N 为编译期非负整数（数组长度），或某个常量泛型参数 `N`，
    /// T 为元素类型。元素类型支持标量、指针、结构体以及嵌套数组。
    Array(Box<Type>, ArrayLen),
    /// 泛型应用类型，如 `List<T>` / `Pair<int, 3>`：路径 + 实参列表。
    /// `lowering` 阶段解析路径为结构体定义，并把实参映射为类型 / 常量参数。
    Applied(Path, Vec<TypeOrConst>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Path {
    pub segments: Vec<String>,
}
impl Display for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.segments.iter();
        if let Some(first) = iter.next() {
            write!(f, "{}", first)?;
            for segment in iter {
                write!(f, "::{}", segment)?;
            }
        }
        Ok(())
    }
}
impl Path {
    pub fn new(segments: Vec<String>) -> Self {
        Self { segments }
    }

    pub fn simple(name: impl Into<String>) -> Self {
        Self {
            segments: vec![name.into()],
        }
    }

    pub fn last(&self) -> Option<&str> {
        self.segments.last().map(|s| s.as_str())
    }
}

/// 注解（attribute）的通用内部结构，源码语法为 `#[ ... ]`。
///
/// 被设计为「元项（meta item）」树，以支持三类形态，方便以后与用户自定义注解扩展：
/// - 裸标记：`#[no_mangle]`
/// - 键值对：`#[link_name = "foo"]`
/// - 列表（为将来复杂注解预留，如 `#[cfg(any(a, b))]`）
///
/// 由于所有注解都收敛到同一棵树，新增注解**无需改动词法 / 语法 / IR 结构**，
/// 只需在读取处（如 [`attr_string_value`]）增加按名匹配即可——这是「通用注解机制」
/// 与「只特殊处理 link_name」的关键区别。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Meta {
    /// 标识符或带命名空间的路径（`no_mangle`、`std::link`）。
    /// 既可作「裸标记注解」，也可作键值 / 列表的名字部分。
    Path(Path),

    /// `name = value`。当前 `value` 仅支持字符串字面量（`#[link_name = "foo"]`），
    /// 以便将来还能容纳整型 / 布尔 / 路径等值。
    NameValue { name: Path, value: Literal },

    /// `name(...)`，为将来复杂注解预留（如 `#[cfg(...)]`）。
    List { name: Path, items: Vec<Meta> },
}

impl Meta {
    /// 取该元项的「名字路径」，用于按名检索注解。
    pub fn name(&self) -> &Path {
        match self {
            Meta::Path(p) => p,
            Meta::NameValue { name, .. } => name,
            Meta::List { name, .. } => name,
        }
    }

    /// 该注解的「裸名」（名字路径的最后一段），如 `link_name`、`no_mangle`。
    pub fn simple_name(&self) -> Option<&str> {
        self.name().last()
    }
}

/// 在注解列表中查找名为 `key` 的「键值对」注解（如 `#[link_name = "xxx"]`）。
///
/// 这是**通用注解读取**的入口：以后新增任意键值型注解时，只要在此类 helper 中
/// 增加一行按名匹配即可，注解的解析与存储全程无需改动。
/// 仅当 `value` 为字符串字面量时返回其值（`#[link_name = 3]` 这类形状暂不支持，会
/// 被忽略，便于将来扩展）。
pub fn attr_string_value<'a>(attrs: &'a [Meta], key: &str) -> Option<&'a str> {
    for a in attrs {
        if let Meta::NameValue { name, value } = a
            && name.last() == Some(key)
            && let Literal::String(s) = value
        {
            return Some(s);
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilationUnit {
    pub declarations: Vec<TopLevelDecl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopLevelDecl {
    ModDecl {
        /// 该声明上的注解（`#[...]`）。这些注解被原样保留，供后续阶段按名解读。
        attributes: Vec<Meta>,
        name: String,
    },

    UseDecl {
        attributes: Vec<Meta>,
        path: Path,
    },

    /// 外部 crate 声明：`extern crate NAME;`，可前置 `#[path = "..."]` 注解
    /// 指定该 crate 源文件的查找路径（相对输入目录，或绝对路径）。
    /// 语义上等价于把一个外部编译单元作为名为 `NAME` 的模块引入，
    /// 其公开项可通过 `NAME::item` 或 `use NAME::item;` 访问。
    ExternCrate {
        attributes: Vec<Meta>,
        name: String,
    },

    ExternFnDef {
        attributes: Vec<Meta>,
        name: String,
        param_types: Vec<Type>,
        return_ty: Type,
        is_variadic: bool,
    },

    FnDef {
        attributes: Vec<Meta>,
        name: String,
        generics: Vec<GenericParam>,
        params: Vec<Param>,
        return_ty: Type,
        body: Vec<Stmt>,
    },

    StructDef {
        attributes: Vec<Meta>,
        name: String,
        generics: Vec<GenericParam>,
        fields: Vec<(String, Type)>,
    },

    /// `impl <类型> { ... }`：为某类型添加方法（每个方法都是普通函数，
    /// 第一个（隐式）参数 `self` 是接收者，类型即该 `impl` 的类型）。
    Impl {
        attributes: Vec<Meta>,
        generics: Vec<GenericParam>,
        /// `impl Trait for T` 时为 `Some(trait 名)`；普通 `impl T` 为 `None`。
        trrait: Option<Path>,
        ty: Type,
        methods: Vec<ImplMethod>,
    },

    /// `trait <名> { ... }`：声明一组方法签名（编译期契约）。
    /// 方法体可选：无体（`fn m();`）为「必须由实现方提供」；
    /// 有体（`fn m() {}`，体可空）为「默认实现」，类型可重写。
    Trait {
        attributes: Vec<Meta>,
        name: String,
        generics: Vec<GenericParam>,
        methods: Vec<TraitMethod>,
    },

    /// 顶层全局变量（`static NAME: TY = INIT;`）。
    Static {
        attributes: Vec<Meta>,
        name: String,
        ty: Type,
        init: Expr,
    },

    /// 顶层编译期常量（`const NAME: TY = INIT;`）。
    Const {
        attributes: Vec<Meta>,
        name: String,
        ty: Type,
        init: Expr,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Literal(Literal),

    Ident(Path),

    Paren(Box<Expr>),

    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },

    Call {
        callee: Path,
        args: Vec<Expr>,
        /// 涡轮鱼实参（`::<...>`）。位置与泛型参数声明一一对应；空表示依赖推断。
        turbofish: Vec<TypeOrConst>,
    },

    FieldAccess {
        object: Box<Expr>,
        field: String,
    },

    StructLiteral {
        name: Path,
        fields: Vec<(String, Expr)>,
        /// 涡轮鱼实参（`::<...>`）。位置与泛型参数声明一一对应；空表示依赖推断（字段初始化表达式）。
        turbofish: Vec<TypeOrConst>,
    },

    /// 数组字面量。
    /// - `elements`：所有元素表达式。列表形式 `[1, 2, 3]` 时即各元素；
    ///   重复形式 `[v; n]` 时仅含一个模板元素 `v`。
    /// - `repeat`：重复形式的重复次数（即 `[v; n]` 中的 `n`，须为整型字面量）；
    ///   列表形式为 `None`。
    ArrayLiteral {
        elements: Vec<Expr>,
        repeat: Option<Box<Expr>>,
    },

    /// 数组下标访问 `a[i]`（既可作为右值读取，也可作为赋值左值写入）。
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
    },

    /// 取地址符 `&e`：得到一个指向 `e` 的指针。
    AddrOf(Box<Expr>),

    /// 解引用 `*p`：读取（或作为赋值目标写入）指针 `p` 所指向的值。
    Deref(Box<Expr>),

    /// 方法调用 `obj.method(args)`。`object` 求值得到接收者，其类型用于
    /// 在类型检查阶段解析出对应的方法 [`DefId`]。方法名 `name` 在此仍保留字符串，
    /// 待类型检查依据接收者类型解析。
    MethodCall {
        object: Box<Expr>,
        name: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
}

/// `impl` 块内的方法声明。结构与 [`TopLevelDecl::FnDef`] 相同，
/// 区别是它属于某个 `impl` 快，并隐式带有一个 `self` 接收者参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplMethod {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub body: Vec<Stmt>,
}

/// `trait` 声明里的一个方法。
/// - `default_body` 为 `None` 表示「必须由实现方提供」（以 `fn m();` 结尾）；
/// - 为 `Some(body)` 表示「默认实现」（以 `fn m() {}` 给出，可被重写）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitMethod {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub default_body: Option<Vec<Stmt>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Let {
        name: String,
        ty: Option<Type>,
        init: Expr,
    },

    Static {
        name: String,
        ty: Type,
        init: Expr,
    },

    Const {
        name: String,
        ty: Type,
        init: Expr,
    },

    Assign {
        target: Box<Expr>,
        value: Expr,
    },

    Expr(Expr),

    If {
        cond: Expr,
        then_branch: Vec<Stmt>,
        else_branch: Option<Vec<Stmt>>,
    },

    While {
        cond: Expr,
        body: Vec<Stmt>,
    },

    Return(Option<Expr>),
}
