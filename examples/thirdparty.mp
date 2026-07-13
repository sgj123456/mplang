// 外部 crate 示例库：被 `extern crate` 以 `#[path]` 引入，对外暴露若干函数。
// 一个 crate 是一个独立的编译单元（单独的 .mp 文件），本身无需 `main`。

fn add_twice(a: int) -> int {
    return a + a;
}

fn message() -> int {
    return 100;
}
