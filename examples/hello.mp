// FFI 基础：外部函数声明（含链接名重命名与可变参数）、字符串字面量、基础控制流
#[link_name = "printf"] extern fn print(*char, ...) -> int;   // 链接到 C 的 printf（注解指定导入符号名）
extern fn puts(*char) -> int;
extern fn time(int) -> int;

fn main() -> int {
    print("Hello, MPLang!\n");
    puts("FFI works");

    // 字符串字面量即 *char（指向 char 的指针）
    let s: *char = "MPLang";
    print("str = %s\n", s);

    // 算术：+ - * /
    let a: int = 10;
    let b: int = 3;
    let sum: int = a + b;
    let diff: int = a - b;
    let prod: int = a * b;
    let quot: int = a / b;
    print("a=%d b=%d sum=%d prod=%d quot=%d\n", a, b, sum, prod, quot);

    // 分支
    if (a > b) {
        print("a > b\n");
    } else {
        print("a <= b\n");
    }

    // 不等比较
    let neq: int = (a != b);
    print("a != b ? %d\n", neq);

    // 循环
    let i: int = 0;
    let acc: int = 0;
    while (i < 5) {
        acc = acc + i;
        i = i + 1;
    }
    print("sum 0..4 = %d\n", acc);

    // 省略类型标注，由初始化表达式推断类型
    let now = time(0);
    print("time = %d\n", now);

    return 0;
}
