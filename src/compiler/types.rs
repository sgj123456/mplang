use super::Compiler;
use crate::hir::{DefId, HirType};
use cranelift::prelude::*;
use cranelift_module::Module;

impl<T: Module> Compiler<T> {
    pub fn ptr_type(&self) -> types::Type {
        self.module.isa().pointer_type()
    }

    /// 把语言层的 [`HirType`] 映射到 Cranelift 标量类型。
    pub fn var_type_to_cranelift(&self, ty: &HirType) -> types::Type {
        match ty {
            HirType::Int => types::I64,
            HirType::Unit => types::I64,
            // `char` 映射为 i8（有符号 8 位）。
            HirType::Char => types::I8,
            // 所有指针（含 `char*`）都是机器指针宽度。
            HirType::Pointer(_) => self.ptr_type(),
            // 定长数组作为「指向连续存储的指针值」参与运算（与结构体 Named 同约定）。
            HirType::Array(_, _) => self.ptr_type(),
            HirType::Named(struct_def) => {
                assert!(
                    self.struct_map.contains_key(struct_def),
                    "unknown named type: {:?}",
                    struct_def
                );
                self.ptr_type()
            }
            // 以下变体只存在于泛型「模板」中，单态化后不会出现在代码生成阶段。
            HirType::Var(_) | HirType::Generic(_, _, _) => {
                unreachable!("单态化后不应出现泛型类型占位符：{:?}", ty)
            }
        }
    }

    pub fn var_type_size(&self, ty: &HirType) -> u32 {
        match ty {
            HirType::Int | HirType::Unit => 8,
            HirType::Char => 1,
            HirType::Pointer(_) => self.ptr_type().bytes(),
            HirType::Array(elem, len) => {
                // 元素步长即其大小（已按自身对齐向上取整），整体大小为步长 × 长度。
                let stride = self.var_type_size(elem);
                stride * (len.known() as u32)
            }
            HirType::Named(struct_def) => self.struct_size(struct_def),
            HirType::Var(_) | HirType::Generic(_, _, _) => {
                unreachable!("单态化后不应出现泛型类型占位符")
            }
        }
    }

    pub fn var_type_align(&self, ty: &HirType) -> u32 {
        match ty {
            HirType::Int | HirType::Unit => 8,
            HirType::Char => 1,
            HirType::Pointer(_) => self.ptr_type().bytes(),
            HirType::Array(elem, _) => self.var_type_align(elem),
            HirType::Named(struct_def) => {
                let layout = self.struct_map.get(struct_def).expect("unknown struct");
                layout
                    .iter()
                    .map(|f| self.var_type_align(&f.field_type))
                    .max()
                    .unwrap_or(1)
            }
            HirType::Var(_) | HirType::Generic(_, _, _) => {
                unreachable!("单态化后不应出现泛型类型占位符")
            }
        }
    }

    pub fn struct_size(&self, struct_def: &DefId) -> u32 {
        let layout = self.struct_map.get(struct_def).expect("unknown struct");
        if layout.is_empty() {
            return 0;
        }
        let last = layout.last().unwrap();
        let raw = last.offset + self.var_type_size(&last.field_type);
        // 整体大小需要按「结构体自身对齐」向上取整，而非最后字段的对齐：
        // 否则形如 `struct S { a:int; b:char; }` 的结构会被低估，在数组 / 嵌套场景
        // 下得到错误的步长，造成成员重叠。
        let align = self.var_type_align(&HirType::Named(*struct_def));
        (raw + align - 1) & !(align - 1)
    }

    pub fn field_offset(&self, struct_def: &DefId, field_def: &DefId) -> u32 {
        self.struct_map
            .get(struct_def)
            .and_then(|layout| layout.iter().find(|f| &f.field_def_id == field_def))
            .map(|f| f.offset)
            .unwrap_or_else(|| panic!("field {:?} not found in struct {:?}", field_def, struct_def))
    }

    pub fn field_type(&self, struct_def: &DefId, field_def: &DefId) -> HirType {
        self.struct_map
            .get(struct_def)
            .and_then(|layout| layout.iter().find(|f| &f.field_def_id == field_def))
            .map(|f| f.field_type.clone())
            .unwrap_or_else(|| panic!("struct {:?} has no field {:?}", struct_def, field_def))
    }

    pub fn assert_ty(&self, actual: &HirType, expected: &HirType, ctx: &str) {
        assert_eq!(
            actual, expected,
            "type mismatch in {}: expected {:?}, got {:?}",
            ctx, expected, actual
        );
    }
}
