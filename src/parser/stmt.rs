use crate::error::MplangError;
use crate::{
    ast::{Expr, Path, Stmt, Type},
    token::TokenKind,
};

use super::Parser;

impl Parser {
    // ========================================================================
    // 函数体语句（仅保留执行逻辑，定义类语句在顶层处理）
    // ========================================================================

    pub(crate) fn statements(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();
        while !self.is_at_end() && !self.check(&TokenKind::RightBrace) {
            statements.push(self.statement());
        }
        statements
    }

    fn statement(&mut self) -> Stmt {
        if self.check(&TokenKind::Let) {
            return self.let_stmt();
        }
        if self.check(&TokenKind::Static) {
            return self.static_stmt();
        }
        if self.check(&TokenKind::Const) {
            return self.const_stmt();
        }
        if self.check(&TokenKind::If) {
            return self.if_stmt();
        }
        if self.check(&TokenKind::While) {
            return self.while_stmt();
        }
        if self.check(&TokenKind::Return) {
            return self.return_stmt();
        }

        // 赋值目标可为左值：变量名、解引用 `*p` 或字段访问 `s.x`。
        // 先试探解析左值，若其后紧跟 `=` 则视为赋值；否则回退交给普通表达式语句。
        // 注：此处用了手写回溯（保存/恢复 `current`），因为赋值与表达式语句的前缀相同，
        // 需通过后看 `=` 来区分。功能正确，但属较脆弱的解析手段。
        if self.check(&TokenKind::Ident) || self.check(&TokenKind::Star) {
            let save = self.current;
            let target = self.parse_lvalue();
            if self.check(&TokenKind::Assign) {
                self.advance();
                let value = self.expression();
                self.consume(TokenKind::Semicolon, "Expected ';' after assignment");
                return Stmt::Assign {
                    target: Box::new(target),
                    value,
                };
            }
            // 不是赋值（例如 `*p;` 表达式语句），回退后继续。
            self.current = save;
        }

        let expr = self.expression();
        self.consume(
            TokenKind::Semicolon,
            "Expected ';' after expression statement",
        );
        Stmt::Expr(expr)
    }

    /// 解析赋值左值（lvalue）：变量名、解引用 `*p`、或字段访问 `s.x`（可多层嵌套）。
    /// 仅用于 `lvalue = expr` 形式赋值的左侧解析。
    fn parse_lvalue(&mut self) -> Expr {
        if self.check(&TokenKind::Star) {
            self.advance();
            return Expr::Deref(Box::new(self.parse_lvalue()));
        }
        let name = self
            .consume(
                TokenKind::Ident,
                "Expected variable name in assignment target",
            )
            .lexeme
            .clone();
        let mut target = Expr::Ident(Path::simple(name));
        loop {
            if self.check(&TokenKind::Dot) {
                self.advance();
                let field = self
                    .consume(TokenKind::Ident, "Expected field name after '.'")
                    .lexeme
                    .clone();
                target = Expr::FieldAccess {
                    object: Box::new(target),
                    field,
                };
            } else if self.check(&TokenKind::LeftBracket) {
                // 数组下标作为赋值左值：`a[i] = v`。
                self.advance();
                let index = self.expression();
                self.consume(
                    TokenKind::RightBracket,
                    "Expected ']' after array index in assignment target",
                );
                target = Expr::Index {
                    array: Box::new(target),
                    index: Box::new(index),
                };
            } else {
                break;
            }
        }
        target
    }

    fn let_stmt(&mut self) -> Stmt {
        let (name, ty, init) = self.parse_typed_binding(false);
        Stmt::Let { name, ty, init }
    }

    fn static_stmt(&mut self) -> Stmt {
        let (name, ty, init) = self.parse_typed_binding(true);
        Stmt::Static {
            name,
            ty: ty.expect("static 声明必须有类型注解"),
            init,
        }
    }

    fn const_stmt(&mut self) -> Stmt {
        let (name, ty, init) = self.parse_typed_binding(true);
        Stmt::Const {
            name,
            ty: ty.expect("const 声明必须有类型注解"),
            init,
        }
    }

    fn if_stmt(&mut self) -> Stmt {
        self.consume(TokenKind::If, "Expected 'if'");
        self.consume(TokenKind::LeftParen, "Expected '(' after 'if'");
        let cond = self.expression();
        self.consume(TokenKind::RightParen, "Expected ')' after 'if'");

        self.consume(TokenKind::LeftBrace, "Expected '{' after 'if'");
        let then_branch = self.statements();
        self.consume(TokenKind::RightBrace, "Expected '}' after 'if'");

        let mut else_branch = None;
        if self.check(&TokenKind::Else) {
            self.advance();
            if self.check(&TokenKind::If) {
                else_branch = Some(Vec::from([self.if_stmt()]));
            } else {
                self.consume(TokenKind::LeftBrace, "Expected '{' after 'else'");
                else_branch = Some(self.statements());
                self.consume(TokenKind::RightBrace, "Expected '}' after 'else'");
            }
        }
        Stmt::If {
            cond,
            then_branch,
            else_branch,
        }
    }

    fn while_stmt(&mut self) -> Stmt {
        self.consume(TokenKind::While, "Expected 'while'");
        self.consume(TokenKind::LeftParen, "Expected '(' after 'while'");
        let cond = self.expression();
        self.consume(TokenKind::RightParen, "Expected ')' after 'while'");

        self.consume(TokenKind::LeftBrace, "Expected '{' after 'while'");
        let body = self.statements();
        self.consume(TokenKind::RightBrace, "Expected '}' after 'while'");
        Stmt::While { cond, body }
    }

    fn return_stmt(&mut self) -> Stmt {
        self.consume(TokenKind::Return, "Expected 'return'");
        if self.check(&TokenKind::Semicolon) {
            self.advance();
            return Stmt::Return(None);
        }
        let expr = self.expression();
        self.consume(TokenKind::Semicolon, "Expected ';' after return expression");
        Stmt::Return(Some(expr))
    }

    /// 解析「名字 [:类型] = 初始化;」形式的绑定，返回 `(名字, 类型(可空), 初始化表达式)`。
    ///
    /// 被 `let` / `static` / `const`（语句级）与顶层 `static` / `const` 共用，
    /// 消除四者几乎一致的解析主体。`require_ty` 为 `true` 时类型注解为必填
    ///（`static` / `const` 不允许省略类型）。
    pub(crate) fn parse_typed_binding(&mut self, require_ty: bool) -> (String, Option<Type>, Expr) {
        self.advance(); // 吃掉 let / static / const 关键字
        let name_token = self.consume(TokenKind::Ident, "Expected variable name");
        let name = name_token.lexeme.clone();
        let ty = if self.check(&TokenKind::Colon) {
            self.advance();
            Some(self.parse_type())
        } else if require_ty {
            crate::error::fatal(MplangError::parse(format!(
                "期望类型注解 ': ty'，但 '{}' 缺少类型（第 {} 行，第 {} 列）",
                name, name_token.line, name_token.column
            )))
        } else {
            None
        };
        self.consume(TokenKind::Assign, "Expected '=' after variable name");
        let init = self.expression();
        self.consume(
            TokenKind::Semicolon,
            "Expected ';' after variable declaration",
        );
        (name, ty, init)
    }
}
