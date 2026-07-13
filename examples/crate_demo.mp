// crate 机制：用 `extern crate` 声明外部 crate，并用 `#[path = "..."]` 注解
// 指定该 crate 源文件的查找路径（相对输入目录，或绝对路径）。
// 引入后，外部 crate 的公开项可通过 `name::item` 或 `use name::item;` 访问。

#[link_name = "printf"] extern fn print(*char, ...) -> int;

#[path = "thirdparty.mp"] extern crate thirdparty;
use thirdparty::add_twice;
use thirdparty::message;

fn main() -> int {
    let v: int = add_twice(21);
    print("add_twice(21) = %d\n", v);
    let m: int = message();
    print("message = %d\n", m);
    return 0;
}
