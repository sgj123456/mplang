enum Option<T> { None, Some(value: T) }

fn main() -> int {
    let x: Option<int> = Some(42);
    let v: int = match(x) {
        None => { 0; },
        Some(v) => { v; },
    };
    return v;
}
