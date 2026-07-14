// 演示 core crate 的基础能力（数值 / 泛型 / 内存 / 字符串工具）。
#[path = "../library/core.mp"] extern crate core;
use core::min;
use core::max;
use core::abs;
use core::clamp;
use core::pow;
use core::swap;
use core::strlen;
use core::strcmp;
use core::print_int;

fn main() -> int {
    if (min(3, 7) != 3) { return 1; }
    if (max(3, 7) != 7) { return 2; }
    let neg: int = 0 - 9;
    if (abs(neg) != 9) { return 3; }
    if (clamp(15, 0, 10) != 10) { return 4; }
    if (pow(2, 5) != 32) { return 5; }

    // 泛型 swap：通过指针原地交换。
    let x: int = 1;
    let y: int = 2;
    swap(&x, &y);
    if (x != 2) { return 6; }
    if (y != 1) { return 6; }

    // 字符串工具：长度与比较。
    if (strlen("hello") != 5) { return 7; }
    if (strcmp("abc", "abc") != 0) { return 8; }

    print_int(pow(2, 5));
    return 0;
}
