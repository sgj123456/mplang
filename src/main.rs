use std::path::Path;
use std::process::Command;

use clap::Parser as ClapParser;
use cranelift_jit::JITModule;
use cranelift_object::ObjectModule;
use env_logger::Builder as EnvLoggerBuilder;
use env_logger::Env;

use mplangc::{
    ast::CompilationUnit, compiler::Compiler, error::MplangError, error::into_result, lexer::Lexer,
    lowering::Lowerer, monomorphize::monomorphize, parser::Parser, tycheck::TypeChecker,
};

/// MPLang Compiler - 支持 AOT 编译与 JIT 即时执行
#[derive(ClapParser, Debug)]
#[command(name = "mplangc")]
#[command(version, about, long_about = None)]
struct Cli {
    /// 输入的 MPLang 源文件路径
    #[arg(required_unless_present = "eval")]
    input: Option<String>,

    /// 输出文件路径（仅 AOT 模式有效）
    #[arg(short, long, conflicts_with = "jit")]
    output: Option<String>,

    /// 直接执行一段源码字符串（与 input 互斥）
    #[arg(short, long, conflicts_with = "input")]
    eval: Option<String>,

    /// 使用 JIT 模式即时执行（默认 AOT）
    #[arg(long, conflicts_with_all = ["compile_only", "output"])]
    jit: bool,

    /// 仅编译为目标文件(.o)，不进行链接（仅 AOT）
    #[arg(short = 'c', conflicts_with = "jit")]
    compile_only: bool,

    /// 仅打印 token 流
    #[arg(long)]
    dump_tokens: bool,

    /// 打印 AST
    #[arg(long)]
    dump_ast: bool,

    /// 打印 HIR（高层中间表示，已完成名字解析）
    #[arg(long)]
    dump_hir: bool,

    /// 打印 TYPE HIR（类型化高层中间表示，已完成类型检查）
    #[arg(long)]
    dump_typehir: bool,
}

fn main() {
    // 默认仅显示 warn 及以上日志；诊断信息需 `RUST_LOG=debug` 或 `info` 开启。
    EnvLoggerBuilder::from_env(Env::default().default_filter_or("warn")).init();

    if let Err(e) = run() {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), MplangError> {
    let cli = Cli::parse();

    // ---------- 获取源码 ----------
    let (raw, input_path): (Vec<char>, Option<std::path::PathBuf>) =
        if let Some(ref code) = cli.eval {
            (code.chars().collect(), None)
        } else if let Some(ref path) = cli.input {
            let content = std::fs::read_to_string(path)
                .map_err(|e| MplangError::io(format!("无法读取文件 '{}'：{}", path, e)))?;
            (
                content.chars().collect(),
                Some(Path::new(path).to_path_buf()),
            )
        } else {
            unreachable!()
        };

    // ---------- Lexing & Parsing ----------
    let mut lexer = Lexer::new(raw);
    let tokens = lexer.lex()?;
    if cli.dump_tokens {
        for tok in &tokens {
            println!("{:?}", tok);
        }
        return Ok(());
    }

    let mut parser = Parser::new(tokens);
    let ast: CompilationUnit = parser.parse()?;
    if cli.dump_ast {
        println!("{:#?}", ast);
        return Ok(());
    }

    // ---------- 前端：AST -> HIR -> TYPE HIR ----------
    // 1) Lowering：名字解析，产出 HIR（DefId 化的 AST）。
    let mut lowerer = Lowerer::new(input_path.as_deref());
    let hir = lowerer.lower(&ast)?;
    if cli.dump_hir {
        println!("{:#?}", hir);
        return Ok(());
    }

    // 2) 类型检查：产出 TYPE HIR（每个表达式带类型，字段名解析为 DefId）。
    let mut checker = TypeChecker::new(&hir);
    let tyhir = checker.check(&hir)?;
    if cli.dump_typehir {
        println!("{:#?}", tyhir);
        return Ok(());
    }

    // 3) 单态化：把泛型定义按需展开为具体实例，消除所有泛型占位符
    //    （函数/结构体泛型、常量泛型），使后端得到完全具体的 TYPE HIR。
    let tyhir = monomorphize(&tyhir)?;

    // ---------- CodeGen 分发 ----------
    // `--eval` 意为“执行一段源码”，应在内存中即时执行（JIT），不产生任何磁盘文件；
    // 否则默认 AOT 路径可能会把可执行文件（默认 `mplang_program`）留在当前目录。
    if cli.jit || cli.eval.is_some() {
        // ===== JIT 模式 =====
        // 代码生成阶段的内部错误（如缺少 `main`）以 panic 上报，这里统一收拢为错误。
        let exit_code = into_result(|| Compiler::<JITModule>::new().run(&tyhir))
            .map_err(|e| MplangError::codegen(e.message))?;
        std::process::exit(exit_code);
    } else {
        // ===== AOT 模式 =====
        let obj_bytes = into_result(|| Compiler::<ObjectModule>::new().compile(&tyhir))
            .map_err(|e| MplangError::codegen(e.message))?;

        if cli.compile_only {
            let output = cli.output.unwrap_or_else(|| "mplang_output.o".to_string());
            std::fs::write(&output, &obj_bytes)
                .map_err(|e| MplangError::io(format!("无法写入文件 '{}'：{}", output, e)))?;
            eprintln!("已编译 -> {}", output);
        } else {
            let final_output = cli.output.unwrap_or_else(|| "mplang_program".to_string());
            let tmp_obj = format!("{}.tmp.o", final_output);
            std::fs::write(&tmp_obj, &obj_bytes)
                .map_err(|e| MplangError::io(format!("无法写入文件 '{}'：{}", tmp_obj, e)))?;

            let status = Command::new("cc")
                .arg(&tmp_obj)
                .arg("-o")
                .arg(&final_output)
                .status()
                .map_err(|e| MplangError::link(format!("无法调用链接器 cc：{}", e)))?;

            let _ = std::fs::remove_file(&tmp_obj);
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            } else {
                eprintln!("已编译 -> {}", final_output);
            }
        }
    }

    Ok(())
}
