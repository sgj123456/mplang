// 演示 alloc crate 的 Vec<T> 与 String 容器（实例操作用成员方法语法）。
#[path = "../library/alloc.mp"] extern crate alloc;
use alloc::vec_new;
use alloc::string_from_int;
use alloc::string_from_str;

#[link_name = "puts"] extern fn puts(*char) -> int;

fn main() -> int {
    // ---- Vec<int> ----
    let v0: Vec<int> = vec_new(0);
    let v1: Vec<int> = v0.push(10);
    let v2: Vec<int> = v1.push(20);
    let v3: Vec<int> = v2.push(30);
    if (v3.len() != 3) { return 1; }
    if (v3.get(0) != 10) { return 2; }
    if (v3.get(2) != 30) { return 3; }
    let v4: Vec<int> = v3.set(1, 99);
    if (v4.get(1) != 99) { return 4; }
    let v5: Vec<int> = v4.pop();
    if (v5.len() != 2) { return 5; }

    // ---- String ----
    let s1: String = string_from_int(42);
    if (s1.len() != 2) { return 6; }
    let s2: String = string_from_str("abc");
    if (s2.len() != 3) { return 7; }
    let s3: String = s1.cat(s2);
    if (s3.len() != 5) { return 8; }

    puts(s3.as_ptr());
    return 0;
}
