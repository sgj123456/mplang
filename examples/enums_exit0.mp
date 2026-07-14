// 最小枚举测试：定义 Option<T>，构造 Some(5)，验证变体构造编译通过。

enum Option<T> { None, Some(value: T) }

fn main() -> int {
    let x: Option<int> = Some(5);
    return 0;
}
