/// 符号路径：以分节（segments）形式表示一个名字在嵌套模块中的完整路径。
///
/// 仅用于前端的名字解析（`lowering`），与编译后端的模块概念无关。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SymbolPath {
    pub segments: Vec<String>,
}

impl Default for SymbolPath {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolPath {
    pub fn new() -> SymbolPath {
        SymbolPath {
            segments: Vec::new(),
        }
    }
}
