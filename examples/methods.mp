// 为「所有类型」添加方法，并用 `.` 调用（包括结构体、int、指针、定长数组）。
// 方法体通过隐式的 `self` 接收者访问实例；`obj.method(args)` 等价于
// 把 `obj` 作为方法的第一个参数传入。`self` 由编译器自动注入，无需（也不能）显式声明。

struct Point { x:int, y:int }

// 为结构体 Point 添加方法。
impl Point {
    fn sum() -> int { return self.x + self.y; }
    fn scale(k:int) -> int { return (self.x + self.y) * k; }
}

// 为内置类型 int 添加方法。
impl int {
    fn double() -> int { return self + self; }
    fn add(o:int) -> int { return self + o; }
}

// 为指针类型 `*int` 添加方法。
impl *int {
    fn value() -> int { return *self; }
}

// 为定长数组类型 `[int; 3]` 添加方法。
impl [int; 3] {
    fn total() -> int { return self[0] + self[1] + self[2]; }
}

fn main() -> int {
    let p:Point = Point { x:3, y:4 };
    let s:int = p.sum();          // 7
    let s2:int = p.scale(2);      // 14

    let d:int = 21.double();      // 42
    let a:int = 10.add(5);        // 15

    let x:int = 5;
    let px:*int = &x;
    let v:int = px.value();       // 5

    let arr:[int; 3] = [1, 2, 3];
    let t:int = arr.total();      // 6

    if (s != 7) { return 1; }
    if (s2 != 14) { return 2; }
    if (d != 42) { return 3; }
    if (a != 15) { return 4; }
    if (v != 5) { return 5; }
    if (t != 6) { return 6; }
    return 0;
}
