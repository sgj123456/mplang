// 函数：定义、参数、返回类型、递归、单元返回类型（两种写法）

#[link_name = "printf"] extern fn print(*char, ...) -> int;
extern fn puts(*char) -> int;

fn add(x: int, y: int) -> int {
    return x + y;
}

// 省略返回类型标注时默认为 unit
fn log(msg: *char) {
    puts(msg);
}

// 显式写出 unit 返回类型
fn done() -> unit {
    puts("done");
}

fn fib(n: int) -> int {
    if (n <= 1) {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

fn main() -> int {
    let r: int = add(2, 3);
    print("2 + 3 = %d\n", r);

    let f: int = fib(10);
    print("fib(10) = %d\n", f);

    log("logging via a unit-returning function");
    done();
    return 0;
}
