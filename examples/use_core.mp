// 演示 core crate 的基础能力（数值 / 泛型 / 内存 / 字符串工具）。
// 注意：数值工具现在以 `impl int` 成员方法提供，使用 `x.min(y)` 语法。
#[path = "../library/core.mp"] extern crate core;
use core::swap;
use core::strlen;
use core::strcmp;
use core::print_int;

fn main() -> int {
    if ((3).min(7) != 3) { return 1; }
    if ((3).max(7) != 7) { return 2; }
    let neg: int = 0 - 9;
    if (neg.abs() != 9) { return 3; }
    if ((15).clamp(0, 10) != 10) { return 4; }
    if ((2).pow(5) != 32) { return 5; }

    // 泛型 swap：通过指针原地交换。
    let x: int = 1;
    let y: int = 2;
    swap(&x, &y);
    if (x != 2) { return 6; }
    if (y != 1) { return 6; }

    // 字符串工具：长度与比较。
    if (strlen("hello") != 5) { return 7; }
    if (strcmp("abc", "abc") != 0) { return 8; }

    print_int((2).pow(5));
    return 0;
}
