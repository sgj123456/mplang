// 演示 std crate 的打印、数值工具、容器与文件 I/O（成员方法语法）。
#[path = "../library/std.mp"] extern crate std;
use std::println_int;
use std::println_str;
use std::print_int;
use std::print_str;
use std::swap;

#[link_name = "malloc"] extern fn malloc_char(size: int) -> *char;

fn main() -> int {
    // 打印（带换行）
    println_int(42);
    println_str("hello from std");

    // 数值工具（通过 impl int 成员方法调用）
    if ((3).min(7) != 3) { return 1; }
    if ((3).max(7) != 7) { return 2; }
    let neg: int = 0 - 5;
    if (neg.abs() != 5) { return 3; }
    if ((2).pow(10) != 1024) { return 4; }

    // 容器（Vec 通过成员方法调用）
    let v0: Vec<int> = Vec::new(0);
    let v1: Vec<int> = v0.push(100);
    if (v1.len() != 1) { return 5; }
    if (v1.get(0) != 100) { return 6; }

    // 字符串（String 通过成员方法调用）
    let s1: String = String::from_int(7);
    let s2: String = s1.cat(s1);
    if (s2.len() != 2) { return 7; }

    // 文件 I/O：写两行再读回来。
    let path: *char = "std_tmp.txt";
    let f: *char = std::file_open(path, "w+");
    std::file_write_line(f, "line-one");
    std::file_write_line(f, "line-two");
    std::file_close(f);
    let g: *char = std::file_open(path, "r");
    let buf: *char = malloc_char(128);
    let n1: int = std::file_read_line(g, buf, 128);
    if (n1 < 1) { return 8; }
    println_str(buf);
    let n2: int = std::file_read_line(g, buf, 128);
    if (n2 < 1) { return 9; }
    println_str(buf);
    std::file_close(g);

    print_str("done");
    println_int(0);
    return 0;
}
