// 测试 int 字面量 match。
fn main() -> int {
    let x: int = 1;
    let r: int = match(x) {
        0 => { 100; },
        1 => { 200; },
        _ => { 300; },
    };
    return r - 200;  // 期望 0
}
