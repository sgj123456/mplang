// std —— 对应 Rust 的 `std` crate：在 `core`（与平台 / 堆无关的基础能力）与
// `alloc`（堆分配与容器）之上，补齐「标准库」级的便利能力：
//   * 带换行的打印：`println_int` / `println_str`（其余打印见下方对 core 的重新导出）；
//   * 失败处理：`unreachable` / `abort`；
//   * 基于 libc `FILE*` 的文件 I/O：`file_open` / `file_write_line` / `file_read_line`
//     / `file_close`。
//
// 受 MPLang 能力限制（无 enum、无指针下标、无 cast、无 NULL 字面量）：
//   * 文件句柄以 `*char`（即 C 的 `FILE*`）表示；
//   * 读取以「调用方提供缓冲」的 `file_read_line` 形式给出，规避「堆上可变长缓冲
//     无法按元素索引」的限制；
//   * 不显式判空（整型 0 与指针不可比较），改用「读取返回 0 个字符」作为失败信号；
//   * 名称在整份编译单元（所有 crate）中必须唯一，故 core / alloc 已定义的
//     `print_int` / `printf` 等此处只「重新导出」（`use`），不再重复定义。
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
// 注意：返回的字符串以 '\0' 结尾，但可能包含行尾的 '\n'（由 libc `fgets` 决定）。
// 说明：MPLang 没有 NULL 字面量（整型 0 与指针不可比较），故不显式判空，
// 而是直接依赖 `libc_fgets` 的返回值——句柄无效时 `strlen(buf)` 亦为 0。
fn file_read_line(f: *char, buf: *char, max: int) -> int {
    libc_fgets(buf, max, f);
    return strlen(buf);
}

// ───────────────── 重新导出 core / alloc 的既有能力 ─────────────────
//
// 整份编译单元里函数名必须唯一，故这里用 `use` 复用 core / alloc 中已定义的
// 函数与类型，而不是重新声明（否则会出现 DuplicateDefinition）。

use core::printf;
use core::puts;
use core::print_int;
use core::print_str;
use core::min;
use core::max;
use core::abs;
use core::clamp;
use core::pow;
use core::swap;
use core::identity;

use alloc::Vec;
use alloc::String;
use alloc::vec_new;
use alloc::vec_push;
use alloc::vec_get;
use alloc::vec_set;
use alloc::vec_len;
use alloc::vec_pop;
use alloc::string_new;
use alloc::string_from_int;
use alloc::string_from_str;
use alloc::string_cat;
use alloc::string_len;
use alloc::string_as_ptr;
