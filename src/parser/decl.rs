use crate::error::MplangError;
use crate::{
    ast::{
        EnumVariant, GenericParam, GenericParamKind, ImplMethod, Meta, Param, TopLevelDecl,
        TraitMethod, Type,
    },
    token::TokenKind,
};

use super::Parser;

impl Parser {
    // ========================================================================
    // 顶层声明分发
    // ========================================================================

    /// 顶层声明分发：根据起始关键字选择对应的声明解析器。
    /// 声明前的连续 `#[...]` 注解在此统一解析，并原样转发给具体的声明解析器，
    /// 让「通用注解机制」对每一种声明都可用，而不必为某一种声明特判。
    pub(crate) fn top_level_decl(&mut self) -> TopLevelDecl {
        let attributes = self.parse_attributes();
        if self.check(&TokenKind::Mod) {
            return self.mod_decl(attributes);
        }
        if self.check(&TokenKind::Use) {
            return self.use_decl(attributes);
        }
        if self.check(&TokenKind::Extern) {
            // `extern crate NAME;` 与 `extern fn ...;` 共用 `extern` 关键字，
            // 通过向后看一个 token 区分。
            if self.check_next(&TokenKind::Crate) {
                return self.extern_crate_decl(attributes);
            }
            return self.extern_fn_decl(attributes);
        }
        if self.check(&TokenKind::Fn) {
            return self.fn_decl(attributes);
        }
        if self.check(&TokenKind::Struct) {
            return self.struct_decl(attributes);
        }
        if self.check(&TokenKind::Enum) {
            return self.enum_decl(attributes);
        }
        if self.check(&TokenKind::Impl) {
            return self.impl_decl(attributes);
        }
        if self.check(&TokenKind::Trait) {
            return self.trait_decl(attributes);
        }
        if self.check(&TokenKind::Static) {
            return self.global_decl(true, attributes);
        }
        if self.check(&TokenKind::Const) {
            return self.global_decl(false, attributes);
        }
        crate::error::fatal(MplangError::parse(format!(
            "期望顶层声明（mod/use/extern/fn/struct/static/const），但遇到 {:?}（第 {} 行，第 {} 列）",
            self.current().kind,
            self.current().line,
            self.current().column
        )))
    }

    // ========================================================================
    // 模块级声明解析
    // ========================================================================

    fn mod_decl(&mut self, attributes: Vec<Meta>) -> TopLevelDecl {
        self.consume(TokenKind::Mod, "Expected 'mod'");
        let name = self
            .consume(TokenKind::Ident, "Expected module name after 'mod'")
            .lexeme
            .clone();
        self.consume(TokenKind::Semicolon, "Expected ';' after mod declaration");
        TopLevelDecl::ModDecl { attributes, name }
    }

    fn use_decl(&mut self, attributes: Vec<Meta>) -> TopLevelDecl {
        self.consume(TokenKind::Use, "Expected 'use'");
        let path = self.parse_path();
        self.consume(TokenKind::Semicolon, "Expected ';' after use declaration");
        TopLevelDecl::UseDecl { attributes, path }
    }

    /// 外部 crate 声明：`extern crate NAME;`。
    /// 可前置 `#[path = "..."]` 注解指定该 crate 源文件的查找路径（见 lowering 中的路径解析）。
    fn extern_crate_decl(&mut self, attributes: Vec<Meta>) -> TopLevelDecl {
        self.consume(TokenKind::Extern, "Expected 'extern'");
        self.consume(TokenKind::Crate, "Expected 'crate' after 'extern'");
        let name = self
            .consume(TokenKind::Ident, "Expected crate name after 'extern crate'")
            .lexeme
            .clone();
        self.consume(
            TokenKind::Semicolon,
            "Expected ';' after extern crate declaration",
        );
        TopLevelDecl::ExternCrate { attributes, name }
    }

