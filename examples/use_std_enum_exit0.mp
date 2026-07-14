// 演示使用 std crate 的 Option/Result API、match 与 trait 对象分配器。
#[path = "../library/std.mp"] extern crate std;
use std::println_int;
use std::println_str;
use std::print_int;
use std::print_str;
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
use std::file_read_line_opt;
use std::file_close;
use std::option_is_some;
use std::result_is_ok;

fn main() -> int {
    // 使用 Option：构造 Some 和 None
    let x: Option<int> = Some(42);
    let y: Option<int> = None;

    // option_is_some 测试
    if (option_is_some(x) != 1) { return 1; }
    if (option_is_some(y) != 0) { return 2; }

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

    // 文件 I/O + Option：写入再读取
    let path: *char = "std_enum_tmp.txt";
    let f: *char = file_open(path, "w+");
    file_write_line(f, "option-line");
    file_close(f);

    let g: *char = file_open(path, "r");
    let buf: *char = malloc_char(128);
    let result: Option<*char> = file_read_line_opt(g, buf, 128);
    file_close(g);

    // 检查是否读到内容
    let is_ok: int = option_is_some(result);
    if (is_ok != 1) { return 5; }

    // 打印读到的内容
    println_str(buf);

    // 使用 Result 类型（演示构造与检查）
    let ok_val: Result<int, int> = Ok(100);
    let err_val: Result<int, int> = Err(0 - 1);
    if (result_is_ok(ok_val) != 1) { return 6; }
    if (result_is_ok(err_val) != 0) { return 7; }

    // match on Result
    let rval: int = match(ok_val) {
        Ok(v) => { v; },
        Err(e) => { 0; },
    };
    if (rval != 100) { return 8; }

    // 使用 Vec 和 String（来自 alloc，经 std 重新导出）
    let v0: Vec<int> = vec_new(0);
    let v1: Vec<int> = vec_push(v0, 10);
    let v2: Vec<int> = vec_push(v1, 20);
    if (vec_len(v2) != 2) { return 9; }
    if (vec_get(v2, 0) != 10) { return 10; }
    if (vec_get(v2, 1) != 20) { return 11; }

    let s1: String = string_from_int(256);
    let s2: String = string_cat(s1, s1);
    if (string_len(s2) != 6) { return 12; }

    print_str("all enum tests passed");
    println_int(0);
    return 0;
}

#[link_name = "malloc"] extern fn malloc_char(size: int) -> *char;
