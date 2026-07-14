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
            HirType::Enum(_, _, _) | HirType::TraitObject(_) => self.ptr_type(),
            HirType::Variant(_, _) => {
                unreachable!("Variant 应在 codegen 前被压平为存储写入")
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
            HirType::Enum(def, _, _) => self.enum_size(def),
            HirType::TraitObject(_) => 2 * self.ptr_type().bytes(),
            HirType::Variant(_, _) => {
                unreachable!("Variant 应在 codegen 前被压平")
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
            HirType::Enum(def, _, _) => self.enum_align(def),
            HirType::TraitObject(_) => self.ptr_type().bytes(), // 16B fat pointer, ptr-align
            HirType::Variant(_, _) => {
                unreachable!("Variant 应在 codegen 前被压平")
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

    pub fn enum_size(&self, enum_def: &DefId) -> u32 {
        let layout = self.enum_map.get(enum_def).expect("unknown enum");
        layout.size
    }

    pub fn enum_align(&self, enum_def: &DefId) -> u32 {
        let layout = self.enum_map.get(enum_def).expect("unknown enum");
        layout.align
    }

    pub fn variant_payload_offset(&self, enum_def: &DefId, tag: u32) -> u32 {
        let layout = self.enum_map.get(enum_def).expect("unknown enum");
        layout
            .variants
            .get(tag as usize)
            .map(|v| v.payload_offset)
            .unwrap_or_else(|| panic!("variant tag {} not found in enum {:?}", tag, enum_def))
    }

    pub fn variant_field_offset(&self, enum_def: &DefId, tag: u32, field_def: &DefId) -> u32 {
        let layout = self.enum_map.get(enum_def).expect("unknown enum");
        let variant = &layout.variants[tag as usize];
        variant
            .fields
            .iter()
            .find(|f| &f.field_def_id == field_def)
            .map(|f| variant.payload_offset + f.offset)
            .unwrap_or_else(|| {
                panic!(
                    "field {:?} not found in variant {} of enum {:?}",
                    field_def, tag, enum_def
                )
            })
    }

    /// 枚举的 tag（判别式）作为 i64 写入 offset 0。
    pub fn enum_tag_offset() -> u32 {
        0
    }

    /// 取类型大小；对于 `Var`/`Generic`（泛型占位符）回退为 8 字节。
    /// 枚举布局阶段使用——因为枚举的变体字段可能仍含泛型参数（monomorphize 未替换时）。
    pub fn var_type_size_or_default(&self, ty: &HirType) -> u32 {
        match ty {
            HirType::Var(_) | HirType::Generic(_, _, _) => 8,
            _ => self.var_type_size(ty),
        }
    }

    /// 取类型对齐；对于 `Var`/`Generic` 回退为 8 字节对齐。
    pub fn var_type_align_or_default(&self, ty: &HirType) -> u32 {
        match ty {
            HirType::Var(_) | HirType::Generic(_, _, _) => 8,
            _ => self.var_type_align(ty),
        }
    }
}
