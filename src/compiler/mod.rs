mod abi;
mod backend;
mod codegen;
mod globals;
mod types;
mod values;

use std::collections::HashMap;

use cranelift_module::Module;

use crate::hir::{DefId, HirType};
use cranelift_module::DataId;

/// 函数元信息（以 [`DefId`] 为键存储）。
#[derive(Clone, Debug)]
pub struct FuncInfo {
    pub func_id: cranelift_module::FuncId,
    pub name: String,
    pub param_types: Vec<HirType>,
    pub ret_ty: HirType,
    pub is_variadic: bool,
}

/// 结构体某一字段在内存布局中的信息（以字段 [`DefId`] 标识）。
#[derive(Clone, Debug)]
pub struct StructFieldLayout {
    pub field_def_id: DefId,
    pub field_type: HirType,
    pub offset: u32,
}

/// 枚举某一变体在内存布局中的信息。
#[derive(Clone, Debug)]
pub struct VariantLayout {
    pub name: String,
    pub tag: u32,
    /// 载荷部分在枚举存储中的起始偏移（tag 之后）。
    pub payload_offset: u32,
    /// 载荷大小（含填充）。
    pub payload_size: u32,
    /// 载荷字段的布局（与 struct 类似）。
    pub fields: Vec<StructFieldLayout>,
}

/// 枚举的完整内存布局。
#[derive(Clone, Debug)]
pub struct EnumLayout {
    /// 所有变体的布局（按 tag 序号索引）。
    pub variants: Vec<VariantLayout>,
    /// 枚举整体大小（tag(8) + 最大载荷 + 对齐填充）。
    pub size: u32,
    /// 枚举整体对齐（max(tag_align, max_payload_align)）。
    pub align: u32,
    /// 最大载荷对齐。
    pub max_payload_align: u32,
}

/// 编译器后端（Cranelift）的公共状态。
///
/// 所有「名字 → 后端实体」的映射都以 [`DefId`] 为键，
/// 与 TYPE HIR 中携带的 [`DefId`] 直接对应。
pub struct Compiler<T: Module> {
    pub module: T,
    pub string_pool: HashMap<String, cranelift_module::DataId>,
    pub str_counter: usize,
    /// 函数：定义 ID → 元信息（含 Cranelift `FuncId`）。
    pub func_map: HashMap<DefId, FuncInfo>,
    /// 全局变量 / 常量：定义 ID → （数据 ID，类型）。
    pub data_map: HashMap<DefId, (cranelift_module::DataId, HirType)>,
    /// 结构体布局：结构体定义 ID → 各字段偏移与类型。
    pub struct_map: HashMap<DefId, Vec<StructFieldLayout>>,
    /// 枚举布局：枚举定义 ID → 布局（含变体 tag/载荷偏移与整体大小/对齐）。
    pub enum_map: HashMap<DefId, EnumLayout>,
    /// vtable 表：(trait_def, impl_type_def) → vtable 数据 ID。
    pub vtable_map: HashMap<(DefId, DefId), DataId>,
}
