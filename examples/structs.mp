// 结构体：定义、字面量构造、字段访问（含嵌套）、全局常量 / 变量

#[link_name = "printf"] extern fn print(*char, ...) -> int;

struct Point {
    x: int,
    y: int,
}

struct Line {
    a: Point,   // 嵌套结构体
    b: Point,
}

const SCALE: int = 2;       // 编译期常量（全局）
static COUNTER: int = 0;    // 运行时全局变量（可写）

fn make_point(x: int, y: int) -> Point {
    return Point { x: x, y: y };
}

fn main() -> int {
    let p: Point = Point { x: 1, y: 2 };
    print("p = (%d, %d)\n", p.x, p.y);

    let q: Point = make_point(3, 4);
    let line: Line = Line { a: p, b: q };
    print("line.b.x = %d\n", line.b.x);   // 嵌套字段访问

    print("scale = %d\n", SCALE);
    print("counter = %d\n", COUNTER);
    COUNTER = 7;                          // 写入全局变量
    print("counter after write = %d\n", COUNTER);

    p.x = 10;                             // 字段可写
    print("p.x after = %d\n", p.x);
    return 0;
}
