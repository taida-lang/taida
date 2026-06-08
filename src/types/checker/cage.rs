//! cage — methods split out of the TypeChecker impl.
//! Pure move from the parent module; behaviour unchanged.

use crate::lexer::Span;
use crate::parser::*;
use crate::types::Type;

use super::{BranchInfo, CageBranch, TypeChecker, TypeError};

impl TypeChecker {
    pub(super) fn branch_from_type_arg(&self, expr: &Expr) -> Option<CageBranch> {
        match expr {
            Expr::Ident(name, _) | Expr::TypeLiteral(name, None, _) => CageBranch::from_name(name),
            _ => None,
        }
    }

    fn is_js_rilla_constructor(name: &str) -> bool {
        matches!(
            name,
            "JSGet" | "JSCall" | "JSCallAsync" | "JSNew" | "JSSet" | "JSBind" | "JSSpread"
        )
    }

    pub(super) fn is_cage_runner_constructor(name: &str) -> bool {
        Self::is_js_rilla_constructor(name) || name == "HostCall"
    }

    pub(super) fn js_rilla_constructor_signature(name: &str) -> Option<(usize, &'static str)> {
        match name {
            "JSGet" => Some((2, "JSGet[path, Out]()")),
            "JSCall" => Some((3, "JSCall[path, args, Out]()")),
            "JSCallAsync" => Some((3, "JSCallAsync[path, args, Out]()")),
            "JSNew" => Some((3, "JSNew[path, args, Out]()")),
            "JSSet" => Some((2, "JSSet[path, value]()")),
            "JSBind" => Some((1, "JSBind[path]()")),
            "JSSpread" => Some((1, "JSSpread[source]()")),
            "HostCall" => Some((2, "HostCall[steps, Out]()")),
            _ => None,
        }
    }

    pub(super) fn is_cage_rilla_child(name: &str) -> bool {
        matches!(name, "JSRilla" | "FileRilla" | "BuildRilla")
    }

    pub(super) fn is_hammer_cage_boundary_expr(expr: &Expr) -> bool {
        matches!(expr, Expr::MoldInst(name, _, _, _) if name == "JSON" || name == "JSONRilla")
    }

    pub(super) fn molten_branch_for_expr(&self, expr: &Expr) -> Option<CageBranch> {
        match expr {
            Expr::Ident(name, _) => self.lookup_molten_branch(name),
            Expr::Unmold(inner, _) => self.gorillax_value_branch_for_expr(inner),
            _ => None,
        }
    }

    pub(super) fn gorillax_value_branch_for_expr(&self, expr: &Expr) -> Option<CageBranch> {
        match expr {
            Expr::Ident(name, _) => self.lookup_gorillax_value_branch(name),
            Expr::MoldInst(name, type_args, _, _) if name == "Cage" => type_args
                .get(1)
                .and_then(|runner| self.cage_runner_type(runner))
                .and_then(|runner| {
                    if runner.output == Type::Molten {
                        Some(runner.branch)
                    } else {
                        None
                    }
                }),
            _ => None,
        }
    }

    pub(super) fn branch_info_for_assignment_expr(
        &self,
        expr: &Expr,
        inferred: &Type,
    ) -> BranchInfo {
        match inferred {
            Type::Molten => self
                .molten_branch_for_expr(expr)
                .map(BranchInfo::Molten)
                .unwrap_or(BranchInfo::None),
            Type::Generic(name, args)
                if name == "Gorillax" && args.first().is_some_and(|arg| *arg == Type::Molten) =>
            {
                self.gorillax_value_branch_for_expr(expr)
                    .map(BranchInfo::GorillaxValue)
                    .unwrap_or(BranchInfo::None)
            }
            _ => BranchInfo::None,
        }
    }

    pub(super) fn push_cage_error(&mut self, code: &str, span: &Span, message: String) {
        if self
            .errors
            .iter()
            .any(|err| err.span == *span && err.message.starts_with(code))
        {
            return;
        }
        self.errors.push(TypeError {
            message,
            span: span.clone(),
        });
    }
}
