// alloc —— 对应 Rust 的 `alloc` crate：堆分配与容器。
//
// 受 MPLang 能力限制：
//   * 没有指针下标（`p[i]` 不支持），故堆上的「可变长缓冲」无法按元素索引；
//   * 没有 enum，故不提供 Rust 式 `Option` / `Result`。
// 因此容器采用如下务实设计：
//   * `Vec<T>`：定容数组向量（容量 64，类似 Rust 的 arrayvec）。元素存于内联数组
//     `buf: [T; 64]`，通过「按值传入、返回修改后的副本」的函数式风格操作，规避
//     「指针参数的字段无法作赋值左值」的限制。
//   * `String`：基于 `*char` 缓冲 + libc（sprintf / strcpy / strcat / strlen）构建与
//     拼接，无需 MPLang 侧下标（写入经 libc 完成）。
//   * `Box<T>`：以 `*T` 表示单个堆值，提供具象类型的构造 / 读写。
//
// 堆分配器：用 `#[link_name = "malloc"]` 声明「同 C 符号、不同 MPLang 返回类型」的
// 具象分配器（指针等宽，ABI 上即 reinterpret，无需 cast）。

// ───────────────── 堆分配器（具象 typed FFI） ─────────────────

#[link_name = "malloc"] extern fn malloc_int(size: int) -> *int;
#[link_name = "malloc"] extern fn malloc_char(size: int) -> *char;
#[link_name = "free"]   extern fn free_int(ptr: *int) -> int;
#[link_name = "free"]   extern fn free_char(ptr: *char) -> int;

// ───────────────── Box<T>：单个堆值（以 *T 表示） ─────────────────
//
// 由于没有提供泛型 FFI，这里为常用具象类型提供构造 / 读写函数。

fn box_new_int(x: int) -> *int {
    let p: *int = malloc_int(8);
    *p = x;
    return p;
}

fn box_get_int(p: *int) -> int {
    return *p;
}

fn box_set_int(p: *int, x: int) -> int {
    *p = x;
    return x;
}

fn box_free_int(p: *int) -> int {
    return free_int(p);
}

// ───────────────── Vec<T>：定容 64 的向量 ─────────────────

struct Vec<T> {
    buf: [T; 64],
    len: int,
}

// 以 `zero` 作为初始元素构造一个空向量（len = 0）。
// 注意：数组字面量需由与 T 同类型的值填充，故传入 `zero: T` 而非整型 0，
// 以在跨文件泛型实例化时正确推导元素类型。
fn vec_new<T>(zero: T) -> Vec<T> {
    let v: Vec<T> = Vec::<T> {
        buf: [zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero, zero],
        len: 0,
    };
    return v;
}

// 若未满（len < 64），将 x 追加到末尾。返回更新后的向量。
fn vec_push<T>(v: Vec<T>, x: T) -> Vec<T> {
    if (v.len < 64) {
        v.buf[v.len] = x;
        v.len = v.len + 1;
    }
    return v;
}

fn vec_get<T>(v: Vec<T>, i: int) -> T {
    return v.buf[i];
}

// 将下标 i 处元素设为 x，返回更新后的向量。
fn vec_set<T>(v: Vec<T>, i: int, x: T) -> Vec<T> {
    v.buf[i] = x;
    return v;
}

fn vec_len<T>(v: Vec<T>) -> int {
    return v.len;
}

// 若非空，弹出末尾元素（仅减小长度；返回更新后的向量）。
fn vec_pop<T>(v: Vec<T>) -> Vec<T> {
    if (v.len > 0) {
        v.len = v.len - 1;
    }
    return v;
}

// ───────────────── String：基于 *char 缓冲 + libc ─────────────────

#[link_name = "sprintf"] extern fn libc_sprintf(*char, *char, ...) -> int;
#[link_name = "strlen"]  extern fn libc_strlen(*char) -> int;
#[link_name = "strcpy"]  extern fn libc_strcpy(*char, *char) -> *char;
#[link_name = "strcat"]  extern fn libc_strcat(*char, *char) -> *char;

struct String {
    buf: *char,
    len: int,
    cap: int,
}

// 构造一个空字符串（初始容量 64 字节）。
fn string_new() -> String {
    let b: *char = malloc_char(64);
    libc_strcpy(b, "");
    let s: String = String { buf: b, len: 0, cap: 64 };
    return s;
}

// 从整数构造字符串（使用 libc sprintf）。
fn string_from_int(x: int) -> String {
    let b: *char = malloc_char(64);
    libc_sprintf(b, "%d", x);
    let s: String = String { buf: b, len: libc_strlen(b), cap: 64 };
    return s;
}

// 从 C 字符串构造（深拷贝）。
fn string_from_str(src: *char) -> String {
    let b: *char = malloc_char(64);
    libc_strcpy(b, src);
    let s: String = String { buf: b, len: libc_strlen(b), cap: 64 };
    return s;
}

fn string_len(s: String) -> int {
    return s.len;
}

// 在末尾追加一个字符（用 libc 写入，避免 MPLang 侧指针下标）。
fn string_push(s: String, c: char) -> String {
    let tmp: *char = malloc_char(8);
    libc_sprintf(tmp, "%c", c);
    libc_strcat(s.buf, tmp);
    let n: String = String { buf: s.buf, len: libc_strlen(s.buf), cap: s.cap };
    return n;
}

// 拼接两个字符串（深拷贝到新缓冲）。
fn string_cat(a: String, b: String) -> String {
    let out: *char = malloc_char(128);
    libc_strcpy(out, a.buf);
    libc_strcat(out, b.buf);
    let s: String = String { buf: out, len: libc_strlen(out), cap: 128 };
    return s;
}

// 取得底层 C 字符串指针（用于打印等）。
fn string_as_ptr(s: String) -> *char {
    return s.buf;
}
