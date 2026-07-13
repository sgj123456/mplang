// 模块化：通过 `mod` 引入同目录下的 math.mp，并用 `use` 导入其中符号

#[link_name = "printf"] extern fn print(*char, ...) -> int;

mod math;
use math::add;
use math::sub;

fn main() -> int {
    let x: int = add(10, 5);
    let y: int = sub(x, 3);
    print("add = %d, sub = %d\n", x, y);
    return 0;
}
