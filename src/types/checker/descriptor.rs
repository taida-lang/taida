//! descriptor — methods split out of the TypeChecker impl.
//! Pure move from the parent module; behaviour unchanged.

use crate::lexer::Span;
use crate::parser::*;

use super::{BUILD_DESCRIPTOR_NAMES, DescriptorUseCtx, TypeChecker, TypeError};

impl TypeChecker {
    pub(super) fn is_descriptor_type_name(&self, name: &str) -> bool {
        BUILD_DESCRIPTOR_NAMES.contains(&name) && !self.descriptor_shadow_names.contains(name)
    }

    /// If `expr` evaluates to a build descriptor, return its descriptor type
    /// name. Recognises a direct `Name(...)` constructor and a top-level
    /// binding name previously bound to a descriptor (the only allow-listed
    /// indirection). Anything wrapped further (in a pack, list, call, ...)
    /// is intentionally *not* unwrapped here — that wrapping is itself the
    /// runtime use we want to flag at the wrapper.
    fn descriptor_value_name(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::TypeInst(name, _, _) if self.is_descriptor_type_name(name) => Some(name.clone()),
            Expr::Ident(name, _)
                if self.descriptor_binding_names.contains(name)
                    && !self.descriptor_scope_shadows.contains(name) =>
            {
                Some(name.clone())
            }
            _ => None,
        }
    }

    fn push_descriptor_use_error(&mut self, descriptor_name: &str, span: &Span) {
        self.errors.push(TypeError {
            message: format!(
                "[E1532] '{}' is a build-driver descriptor, not a runtime value, and cannot be \
                 used in this position. Build descriptors (BuildUnit / BuildPlan / AssetBundle / \
                 RouteAsset / BuildHook) are consumed only by `taida build --unit` / `--plan` / \
                 `--all-units`. Hint: bind a descriptor to a name and export it with `<<<`, or nest \
                 it inside another descriptor's field (e.g. a `RouteAsset` inside `BuildUnit.assets`). \
                 See docs/api/build_descriptors.md and docs/reference/diagnostic_codes.md [E1532].",
                descriptor_name
            ),
            span: span.clone(),
        });
    }

    pub(super) fn check_descriptor_runtime_use(&mut self, program: &Program) {
        // Pre-pass 1: a user-declared class-like / mold type that shadows a
        // reserved descriptor name resolves to the user's type, so exclude it.
        self.descriptor_shadow_names.clear();
        for stmt in &program.statements {
            if let Statement::ClassLikeDef(cl) = stmt
                && BUILD_DESCRIPTOR_NAMES.contains(&cl.name.as_str())
            {
                self.descriptor_shadow_names.insert(cl.name.clone());
            }
        }

        // Pre-pass 2: collect top-level bindings whose RHS is a descriptor so
        // `name <= BuildUnit(...)` then `<<< name` is recognised. Only the
        // outermost descriptor binding form is tracked (the allow-listed
        // indirection); the RHS itself is still validated as a binding context.
        self.descriptor_binding_names.clear();
        self.descriptor_scope_shadows.clear();
        for stmt in &program.statements {
            if let Statement::Assignment(assign) = stmt
                && self.descriptor_value_name(&assign.value).is_some()
            {
                self.descriptor_binding_names.insert(assign.target.clone());
            }
        }

        for stmt in &program.statements {
            self.check_descriptor_use_in_stmt(stmt, true);
        }
    }

    fn check_descriptor_use_in_stmt(&mut self, stmt: &Statement, top_level: bool) {
        match stmt {
            // A *top-level* binding RHS is allow-listed when it is directly a
            // descriptor value (`name <= BuildUnit(...)`) — that is the form
            // that carries the descriptor toward an export. The descriptor's
            // own fields are then checked in `Allowed` context (nested
            // descriptors). Any other RHS (`name <= stdout(unit)`,
            // `name <= @(u <= BuildUnit(...))`) is a runtime computation, so it
            // is walked in `Runtime` and a descriptor wrapped inside is flagged.
            // Inside a function body / handler / branch a binding cannot reach
            // a top-level export, so its RHS is always `Runtime`.
            Statement::Assignment(assign) => {
                let rhs_ctx = if top_level && self.descriptor_value_name(&assign.value).is_some() {
                    DescriptorUseCtx::Allowed
                } else {
                    DescriptorUseCtx::Runtime
                };
                self.check_descriptor_use_in_expr(&assign.value, rhs_ctx);
                if !top_level {
                    // A nested-scope binding shadows a same-named top-level
                    // descriptor binding for the rest of the enclosing scope
                    // (the RHS above still sees the outer name).
                    self.descriptor_scope_shadows.insert(assign.target.clone());
                }
            }
            Statement::Expr(e) => {
                self.check_descriptor_use_in_expr(e, DescriptorUseCtx::Runtime);
            }
            Statement::FuncDef(fd) => {
                // A descriptor returned / used inside a function body is a
                // runtime use (functions are not the descriptor build path).
                // Parameters (and the nested function's own name) shadow
                // same-named top-level descriptor bindings within the body.
                if !top_level {
                    self.descriptor_scope_shadows.insert(fd.name.clone());
                }
                let saved = self.descriptor_scope_shadows.clone();
                for p in &fd.params {
                    self.descriptor_scope_shadows.insert(p.name.clone());
                }
                for s in &fd.body {
                    self.check_descriptor_use_in_stmt(s, false);
                }
                self.descriptor_scope_shadows = saved;
            }
            Statement::ErrorCeiling(ec) => {
                let saved = self.descriptor_scope_shadows.clone();
                self.descriptor_scope_shadows.insert(ec.error_param.clone());
                for s in &ec.handler_body {
                    self.check_descriptor_use_in_stmt(s, false);
                }
                self.descriptor_scope_shadows = saved;
            }
            Statement::UnmoldForward(u) => {
                self.check_descriptor_use_in_expr(&u.source, DescriptorUseCtx::Runtime);
                if !top_level {
                    self.descriptor_scope_shadows.insert(u.target.clone());
                }
            }
            Statement::UnmoldBackward(u) => {
                self.check_descriptor_use_in_expr(&u.source, DescriptorUseCtx::Runtime);
                if !top_level {
                    self.descriptor_scope_shadows.insert(u.target.clone());
                }
            }
            // Exports name symbols (`<<< @(name)`); the bound value was
            // validated at its binding site as the allow-listed RHS. Class /
            // enum / import statements carry no descriptor value positions.
            _ => {}
        }
    }

    fn check_descriptor_use_in_expr(&mut self, expr: &Expr, ctx: DescriptorUseCtx) {
        // Flag a descriptor value sitting in a runtime position before
        // recursing — the wrapper position is where the misuse lives.
        if ctx == DescriptorUseCtx::Runtime
            && let Some(descriptor_name) = self.descriptor_value_name(expr)
        {
            self.push_descriptor_use_error(&descriptor_name, expr.span());
            // A bare descriptor `Ident` has no children worth recursing into;
            // for a `TypeInst` we still recurse below so a misuse nested in a
            // field value is also reported.
        }

        match expr {
            // Descriptor constructor: its own field values are the
            // allow-listed nested-descriptor slots, so they are checked in
            // `Allowed` context (a `RouteAsset` inside `BuildUnit.assets` is
            // valid). Non-descriptor named constructors get `Runtime` fields.
            Expr::TypeInst(name, fields, _) => {
                let field_ctx = if self.is_descriptor_type_name(name) {
                    DescriptorUseCtx::Allowed
                } else {
                    DescriptorUseCtx::Runtime
                };
                for f in fields {
                    self.check_descriptor_use_in_expr(&f.value, field_ctx);
                }
            }
            // Anonymous packs and lists pass their context through: a list
            // that is itself a descriptor field (`assets <= @[RouteAsset(...)]`)
            // keeps the descriptor field allowance for its elements, while a
            // runtime list rejects descriptor elements.
            Expr::BuchiPack(fields, _) => {
                for f in fields {
                    self.check_descriptor_use_in_expr(&f.value, ctx);
                }
            }
            Expr::ListLit(items, _) => {
                for item in items {
                    self.check_descriptor_use_in_expr(item, ctx);
                }
            }
            // Every remaining compound position is a runtime computation:
            // descend with `Runtime` so a descriptor anywhere inside is flagged.
            Expr::FuncCall(callee, args, _) => {
                self.check_descriptor_use_in_expr(callee, DescriptorUseCtx::Runtime);
                for arg in args {
                    self.check_descriptor_use_in_expr(arg, DescriptorUseCtx::Runtime);
                }
            }
            Expr::MethodCall(obj, _, args, _) => {
                self.check_descriptor_use_in_expr(obj, DescriptorUseCtx::Runtime);
                for arg in args {
                    self.check_descriptor_use_in_expr(arg, DescriptorUseCtx::Runtime);
                }
            }
            Expr::MoldInst(_, type_args, fields, _) => {
                for arg in type_args {
                    self.check_descriptor_use_in_expr(arg, DescriptorUseCtx::Runtime);
                }
                for f in fields {
                    self.check_descriptor_use_in_expr(&f.value, DescriptorUseCtx::Runtime);
                }
            }
            Expr::BinaryOp(l, _, r, _) => {
                self.check_descriptor_use_in_expr(l, DescriptorUseCtx::Runtime);
                self.check_descriptor_use_in_expr(r, DescriptorUseCtx::Runtime);
            }
            Expr::UnaryOp(_, inner, _) => {
                self.check_descriptor_use_in_expr(inner, DescriptorUseCtx::Runtime);
            }
            Expr::FieldAccess(obj, _, _) => {
                self.check_descriptor_use_in_expr(obj, DescriptorUseCtx::Runtime);
            }
            Expr::Unmold(inner, _) => {
                self.check_descriptor_use_in_expr(inner, DescriptorUseCtx::Runtime);
            }
            Expr::Throw(inner, _) => {
                self.check_descriptor_use_in_expr(inner, DescriptorUseCtx::Runtime);
            }
            Expr::Lambda(params, body, _) => {
                // Lambda parameters shadow same-named top-level descriptor
                // bindings within the lambda body.
                let saved = self.descriptor_scope_shadows.clone();
                for p in params {
                    self.descriptor_scope_shadows.insert(p.name.clone());
                }
                self.check_descriptor_use_in_expr(body, DescriptorUseCtx::Runtime);
                self.descriptor_scope_shadows = saved;
            }
            Expr::Pipeline(exprs, _) => {
                for e in exprs {
                    self.check_descriptor_use_in_expr(e, DescriptorUseCtx::Runtime);
                }
            }
            Expr::CondBranch(arms, _) => {
                // A descriptor selected by a runtime branch is a runtime use:
                // arm bodies are never an allow-listed descriptor position.
                for arm in arms {
                    if let Some(cond) = &arm.condition {
                        self.check_descriptor_use_in_expr(cond, DescriptorUseCtx::Runtime);
                    }
                    // Bindings inside one arm do not shadow names in the
                    // next arm — restore the shadow set per arm.
                    let saved = self.descriptor_scope_shadows.clone();
                    for s in &arm.body {
                        self.check_descriptor_use_in_stmt(s, false);
                    }
                    self.descriptor_scope_shadows = saved;
                }
            }
            // Leaf expressions (handled above for the descriptor case).
            _ => {}
        }
    }
}
