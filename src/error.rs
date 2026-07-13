//! # 统一的错误类型与错误上报辅助
//!
//! 整个编译器前端（词法 / 语法 / 名字解析 / 类型检查）通过本模块提供的
//! [`MplangError`] 统一上报错误。每个阶段「入口」返回
//! [`Result`]，而阶段内部的错误点通过 [`fatal`] 抛出 [`MplangError`]，
//! 再由其入口处的 [`into_result`] 收拢为 `Result`。这样既能给出
//! 携带「种类 + 位置」的清晰中文报错，又避免在前端函数里大面积改返回类型。
//!
//! 代码生成（Cranelift）阶段的错误仍可能以 `panic!` 形式抛出（目前不在本次
//! 重构范围内），上层调用方可按需在边界处捕获。

use std::fmt;
use std::panic;
use std::sync::Once;

/// 错误种类（用于决定报错前缀，不改变处理逻辑）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Io,
    Lex,
    Parse,
    Lowering,
    TypeCheck,
    CodeGen,
    Link,
    Other,
}

/// 源文件中的出错位置（行 / 列）。
#[derive(Debug, Clone)]
pub struct SourceSpan {
    pub line: usize,
    pub col: usize,
    /// 可选的源码行片段（目前未填充，预留扩展）。
    pub snippet: Option<String>,
}

/// 编译器的统一错误类型。
#[derive(Debug, Clone)]
pub struct MplangError {
    pub kind: ErrorKind,
    pub message: String,
    pub span: Option<SourceSpan>,
}

/// 统一的 `Result` 别名，避免在各处重复书写。
pub type Result<T> = std::result::Result<T, MplangError>;

impl MplangError {
    /// 构造一个不带位置信息的错误。
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            span: None,
        }
    }

    /// 附加出错位置（行 / 列）。
    pub fn with_span(mut self, line: usize, col: usize) -> Self {
        self.span = Some(SourceSpan {
            line,
            col,
            snippet: None,
        });
        self
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Io, message)
    }
    pub fn lex(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Lex, message)
    }
    pub fn parse(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Parse, message)
    }
    pub fn lowering(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Lowering, message)
    }
    pub fn type_error(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::TypeCheck, message)
    }
    pub fn codegen(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::CodeGen, message)
    }
    pub fn link(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Link, message)
    }
    pub fn other(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Other, message)
    }
}

fn kind_label(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Io => "IO",
        ErrorKind::Lex => "词法",
        ErrorKind::Parse => "语法",
        ErrorKind::Lowering => "名字解析",
        ErrorKind::TypeCheck => "类型",
        ErrorKind::CodeGen => "代码生成",
        ErrorKind::Link => "链接",
        ErrorKind::Other => "内部",
    }
}

impl fmt::Display for MplangError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.span {
            Some(span) => write!(
                f,
                "[{}] {}（第 {} 行，第 {} 列）",
                kind_label(self.kind),
                self.message,
                span.line,
                span.col
            ),
            None => write!(f, "[{}] {}", kind_label(self.kind), self.message),
        }
    }
}

impl std::error::Error for MplangError {}

impl From<std::io::Error> for MplangError {
    fn from(e: std::io::Error) -> Self {
        MplangError::io(e.to_string())
    }
}

impl From<String> for MplangError {
    fn from(s: String) -> Self {
        MplangError::other(s)
    }
}

impl From<&str> for MplangError {
    fn from(s: &str) -> Self {
        MplangError::other(s.to_string())
    }
}

/// 以「不可恢复的编译器错误」形式抛出 [`MplangError`]，
/// 由阶段入口的 [`into_result`] 捕获并转换为 [`Result`]。
/// 返回值为 `!`，故可放在任何需要返回值的错误分支中。
///
/// # 注意
/// 仅用于上报**应上抛给用户**的编译错误；内部不可能到达的分支
/// （如 `unreachable!()`）不应使用它。
pub fn fatal(err: MplangError) -> ! {
    std::panic::panic_any(err)
}

/// 把可能 `panic` 的阶段逻辑包装为 [`Result`]：
/// - 若内部通过 [`fatal`] 抛出了 [`MplangError`]，则转为 `Err`；
/// - 其它 `panic`（内部 bug）归并为 [`ErrorKind::Other`]；
/// - 正常完成则返回 `Ok`。
pub fn into_result<T>(f: impl FnOnce() -> T) -> Result<T> {
    // 编译器的「预期错误」通过 `panic_any(MplangError)` 上报，再由本函数收拢为
    // `Result`。为避免这类受控 panic 触发 Rust 默认 panic 痕迹
    // （例如 "thread panicked at Box<dyn Any>"），在进程内**仅安装一次**抑制 hook：
    // 仅当 payload **不是** `MplangError` 时才转交给原 hook（用于暴露真正的内部 bug）。
    // 采用一次性安装（而非每次收拢时临时换 hook）以保证并发安全
    // （panic hook 是进程全局的，测试在多线程下并发调用本函数也不会相互干扰）。
    static INSTALL_HOOK: Once = Once::new();
    INSTALL_HOOK.call_once(|| {
        let original = panic::take_hook();
        panic::set_hook(Box::new(move |info: &panic::PanicHookInfo<'_>| {
            if info.payload().downcast_ref::<MplangError>().is_none() {
                (original)(info);
            }
        }));
    });

    match panic::catch_unwind(panic::AssertUnwindSafe(f)) {
        Ok(value) => Ok(value),
        Err(payload) => {
            if let Some(e) = payload.downcast_ref::<MplangError>() {
                Err(e.clone())
            } else if let Some(s) = payload.downcast_ref::<String>() {
                Err(MplangError::other(s.clone()))
            } else if let Some(s) = payload.downcast_ref::<&str>() {
                Err(MplangError::other((*s).to_string()))
            } else {
                Err(MplangError::other(format!("{:?}", payload)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn display_includes_kind_and_span() {
        let e = MplangError::type_error("类型不匹配").with_span(3, 7);
        assert_eq!(format!("{}", e), "[类型] 类型不匹配（第 3 行，第 7 列）");
        assert_eq!(e.kind, ErrorKind::TypeCheck);
        assert!(e.span.is_some());
    }

    #[test]
    fn display_without_span() {
        let e = MplangError::parse("缺分号");
        assert_eq!(format!("{}", e), "[语法] 缺分号");
    }

    #[test]
    fn from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "no such file");
        let e: MplangError = io_err.into();
        assert_eq!(e.kind, ErrorKind::Io);
    }

    #[test]
    fn from_str_error() {
        let e: MplangError = "boom".into();
        assert_eq!(e.kind, ErrorKind::Other);
        assert_eq!(e.message, "boom");
    }

    #[test]
    fn into_result_propagates_value() {
        let r: Result<i32> = into_result(|| 42);
        assert_eq!(r.unwrap(), 42);
    }

    #[test]
    fn into_result_catches_fatal() {
        let r: Result<i32> = into_result(|| {
            fatal(MplangError::parse("boom"));
        });
        let e = r.expect_err("should be error");
        assert_eq!(e.kind, ErrorKind::Parse);
        assert_eq!(e.message, "boom");
    }

    #[test]
    fn into_result_catches_string_panic() {
        let r: Result<i32> = into_result(|| panic!("plain string"));
        assert_eq!(r.unwrap_err().kind, ErrorKind::Other);
    }
}
