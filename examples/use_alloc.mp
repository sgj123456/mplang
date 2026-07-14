// 演示 alloc crate 的 Vec<T> 与 String 容器。
#[path = "../library/alloc.mp"] extern crate alloc;
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

#[link_name = "puts"] extern fn puts(*char) -> int;

fn main() -> int {
    // ---- Vec<int> ----
    let v0: Vec<int> = vec_new(0);
    let v1: Vec<int> = vec_push(v0, 10);
    let v2: Vec<int> = vec_push(v1, 20);
    let v3: Vec<int> = vec_push(v2, 30);
    if (vec_len(v3) != 3) { return 1; }
    if (vec_get(v3, 0) != 10) { return 2; }
    if (vec_get(v3, 2) != 30) { return 3; }
    let v4: Vec<int> = vec_set(v3, 1, 99);
    if (vec_get(v4, 1) != 99) { return 4; }
    let v5: Vec<int> = vec_pop(v4);
    if (vec_len(v5) != 2) { return 5; }

    // ---- String ----
    let s1: String = string_from_int(42);
    if (string_len(s1) != 2) { return 6; }
    let s2: String = string_from_str("abc");
    if (string_len(s2) != 3) { return 7; }
    let s3: String = string_cat(s1, s2);
    if (string_len(s3) != 5) { return 8; }

    puts(string_as_ptr(s3));
    return 0;
}
