// 定长数组（fixed-length arrays）特性示例
extern fn printf(*char, ...) -> int;

// 全局数组常量（编译期初始化）
static G: [int; 3] = [7, 8, 9];

// 数组作为函数参数（按值拷贝传入）
fn sum(a: [int; 5]) -> int {
    let total: int = 0;
    let i: int = 0;
    while (i < 5) {
        total = total + a[i];
        i = i + 1;
    }
    return total;
}

// 数组作为函数返回值（通过 sret 约定）
fn make() -> [int; 4] {
    return [10, 20, 30, 40];
}

fn main() -> int {
    // 列表形式字面量 + 类型标注
    let arr: [int; 5] = [1, 2, 3, 4, 5];
    printf("sum = %d\n", sum(arr));

    // 下标写入
    arr[0] = 100;
    printf("arr[0] = %d\n", arr[0]);

    // 重复形式字面量：[1; 4] 等价 [1, 1, 1, 1]
    let ones: [int; 4] = [1; 4];
    printf("ones[2] = %d\n", ones[2]);

    // 函数返回数组
    let m: [int; 4] = make();
    printf("m[3] = %d\n", m[3]);

    // 嵌套数组 [[int; 2]; 2]
    let na: [[int; 2]; 2] = [[1, 2], [3, 4]];
    printf("na[1][0] = %d\n", na[1][0]);

    // 取地址 + 下标（左值可读写）
    let idx: int = 1;
    let v: int = na[idx][idx];
    printf("na[1][1] = %d\n", v);

    // 全局数组读写
    G[0] = 70;
    printf("G[0] = %d, G[2] = %d\n", G[0], G[2]);

    return 0;
}
