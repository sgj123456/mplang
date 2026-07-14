use std::collections::HashMap;

use cranelift_codegen::settings::{self, Configurable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::Module;
use cranelift_object::{ObjectBuilder, ObjectModule};
use target_lexicon::Triple;

use crate::{
    compiler::{Compiler, FuncInfo},
    tyhir::TyHirCompilationUnit,
};

impl<T: Module> Compiler<T> {
    /// 用已构造好的 `module` 与全新的公共状态装配出一个编译器实例，
    /// 供 `ObjectModule` / `JITModule` 两个后端的 `new()` 共用，避免重复字段初始化。
    pub(crate) fn with_module(module: T) -> Self {
        Self {
            module,
            string_pool: HashMap::new(),
            str_counter: 0,
            func_map: HashMap::new(),
            data_map: HashMap::new(),
            struct_map: HashMap::new(),
            enum_map: HashMap::new(),
            vtable_map: HashMap::new(),
        }
    }
}

impl Default for Compiler<ObjectModule> {
    fn default() -> Self {
        Self::new()
    }
}

impl Compiler<ObjectModule> {
    pub fn new() -> Self {
        let triple = Triple::host();
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed").unwrap();
        flag_builder.set("is_pic", "true").unwrap();
        let flags = settings::Flags::new(flag_builder);

        let isa = cranelift_codegen::isa::lookup(triple.clone())
            .expect("failed to lookup ISA for host triple")
            .finish(flags)
            .expect("failed to construct TargetIsa");

        let builder = ObjectBuilder::new(
            isa,
            triple.to_string(),
            cranelift_module::default_libcall_names(),
        )
        .expect("failed to create object builder");

        let module = ObjectModule::new(builder);
        Self::with_module(module)
    }

    pub fn compile(mut self, prog: &TyHirCompilationUnit) -> Vec<u8> {
        self.translate(prog);
        let product = self.module.finish();
        product.emit().expect("failed to emit object file")
    }
}

impl Default for Compiler<JITModule> {
    fn default() -> Self {
        Self::new()
    }
}

impl Compiler<JITModule> {
    pub fn new() -> Self {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed").unwrap();
        flag_builder.set("is_pic", "true").unwrap();

        let builder = JITBuilder::new(cranelift_module::default_libcall_names())
            .expect("failed to create JIT builder");

        let module = JITModule::new(builder);
        Self::with_module(module)
    }

    pub fn run(mut self, prog: &TyHirCompilationUnit) -> i32 {
        self.translate(prog);
        self.module.finalize_definitions().unwrap();

        let FuncInfo {
            func_id,
            param_types,
            ret_ty,
            is_variadic,
            ..
        } = self
            .func_map
            .values()
            .find(|f| f.name == "main")
            .cloned()
            .expect("JIT error: no 'main' function defined in source");

        if ret_ty != crate::hir::HirType::Int || !param_types.is_empty() || is_variadic {
            panic!(
                "JIT error: 'main' must have signature fn() -> int, got fn({:?}) -> {:?}",
                param_types, ret_ty
            );
        }

        assert_eq!(
            self.ptr_type().bytes(),
            8,
            "JIT error: only 64-bit platforms are supported for JIT execution"
        );

        let ptr = self.module.get_finalized_function(func_id);
        let main_fn: unsafe extern "C" fn() -> i64 = unsafe { std::mem::transmute(ptr) };
        let exit_code = unsafe { main_fn() };

        exit_code as i32
    }
}
