// 指针：类型标注、取地址 &、解引用 *、指针比较、取字段地址、char*

#[link_name = "printf"] extern fn print(*char, ...) -> int;

struct S {
    v: int,
}

fn main() -> int {
    let x: int = 42;
    let p: *int = &x;
    print("*p = %d\n", *p);

    *p = 100;
    print("x after *p = 100 -> %d\n", x);

    // 取结构体字段的地址，再解引用改写字段
    let s: S = S { v: 7 };
    let pv: *int = &s.v;
    *pv = 99;
    print("s.v = %d\n", s.v);

    // 指针比较（按地址比较）
    let a: int = 1;
    let b: int = 1;
    let pa: *int = &a;
    let pb: *int = &b;
    let same: int = (pa == pb);
    print("pa == pb ? %d (应为 0：二者地址不同)\n", same);

    // *char：字符串字面量即为指向 char 的指针
    let msg: *char = "hi";
    print("msg = %s\n", msg);

    return 0;
}
