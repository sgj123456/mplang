// 被 `modules.mp` 通过 `mod math;` 引入的辅助模块（自身不含 main，集成测试会自动跳过）

fn add(a: int, b: int) -> int {
    return a + b;
}

fn sub(a: int, b: int) -> int {
    return a - b;
}
