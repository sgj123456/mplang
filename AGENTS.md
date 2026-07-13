# AGENTS.md

This file provides guidance to agents (and humans) working with code in this repository.

## Project overview

MPLang (`mplangc`) is a compiler for a small statically-typed language (`.mp` source files), written in Rust (Edition 2024), built on the Cranelift codegen backend. It supports **two execution modes**:

- **AOT** (default): lower the typed IR to a native object file, then link with the system `cc` to produce a standalone executable (output `mplang_program` by default).
- **JIT** (`--jit` / `--eval`): compile and run in memory, returning the program's `main` exit code. No disk artifacts.

Requires a recent Rust toolchain (Edition 2024) and a C compiler (`cc`) only for the AOT link step.

## Commands

```bash
# Build
cargo build --release          # binary at target/release/mplangc
cargo build                     # debug binary at target/debug/mplangc

# Run the compiler (CLI entry: src/main.rs)
./target/release/mplangc examples/hello.mp          # AOT -> ./mplang_program, then run it
./target/release/mplangc --jit examples/functions.mp # JIT execute in memory
./target/release/mplangc --eval 'fn main() -> int { return 0; }'  # run a string
./target/release/mplangc -c examples/functions.mp    # emit .o only (no link)
./target/release/mplangc --dump-tokens|--dump-ast|--dump-hir|--dump-typehir <file>

# Tests (integration + unit)
cargo test                                   # all tests

# Run a single integration test file / named test
cargo test --test cli                        # the tests/cli.rs integration suite
cargo test --test pipeline
cargo test --test test_examples              # runs the whole examples suite (test fn `run_all_examples`)
cargo test run_all_examples                  # same, by test name
cargo test cli_valid_program_exits_zero      # a single test by name
cargo test --lib parse_empty_function        # a unit test inside src/*

# Lint / format (standard cargo tooling; no custom config in the repo)
cargo clippy
cargo fmt

# Diagnostics
RUST_LOG=debug ./target/release/mplangc ...  # env_logger: default filter is "warn"
```

Note: `--eval` and `--jit` cannot be combined with file input or AOT-only flags (`-o`, `-c`).

## Architecture: the compilation pipeline

Source flows through staged passes, each producing a distinct IR. The CLI in `src/main.rs` wires them together in order:

```
.mp source
  → Lexer        (src/lexer.rs, token.rs)        → Vec<Token>
  → Parser       (src/parser/* , ast.rs)         → ast::CompilationUnit
  → Lowerer      (src/lowering.rs, hir.rs)        → hir::HirCompilationUnit
  → TypeChecker  (src/tycheck.rs, tyhir.rs)       → tyhir::TyHirCompilationUnit
  → Compiler<T>  (src/compiler/*)                 → object bytes (AOT) | exit code (JIT)
```

The key insight for working in this codebase is the **`DefId`** type (`src/hir.rs`): every definition (function, struct, field, param, local, global, module) gets a unique `DefId`. All `Path` references in the AST are resolved to `DefId`s during lowering, and the backend maps `DefId` → Cranelift entity. When touching frontend → backend handoff, follow the `DefId` rather than names.

- **Lexer / Parser**: Recursive-descent parser in `src/parser/` split into `decl.rs`, `expr.rs`, `stmt.rs`. Produces `ast::CompilationUnit` (untyped).
- **Lowering** (`src/lowering.rs`): Two-pass name resolution. Pass A registers all defs into `path_to_def`; Pass B lowers and registers `use` aliases. Field names stay as strings until type checking (the type of the object is needed to resolve them). `mod x;` loads `x.mp` from the input file's directory (`base_dir`); `--eval` has no `base_dir` so module loading is unavailable there. Supports forward references and cross-file modules.
- **TypeCheck** (`src/tycheck.rs`): The trust boundary. Assigns a `HirType` to every expression (carried as `TyHirExpr.ty`), resolves field-name strings to field `DefId`s, and validates operand/return/argument types. TypeHIR is guaranteed well-typed, so codegen can blindly translate it.

