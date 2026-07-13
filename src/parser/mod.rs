use crate::error::{MplangError, into_result};
use crate::{
    ast::{CompilationUnit, Literal, Meta, Path},
    token::{Token, TokenKind},
};

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Parser {
        Parser { tokens, current: 0 }
    }

    // ========================================================================
    // 工具方法
    // ========================================================================

    fn is_at_end(&self) -> bool {
        if self.current >= self.tokens.len() {
            return true;
        }
        self.tokens[self.current].kind == TokenKind::Eof
    }

    fn current(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        &self.tokens[self.current - 1]
    }

    fn check(&self, kind: &TokenKind) -> bool {
        if self.is_at_end() {
            return false;
        }
        &self.current().kind == kind
    }

    fn check_next(&self, kind: &TokenKind) -> bool {
        if self.current + 1 >= self.tokens.len() {
            return false;
        }
        &self.tokens[self.current + 1].kind == kind
    }

    fn consume(&mut self, kind: TokenKind, message: &str) -> Token {
        if self.check(&kind) {
            self.advance().clone()
        } else {
            crate::error::fatal(MplangError::parse(format!(
                "{}（第 {} 行，第 {} 列）：期望 {:?}，实际 {:?}",
                message,
                self.current().line,
                self.current().column,
                kind,
                self.current().kind
            )))
        }
    }

    // ========================================================================
    // 路径解析
    // ========================================================================

    /// 解析限定路径，如 add::add 或 printf
    fn parse_path(&mut self) -> Path {
        let first = self
            .consume(TokenKind::Ident, "Expected identifier in path")
            .lexeme
            .clone();
        let mut segments = vec![first];

        // 连续匹配 ::ident
        while self.check(&TokenKind::ColonColon) {
            self.advance(); // consume ::
            let seg = self
                .consume(TokenKind::Ident, "Expected identifier after '::'")
                .lexeme
                .clone();
            segments.push(seg);
        }

        Path::new(segments)
    }

    // ========================================================================
    // 程序入口
    // ========================================================================

    fn parse_program(&mut self) -> CompilationUnit {
        let mut declarations = Vec::new();
        while !self.is_at_end() {
            declarations.push(self.top_level_decl());
        }
        log::debug!("语法分析完成");
        CompilationUnit { declarations }
    }

    // ========================================================================
    // 注解（attribute）解析：语法 `#[ ... ]`
    // ========================================================================

    /// 解析紧接在声明前的一组注解：`#[a] #[b = "x"] ...`。
    /// 一次消费所有连续的 `#[...]`，便于「通用注解机制」把任意数量注解附加到声明上。
    fn parse_attributes(&mut self) -> Vec<Meta> {
        let mut attrs = Vec::new();
        while self.check(&TokenKind::Hash) {
            attrs.push(self.parse_attribute());
        }
        attrs
    }

    /// 解析单个注解：`#[ <meta-item> ]`。
    fn parse_attribute(&mut self) -> Meta {
        self.consume(TokenKind::Hash, "期望 '#' 作为注解起始");
        self.consume(
            TokenKind::LeftBracket,
            "期望 '['（'#' 后应为 '[' 形成 '#['）",
        );
        let meta = self.parse_meta_item();
        self.consume(TokenKind::RightBracket, "期望 ']' 闭合注解");
        meta
    }

    /// 解析注解内部的一个「元项」（meta item），支持三类形态：
    /// - `name`（裸标记/路径）→ [`Meta::Path`]
    /// - `name = value`（当前 value 仅限字符串字面量）→ [`Meta::NameValue`]
    /// - `name(...)`（为将来复杂注解预留）→ [`Meta::List`]
    fn parse_meta_item(&mut self) -> Meta {
        let name = self.parse_path();
        if self.check(&TokenKind::Assign) {
            // `name = value`
            self.advance();
            let value_tok = self.consume(
                TokenKind::StringLiteral,
                "注解键值对的值目前仅支持字符串字面量（如 #[link_name = \"foo\"]）",
            );
            let raw = &value_tok.lexeme;
            // 去掉首尾引号，得到注解的实际字符串值。
            let inner = raw[1..raw.len().saturating_sub(1)].to_string();
            Meta::NameValue {
                name,
                value: Literal::String(inner),
            }
        } else if self.check(&TokenKind::LeftParen) {
            // `name(...)`：为将来复杂注解预留。
            self.advance();
            let mut items = Vec::new();
            while !self.check(&TokenKind::RightParen) && !self.is_at_end() {
                items.push(self.parse_meta_item());
                if self.check(&TokenKind::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.consume(TokenKind::RightParen, "期望 ')' 闭合注解列表");
            Meta::List { name, items }
        } else {
            // `name`（裸标记 / 路径）
            Meta::Path(name)
        }
    }

    /// 语法分析入口，返回 [`Result`]，由 [`into_result`] 收拢解析期错误。
    pub fn parse(&mut self) -> Result<CompilationUnit, MplangError> {
        into_result(|| self.parse_program())
    }
}

// 让测试模块 `use super::*` 能直接取到 `unescape_string`（定义于 expr.rs）；仅测试用到。
#[cfg(test)]
pub(crate) use expr::unescape_string;

mod decl;
mod expr;
mod stmt;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;
    use crate::error::{ErrorKind, MplangError};
    use crate::lexer::Lexer;

    fn parse(src: &str) -> CompilationUnit {
        let toks = Lexer::new(src.chars().collect()).lex().unwrap();
        Parser::new(toks).parse().unwrap()
    }

    fn parse_err(src: &str) -> MplangError {
        let toks = Lexer::new(src.chars().collect()).lex().unwrap();
        Parser::new(toks).parse().unwrap_err()
    }

    #[test]
    fn parse_empty_function() {
        let u = parse("fn main() {}");
        assert_eq!(u.declarations.len(), 1);
        if let TopLevelDecl::FnDef {
            name,
            params,
            return_ty,
            body,
            ..
        } = &u.declarations[0]
        {
            assert_eq!(name, "main");
            assert!(params.is_empty());
            assert_eq!(return_ty, &Type::Unit);
            assert!(body.is_empty());
        } else {
            panic!("expected FnDef");
        }
    }

    #[test]
    fn parse_let_with_binary_expr() {
        let u = parse("fn main() { let x:int = 1 + 2; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Let { ty, init, .. } = &body[0] {
                assert_eq!(ty, &Some(Type::Int));
                assert!(matches!(init, Expr::Binary { op: BinOp::Add, .. }));
            } else {
                panic!("expected let");
            }
        } else {
            panic!("expected fndef");
        }
    }

    #[test]
    fn parse_struct_declaration() {
        let u = parse("struct Point { x:int, y:int }");
        if let TopLevelDecl::StructDef { name, fields, .. } = &u.declarations[0] {
            assert_eq!(name, "Point");
            assert_eq!(
                fields,
                &vec![("x".to_string(), Type::Int), ("y".to_string(), Type::Int)]
            );
        } else {
            panic!("expected struct");
        }
    }

    #[test]
    fn parse_use_path_with_coloncolon() {
        let u = parse("use add::add;");
        if let TopLevelDecl::UseDecl { path, .. } = &u.declarations[0] {
            assert_eq!(path.segments, vec!["add".to_string(), "add".to_string()]);
        } else {
            panic!("expected use");
        }
    }

    #[test]
    fn parse_extern_variadic() {
        let u = parse("extern fn printf(fmt:*char, ...);");
        if let TopLevelDecl::ExternFnDef {
            attributes,
            name,
            is_variadic,
            param_types,
            ..
        } = &u.declarations[0]
        {
            assert_eq!(name, "printf");
            assert!(is_variadic);
            assert_eq!(param_types, &vec![Type::Pointer(Box::new(Type::Char))]);
            // 没有 link_name 注解时，链接名缺省为 None。
            assert_eq!(crate::ast::attr_string_value(attributes, "link_name"), None);
        } else {
            panic!("expected extern fn");
        }
    }

    /// `#[link_name = "..."]` 注解取代原先 `extern "..."` 的写法，提供导入符号名。
    #[test]
    fn parse_extern_link_name_attribute() {
        let u = parse("#[link_name = \"C\"] extern fn printf(fmt:*char, ...);");
        if let TopLevelDecl::ExternFnDef {
            attributes,
            name,
            is_variadic,
            ..
        } = &u.declarations[0]
        {
            assert_eq!(name, "printf");
            assert!(is_variadic);
            assert_eq!(
                crate::ast::attr_string_value(attributes, "link_name"),
                Some("C")
            );
        } else {
            panic!("expected extern fn");
        }
    }

    /// 裸标记注解（无值）也能被正确解析为 `Meta::Path` 并附加到声明上，
    /// 体现「通用注解机制」不局限于 link_name。
    #[test]
    fn parse_bare_attribute() {
        let u = parse("#[no_mangle] fn f() {}");
        if let TopLevelDecl::FnDef { attributes, .. } = &u.declarations[0] {
            assert_eq!(attributes.len(), 1);
            assert_eq!(attributes[0].simple_name(), Some("no_mangle"));
        } else {
            panic!("expected fn");
        }
    }

    /// 多个注解可以连续叠加，并被原样保留在 `attributes` 中。
    #[test]
    fn parse_multiple_attributes() {
        let u = parse("#[no_mangle] #[link_name = \"real\"] extern fn g() -> int;");
        if let TopLevelDecl::ExternFnDef {
            attributes, name, ..
        } = &u.declarations[0]
        {
            assert_eq!(name, "g");
            assert_eq!(attributes.len(), 2);
            assert_eq!(attributes[0].simple_name(), Some("no_mangle"));
            assert_eq!(
                crate::ast::attr_string_value(attributes, "link_name"),
                Some("real")
            );
        } else {
            panic!("expected extern fn");
        }
    }

    #[test]
    fn syntax_error_reports_parse_kind() {
        let e = parse_err("fn main() { let x:int = ; }");
        assert_eq!(e.kind, ErrorKind::Parse);
    }

    /// `extern crate NAME;` 解析为 `ExternCrate`，并记录 crate 名。
    #[test]
    fn parse_extern_crate() {
        let u = parse("extern crate foo;");
        if let TopLevelDecl::ExternCrate {
            name, attributes, ..
        } = &u.declarations[0]
        {
            assert_eq!(name, "foo");
            assert!(attributes.is_empty());
        } else {
            panic!("expected extern crate");
        }
    }

    /// `#[path = "..."]` 注解为 `extern crate` 指定源文件查找路径。
    #[test]
    fn parse_extern_crate_with_path() {
        let u = parse("#[path = \"lib/foo.mp\"] extern crate foo;");
        if let TopLevelDecl::ExternCrate {
            name, attributes, ..
        } = &u.declarations[0]
        {
            assert_eq!(name, "foo");
            assert_eq!(
                crate::ast::attr_string_value(attributes, "path"),
                Some("lib/foo.mp")
            );
        } else {
            panic!("expected extern crate");
        }
    }

    /// 指针类型 `*T` 被正确解析（右结合，可多层嵌套）。
    #[test]
    fn parse_pointer_type() {
        let u = parse("fn f(p:*int) {}");
        if let TopLevelDecl::FnDef { params, .. } = &u.declarations[0] {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "p");
            assert_eq!(params[0].ty, Type::Pointer(Box::new(Type::Int)));
        } else {
            panic!("expected FnDef");
        }
    }

    /// 源码里用 `char*`（或 `*char`）表示「指向 char 的指针」，解析为 `Pointer(Char)`。
    #[test]
    fn parse_char_ptr_is_char_ptr() {
        let u = parse("fn f(s:*char) {}");
        if let TopLevelDecl::FnDef { params, .. } = &u.declarations[0] {
            assert_eq!(params[0].ty, Type::Pointer(Box::new(Type::Char)));
        } else {
            panic!("expected FnDef");
        }
    }

    /// 取地址符 `&e` 解析为 [`Expr::AddrOf`]。
    #[test]
    fn parse_address_of() {
        let u = parse("fn main() { let p:*int = &x; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Let { init, .. } = &body[0] {
                assert!(matches!(init, Expr::AddrOf(_)));
            } else {
                panic!("expected let");
            }
        } else {
            panic!("expected FnDef");
        }
    }

    /// 解引用 `*p` 解析为 [`Expr::Deref`]。
    #[test]
    fn parse_deref() {
        let u = parse("fn main() { let y:int = *p; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Let { init, .. } = &body[0] {
                assert!(matches!(init, Expr::Deref(_)));
            } else {
                panic!("expected let");
            }
        } else {
            panic!("expected FnDef");
        }
    }

    /// 通过解引用写回 `*p = v` 解析为赋值语句，目标为 [`Expr::Deref`]。
    #[test]
    fn parse_assign_through_deref() {
        let u = parse("fn main() { *p = 2; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Assign { target, .. } = &body[0] {
                assert!(matches!(&**target, Expr::Deref(_)));
            } else {
                panic!("expected assign");
            }
        } else {
            panic!("expected FnDef");
        }
    }

    /// 结构体字段赋值 `s.x = v` 解析为赋值语句，目标为 [`Expr::FieldAccess`]。
    #[test]
    fn parse_assign_to_struct_field() {
        let u = parse("fn main() { s.x = 3; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Assign { target, .. } = &body[0] {
                assert!(matches!(&**target, Expr::FieldAccess { .. }));
            } else {
                panic!("expected assign");
            }
        } else {
            panic!("expected FnDef");
        }
    }

    #[test]
    fn unescape_handles_escapes() {
        assert_eq!(unescape_string("a\\nb\\t").unwrap(), "a\nb\t");
        assert!(unescape_string("bad\\x").is_err());
    }

    /// 定长数组类型 `[T; N]` 被正确解析为 [`Type::Array`]。
    #[test]
    fn parse_array_type() {
        let u = parse("fn f(a: [int; 4]) {}");
        if let TopLevelDecl::FnDef { params, .. } = &u.declarations[0] {
            assert_eq!(
                params[0].ty,
                Type::Array(Box::new(Type::Int), crate::ast::ArrayLen::Known(4))
            );
        } else {
            panic!("expected FnDef");
        }
    }

    /// 嵌套数组类型 `[[int; 2]; 3]` 正确解析。
    #[test]
    fn parse_nested_array_type() {
        let u = parse("fn f(a: [[int; 2]; 3]) {}");
        if let TopLevelDecl::FnDef { params, .. } = &u.declarations[0] {
            assert_eq!(
                params[0].ty,
                Type::Array(
                    Box::new(Type::Array(
                        Box::new(Type::Int),
                        crate::ast::ArrayLen::Known(2)
                    )),
                    crate::ast::ArrayLen::Known(3)
                )
            );
        } else {
            panic!("expected FnDef");
        }
    }

    /// 数组列表形式字面量 `[1, 2, 3]` 解析为 3 个元素、非重复。
    #[test]
    fn parse_array_literal_list() {
        let u = parse("fn main() { let a: [int; 3] = [1, 2, 3]; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Let { init, .. } = &body[0] {
                assert!(matches!(
                    init,
                    Expr::ArrayLiteral { elements, repeat }
                        if elements.len() == 3 && repeat.is_none()
                ));
            } else {
                panic!("expected let");
            }
        } else {
            panic!("expected FnDef");
        }
    }

    /// 数组重复形式字面量 `[1; 5]` 解析为 1 个模板元素 + 重复次数。
    #[test]
    fn parse_array_literal_repeat() {
        let u = parse("fn main() { let a: [int; 5] = [1; 5]; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Let { init, .. } = &body[0] {
                assert!(matches!(
                    init,
                    Expr::ArrayLiteral { elements, repeat }
                        if elements.len() == 1 && repeat.is_some()
                ));
            } else {
                panic!("expected let");
            }
        } else {
            panic!("expected FnDef");
        }
    }

    /// 下标访问 `a[1]` 解析为 [`Expr::Index`]。
    #[test]
    fn parse_index_access() {
        let u = parse("fn main() { let x: int = a[1]; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Let { init, .. } = &body[0] {
                assert!(matches!(init, Expr::Index { .. }));
            } else {
                panic!("expected let");
            }
        } else {
            panic!("expected FnDef");
        }
    }

    /// 通过下标赋值的左值 `a[i] = v` 解析为赋值语句，目标为 [`Expr::Index`]。
    #[test]
    fn parse_assign_to_index() {
        let u = parse("fn main() { a[1] = 2; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Assign { target, .. } = &body[0] {
                assert!(matches!(&**target, Expr::Index { .. }));
            } else {
                panic!("expected assign");
            }
        } else {
            panic!("expected FnDef");
        }
    }

    /// `impl Type { fn ... }` 解析为 [`TopLevelDecl::Impl`]，方法体记录名字。
    #[test]
    fn parse_impl_block() {
        let u = parse("struct Point { x:int } impl Point { fn sum() -> int { return 1; } }");
        let impl_decl = u
            .declarations
            .iter()
            .find(|d| matches!(d, TopLevelDecl::Impl { .. }))
            .expect("expected Impl decl");
        if let TopLevelDecl::Impl {
            ty,
            trrait,
            methods,
            ..
        } = impl_decl
        {
            assert_eq!(ty, &Type::Named(Path::simple("Point")));
            assert!(trrait.is_none());
            assert_eq!(methods.len(), 1);
            assert_eq!(methods[0].name, "sum");
            assert!(methods[0].params.is_empty());
        } else {
            panic!("expected Impl");
        }
    }

    /// `trait Name { fn req(); fn def() {} }` 解析为必填/默认两类方法。
    #[test]
    fn parse_trait_block() {
        let u = parse("trait Show { fn show() -> int; fn pretty() -> int { return 1; } }");
        let tr = u
            .declarations
            .iter()
            .find(|d| matches!(d, TopLevelDecl::Trait { .. }))
            .expect("expected Trait decl");
        if let TopLevelDecl::Trait { name, methods, .. } = tr {
            assert_eq!(name, "Show");
            assert_eq!(methods.len(), 2);
            // 第一个是必填（无默认体），第二个带默认体。
            assert!(methods[0].default_body.is_none());
            assert_eq!(methods[0].name, "show");
            assert!(methods[1].default_body.is_some());
            assert_eq!(methods[1].name, "pretty");
        } else {
            panic!("expected Trait");
        }
    }

    /// `impl Trait for Type` 解析出 trait 名（作为 `trrait` 字段）。
    #[test]
    fn parse_impl_for_trait() {
        let u = parse(
            "struct Point { x:int } trait Show { fn show() -> int; } impl Show for Point { fn show() -> int { return 1; } }",
        );
        let impl_decl = u
            .declarations
            .iter()
            .find(|d| {
                matches!(
                    d,
                    TopLevelDecl::Impl {
                        trrait: Some(_),
                        ..
                    }
                )
            })
            .expect("expected Impl-for decl");
        if let TopLevelDecl::Impl { trrait, ty, .. } = impl_decl {
            assert_eq!(trrait.as_ref().unwrap(), &Path::simple("Show"));
            assert_eq!(ty, &Type::Named(Path::simple("Point")));
        } else {
            panic!("expected Impl-for");
        }
    }

    /// `obj.method(args)` 解析为 [`Expr::MethodCall`]；`obj.field` 仍为 [`Expr::FieldAccess`]。
    #[test]
    fn parse_method_call_vs_field_access() {
        let u = parse("fn main() { let v:int = p.sum(1, 2); let f:int = p.x; }");
        if let TopLevelDecl::FnDef { body, .. } = &u.declarations[0] {
            if let Stmt::Let { init, .. } = &body[0] {
                assert!(matches!(
                    init,
                    Expr::MethodCall {
                        name,
                        args,
                        ..
                    } if name == "sum" && args.len() == 2
                ));
            } else {
                panic!("expected let v");
            }
            if let Stmt::Let { init, .. } = &body[1] {
                assert!(matches!(init, Expr::FieldAccess { field, .. } if field == "x"));
            } else {
                panic!("expected let f");
            }
        } else {
            panic!("expected FnDef");
        }
    }
}
