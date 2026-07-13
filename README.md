# MPLang

> 🚀 一个基于 Rust 和 Cranelift 构建的轻量级 AOT + JIT 编程语言

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
[![Rust Edition](https://img.shields.io/badge/Rust-2024-orange.svg)](./Cargo.toml)
[![Version](https://img.shields.io/badge/version-0.1.0-blue.svg)](./Cargo.toml)

## 📖 简介

**MPLang** 是一门静态类型、支持结构体、指针、模块与外部函数接口（FFI）的编译型语言。它采用经典的编译器前端架构（Lexer → Parser → AST → Lowering → TypeCheck），后端基于 [Cranelift](https://github.com/bytecodealliance/wasmtime/tree/main/cranelift) 生成原生机器码，同时支持 **AOT 编译**产出独立可执行文件与 **JIT 即时执行**两种运行模式。

整个编译流程如下：

```
源码 (.mp)
   │
   ▼  Lexer          词法分析 → Token 流
   ▼  Parser        语法分析 → AST（CompilationUnit）
   ▼  Lowering      名字解析、DefId 化 → HIR
   ▼  TypeCheck     类型推导与校验 → TypeHIR（每个表达式带类型）
   │
   ├─────────────┐
   ▼             ▼
 AOT 模式       JIT 模式
 ObjectModule   JITModule
   │             │
 目标文件(.o)   内存中直接执行
   │             │
 系统链接器 cc   （零磁盘 I/O）
   ▼
 独立可执行文件
```

### ✨ 核心特性

-   **双模执行引擎**：
    -   **AOT 编译**：生成标准目标文件并通过系统链接器产出独立可执行文件
    -   **JIT 即时执行**：内存中直接编译运行，零磁盘 I/O，适合快速原型验证与测试
-   **静态类型系统**：支持 `int`、`char`、指针（`*T`，字符串写作 `*char`）、`unit`（空类型，对应 `void`）及自定义 `struct`
-   **指针与内存操作**：支持取地址（`&x`）、解引用（`*p`）、指针比较，可用于底层操作
-   **定长数组**：`[T; N]` 类型与 `[a, b, c]` / `[v; n]`（等价 `[v, v, ..., v]`）字面量，支持取下标读写（`a[i]`），可整体作为函数参数与返回值（按聚合类型以指针方式传递/返回）
-   **结构化编程**：完整的函数定义、`if/else` 分支、`while` 循环、`return` 语句及 `let` 变量绑定
-   **数据抽象**：支持 `struct` 定义、字面量构造（`T{ field: value }`）及字段访问（`.` 语法），含嵌套结构体
-   **模块化**：支持多文件程序，通过 `mod` 声明模块、`use` 引入符号
-   **全局状态**：支持 `const`（编译期常量）与 `static`（运行时全局变量）
-   **FFI 支持**：通过 `extern` 声明无缝调用 C 标准库函数（如 `printf`, `puts`, `time`），支持可变参数（`...`）
-   **开发者工具链**：内置 `--dump-tokens`、`--dump-ast`、`--dump-hir`、`--dump-typehir` 多级调试选项，方便语言开发与诊断
-   **跨平台代码生成**：自动适配宿主平台 ABI（SystemV / Windows Fastcall）

## 🛠️ 技术栈

| 组件 | 技术选型 | 说明 |
| :--- | :--- | :--- |
| 语言 | Rust (Edition 2024) | 编译器实现语言 |
| 代码生成 | Cranelift 0.133 | 高性能 IR 到机器码的后端 |
| JIT 运行时 | cranelift-jit | 内存中编译并执行原生代码 |
| 目标文件 | cranelift-object / object 0.39 | 生成标准 ELF/COFF/Mach-O 对象文件（AOT） |
| CLI 框架 | Clap 4.6 | 命令行参数解析 |
| 链接 | 系统 cc | 复用主机工具链完成最终链接（AOT） |
| 日志 | env_logger / log | 分级诊断输出（RUST_LOG=debug） |

## 🚀 快速开始

### 环境要求

-   Rust Toolchain (Edition 2024+)
-   系统 C 编译器（GCC/Clang/MSVC，仅 AOT 链接阶段需要）

### 安装与构建

```bash
git clone <repository-url>
cd mplang
cargo build --release
```

构建完成后，编译器二进制为 `mplangc`，位于 `target/release/mplangc`。

### 运行示例

```bash
# AOT 模式：默认编译为独立可执行文件 mplang_program
./target/release/mplangc examples/hello.mp
./mplang_program

# JIT 模式：在内存中即时编译并执行（无磁盘产物）
./target/release/mplangc --jit examples/functions.mp

# 直接执行一段源码字符串
./target/release/mplangc --eval 'extern fn puts(*char); fn main() -> int { puts("hi"); return 0; }'

# 仅编译为目标文件（不进行链接）
./target/release/mplangc -c examples/functions.mp -o funcs.o

# 多级调试输出
./target/release/mplangc --dump-tokens  examples/hello.mp
./target/release/mplangc --dump-ast     examples/hello.mp
./target/release/mplangc --dump-hir     examples/hello.mp
./target/release/mplangc --dump-typehir examples/hello.mp
```

### 示例一览（`examples/`）

每个示例都是一个可独立编译运行的完整程序，并以退出码 `0` 结束；集成测试会自动对它们执行「解析 → 前端 → JIT 运行」全流程：

| 文件 | 覆盖的语法特性 |
| :--- | :--- |
| `hello.mp` | FFI（`extern` / 链接名重命名 / 可变参数）、字符串字面量、算术、比较、`if/else`、`while`、类型推断 |
| `functions.mp` | 函数定义、参数与返回类型、递归、`unit` 返回（显式与省略两种）、`if` |
| `structs.mp` | 结构体定义与字面量、字段访问（含嵌套）、全局 `const` / `static`、字段写入 |
| `pointers.mp` | 指针类型、取地址 `&`、解引用 `*`、指针比较、取字段地址、`*char` |
| `arrays.mp` | 定长数组类型 `[T; N]`、列表 / 重复字面量、下标读写、嵌套数组、数组参数与返回值、全局数组 |
| `modules.mp` + `math.mp` | 模块化：`mod` 引入同目录模块、`use` 导入符号（跨文件定义） |

### 命令行选项

| 选项 | 说明 |
| :--- | :--- |
| `<input>` | 输入的 MPLang 源文件路径（与 `--eval` 互斥） |
| `-o, --output <path>` | 输出文件路径（AOT 模式，默认 `mplang_program`） |
| `-e, --eval <code>` | 直接执行一段源码字符串（自动进入 JIT 模式） |
| `--jit` | 使用 JIT 模式即时执行（默认 AOT） |
| `-c, --compile-only` | 仅编译为目标文件 `.o`，不进行链接 |
| `--dump-tokens` | 仅打印词法分析的 Token 流 |
| `--dump-ast` | 打印抽象语法树（AST） |
| `--dump-hir` | 打印 HIR（已完成名字解析的中间表示） |
| `--dump-typehir` | 打印 TypeHIR（已完成类型检查的中间表示） |

## 📝 语言速览

### 函数与变量

```mplang
fn add(a: int, b: int) -> int {
    return a + b;
}

fn main() -> int {
    let sum: int = add(42, 100);
    return sum;
}
```

### 类型系统

-   基础类型：`int`、`char`
-   指针：`*T`（字符串字面量为 `*char`）
-   空类型：`unit`（无返回值的函数可写 `-> unit` 或省略返回类型）
-   结构体：自定义 `struct`，通过 `T{ field: value }` 构造

### 控制流

```mplang
fn fib(n: int) -> int {
    if (n <= 1) {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

fn count() -> int {
    let i: int = 0;
    while (i < 10) {
        i = i + 1;
    }
    return i;
}
```

### 指针与结构体

```mplang
fn main() -> int {
    let x: int = 10;
    let p: *int = &x;     // 取地址
    *p = 20;              // 解引用赋值

    let pt: Point = Point { x: 1, y: 2 };
    let fx: *int = &pt.x;
    *fx = 100;            // 通过指针修改字段
    return 0;
}
```

### 定长数组

```mplang
// 类型标注：[元素类型; 长度]
let arr: [int; 5] = [1, 2, 3, 4, 5];

// 重复形式：等价于 [1, 1, 1, 1]
let ones: [int; 4] = [1; 4];

// 下标读取（结果类型为元素类型）
let first: int = arr[0];

// 下标写入（左下标是左值）
arr[0] = 100;

// 数组可作为函数参数（按值拷贝传入）与返回值（通过 sret 约定）
fn make() -> [int; 2] {
    return [10, 20];
}

// 嵌套数组
let na: [[int; 2]; 2] = [[1, 2], [3, 4]];
let v: int = na[1][0];   // 3

// 取地址 + 下标（左值可读写）
let i: int = 1;
na[i][i] = 9;
```

> 注：下标为整数索引，按 `基地址 + 索引 × 元素大小` 计算，行为对齐 C 风格（不插入运行时边界检查）。

### 全局状态

```mplang
const NUM: int = 100;     // 编译期常量
static AGE: int = 30;     // 运行时全局变量
```

### FFI（外部函数接口）

```mplang
extern fn printf(*char, ...) -> int;   // 可变参数
extern fn puts(*char) -> int;
#[link_name = "printf"] extern fn print(*char, ...) -> int;  // 用注解指定导入符号名
```

### 注解（attribute）

`#[...]` 注解可放在声明前，为声明附加元信息。注解采用统一的「元项（meta item）」
树表示，支持三类形态，便于以后与用户自定义注解扩展——**新增注解无需改动
词法 / 语法 / 中间表示的结构**，只需在使用处按名读取：

- 裸标记：`#[no_mangle]`
- 键值对：`#[link_name = "real_symbol"]`
- 列表（预留）：`#[cfg(any(a, b))]`

目前实现的注解：

- `#[link_name = "sym"]`：置于 `extern fn` 前，指定该外部函数的**导入符号名**
  （即原先 `extern "sym" fn ...` 语法里字符串的部分）。省略时回退为 mplang 函数名。

```mplang
#[link_name = "printf"] extern fn print(*char, ...) -> int;  // 实际链接到 C 的 printf
extern fn puts(*char) -> int;                                // 导入符号名就是 puts
```

### crate 机制（外部依赖）

用 `extern crate NAME;` 声明对一个**外部 crate（独立编译单元 / `.mp` 文件）** 的依赖。
被引入的 crate 会作为名为 `NAME` 的模块存在，其公开项可通过 `NAME::item` 或
`use NAME::item;` 访问。

默认按 `<NAME>.mp`（相对输入目录）查找；可用 `#[path = "..."]` 注解覆盖查找路径：

- 相对路径相对于输入文件所在目录；也可写绝对路径。
- 若路径指向一个**目录**，则取其下的 `<NAME>.mp`。
- 注解值写成字符串字面量即可。

```mplang
// thirdparty.mp（外部 crate 库，自身无需 main）
fn add_twice(a: int) -> int { return a + a; }

// main.mp
#[path = "thirdparty.mp"] extern crate thirdparty;  // 也可省略 #[path]，默认读 thirdparty.mp
use thirdparty::add_twice;

fn main() -> int {
    let v = add_twice(21);
    return 0;
}
```

`#[path]` 与 `#[link_name]` 同样以「通用注解」方式读取——新增注解无需改动
词法 / 语法 / 中间表示，只需在使用处按名读取即可。

### 模块化（多文件）

```mplang
// math.mp
fn add(a: int, b: int) -> int {
    return a + b;
}

// main.mp
mod math;                // 引入同目录下的 math.mp 模块
use math::add;           // 引入 add 函数

fn main() -> int {
    let result = add(42, 100);
    return 0;
}
```

## 🧪 测试

项目内置 `examples/` 下的大量示例，集成测试会自动对它们执行「解析 → 前端（Lowering + 类型检查）→ JIT 运行」全流程：

```bash
cargo test
```

测试约定（依据文件名推断预期结果）：

-   `xxx_exit<N>.mp`：期望以退出码 `N` 结束
-   `xxx_fail.mp` / `xxx_error.mp` / `xxx_panic.mp`：期望编译/运行失败（用于负面测试）
-   其他：期望以退出码 `0` 成功运行
-   仅含 `mod`/`fn` 定义、没有 `main` 的文件（如 `math.mp`）会被跳过 JIT 执行，仅校验前端

## 🏗️ 项目结构

```
src/
├── lexer.rs / token.rs      词法分析
├── parser.rs / ast.rs      语法分析与 AST
├── lowering.rs / hir.rs    名字解析 → HIR
├── tycheck.rs / tyhir.rs   类型检查 → TypeHIR
├── compiler/               后端（CodeGen）
│   ├── backend.rs           Cranelift 代码生成
│   ├── codegen.rs           表达式 / 语句翻译
│   ├── module.rs            AOT / JIT 模块抽象
│   ├── abi.rs               平台 ABI 适配
│   ├── types.rs / memory.rs类型与内存布局
│   └── mod.rs
├── error.rs                 错误类型与处理
├── lib.rs                   库入口
└── main.rs                  CLI 入口
examples/                   语言示例（覆盖各语法特性的完整可运行程序）
tests/                      集成测试
```

## 📄 许可证

本项目基于 [MIT 许可证](./LICENSE) 开源。
