// 泛型示例：函数泛型、常量泛型、结构体泛型，常量可传给普通函数与变量。
// 涡轮鱼语法为 Rust 风格 `name::<实参>`。

// 函数泛型：恒等函数（类型参数 T）。
fn id<T>(x: T) -> T {
    return x;
}

// 常量泛型：返回数组长度 N。
fn length_of<const N: int>(v: [int; N]) -> int {
    return N;
}

// 常量泛型：把常量 N 传给普通函数，并赋给普通变量。
fn consume(n: int) -> int {
    return n + 1;
}

fn use_const_arg<const N: int>() -> int {
    let x: int = N;            // 常量泛型作为普通变量
    let y: int = consume(N);   // 常量泛型传给普通函数
    return x + y;
}

// 结构体泛型：包含一个元素与定长数组。
struct Pair<T, const N: int> {
    first: T,
    rest: [int; N],
}

// 泛型函数 + 结构体泛型组合（用涡轮鱼实例化结构体）。
fn make_pair<T, const N: int>(a: T, b: [int; N]) -> Pair<T, N> {
    return Pair::<T, N> { first: a, rest: b };
}

fn main() -> int {
    // 函数泛型：显式涡轮鱼与推断。
    let a: int = id::<int>(42);
    let b: int = id(7);
    let c: int = a - b;        // 42 - 7 = 35

    // 常量泛型：返回数组长度。
    let len: int = length_of::<3>([1, 2, 3]);   // = 3

    // 常量泛型作为参数使用。
    let uc: int = use_const_arg::<10>();        // = 10 + (10+1) = 21

    // 结构体泛型（涡轮鱼实例化）。
    let p: Pair<int, 3> = Pair::<int, 3> { first: 5, rest: [1, 2, 3] };
    let f: int = p.first;                  // = 5
    let r0: int = p.rest[0];               // = 1

    // 泛型函数返回泛型结构体。
    let p2: Pair<int, 2> = make_pair::<int, 2>(9, [4, 5]);
    let f2: int = p2.first;                // = 9

    // 汇总（用差异化的常量确保各分支都被求值）。
    let total: int = c + len + uc + f + r0 + f2;  // 35+3+21+5+1+9 = 74
    return total;
}
