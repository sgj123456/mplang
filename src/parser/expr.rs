use crate::error::MplangError;
use crate::{
    ast::{ArrayLen, BinOp, Expr, Literal, Path, Type, TypeOrConst},
    token::TokenKind,
};

use super::Parser;

impl Parser {
    // ========================================================================
    // 类型解析（支持路径与指针 / 数组 / 泛型应用）
    // ========================================================================

    pub(crate) fn parse_type(&mut self) -> Type {
        // 指针类型 `*T`（右结合）：`*` 出现在类型开头时表示指针。
        if self.check(&TokenKind::Star) {
            self.advance();
            return Type::Pointer(Box::new(self.parse_type()));
        }
        // 定长数组类型 `[T; N]`：先解析元素类型，再解析分号后的长度常量。
        if self.check(&TokenKind::LeftBracket) {
            self.advance();
            let elem = self.parse_type();
            self.consume(TokenKind::Semicolon, "数组类型期望形如 [T; N]，但缺少 ';'");
            // `;` 之后既可以是整型字面量，也可以是某个（常量泛型）参数名。
            let len = if self.check(&TokenKind::Ident) {
                let name = self.advance().lexeme.clone();
                ArrayLen::Param(name)
            } else {
                let size_tok = self.consume(
                    TokenKind::IntLiteral,
                    "数组类型 [T; N] 中 ';' 之后期望长度（非负整数或常量泛型参数名）",
                );
                let size = size_tok.lexeme.parse::<usize>().unwrap_or_else(|_| {
                    crate::error::fatal(MplangError::parse(format!(
                        "无效数组长度 '{}'（第 {} 行，第 {} 列）",
                        size_tok.lexeme, size_tok.line, size_tok.column
                    )))
                });
                ArrayLen::Known(size)
            };
            self.consume(TokenKind::RightBracket, "数组类型 [T; N] 缺少结尾的 ']'");
            return Type::Array(Box::new(elem), len);
        }
        // 先尝试读取标识符判断是否为内置类型
        if self.check(&TokenKind::Ident) {
            match self.current().lexeme.as_str() {
                "int" => {
                    self.advance();
                    return Type::Int;
                }
                "char" => {
                    self.advance();
                    return Type::Char;
                }
                "unit" => {
                    self.advance();
                    return Type::Unit;
                }
                _ => {}
            }
        }
        // 非内置类型 → 解析为 Path（支持 std::types::Point 等）；若其后紧跟 `<`，则为泛型应用。
        let path = self.parse_path();
        if self.check(&TokenKind::Less) {
            return Type::Applied(path, self.parse_generic_args());
        }
        Type::Named(path)
    }

