// core —— 对应 Rust 的 `core` crate：与平台 / 堆无关的基础能力。
//
// 这里提供数值工具、泛型算法、内存与字符串的 libc 薄封装，以及打印辅助。
// 本 crate 不依赖堆，可独立被 `extern crate core;` 引入使用。
//
// 注意：MPLang 没有 enum，故不提供 Rust 式的 `Option` / `Result` 类型；
// 需要「可能为空」的值时，请用裸指针（NULL 约定为 0）或额外的标志位表达。

// ───────────────── 打印辅助（供 alloc / std 复用） ─────────────────

#[link_name = "printf"] extern fn printf(*char, ...) -> int;
#[link_name = "puts"]   extern fn puts(*char) -> int;

fn print_int(x: int) -> int {
    return printf("%d", x);
}

fn print_str(s: *char) -> int {
    return puts(s);
}

// ───────────────── 数值工具 ─────────────────

fn min(a: int, b: int) -> int {
    if (a < b) { return a; }
    return b;
}

fn max(a: int, b: int) -> int {
    if (a > b) { return a; }
    return b;
}

fn abs(x: int) -> int {
    if (x < 0) { return 0 - x; }
    return x;
}

// 把 `x` 钳制到 [lo, hi] 区间。
fn clamp(x: int, lo: int, hi: int) -> int {
    if (x < lo) { return lo; }
    if (x > hi) { return hi; }
    return x;
}

fn cmp(a: int, b: int) -> int {
    if (a < b) { return 0 - 1; }
    if (a > b) { return 1; }
    return 0;
}

fn is_equal(a: int, b: int) -> int {
    if (a == b) { return 1; }
    return 0;
}

// 整型幂：pow(base, exp) = base^exp（exp >= 0）。
fn pow(base: int, exp: int) -> int {
    let r: int = 1;
    let i: int = 0;
    while (i < exp) {
        r = r * base;
        i = i + 1;
    }
    return r;
}

// ───────────────── 泛型算法 ─────────────────

// 交换两个同类型变量的值（通过指针原地修改）。
fn swap<T>(a: *T, b: *T) -> unit {
    let tmp: T = *a;
    *a = *b;
    *b = tmp;
    return;
}

// 恒等函数。
fn identity<T>(x: T) -> T {
    return x;
}

// ───────────────── 内存 / 字符串工具（直接 extern libc） ─────────────────
//
// 注意：这些函数直接以 libc 符号名为 MPLang 名声明（配合 `#[link_name]`），
// 不再包一层 MPLang 包装函数——否则 MPLang 函数名会与 libc 符号同名，在链接时
// 本地定义覆盖 libc 导入，导致包装函数递归调用自身（栈溢出）。

#[link_name = "memcpy"]  extern fn memcpy(*char, *char, int) -> *char;
#[link_name = "memset"]  extern fn memset(*char, int, int) -> *char;
#[link_name = "memcmp"]  extern fn memcmp(*char, *char, int) -> int;
#[link_name = "strlen"]  extern fn strlen(*char) -> int;
#[link_name = "strcmp"]  extern fn strcmp(*char, *char) -> int;
#[link_name = "strcpy"]  extern fn strcpy(*char, *char) -> *char;
#[link_name = "strcat"]  extern fn strcat(*char, *char) -> *char;
