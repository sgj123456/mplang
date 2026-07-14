// std —— 对应 Rust 的 `std` crate：在 `core`（与平台 / 堆无关的基础能力）与
// `alloc`（堆分配与容器）之上，补齐「标准库」级的便利能力：
//   * 带换行的打印：`println_int` / `println_str`（其余打印见下方对 core 的重新导出）；
//   * 失败处理：`unreachable` / `abort`；
//   * 基于 libc `FILE*` 的文件 I/O：`file_open` / `file_write_line` / `file_read_line`
//     / `file_close`；
//   * 演示用 `Allocator` trait 与 trait 对象。
//
// 路径说明：这里不写 `#[path]`。编译器在解析 `extern crate` 时会从顶层输入目录
// 向上逐级寻找 `<祖先>/library/<name>.mp`，因此 `std` 被任意目录的程序引入时
// 都能正确定位到同仓库的 `library/core.mp` 与 `library/alloc.mp`。

extern crate core;
extern crate alloc;

// ───────────────── 打印（带换行） ─────────────────
//
// `printf` 来自 core 的重新导出（见文末 `use core::printf;`），
// 这里仅新增「带换行」的便利封装。

fn println_int(x: int) -> int {
    return printf("%d\n", x);
}

fn println_str(s: *char) -> int {
    return printf("%s\n", s);
}

// ───────────────── 失败处理 ─────────────────

#[link_name = "abort"] extern fn libc_abort() -> int;

// 标记「不可能到达」的分支：打印提示并终止进程。
fn unreachable() -> unit {
    printf("%s\n", "unreachable executed");
    libc_abort();
    return;
}

// 直接中止进程。
fn abort() -> unit {
    libc_abort();
    return;
}

// ───────────────── 文件 I/O（libc FILE*） ─────────────────

#[link_name = "fopen"]  extern fn libc_fopen(*char, *char) -> *char;
#[link_name = "fclose"] extern fn libc_fclose(*char) -> int;
#[link_name = "fputs"]  extern fn libc_fputs(*char, *char) -> int;
#[link_name = "fgets"]  extern fn libc_fgets(*char, int, *char) -> *char;
#[link_name = "strlen"] extern fn strlen(*char) -> int;

// 打开文件，返回 `FILE*`。
fn file_open(path: *char, mode: *char) -> *char {
    return libc_fopen(path, mode);
}

// 关闭文件。
fn file_close(f: *char) -> int {
    return libc_fclose(f);
}

// 把 C 字符串写入文件（追加换行）。
fn file_write_line(f: *char, s: *char) -> int {
    libc_fputs(s, f);
    return libc_fputs("\n", f);
}

// 从文件读取一行到调用方提供的缓冲 `buf`（容量 `max`，含结尾 '\0'）。
// 成功返回读取的字符数（不含结尾 '\0'）；到达文件尾且无内容时返回 0。
fn file_read_line(f: *char, buf: *char, max: int) -> int {
    libc_fgets(buf, max, f);
    return strlen(buf);
}

// ───────────────── 演示：Allocator trait 与 trait 对象 ─────────────────

#[link_name = "malloc"] extern fn malloc_char(size: int) -> *char;
#[link_name = "free"]   extern fn free_char(ptr: *char) -> int;

trait Allocator {
    fn alloc(size: int) -> *char;
    fn dealloc(p: *char) -> int;
}

struct SysAllocator {}

impl Allocator for SysAllocator {
    fn alloc(size: int) -> *char {
        return malloc_char(size);
    }
    fn dealloc(p: *char) -> int {
        return free_char(p);
    }
}

// 通过 trait 对象调用分配器（演示动态分发）。
fn alloc_with(a: *Allocator, size: int) -> int {
    return a.alloc(size);
}

// ───────────────── 重新导出 core / alloc 的既有能力 ─────────────────
//
// 整份编译单元里函数名必须唯一，故这里用 `use` 复用 core / alloc 中已定义的
// 函数与类型，而不是重新声明（否则会出现 DuplicateDefinition）。

use core::printf;
use core::puts;
use core::print_int;
use core::print_str;
use core::swap;
use core::identity;

// 重新导出 alloc 的构造器（静态方法）与类型。
use alloc::Vec;
use alloc::String;
