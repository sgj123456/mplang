// 演示使用 std crate 的 Option/Result API（成员方法）、match 与 trait 对象分配器。
#[path = "../library/std.mp"] extern crate std;
use std::println_int;
use std::println_str;
use std::print_int;
use std::print_str;

#[link_name = "malloc"] extern fn malloc_char(size: int) -> *char;

fn main() -> int {
    // 使用 Option：构造 Some 和 None
    let x: Option<int> = Some(42);
    let y: Option<int> = None;

    // is_some 成员方法测试
    if (x.is_some() != 1) { return 1; }
    if (y.is_some() != 0) { return 2; }

    // match on Option
    let val: int = match(x) {
        None => { 0; },
        Some(v) => { v; },
    };
    if (val != 42) { return 3; }

    let val2: int = match(y) {
        None => { 99; },
        Some(v) => { v; },
    };
    if (val2 != 99) { return 4; }

    // 文件 I/O：写入再读取
    let path: *char = "std_enum_tmp.txt";
    let f: *char = std::file_open(path, "w+");
    std::file_write_line(f, "option-line");
    std::file_close(f);

    let g: *char = std::file_open(path, "r");
    let buf: *char = malloc_char(128);
    let n: int = std::file_read_line(g, buf, 128);
    std::file_close(g);
    if (n < 1) { return 5; }

    // 打印读到的内容
    println_str(buf);

    // 使用 Result 类型（演示构造与检查）
    let ok_val: Result<int, int> = Ok(100);
    let err_val: Result<int, int> = Err(0 - 1);
    if (ok_val.is_ok() != 1) { return 6; }
    if (err_val.is_ok() != 0) { return 7; }

    // match on Result
    let rval: int = match(ok_val) {
        Ok(v) => { v; },
        Err(e) => { 0; },
    };
    if (rval != 100) { return 8; }

    // 使用 Vec 和 String（成员方法语法）
    let v0: Vec<int> = Vec::new(0);
    let v1: Vec<int> = v0.push(10);
    let v2: Vec<int> = v1.push(20);
    if (v2.len() != 2) { return 9; }
    if (v2.get(0) != 10) { return 10; }
    if (v2.get(1) != 20) { return 11; }

    let s1: String = String::from_int(256);
    let s2: String = s1.cat(s1);
    if (s2.len() != 6) { return 12; }

    print_str("all enum tests passed");
    println_int(0);
    return 0;
}
