// 演示通过 impl 方法实现的打印「特化」。
// `int` 和 `*char` 各自实现了 `print()` 和 `println()` 方法。
#[path = "../library/core.mp"] extern crate core;

#[link_name = "printf"] extern fn printf(*char, ...) -> int;

fn main() -> int {
    let x: int = 42;
    let s: *char = "hello";

    // 通过方法分发实现的类型特化打印：
    x.print();
    printf(" ");
    s.print();
    printf("\n");

    // println 版本（自动追加换行）
    (100).println();
    "world".println();

    return 0;
}
