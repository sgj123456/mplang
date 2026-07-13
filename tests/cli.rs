//! 集成测试：直接调用编译后的 `mplangc` 二进制，验证端到端行为——
//! 退出码、错误信息的友好呈现，以及 `--dump-*` 选项的输出。

use std::process::Command;

/// Cargo 在编译集成测试时会把二进制路径注入该环境变量。
const BIN: &str = env!("CARGO_BIN_EXE_mplangc");

/// 运行 `mplangc <args>`，返回 (是否成功, stdout, stderr)。
fn run(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to launch mplangc");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (out.status.success(), stdout, stderr)
}

#[test]
fn cli_valid_program_exits_zero() {
    let (ok, _out, _err) = run(&["--eval", "fn main() -> int { return 0; }"]);
    assert!(ok, "valid program should exit 0");
}

#[test]
fn cli_type_error_exits_nonzero_with_message() {
    let (ok, _out, err) = run(&["--eval", "fn main() { let x:int = \"hi\"; }"]);
    assert!(!ok, "type error should be non-zero exit");
    assert!(
        err.contains("error: [类型]"),
        "stderr should contain friendly type error, got: {}",
        err
    );
}

#[test]
fn cli_syntax_error_exits_nonzero_with_message() {
    let (ok, _out, err) = run(&["--eval", "fn main() { let x:int = ; }"]);
    assert!(!ok, "syntax error should be non-zero exit");
    assert!(
        err.contains("error: [语法]"),
        "stderr should contain friendly syntax error, got: {}",
        err
    );
}

#[test]
fn cli_lex_error_exits_nonzero_with_message() {
    let (ok, _out, err) = run(&["--eval", "fn main() { let @ = 1; }"]);
    assert!(!ok, "lex error should be non-zero exit");
    assert!(
        err.contains("error: [词法]"),
        "stderr should contain friendly lex error, got: {}",
        err
    );
}

#[test]
fn cli_address_of_rvalue_exits_nonzero_with_message() {
    let (ok, _out, err) = run(&["--eval", "fn main() { let p:*int = &1; }"]);
    assert!(!ok, "taking address of rvalue should be non-zero exit");
    assert!(
        err.contains("error: [类型]"),
        "stderr should contain friendly type error, got: {}",
        err
    );
}

#[test]
fn cli_dump_typehir_prints_ir() {
    let (ok, out, _err) = run(&["--eval", "fn main() {}", "--dump-typehir"]);
    assert!(ok);
    assert!(
        out.contains("TyHirCompilationUnit"),
        "stdout should contain TYPE HIR dump, got: {}",
        out
    );
}

#[test]
fn cli_dump_hir_prints_ir() {
    let (ok, out, _err) = run(&["--eval", "fn main() {}", "--dump-hir"]);
    assert!(ok);
    assert!(
        out.contains("HirModule"),
        "stdout should contain HIR dump, got: {}",
        out
    );
}