## The backend (`src/compiler/`)

The backend is **generic over the Cranelift `Module` trait**:

```rust
Compiler<T: Module>   // src/compiler/mod.rs
```

- `Compiler<ObjectModule>` (`backend.rs::new/compile`): AOT. Emits object-file bytes; `main.rs` then shells out to `cc` to link.
- `Compiler<JITModule>` (`backend.rs::new/run`): JIT. Finalizes, locates `main`, and calls it. **Hard constraints**: `main` must have signature `fn() -> int` and only runs on 64-bit platforms (asserts `ptr_type().bytes() == 8`).

`Compiler` common state (`src/compiler/mod.rs`): `func_map: DefId → FuncInfo`, `data_map: DefId → (DataId, type)`, `struct_map: DefId → field layouts`, and `string_pool` for string literals. These all key on `DefId`.

`translate()` in `src/compiler/codegen/mod.rs` is the entry point and runs four ordered passes over items:
1. Global `static`/`const` values (`globals.rs::const_init`).
2. Struct field layouts (offset/alignment via `types.rs`).
3. Extern function declarations (`build_signature` in `abi.rs`).
4. Functions: declare all signatures first (enables forward calls), then define each body.

The codegen is split by AST shape: `codegen/expr.rs`, `codegen/stmt.rs`, `codegen/function.rs`, `codegen/addr_taken.rs`. `addr_taken.rs::collect_addr_taken` decides per-local storage: variables whose address is taken (`&x`) are kept in a stack slot and accessed via loads/stores, while others use SSA `Variable`s. `abi.rs` adapts the calling convention (SystemV / Windows Fastcall) via `default_call_conv()`, and applies the `sret` convention (return-by-pointer) for struct/array return types.

`HirType` → Cranelift type conversion lives in `types.rs`/`values.rs`; `ptr_type()` is 64-bit.

## Error handling pattern (important when editing)

The frontend uses a **panic-as-error** convention (`src/error.rs`):
- Error points call `error::fatal(MplangError)` to `panic_any(MplangError)`.
- Each stage entry wraps its body in `error::into_result(|| ...)`, which catches the panic and converts it to `Result<_, MplangError>`. A one-time panic hook suppresses the normal Rust panic backtrace for expected compiler errors.
- `MplangError` carries an `ErrorKind` (Io/Lex/Parse/Lowering/TypeCheck/CodeGen/Link/Other) plus an optional `SourceSpan` (line/col), and prints Chinese messages like `[类型] ...`.

Codegen still uses raw `panic!` for internal errors (not yet migrated to `MplangError`). When extending a frontend stage, prefer `fatal(...)` + `into_result` over changing return types.

## Tests

- `tests/cli.rs`: end-to-end, invokes the built `mplangc` binary via `CARGO_BIN_EXE_mplangc`; checks exit codes and `--dump-*` output and friendly error prefixes (`[类型]`, `[语法]`, `[词法]`).
- `tests/pipeline.rs`: drives the public library API (Lexer→Parser→Lower→TypeCheck→`Compiler::<ObjectModule>::compile`) with inline source strings; includes cross-file module loading via `examples/modules.mp`.
- `tests/test_examples.rs`: `run_all_examples` loads every `examples/*.mp`, runs frontend + JIT, and asserts the exit code inferred from the filename:
  - `*_exit<N>.mp` → expect exactly `N`
  - `*_fail.mp` / `*_error.mp` / `*_panic.mp` → expect any failure
  - otherwise → expect exit `0`
  - files without a `main` (e.g. `math.mp`) are skipped at JIT but still frontend-checked.
- Unit tests live inline in modules (e.g. `src/parser/mod.rs`, `src/error.rs`), run via `cargo test --lib`.

## Conventions for example files (`examples/`)

Each `examples/*.mp` must be a complete program that exits `0` (or match the failure naming above) so the integration suite passes. They are the primary end-to-end coverage for language features; add a new example to cover a feature rather than a hand-written harness.
