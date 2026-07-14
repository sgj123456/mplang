// 演示 std crate 的打印、数值工具、容器与文件 I/O。
#[path = "../library/std.mp"] extern crate std;
use std::println_int;
use std::println_str;
use std::print_int;
use std::print_str;
use std::min;
use std::max;
use std::abs;
use std::pow;
use std::vec_new;
use std::vec_push;
use std::vec_get;
use std::vec_len;
use std::String;
use std::string_from_int;
use std::string_cat;
use std::string_len;
use std::string_as_ptr;
use std::file_open;
use std::file_write_line;
use std::file_read_line;
use std::file_close;

fn main() -> int {
    // 打印（带换行）
    println_int(42);
    println_str("hello from std");

    // 数值工具（来自 core 的重新导出）
    if (min(3, 7) != 3) { return 1; }
    if (max(3, 7) != 7) { return 2; }
    if (abs(0 - 5) != 5) { return 3; }
    if (pow(2, 10) != 1024) { return 4; }

    // 容器（来自 alloc 的重新导出）
    let v0: Vec<int> = vec_new(0);
    let v1: Vec<int> = vec_push(v0, 100);
    if (vec_len(v1) != 1) { return 5; }
    if (vec_get(v1, 0) != 100) { return 6; }

    // 字符串（来自 alloc 的重新导出）
    let s1: String = string_from_int(7);   // "7"，长度 1
    let s2: String = string_cat(s1, s1);   // "77"，长度 2
    if (string_len(s2) != 2) { return 7; }

    // 文件 I/O：写两行再读回来。
    // 注意：MPLang 没有 NULL 字面量（整型 0 与指针不可比较），故此处不显式判空，
    // 而是用「读取返回 0 个字符」作为失败信号（file_read_line 在句柄无效时返回 0）。
    let path: *char = "std_tmp.txt";
    let f: *char = file_open(path, "w+");
    file_write_line(f, "line-one");
    file_write_line(f, "line-two");
    // 回到开头重新读取（简单的 rewind：关闭再以读模式重开）。
    file_close(f);
    let g: *char = file_open(path, "r");
    let buf: *char = malloc_char(128);
    let n1: int = file_read_line(g, buf, 128);
    if (n1 < 1) { return 8; }
    println_str(buf);                  // 期望 "line-one\n"（含换行，由 fgets 保留）
    let n2: int = file_read_line(g, buf, 128);
    if (n2 < 1) { return 9; }
    println_str(buf);                  // 期望 "line-two\n"
    file_close(g);

    print_str("done");
    println_int(0);
    return 0;
}

// std 提供 malloc_char（经 alloc 的具象分配器）；此处直接复用其核心声明。
#[link_name = "malloc"] extern fn malloc_char(size: int) -> *char;
