use std::collections::HashSet;

use crate::hir::DefId;
use crate::tyhir;

/// 收集函数体中被 `&` 取过地址的所有变量 `DefId`（含参数）。
/// 这些变量在 codegen 阶段需要被分配栈槽而非纯 SSA 寄存器。
pub(crate) fn collect_addr_taken(stmts: &[tyhir::TyHirStmt], set: &mut HashSet<DefId>) {
    for s in stmts {
        match s {
            tyhir::TyHirStmt::Let { init, .. } => collect_addr_taken_expr(init, set),
            tyhir::TyHirStmt::Assign { target, value } => {
                collect_addr_taken_expr(target, set);
                collect_addr_taken_expr(value, set);
            }
            tyhir::TyHirStmt::Expr(e) => collect_addr_taken_expr(e, set),
            tyhir::TyHirStmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                collect_addr_taken_expr(cond, set);
                collect_addr_taken(&then_branch.stmts, set);
                if let Some(eb) = else_branch {
                    collect_addr_taken(&eb.stmts, set);
                }
            }
            tyhir::TyHirStmt::While { cond, body } => {
                collect_addr_taken_expr(cond, set);
                collect_addr_taken(&body.stmts, set);
            }
            tyhir::TyHirStmt::Return(e) => {
                if let Some(e) = e {
                    collect_addr_taken_expr(e, set);
                }
            }
        }
    }
}

fn collect_addr_taken_expr(e: &tyhir::TyHirExpr, set: &mut HashSet<DefId>) {
    match &e.kind {
        tyhir::TyHirExprKind::AddressOf(inner) => {
            if let Some(d) = lvalue_root_defid(inner) {
                set.insert(d);
            }
            collect_addr_taken_expr(inner, set);
        }
        tyhir::TyHirExprKind::Deref(inner) => collect_addr_taken_expr(inner, set),
        tyhir::TyHirExprKind::Binary { lhs, rhs, .. } => {
            collect_addr_taken_expr(lhs, set);
            collect_addr_taken_expr(rhs, set);
        }
        tyhir::TyHirExprKind::Call { args, .. } => {
            for a in args {
                collect_addr_taken_expr(a, set);
            }
        }
        tyhir::TyHirExprKind::FieldAccess { object, .. } => collect_addr_taken_expr(object, set),
        tyhir::TyHirExprKind::Index { array, index } => {
            collect_addr_taken_expr(array, set);
            collect_addr_taken_expr(index, set);
        }
        tyhir::TyHirExprKind::StructLiteral { fields, .. } => {
            for (_, f) in fields {
                collect_addr_taken_expr(f, set);
            }
        }
        tyhir::TyHirExprKind::ArrayLiteral { elements, .. } => {
            for e in elements {
                collect_addr_taken_expr(e, set);
            }
        }
        tyhir::TyHirExprKind::Path(_) | tyhir::TyHirExprKind::Literal(_) => {}
    }
}

/// 取左值表达式“根”变量 `DefId`（`&x`→x，`&s.x`→s，`&(*p)`→p）。
fn lvalue_root_defid(e: &tyhir::TyHirExpr) -> Option<DefId> {
    match &e.kind {
        tyhir::TyHirExprKind::Path(d) => Some(*d),
        tyhir::TyHirExprKind::FieldAccess { object, .. } => lvalue_root_defid(object),
        tyhir::TyHirExprKind::Index { array, .. } => lvalue_root_defid(array),
        tyhir::TyHirExprKind::AddressOf(inner) | tyhir::TyHirExprKind::Deref(inner) => {
            lvalue_root_defid(inner)
        }
        _ => None,
    }
}
