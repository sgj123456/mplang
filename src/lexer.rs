use crate::error::MplangError;
use crate::token::{Token, TokenKind};

#[derive(Debug)]
pub struct Lexer {
    raw: Vec<char>,
    start: usize,
    current: usize,
    line: usize,
    column: usize,
}

impl Lexer {
    pub fn new(raw: Vec<char>) -> Lexer {
        Lexer {
            raw,
            start: 0,
            current: 0,
            line: 1,
            column: 1,
        }
    }

    fn is_at_end(&self) -> bool {
        self.current >= self.raw.len()
    }

    fn current_char(&self) -> char {
        self.raw[self.current]
    }

    fn peek_next(&self) -> Option<char> {
        if self.current + 1 >= self.raw.len() {
            return None;
        }
        Some(self.raw[self.current + 1])
    }

    fn advance(&mut self) -> char {
        let ch = self.raw[self.current];
        self.current += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        ch
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while !self.is_at_end() && self.current_char().is_whitespace() {
                self.advance();
            }
            if !self.is_at_end() && self.current_char() == '/' && self.peek_next() == Some('/') {
                while !self.is_at_end() && self.current_char() != '\n' {
                    self.advance();
                }
                continue;
            }
            if !self.is_at_end() && self.current_char() == '/' && self.peek_next() == Some('*') {
                self.advance();
                self.advance();
                let mut depth = 1;
                while !self.is_at_end() && depth > 0 {
                    if self.current_char() == '/' && self.peek_next() == Some('*') {
                        self.advance();
                        self.advance();
                        depth += 1;
                    } else if self.current_char() == '*' && self.peek_next() == Some('/') {
                        self.advance();
                        self.advance();
                        depth -= 1;
                    } else {
                        self.advance();
                    }
                }
                continue;
            }
            break;
        }
    }

    fn make_token(
        &self,
        token_type: TokenKind,
        lexeme: String,
        start_line: usize,
        start_col: usize,
    ) -> Token {
        Token::new(start_line, start_col, lexeme.len(), lexeme, token_type)
    }

    pub fn next_token(&mut self) -> Result<Token, MplangError> {
        self.skip_whitespace_and_comments();

        if self.is_at_end() {
            return Ok(Token::new(
                self.line,
                self.column,
                0,
                String::new(),
                TokenKind::Eof,
            ));
        }

        let start_line = self.line;
        let start_col = self.column;
        let start_pos = self.current;

        let ch = self.advance();

        let token = match ch {
            '+' => self.make_token(TokenKind::Plus, "+".into(), start_line, start_col),
            '-' => {
                if !self.is_at_end() && self.current_char() == '>' {
                    self.advance();
                    self.make_token(TokenKind::Arrow, "->".into(), start_line, start_col)
                } else {
                    self.make_token(TokenKind::Minus, "-".into(), start_line, start_col)
                }
            }
            '*' => self.make_token(TokenKind::Star, "*".into(), start_line, start_col),
            '&' => self.make_token(TokenKind::Amp, "&".into(), start_line, start_col),
            '/' => self.make_token(TokenKind::Slash, "/".into(), start_line, start_col),
            ',' => self.make_token(TokenKind::Comma, ",".into(), start_line, start_col),
            '(' => self.make_token(TokenKind::LeftParen, "(".into(), start_line, start_col),
            ')' => self.make_token(TokenKind::RightParen, ")".into(), start_line, start_col),
            '[' => self.make_token(TokenKind::LeftBracket, "[".into(), start_line, start_col),
            ']' => self.make_token(TokenKind::RightBracket, "]".into(), start_line, start_col),
            '#' => self.make_token(TokenKind::Hash, "#".into(), start_line, start_col),
            '{' => self.make_token(TokenKind::LeftBrace, "{".into(), start_line, start_col),
            '}' => self.make_token(TokenKind::RightBrace, "}".into(), start_line, start_col),
            ';' => self.make_token(TokenKind::Semicolon, ";".into(), start_line, start_col),
            ':' => {
                if !self.is_at_end() && self.current_char() == ':' {
                    self.advance();
                    self.make_token(TokenKind::ColonColon, "::".into(), start_line, start_col)
                } else {
                    self.make_token(TokenKind::Colon, ":".into(), start_line, start_col)
                }
            }
            '.' => {
                if !self.is_at_end() && self.current_char() == '.' && self.peek_next() == Some('.')
                {
                    self.advance();
                    self.advance();
                    self.make_token(TokenKind::Ellipsis, "...".into(), start_line, start_col)
                } else {
                    self.make_token(TokenKind::Dot, ".".into(), start_line, start_col)
                }
            }
            '=' => {
                if !self.is_at_end() && self.current_char() == '=' {
                    self.advance();
                    self.make_token(TokenKind::Equal, "==".into(), start_line, start_col)
                } else {
                    self.make_token(TokenKind::Assign, "=".into(), start_line, start_col)
                }
            }
            '>' => {
                if !self.is_at_end() && self.current_char() == '=' {
                    self.advance();
                    self.make_token(TokenKind::GreaterEqual, ">=".into(), start_line, start_col)
                } else {
                    self.make_token(TokenKind::Greater, ">".into(), start_line, start_col)
                }
            }
            '<' => {
                if !self.is_at_end() && self.current_char() == '=' {
                    self.advance();
                    self.make_token(TokenKind::LessEqual, "<=".into(), start_line, start_col)
                } else {
                    self.make_token(TokenKind::Less, "<".into(), start_line, start_col)
                }
            }
            '!' => {
                if !self.is_at_end() && self.current_char() == '=' {
                    self.advance();
                    self.make_token(TokenKind::NotEqual, "!=".into(), start_line, start_col)
                } else {
                    self.make_token(TokenKind::Not, "!".into(), start_line, start_col)
                }
            }
            '"' => {
                self.advance();
                while !self.is_at_end() && self.current_char() != '"' {
                    self.advance();
                }
                self.advance();
                let lexeme: String = self.raw[start_pos..self.current].iter().collect();
                self.make_token(TokenKind::StringLiteral, lexeme, start_line, start_col)
            }

            '0'..='9' => {
                while !self.is_at_end() && self.current_char().is_ascii_digit() {
                    self.advance();
                }
                let lexeme: String = self.raw[start_pos..self.current].iter().collect();
                self.make_token(TokenKind::IntLiteral, lexeme, start_line, start_col)
            }

            c if c.is_alphabetic() || c == '_' => {
                while !self.is_at_end()
                    && (self.current_char().is_alphanumeric() || self.current_char() == '_')
                {
                    self.advance();
                }
                let lexeme: String = self.raw[start_pos..self.current].iter().collect();
                let token_type = match lexeme.as_str() {
                    "let" => TokenKind::Let,
                    "for" => TokenKind::For,
                    "while" => TokenKind::While,
                    "if" => TokenKind::If,
                    "else" => TokenKind::Else,
                    "fn" => TokenKind::Fn,
                    "return" => TokenKind::Return,
                    "struct" => TokenKind::Struct,
                    "static" => TokenKind::Static,
                    "const" => TokenKind::Const,
                    "extern" => TokenKind::Extern,
                    "mod" => TokenKind::Mod,
                    "use" => TokenKind::Use,
                    "crate" => TokenKind::Crate,
                    "impl" => TokenKind::Impl,
                    "trait" => TokenKind::Trait,
                    _ => TokenKind::Ident,
                };
                self.make_token(token_type, lexeme, start_line, start_col)
            }

            _ => {
                return Err(MplangError::lex(format!(
                    "无法识别的字符 '{}'（第 {} 行，第 {} 列）",
                    ch, start_line, start_col
                ))
                .with_span(start_line, start_col));
            }
        };

        self.start = self.current;
        Ok(token)
    }

    pub fn lex(&mut self) -> Result<Vec<Token>, MplangError> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let is_eof = matches!(token.kind, TokenKind::Eof);
            tokens.push(token);
            if is_eof {
                break;
            }
        }
        log::debug!("词法分析完成，共 {} 个 token", tokens.len());
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::token::{Token, TokenKind};

    fn lex(src: &str) -> Vec<Token> {
        Lexer::new(src.chars().collect()).lex().unwrap()
    }

    #[test]
    fn lex_keyword_ident_and_int() {
        let toks = lex("let x = 123");
        assert_eq!(toks[0].kind, TokenKind::Let);
        assert_eq!(toks[1].kind, TokenKind::Ident);
        assert_eq!(toks[1].lexeme, "x");
        assert_eq!(toks[2].kind, TokenKind::Assign);
        assert_eq!(toks[3].kind, TokenKind::IntLiteral);
        assert_eq!(toks[3].lexeme, "123");
        assert_eq!(toks[4].kind, TokenKind::Eof);
    }

    #[test]
    fn lex_operators_and_delimiters() {
        let toks = lex("(a + b) * c;");
        assert_eq!(toks[0].kind, TokenKind::LeftParen);
        assert_eq!(toks[2].kind, TokenKind::Plus);
        assert_eq!(toks[4].kind, TokenKind::RightParen);
        assert_eq!(toks[5].kind, TokenKind::Star);
        assert_eq!(toks[7].kind, TokenKind::Semicolon);
    }

    #[test]
    fn lex_coloncolon_for_paths() {
        // 验证路径分隔符被识别为 ColonColon（曾因未识别导致模块加载失败）。
        let toks = lex("a::b");
        assert_eq!(toks[0].kind, TokenKind::Ident);
        assert_eq!(toks[1].kind, TokenKind::ColonColon);
        assert_eq!(toks[2].kind, TokenKind::Ident);
    }

    #[test]
    fn lex_string_literal() {
        let toks = lex("\"hello\"");
        assert_eq!(toks[0].kind, TokenKind::StringLiteral);
        assert_eq!(toks[0].lexeme, "\"hello\"");
    }

    #[test]
    fn lex_invalid_char_is_error() {
        let err = Lexer::new("@".chars().collect()).lex().unwrap_err();
        assert_eq!(err.kind, ErrorKind::Lex);
        assert!(err.message.contains('@'));
        // 错误应携带位置信息。
        let span = err.span.expect("error should carry a source span");
        assert_eq!(span.line, 1);
        assert_eq!(span.col, 1);
    }

    #[test]
    fn lex_skips_comments() {
        let toks = lex("// a comment\nlet x = 1; /* block */");
        assert_eq!(toks[0].kind, TokenKind::Let);
        assert_eq!(toks.last().unwrap().kind, TokenKind::Eof);
    }

    #[test]
    fn lex_attribute_hash() {
        // `#[link_name = "x"]` 中的 `#` 应被识别为 Hash，随后是普通的 `[` 与字符串。
        let toks = lex("#[link_name = \"x\"] extern fn f();");
        assert_eq!(toks[0].kind, TokenKind::Hash);
        assert_eq!(toks[1].kind, TokenKind::LeftBracket);
        assert_eq!(toks[2].kind, TokenKind::Ident);
        assert_eq!(toks[3].kind, TokenKind::Assign);
        assert_eq!(toks[4].kind, TokenKind::StringLiteral);
        assert_eq!(toks[5].kind, TokenKind::RightBracket);
    }
}
