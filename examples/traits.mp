// trait：一组「方法契约」，可由具体类型通过 `impl Trait for T` 实现。
//
// - 无体的方法（以 `;` 结尾）是「必填」的，实现方必须提供；
// - 带体的方法（`{ ... }`，体可空）是「默认实现」，实现方可省略（自动合成）。
//
// 实现方通过隐式 `self` 访问实例；`obj.method()` 按接收者类型静态解析到
// 对应实现（本语言无泛型，trait 仅作编译期契约 + 默认方法）。

trait Showable {
    fn to_int() -> int;                       // 必填
    fn describe() -> int { return self.to_int() + self.to_int(); }  // 默认实现
}

struct Counter { v:int }

// 为 Counter 实现 Showable：只提供必填的 to_int，describe 用默认实现。
impl Showable for Counter {
    fn to_int() -> int { return self.v; }
}

// 普通 impl（非 trait）方法照常可用。
impl int {
    fn inc() -> int { return self + 1; }
}

fn main() -> int {
    let c:Counter = Counter { v:21 };
    let d:int = c.describe();   // 默认方法 -> 42
    let e:int = c.to_int();     // 必填方法 -> 21
    let f:int = 7.inc();       // 普通 impl 方法 -> 8

    if (d != 42) { return 1; }
    if (e != 21) { return 2; }
    if (f != 8) { return 3; }
    return 0;
}
