// trait 对象（动态分发）：`*Trait` 是 fat pointer（数据指针 + vtable 指针）。
//
// - 用 `&struct` 把一个 struct 引用转换为 trait 对象（TraitCast）。
// - 通过 trait 对象的方法调用走 vtable 间接跳转（DynamicMethodCall）。

trait Showable {
    fn to_int() -> int;                       // 必填
    fn describe() -> int { return self.to_int() + self.to_int(); }  // 默认实现
}

struct Counter { v:int }

impl Showable for Counter {
    fn to_int() -> int { return self.v; }
}

fn main() -> int {
    let c: Counter = Counter { v:42 };
    // 通过 `&c` 创建 trait 对象 `*Showable`（fat pointer）
    let s: *Showable = &c;
    // 通过 trait 对象调用方法（动态分发）
    let val: int = s.to_int();
    return val - 42;  // 应返回 0
}