    /// 解析泛型实参列表 `< T, 3, U >`，产出 [`TypeOrConst`] 序列（位置对应泛型参数声明）。
    fn parse_generic_args(&mut self) -> Vec<TypeOrConst> {
        self.consume(TokenKind::Less, "Expected '<' in generic arguments");
        let mut args = Vec::new();
        while !self.check(&TokenKind::Greater) && !self.is_at_end() {
            if self.check(&TokenKind::Const) {
                // `const N` 形式的常量实参（罕见，一般直接写字面量）。
                self.advance();
                self.consume(TokenKind::Colon, "Expected ':' after const generic arg");
                let ty = self.parse_type();
                let _ = ty; // 当前仅支持 int，类型在此被解析但不用于进一步校验
                args.push(TypeOrConst::Const(
                    self.consume(TokenKind::IntLiteral, "期望整型字面量作为常量泛型实参")
                        .lexeme
                        .parse()
                        .unwrap_or(0),
                ));
            } else if self.check(&TokenKind::IntLiteral) {
                let v = self.advance().lexeme.parse::<i64>().unwrap_or_else(|_| {
                    crate::error::fatal(MplangError::parse("无效整型常量泛型实参"))
                });
                args.push(TypeOrConst::Const(v));
            } else {
                // 类型实参（可能是 `T`、具体类型、或 `List<U>` 这类应用类型）。
                args.push(TypeOrConst::Type(self.parse_type()));
            }
            if self.check(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.consume(TokenKind::Greater, "Expected '>' after generic arguments");
        args
    }

    /// 解析涡轮鱼 `::< ... >`；若不存在则返回空。
    fn parse_turbofish(&mut self) -> Vec<TypeOrConst> {
        if self.check(&TokenKind::ColonColon) && self.check_next(&TokenKind::Less) {
            self.advance(); // ::
            self.parse_generic_args()
        } else {
            Vec::new()
        }
    }

    // ========================================================================
    // 表达式解析（Ident/Call/StructLiteral 改用 Path）
    // ========================================================================

    fn call_argument(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        while !self.is_at_end() && !self.check(&TokenKind::RightParen) {
            args.push(self.expression());
            if !self.check(&TokenKind::Comma) {
                break;
            }
            self.advance();
        }
        args
    }

    pub(crate) fn expression(&mut self) -> Expr {
        self.comparison()
    }

    fn postfix(&mut self, expr: Expr) -> Expr {
        let mut result = expr;
        loop {
            if self.check(&TokenKind::Dot) {
                self.advance();
                let field = self
                    .consume(TokenKind::Ident, "Expected field or method name after '.'")
                    .lexeme
                    .clone();
                if self.check(&TokenKind::LeftParen) {
                    // `obj.method(args)`：方法调用。`method` 依据接收者类型在类型检查阶段解析。
                    self.advance();
                    let args = self.call_argument();
                    self.consume(TokenKind::RightParen, "Expected ')' after method arguments");
                    result = Expr::MethodCall {
                        object: Box::new(result),
                        name: field,
                        args,
                    };
                } else {
                    result = Expr::FieldAccess {
                        object: Box::new(result),
                        field,
                    };
                }
            } else if self.check(&TokenKind::LeftParen) {
                // callee 现在是 Path；调用前可能跟随涡轮鱼 `::<...>`。
                if let Expr::Ident(callee) = result {
                    let turbofish = self.parse_turbofish();
                    self.advance();
                    let args = self.call_argument();
                    self.consume(TokenKind::RightParen, "Expected ')' after arguments");
                    result = Expr::Call {
                        callee,
                        args,
                        turbofish,
                    };
                } else {
                    break;
                }
            } else if self.check(&TokenKind::LeftBracket) {
                // 数组下标访问 `a[i]`。
                self.advance();
                let index = self.expression();
                self.consume(TokenKind::RightBracket, "Expected ']' after array index");
                result = Expr::Index {
                    array: Box::new(result),
                    index: Box::new(index),
                };
            } else {
                break;
            }
        }
        result
    }

    fn comparison(&mut self) -> Expr {
        let mut left = self.addition();
        while !self.is_at_end()
            && (self.check(&TokenKind::Equal)
                || self.check(&TokenKind::NotEqual)
                || self.check(&TokenKind::Greater)
                || self.check(&TokenKind::GreaterEqual)
                || self.check(&TokenKind::Less)
                || self.check(&TokenKind::LessEqual))
        {
            let op = self.advance().clone();
            let right = self.addition();
            left = match op.kind {
                TokenKind::Equal => Expr::Binary {
                    op: BinOp::Equal,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                TokenKind::NotEqual => Expr::Binary {
                    op: BinOp::NotEqual,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                TokenKind::Greater => Expr::Binary {
                    op: BinOp::Greater,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                TokenKind::GreaterEqual => Expr::Binary {
                    op: BinOp::GreaterEqual,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                TokenKind::Less => Expr::Binary {
                    op: BinOp::Less,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                TokenKind::LessEqual => Expr::Binary {
                    op: BinOp::LessEqual,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                _ => unreachable!(),
            };
        }
        left
    }

    fn addition(&mut self) -> Expr {
        let mut left = self.term();
        while !self.is_at_end() && (self.check(&TokenKind::Plus) || self.check(&TokenKind::Minus)) {
            let op = self.advance().clone();
            let right = self.term();
            left = match op.kind {
                TokenKind::Plus => Expr::Binary {
                    op: BinOp::Add,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                TokenKind::Minus => Expr::Binary {
                    op: BinOp::Sub,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                _ => unreachable!(),
            };
        }
        left
    }

    fn term(&mut self) -> Expr {
        let mut left = self.unary();
        while !self.is_at_end() && (self.check(&TokenKind::Star) || self.check(&TokenKind::Slash)) {
            let op = self.advance().clone();
            let right = self.unary();
            left = match op.kind {
                TokenKind::Star => Expr::Binary {
                    op: BinOp::Mul,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                TokenKind::Slash => Expr::Binary {
                    op: BinOp::Div,
                    lhs: Box::new(left),
                    rhs: Box::new(right),
                },
                _ => unreachable!(),
            };
        }
        left
    }

    /// 一元前缀层：处理取地址符 `&` 与解引用 `*`。
    /// `*` 在此作为解引用前缀；中缀乘法由 [`Parser::term`] 处理（靠位置区分）。
    fn unary(&mut self) -> Expr {
        if self.check(&TokenKind::Amp) {
            self.advance();
            return Expr::AddrOf(Box::new(self.unary()));
        }
        if self.check(&TokenKind::Star) {
            self.advance();
            return Expr::Deref(Box::new(self.unary()));
        }
        self.factor()
    }

    fn factor(&mut self) -> Expr {
        let token = self.current().clone();
        let base = match token.kind {
            TokenKind::IntLiteral => {
                self.advance();
                let value = token.lexeme.parse::<i64>().unwrap_or_else(|_| {
                    crate::error::fatal(MplangError::parse(format!(
                        "无效整数 '{}'（第 {} 行，第 {} 列）",
                        token.lexeme, token.line, token.column
                    )))
                });
                Expr::Literal(Literal::Int(value))
            }
            TokenKind::StringLiteral => {
                self.advance();
                let raw = &token.lexeme[1..token.lexeme.len() - 1];
                let value = unescape_string(raw).unwrap_or_else(|e| {
                    crate::error::fatal(MplangError::parse(format!("字符串字面量错误：{}", e)))
                });
                Expr::Literal(Literal::String(value))
            }
            TokenKind::Ident => {
                // 结构体字面量检测：先解析完整 path，再检查是否紧跟 {
                let path = self.parse_path();
                let turbofish = self.parse_turbofish();
                if self.check(&TokenKind::LeftBrace) {
                    return self.struct_literal_with_path(path, turbofish);
                }
                if !turbofish.is_empty() {
                    crate::error::fatal(MplangError::parse(
                        "涡轮鱼 `::<...>` 只能用于函数调用或结构体字面量之前",
                    ));
                }
                Expr::Ident(path)
            }
            TokenKind::LeftParen => {
                self.advance();
                let expr = self.expression();
                self.consume(
                    TokenKind::RightParen,
                    "Expected ')' after grouped expression",
                );
                Expr::Paren(Box::new(expr))
            }
            TokenKind::LeftBracket => {
                // 数组字面量（列表形式 `[a, b, c]` 或重复形式 `[v; n]`）。
                self.advance();
                self.parse_array_literal()
            }
            other => crate::error::fatal(MplangError::parse(format!(
                "表达式中出现意外记号 {:?}（'{}'，第 {} 行，第 {} 列）",
                other, token.lexeme, token.line, token.column
            ))),
        };
        self.postfix(base)
    }

    /// 结构体字面量接收已解析的 Path 与（可选的）涡轮鱼
    fn struct_literal_with_path(&mut self, name: Path, turbofish: Vec<TypeOrConst>) -> Expr {
        self.consume(TokenKind::LeftBrace, "Expected '{' after struct name");
        let mut fields = Vec::new();
        while !self.is_at_end() && !self.check(&TokenKind::RightBrace) {
            let field_name = self
                .consume(TokenKind::Ident, "Expected field name")
                .lexeme
                .clone();
            self.consume(TokenKind::Colon, "Expected ':' after field name");
            let value = self.expression();
            fields.push((field_name, value));
            if self.check(&TokenKind::Comma) {
                self.advance();
            }
        }
        self.consume(TokenKind::RightBrace, "Expected '}' after struct literal");
        Expr::StructLiteral {
            name,
            fields,
            turbofish,
        }
    }

    /// 解析数组字面量：入口调用方已 consume 掉开头的 `[`。
    /// - 列表形式 `[e1, e2, ..., en]`
    /// - 重复形式 `[v; n]`（`n` 须为整型字面量，表示重复 `n` 次）
    fn parse_array_literal(&mut self) -> Expr {
        let first = self.expression();
        if self.check(&TokenKind::Semicolon) {
            // 重复形式：[v; n]
            self.advance();
            let count = self.expression();
            self.consume(
                TokenKind::RightBracket,
                "重复形式数组字面量 [v; n] 缺少结尾的 ']'",
            );
            return Expr::ArrayLiteral {
                elements: vec![first],
                repeat: Some(Box::new(count)),
            };
        }
        // 列表形式：[e1, e2, ...]
        let mut elements = vec![first];
        while self.check(&TokenKind::Comma) {
            self.advance();
            // 允许尾随逗号：[1, 2, 3,]
            if self.check(&TokenKind::RightBracket) {
                break;
            }
            elements.push(self.expression());
        }
        self.consume(TokenKind::RightBracket, "数组字面量缺少结尾的 ']'");
        Expr::ArrayLiteral {
            elements,
            repeat: None,
        }
    }
}

/// 解析字符串转义序列（\n \t \r \\ \" \0 \xNN）。
pub(crate) fn unescape_string(s: &str) -> Result<String, String> {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('\"') => result.push('\"'),
                Some('0') => result.push('\0'),
                Some('x') => {
                    let hex: String = chars.by_ref().take(2).collect();
                    let byte = u8::from_str_radix(&hex, 16)
                        .map_err(|_| format!("invalid hex escape: \\x{}", hex))?;
                    result.push(byte as char);
                }
                Some(other) => return Err(format!("unknown escape sequence: \\{}", other)),
                None => return Err("unexpected end of string after backslash".to_string()),
            }
        } else {
            result.push(c);
        }
    }
    Ok(result)
}
