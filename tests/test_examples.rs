use std::fs;
use std::path::{Path, PathBuf};

use mplangc::compiler::Compiler;
use mplangc::lexer::Lexer;
use mplangc::lowering::Lowerer;
use mplangc::parser::Parser;
use mplangc::tycheck::TypeChecker;

/// 收集 examples/ 下所有 .mp 文件并按文件名排序
fn collect_examples() -> Vec<PathBuf> {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    assert!(
        examples_dir.is_dir(),
        "examples/ directory not found at {:?}",
        examples_dir
    );

    let mut files: Vec<PathBuf> = fs::read_dir(&examples_dir)
        .expect("failed to read examples/")
        .filter_map(|e| {
            let path = e.ok()?.path();
            (path.extension().map_or(false, |ext| ext == "mp")).then_some(path)
        })
        .collect();

    files.sort();
    assert!(!files.is_empty(), "no .mp files found in examples/");
    files
}

/// 从文件名推断预期退出码
/// - `_exit<N>.mp` → 期望恰好 N
/// - `_fail.mp` / `_error.mp` / `_panic.mp` → 期望非零（任意失败）
/// - 其他 → 期望 0
fn expected_exit_code(path: &Path) -> ExpectedResult {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    if let Some(pos) = stem.rfind("_exit") {
        if let Ok(code) = stem[pos + 5..].parse::<i32>() {
            return ExpectedResult::Exact(code);
        }
    }
    if stem.contains("_fail") || stem.contains("_error") || stem.contains("_panic") {
        return ExpectedResult::ShouldFail;
    }
    ExpectedResult::Success
}

#[derive(Debug)]
enum ExpectedResult {
    Success,    // exit code == 0
    Exact(i32), // exit code == N
    ShouldFail, // any non-zero / panic
}

/// 解析源码并返回 AST（CompilationUnit），失败时返回错误信息字符串
fn parse_source(source: &str) -> Result<mplangc::ast::CompilationUnit, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut lexer = Lexer::new(chars);
    let tokens = lexer.lex().map_err(|e| e.to_string())?;
    let mut parser = Parser::new(tokens);
    parser.parse().map_err(|e| e.to_string())
}

/// 对单个 example 执行 JIT 编译运行，返回实际退出码或 panic 信息
fn run_jit(prog: &mplangc::tyhir::TyHirCompilationUnit) -> Result<i32, String> {
    use cranelift_jit::JITModule;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let compiler = Compiler::<JITModule>::new();
        compiler.run(prog)
    }));

    match result {
        Ok(code) => Ok(code),
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic during JIT execution".to_string()
            };
            Err(msg)
        }
    }
}

/// 运行前端（Lowering + 类型检查），产出 TYPE HIR；
/// 失败时将错误信息作为 `Err(String)` 返回。
fn run_frontend(
    path: &Path,
    unit: &mplangc::ast::CompilationUnit,
) -> Result<mplangc::tyhir::TyHirCompilationUnit, String> {
    let mut lowerer = Lowerer::new(Some(path));
    let hir = lowerer.lower(unit).map_err(|e| e.to_string())?;
    let mut checker = TypeChecker::new(&hir);
    let tyhir = checker.check(&hir).map_err(|e| e.to_string())?;
    // 单态化：把泛型定义按需展开为具体实例，消除所有泛型占位符
    // （函数/结构体泛型、常量泛型），使后端得到完全具体的 TYPE HIR。
    mplangc::monomorphize::monomorphize(&tyhir).map_err(|e| e.to_string())
}

/// 该编译单元是否定义了名为 `main` 的顶层函数。
fn has_main(unit: &mplangc::ast::CompilationUnit) -> bool {
    fn scan(items: &[mplangc::ast::TopLevelDecl]) -> bool {
        for decl in items {
            if let mplangc::ast::TopLevelDecl::FnDef { name, .. } = decl {
                if name == "main" {
                    return true;
                }
            }
        }
        false
    }
    scan(&unit.declarations)
}

fn assert_example(path: &Path, source: &str) {
    let expected = expected_exit_code(path);

    // Step 1: Parse
    let unit = match parse_source(source) {
        Ok(unit) => unit,
        Err(parse_err) => {
            match &expected {
                ExpectedResult::ShouldFail => return, // 解析失败也是预期的失败
                _ => panic!(
                    "[{}] parse failed unexpectedly:\n{}",
                    path.display(),
                    parse_err
                ),
            }
        }
    };

    // Step 2: 前端：AST -> HIR -> TYPE HIR（含类型检查）
    let tyhir = match run_frontend(path, &unit) {
        Ok(prog) => prog,
        Err(msg) => match &expected {
            ExpectedResult::ShouldFail => return, // 前端 / 类型错误也是预期的失败
            _ => panic!("[{}] frontend failed:\n{}", path.display(), msg),
        },
    };

    // 仅含 `mod`/`fn` 模块、没有 `main` 的文件（如 math.mp）
    // 无法独立运行，仅校验前端后跳过 JIT 执行。
    if !has_main(&unit) {
        eprintln!("    (skipped: no `main`, module file)");
        return;
    }

    // Step 3: JIT Run
    match run_jit(&tyhir) {
        Ok(actual_code) => match &expected {
            ExpectedResult::Success => assert_eq!(
                actual_code,
                0,
                "[{}] expected exit 0, got {}",
                path.display(),
                actual_code
            ),
            ExpectedResult::Exact(code) => assert_eq!(
                actual_code,
                *code,
                "[{}] expected exit {}, got {}",
                path.display(),
                code,
                actual_code
            ),
            ExpectedResult::ShouldFail => panic!(
                "[{}] expected failure but exited with code {}",
                path.display(),
                actual_code
            ),
        },
        Err(runtime_err) => match &expected {
            ExpectedResult::ShouldFail => { /* 预期内的失败 */ }
            _ => panic!(
                "[{}] JIT runtime error (expected success):\n{}",
                path.display(),
                runtime_err
            ),
        },
    }
}

// ─── Test Entry Point ────────────────────────────────────────────────────────

#[test]
fn run_all_examples() {
    let examples = collect_examples();
    eprintln!("Running {} examples via direct API...", examples.len());

    let mut failures: Vec<(PathBuf, String)> = Vec::new();

    for path in &examples {
        eprintln!("  ▶ {}", path.display());
        let source = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_example(path, &source);
        }));

        if let Err(e) = result {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                format!("{:?}", e)
            };
            eprintln!("    ✗ FAILED: {}", msg);
            failures.push((path.clone(), msg));
        } else {
            eprintln!("    ✓ ok");
        }
    }

    if !failures.is_empty() {
        panic!(
            "\n{}/{} examples failed:\n{}",
            failures.len(),
            examples.len(),
            failures
                .iter()
                .map(|(p, m)| format!("  ✗ {}\n    {}", p.display(), m))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    eprintln!("All {} examples passed.", examples.len());
}