    fn extern_fn_decl(&mut self, attributes: Vec<Meta>) -> TopLevelDecl {
        self.consume(TokenKind::Extern, "Expected 'extern'");

        // 历史语法 `extern "linkage" fn ...` 中的 linkage 字符串已废弃，
        // 改由通用注解 `#[link_name = "..."]` 提供（见 lowering 中读取逻辑）。
        // 这里 `extern` 之后必须直接跟 `fn`。
        self.consume(TokenKind::Fn, "Expected 'fn' after 'extern'");
        let name = self
            .consume(TokenKind::Ident, "Expected function name after 'extern fn'")
            .lexeme
            .clone();
        self.consume(
            TokenKind::LeftParen,
            "Expected '(' after extern function name",
        );

        let mut param_types = Vec::new();
        let mut is_variadic = false;

        while !self.is_at_end() && !self.check(&TokenKind::RightParen) {
            if self.check(&TokenKind::Ellipsis) {
                self.advance();
                is_variadic = true;
                if self.check(&TokenKind::Comma) {
                    self.advance();
                }
                break;
            }
            // 支持 `name:type` 与裸 `type` 两种参数写法。
            if self.check(&TokenKind::Ident) && self.check_next(&TokenKind::Colon) {
                self.advance(); // 跳过参数名
                self.advance(); // 跳过冒号
            }
            param_types.push(self.parse_type());
            if self.check(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.consume(
            TokenKind::RightParen,
            "Expected ')' after extern function parameters",
        );

        let return_ty = if self.check(&TokenKind::Arrow) {
            self.advance();
            self.parse_type()
        } else {
            Type::Unit
        };

        self.consume(
            TokenKind::Semicolon,
            "Expected ';' after extern function declaration",
        );

        TopLevelDecl::ExternFnDef {
            attributes,
            name,
            param_types,
            return_ty,
            is_variadic,
        }
    }

    fn fn_decl(&mut self, attributes: Vec<Meta>) -> TopLevelDecl {
        self.consume(TokenKind::Fn, "Expected 'fn'");
        let f = self.parse_fn_parts();
        TopLevelDecl::FnDef {
            attributes,
            name: f.name,
            generics: f.generics,
            params: f.params,
            return_ty: f.return_ty,
            body: f.body,
        }
    }

    /// 解析一个（普通或 `impl` 内的）函数声明，返回其结构。
    /// `fn` 关键字必须由调用方先 consume，这里从函数名开始解析。
    fn parse_fn_parts(&mut self) -> ImplMethod {
        let name = self
            .consume(TokenKind::Ident, "Expected function name")
            .lexeme
            .clone();
        // 函数名之后、参数列表之前可跟泛型参数列表 `<T, const N: int>`。
        let generics = if self.check(&TokenKind::Less) {
            self.parse_generics()
        } else {
            Vec::new()
        };
        self.consume(TokenKind::LeftParen, "Expected '(' after function name");
        let params = self.fn_params();
        self.consume(TokenKind::RightParen, "Expected ')' after parameters");

        let return_ty = if self.check(&TokenKind::Arrow) {
            self.advance();
            self.parse_type()
        } else {
            Type::Unit
        };

        self.consume(TokenKind::LeftBrace, "Expected '{' before function body");
        let body = self.statements();
        self.consume(TokenKind::RightBrace, "Expected '}' after function body");

        ImplMethod {
            name,
            generics,
            params,
            return_ty,
            body,
        }
    }

    /// 解析泛型参数列表 `< T, const N: int, U >`，返回 [`GenericParam`] 序列。
    fn parse_generics(&mut self) -> Vec<GenericParam> {
        self.consume(TokenKind::Less, "期望 '<' 开始泛型参数列表");
        let mut params = Vec::new();
        while !self.check(&TokenKind::Greater) && !self.is_at_end() {
            let kind = if self.check(&TokenKind::Const) {
                self.advance();
                GenericParamKind::Const
            } else {
                GenericParamKind::Type
            };
            let name = self
                .consume(TokenKind::Ident, "期望泛型参数名")
                .lexeme
                .clone();
            if kind == GenericParamKind::Const {
                // 常量参数形如 `const N: int`（当前仅支持 `int`）。
                self.consume(TokenKind::Colon, "期望 ':' 后接常量参数类型");
                let _ = self.parse_type();
            }
            params.push(GenericParam { name, kind });
            if self.check(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.consume(TokenKind::Greater, "期望 '>' 结束泛型参数列表");
        params
    }

    fn impl_decl(&mut self, attributes: Vec<Meta>) -> TopLevelDecl {
        self.consume(TokenKind::Impl, "Expected 'impl'");
        // `impl<T> List<T>`：impl 之后、类型之前可跟泛型参数列表 `<T>`。
        let generics = if self.check(&TokenKind::Less) {
            self.parse_generics()
        } else {
            Vec::new()
        };
        // `impl Trait for T` 或普通 `impl T`：先解析首个类型，
        // 若其后紧跟 `for` 则是 trait 实现形式。
        let first = self.parse_type();
        let (trait_ref, ty) = if self.check(&TokenKind::For) {
            self.advance();
            let path = match &first {
                Type::Named(p) => p.clone(),
                _ => crate::error::fatal(MplangError::parse(
                    "impl <trait> for <type> 中的 trait 名必须是路径",
                )),
            };
            let ty = self.parse_type();
            (Some(path), ty)
        } else {
            (None, first)
        };
        self.consume(TokenKind::LeftBrace, "Expected '{' after impl type");
        let mut methods = Vec::new();
        while !self.is_at_end() && !self.check(&TokenKind::RightBrace) {
            self.consume(TokenKind::Fn, "Expected 'fn' inside impl block");
            let f = self.parse_fn_parts();
            methods.push(f);
            // 方法之间可选的分号：`fn foo() {};` 或 `fn foo() {}`。
            if self.check(&TokenKind::Semicolon) {
                self.advance();
            }
        }
        self.consume(TokenKind::RightBrace, "Expected '}' after impl block");
        TopLevelDecl::Impl {
            attributes,
            generics,
            trrait: trait_ref,
            ty,
            methods,
        }
    }

    fn trait_decl(&mut self, attributes: Vec<Meta>) -> TopLevelDecl {
        self.consume(TokenKind::Trait, "Expected 'trait'");
        let name = self
            .consume(TokenKind::Ident, "Expected trait name")
            .lexeme
            .clone();
        // trait 名之后可跟泛型参数列表 `<T>`。
        let generics = if self.check(&TokenKind::Less) {
            self.parse_generics()
        } else {
            Vec::new()
        };
        self.consume(TokenKind::LeftBrace, "Expected '{' after trait name");
        let mut methods = Vec::new();
        while !self.is_at_end() && !self.check(&TokenKind::RightBrace) {
            self.consume(TokenKind::Fn, "Expected 'fn' inside trait block");
            let mname = self
                .consume(TokenKind::Ident, "Expected trait method name")
                .lexeme
                .clone();
            // trait 方法名之后、参数之前可跟泛型参数列表 `<U>`。
            let mgenerics = if self.check(&TokenKind::Less) {
                self.parse_generics()
            } else {
                Vec::new()
            };
            self.consume(TokenKind::LeftParen, "Expected '(' after trait method name");
            let params = self.fn_params();
            self.consume(
                TokenKind::RightParen,
                "Expected ')' after trait method parameters",
            );
            let return_ty = if self.check(&TokenKind::Arrow) {
                self.advance();
                self.parse_type()
            } else {
                Type::Unit
            };
            // 必填方法以 `;` 结尾；带默认实现的方法给出 `{ ... }`。
            let default_body = if self.check(&TokenKind::LeftBrace) {
                self.consume(TokenKind::LeftBrace, "Expected '{' for default body");
                let body = self.statements();
                self.consume(TokenKind::RightBrace, "Expected '}' after default body");
                Some(body)
            } else {
                self.consume(
                    TokenKind::Semicolon,
                    "Expected ';' (必填方法) 或 '{' (默认实现)",
                );
                None
            };
            methods.push(TraitMethod {
                name: mname,
                generics: mgenerics,
                params,
                return_ty,
                default_body,
            });
            // 方法之间可选的分号。
            if self.check(&TokenKind::Semicolon) {
                self.advance();
            }
        }
        self.consume(TokenKind::RightBrace, "Expected '}' after trait block");
        TopLevelDecl::Trait {
            attributes,
            name,
            generics,
            methods,
        }
    }

    fn struct_decl(&mut self, attributes: Vec<Meta>) -> TopLevelDecl {
        self.consume(TokenKind::Struct, "Expected 'struct'");
        let name = self
            .consume(TokenKind::Ident, "Expected struct name")
            .lexeme
            .clone();
        // 结构体名之后、字段列表之前可跟泛型参数列表 `<T, const N: int>`。
        let generics = if self.check(&TokenKind::Less) {
            self.parse_generics()
        } else {
            Vec::new()
        };
        self.consume(TokenKind::LeftBrace, "Expected '{' after struct name");

        let mut fields = Vec::new();
        while !self.is_at_end() && !self.check(&TokenKind::RightBrace) {
            let field_name = self
                .consume(TokenKind::Ident, "Expected field name")
                .lexeme
                .clone();
            self.consume(TokenKind::Colon, "Expected ':' after field name");
            let field_ty = self.parse_type();
            fields.push((field_name, field_ty));
            if self.check(&TokenKind::Comma) {
                self.advance();
            }
        }
        self.consume(TokenKind::RightBrace, "Expected '}' after struct fields");

        TopLevelDecl::StructDef {
            attributes,
            name,
            generics,
            fields,
        }
    }

    fn enum_decl(&mut self, attributes: Vec<Meta>) -> TopLevelDecl {
        self.consume(TokenKind::Enum, "Expected 'enum'");
        let name = self
            .consume(TokenKind::Ident, "Expected enum name")
            .lexeme
            .clone();
        // 枚举名之后可跟泛型参数列表 `<T, E>`。
        let generics = if self.check(&TokenKind::Less) {
            self.parse_generics()
        } else {
            Vec::new()
        };
        self.consume(TokenKind::LeftBrace, "Expected '{' after enum name");

        let mut variants = Vec::new();
        while !self.is_at_end() && !self.check(&TokenKind::RightBrace) {
            let variant_name = self
                .consume(TokenKind::Ident, "Expected variant name")
                .lexeme
                .clone();
            // 变体可带载荷：`Some(T)` / `Pair(x:int, y:int)`，或单元变体 `None`。
            let fields = if self.check(&TokenKind::LeftParen) {
                self.advance();
                let mut fs = Vec::new();
                while !self.is_at_end() && !self.check(&TokenKind::RightParen) {
                    let fname = self
                        .consume(TokenKind::Ident, "Expected field name")
                        .lexeme
                        .clone();
                    self.consume(TokenKind::Colon, "Expected ':' after field name");
                    let fty = self.parse_type();
                    fs.push((fname, fty));
                    if self.check(&TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.consume(TokenKind::RightParen, "Expected ')' after variant fields");
                fs
            } else {
                Vec::new()
            };
            variants.push(EnumVariant {
                name: variant_name,
                fields,
            });
            if self.check(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.consume(TokenKind::RightBrace, "Expected '}' after enum variants");

        TopLevelDecl::Enum {
            attributes,
            name,
            generics,
            variants,
        }
    }

    /// 顶层全局声明（`static` / `const`），返回 [`TopLevelDecl`]。
    /// 名字 / 类型 / 初始化的解析复用 [`Parser::parse_typed_binding`]，仅末尾按 const 与否选择变体。
    fn global_decl(&mut self, is_const: bool, attributes: Vec<Meta>) -> TopLevelDecl {
        let (name, ty, init) = self.parse_typed_binding(true);
        let ty = ty.expect("全局声明必须有类型注解");
        if is_const {
            TopLevelDecl::Const {
                attributes,
                name,
                ty,
                init,
            }
        } else {
            TopLevelDecl::Static {
                attributes,
                name,
                ty,
                init,
            }
        }
    }

    fn fn_params(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        while !self.is_at_end() && !self.check(&TokenKind::RightParen) {
            let name = self
                .consume(TokenKind::Ident, "Expected parameter name")
                .lexeme
                .clone();
            self.consume(TokenKind::Colon, "Expected ':' after parameter name");
            let ty = self.parse_type();
            params.push(Param { name, ty });
            if self.check(&TokenKind::Comma) {
                self.advance();
            }
        }
        params
    }
}
