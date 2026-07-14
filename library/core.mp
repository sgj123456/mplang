// core —— 对应 Rust 的 `core` crate：与平台 / 堆无关的基础能力。
//
// 这里提供数值工具、泛型算法、内存与字符串的 libc 薄封装，以及打印辅助。
// 本 crate 不依赖堆，可独立被 `extern crate core;` 引入使用。
//
// 注意：MPLang 现在支持 enum 与 match，故提供 Rust 式的 `Option` / `Result` 类型。

// ───────────────── Option / Result 枚举 ─────────────────

enum Option<T> { None, Some(value: T) }

enum Result<T, E> { Ok(value: T), Err(error: E) }

impl<T> Option<T> {
    fn is_some() -> int {
        return match(self) {
            None => { 0; },
            Some(v) => { 1; },
        };
    }
}

impl<T, E> Result<T, E> {
    fn is_ok() -> int {
        return match(self) {
            Ok(v) => { 1; },
            Err(e) => { 0; },
        };
    }
}

// ───────────────── 打印辅助（供 alloc / std 复用） ─────────────────

#[link_name = "printf"] extern fn printf(*char, ...) -> int;
#[link_name = "puts"]   extern fn puts(*char) -> int;

// 自由函数版（向后兼容）。
fn print_int(x: int) -> int {
    return printf("%d", x);
}

fn print_str(s: *char) -> int {
    return puts(s);
}

// 以 impl 方法提供 `print` / `println`（通过方法分发实现「特化」效果）。
impl int {
    fn print() -> int {
        return printf("%d", self);
    }

    fn println() -> int {
        return printf("%d\n", self);
    }
}

impl *char {
    fn print() -> int {
        return puts(self);
    }

    fn println() -> int {
        return printf("%s\n", self);
    }
}

// ───────────────── 数值工具（以 impl int 方法提供） ─────────────────

impl int {
    fn min(b: int) -> int {
        if (self < b) { return self; }
        return b;
    }

    fn max(b: int) -> int {
        if (self > b) { return self; }
        return b;
    }

    fn abs() -> int {
        if (self < 0) { return 0 - self; }
        return self;
    }

    // 把 self 钳制到 [lo, hi] 区间。
    fn clamp(lo: int, hi: int) -> int {
        if (self < lo) { return lo; }
        if (self > hi) { return hi; }
        return self;
    }

    fn cmp(b: int) -> int {
        if (self < b) { return 0 - 1; }
        if (self > b) { return 1; }
        return 0;
    }

    fn is_equal(b: int) -> int {
        if (self == b) { return 1; }
        return 0;
    }

    // 整型幂：self.pow(exp) = self^exp（exp >= 0）。
    fn pow(exp: int) -> int {
        let r: int = 1;
        let i: int = 0;
        while (i < exp) {
            r = r * self;
            i = i + 1;
        }
        return r;
    }
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
