use crate::compiler::Compiler;
use crate::hir::HirType;

use cranelift_codegen::ir::AbiParam;
use cranelift_module::Module;

impl<T: Module> Compiler<T> {
    pub fn build_signature(
        &self,
        param_types: &[HirType],
        ret_ty: &HirType,
        extra_variadic_args: Option<&[HirType]>,
    ) -> cranelift_codegen::ir::Signature {
        let mut sig = self.module.make_signature();
        sig.call_conv = default_call_conv();

        if matches!(
            ret_ty,
            HirType::Named(_)
                | HirType::Array(_, _)
                | HirType::Enum(_, _, _)
                | HirType::TraitObject(_)
        ) {
            sig.params.push(AbiParam::new(self.ptr_type()));
        }

        for ty in param_types {
            sig.params
                .push(AbiParam::new(self.var_type_to_cranelift(ty)));
        }

        if let Some(extra) = extra_variadic_args {
            for ty in extra {
                sig.params
                    .push(AbiParam::new(self.var_type_to_cranelift(ty)));
            }
        }

        if !matches!(
            ret_ty,
            HirType::Named(_)
                | HirType::Array(_, _)
                | HirType::Enum(_, _, _)
                | HirType::TraitObject(_)
        ) {
            sig.returns
                .push(AbiParam::new(self.var_type_to_cranelift(ret_ty)));
        }

        sig
    }

    pub fn needs_sret(ret_ty: &HirType) -> bool {
        matches!(
            ret_ty,
            HirType::Named(_)
                | HirType::Array(_, _)
                | HirType::Enum(_, _, _)
                | HirType::TraitObject(_)
        )
    }
}

fn default_call_conv() -> cranelift_codegen::isa::CallConv {
    #[cfg(target_os = "windows")]
    {
        cranelift_codegen::isa::CallConv::WindowsFastcall
    }
    #[cfg(not(target_os = "windows"))]
    {
        cranelift_codegen::isa::CallConv::SystemV
    }
}
