#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub line: usize,
    pub column: usize,
    pub span_len: usize,
    pub lexeme: String,
    pub kind: TokenKind,
}

impl Token {
    pub fn new(
        line: usize,
        column: usize,
        span_len: usize,
        lexeme: String,
        kind: TokenKind,
    ) -> Self {
        Self {
            line,
            column,
            span_len,
            lexeme,
            kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // Arithmetic
    Plus,
    Minus,
    Star,
    Slash,

    // Pointer / address-of
    Amp,

    // Delimiters
    Semicolon,
    Comma,
    Colon,
    Dot,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    /// 注解起始符 `#`，用于 `#[...]`。
    Hash,
    Arrow,
    Ellipsis,
    ColonColon,

    // Assignment & Comparison
    Assign,
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    Not,

    // Literals
    IntLiteral,
    StringLiteral,

    // Keywords
    Let,
    Static,
    Const,
    Struct,
    Fn,
    Return,
    If,
    Else,
    For,
    While,
    Extern,
    Mod,
    Use,
    Crate,
    Impl,
    Trait,

    // Identifier
    Ident,

    // End of file
    Eof,
}
