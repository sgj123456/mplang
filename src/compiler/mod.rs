mod abi;
mod backend;
mod codegen;
mod globals;
mod types;
mod values;

use std::collections::HashMap;

use cranelift_module::Module;

use crate::hir::{DefId, HirType};

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
}
