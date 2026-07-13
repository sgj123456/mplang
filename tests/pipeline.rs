//! 集成测试：通过公开 API 走完「词法 → 语法 → HIR → TYPE HIR → 代码生成」全链路，
//! 覆盖正常程序与各类错误场景；并复用 `examples/modules.mp` 验证跨文件模块加载。

use std::path::Path;

use cranelift_object::ObjectModule;
use mplangc::compiler::Compiler;
use mplangc::error::ErrorKind;
use mplangc::lexer::Lexer;
use mplangc::lowering::Lowerer;
use mplangc::parser::Parser;
use mplangc::tycheck::TypeChecker;

/// 解析 -> 降级 -> 类型检查 全链路，返回 TyHIR 或首个错误。
fn frontend(
    src: &str,
) -> Result<mplangc::tyhir::TyHirCompilationUnit, mplangc::error::MplangError> {
    let toks = Lexer::new(src.chars().collect()).lex()?;
    let ast = Parser::new(toks).parse()?;
    let hir = Lowerer::new(None).lower(&ast)?;
    TypeChecker::new(&hir).check(&hir)
}

/// 完整的 AOT 编译（产出目标文件字节），用于验证端到端可被后端消费。
fn compile(src: &str) -> Vec<u8> {
    let tyhir = frontend(src).expect("frontend should succeed");
    Compiler::<ObjectModule>::new().compile(&tyhir)
}

#[test]
fn valid_programs_typecheck() {
    for src in [
        "fn main() {}",
        "fn main() { let x:int = 1; let y:int = x + 2; }",
        "struct Point { x:int, y:int } fn main() { let p:Point = Point { x:1, y:2 }; let d:int = p.x + p.y; }",
        "fn add(a:int,b:int)->int { return a+b; } fn main() { let r:int = add(3,4); }",
    ] {
        assert!(frontend(src).is_ok(), "expected to typecheck: {}", src);
    }
}

#[test]
fn valid_programs_compile_to_object() {
    for src in [
        "fn main() {}",
        "fn add(a:int,b:int)->int { return a+b; } fn main() { let r:int = add(3,4); }",
    ] {
        let bytes = compile(src);
        assert!(!bytes.is_empty(), "object should not be empty: {}", src);
    }
}

#[test]
fn negative_type_mismatch() {
    let e = frontend("fn main() { let x:int = \"hi\"; }").unwrap_err();
    assert_eq!(e.kind, ErrorKind::TypeCheck);
}

#[test]
fn negative_undefined_function() {
    let e = frontend("fn main() { let r:int = missing(1); }").unwrap_err();
    assert_eq!(e.kind, ErrorKind::Lowering);
}

#[test]
fn negative_syntax_error() {
    let toks = Lexer::new("fn main() { let x:int = ; }".chars().collect())
        .lex()
        .unwrap();
    assert!(Parser::new(toks).parse().is_err());
}

#[test]
fn cross_file_module_via_examples() {
    // 复用 examples/modules.mp：其 `mod math; use math::add/sub` 加载 examples/math.mp。
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let src = std::fs::read_to_string(dir.join("modules.mp")).unwrap();
    let toks = Lexer::new(src.chars().collect()).lex().unwrap();
    let ast = Parser::new(toks).parse().unwrap();
    let hir = Lowerer::new(Some(&dir.join("modules.mp")))
        .lower(&ast)
        .unwrap();
    let tyhir = TypeChecker::new(&hir).check(&hir).unwrap();
    // 端到端编译为对象，验证模块加载 + 类型检查 + 代码生成链路连通。
    let bytes = Compiler::<ObjectModule>::new().compile(&tyhir);
    assert!(!bytes.is_empty());
}

/// 端到端验证指针类型 / 取地址符 / 解引用 / char* 全链路可编译为对象文件。
#[test]
fn pointer_pipeline() {
    let src = "\
        struct Point { x:int, y:int }
        fn main() {
            let s:*char = \"hi\";
            let n:int = 1;
            let p:*int = &n;
            *p = 2;
            let v:int = *p;
            let pt:Point = Point { x:3, y:4 };
            let fx:*int = &pt.x;
            *fx = 5;
            let c:int = p == fx;
        }
    ";
    let bytes = compile(src);
    assert!(!bytes.is_empty());
}
