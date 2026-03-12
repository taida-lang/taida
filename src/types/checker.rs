use super::types::{Type, TypeRegistry};
use crate::lexer::Span;
use crate::parser::*;
/// Type checker for Taida Lang.
///
/// Performs type inference and type checking on the AST.
/// Key principles:
/// - No null/undefined (all types have default values)
/// - No implicit type conversion
/// - Structural subtyping (width subtyping)
/// - Scope-aware type inference
use std::collections::{HashMap, HashSet};

/// Type checking error.
#[derive(Debug, Clone)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Type error at line {}, column {}: {}",
            self.span.line, self.span.column, self.message
        )
    }
}

/// Type checker state.
pub struct TypeChecker {
    pub registry: TypeRegistry,
    pub errors: Vec<TypeError>,
    /// Scope stack for variable type tracking.
    /// Each scope maps variable names to their inferred types.
    scope_stack: Vec<HashMap<String, Type>>,
    /// Function return types (name -> return type).
    func_types: HashMap<String, Type>,
    /// Function parameter counts (name -> arity upper bound).
    func_param_counts: HashMap<String, usize>,
    /// Function parameter types (name -> param types). Used for partial application type inference.
    func_param_types: HashMap<String, Vec<Type>>,
    /// Custom mold field definitions (name -> raw AST fields).
    /// Used for `[]` / `()` binding validation.
    mold_field_defs: HashMap<String, Vec<FieldDef>>,
    /// Custom mold header declarations (name -> raw header args from `Mold[...]`).
    mold_header_args: HashMap<String, Vec<MoldHeaderArg>>,
    /// Whether we are currently inside a pipeline expression.
    /// Used to allow `_` (Placeholder) in pipeline context while rejecting it elsewhere.
    in_pipeline: bool,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            registry: TypeRegistry::new(),
            errors: Vec::new(),
            scope_stack: vec![HashMap::new()], // global scope
            func_types: HashMap::new(),
            func_param_counts: HashMap::new(),
            func_param_types: HashMap::new(),
            mold_field_defs: HashMap::new(),
            mold_header_args: HashMap::new(),
            in_pipeline: false,
        }
    }

    fn binding_diag(code: &str, message: String, hint: &str) -> String {
        format!("[{}] {} Hint: {}", code, message, hint)
    }

    fn find_forbidden_default_ref(expr: &Expr, forbidden: &HashSet<String>) -> Option<String> {
        match expr {
            Expr::Ident(name, _) => {
                if forbidden.contains(name) {
                    Some(name.clone())
                } else {
                    None
                }
            }
            Expr::IntLit(_, _)
            | Expr::FloatLit(_, _)
            | Expr::StringLit(_, _)
            | Expr::TemplateLit(_, _)
            | Expr::BoolLit(_, _)
            | Expr::Gorilla(_)
            | Expr::Placeholder(_)
            | Expr::Hole(_) => None,
            Expr::BuchiPack(fields, _) => fields
                .iter()
                .find_map(|field| Self::find_forbidden_default_ref(&field.value, forbidden)),
            Expr::ListLit(items, _) => items
                .iter()
                .find_map(|item| Self::find_forbidden_default_ref(item, forbidden)),
            Expr::BinaryOp(left, _, right, _) => Self::find_forbidden_default_ref(left, forbidden)
                .or_else(|| Self::find_forbidden_default_ref(right, forbidden)),
            Expr::UnaryOp(_, inner, _) => Self::find_forbidden_default_ref(inner, forbidden),
            Expr::FuncCall(callee, args, _) => Self::find_forbidden_default_ref(callee, forbidden)
                .or_else(|| {
                    args.iter()
                        .find_map(|arg| Self::find_forbidden_default_ref(arg, forbidden))
                }),
            Expr::MethodCall(obj, _, args, _) => Self::find_forbidden_default_ref(obj, forbidden)
                .or_else(|| {
                    args.iter()
                        .find_map(|arg| Self::find_forbidden_default_ref(arg, forbidden))
                }),
            Expr::FieldAccess(obj, _, _) => Self::find_forbidden_default_ref(obj, forbidden),
            Expr::CondBranch(arms, _) => arms.iter().find_map(|arm| {
                arm.condition
                    .as_ref()
                    .and_then(|cond| Self::find_forbidden_default_ref(cond, forbidden))
                    .or_else(|| {
                        arm.body.iter().find_map(|stmt| {
                            if let Statement::Expr(e) = stmt {
                                Self::find_forbidden_default_ref(e, forbidden)
                            } else {
                                None
                            }
                        })
                    })
            }),
            Expr::Pipeline(exprs, _) => exprs
                .iter()
                .find_map(|node| Self::find_forbidden_default_ref(node, forbidden)),
            Expr::MoldInst(_, type_args, fields, _) => type_args
                .iter()
                .find_map(|arg| Self::find_forbidden_default_ref(arg, forbidden))
                .or_else(|| {
                    fields
                        .iter()
                        .find_map(|field| Self::find_forbidden_default_ref(&field.value, forbidden))
                }),
            Expr::Unmold(inner, _) => Self::find_forbidden_default_ref(inner, forbidden),
            Expr::Lambda(params, body, _) => {
                let mut nested_forbidden = forbidden.clone();
                for param in params {
                    nested_forbidden.remove(&param.name);
                }
                Self::find_forbidden_default_ref(body, &nested_forbidden)
            }
            Expr::TypeInst(_, fields, _) => fields
                .iter()
                .find_map(|field| Self::find_forbidden_default_ref(&field.value, forbidden)),
            Expr::Throw(inner, _) => Self::find_forbidden_default_ref(inner, forbidden),
        }
    }

    /// Check if a type contains Unknown anywhere in its structure.
    fn contains_unknown(ty: &Type) -> bool {
        match ty {
            Type::Unknown => true,
            Type::List(inner) => Self::contains_unknown(inner),
            Type::Generic(_, args) => args.iter().any(Self::contains_unknown),
            Type::Function(params, ret) => {
                params.iter().any(Self::contains_unknown) || Self::contains_unknown(ret)
            }
            _ => false,
        }
    }

    /// Push a new scope (e.g., entering a function body).
    fn push_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    /// Pop a scope (e.g., leaving a function body).
    fn pop_scope(&mut self) {
        self.scope_stack.pop();
    }

    /// Define a variable in the current scope.
    /// If `span` is provided and the name already exists in the current scope,
    /// a compile error is reported (same-scope redefinition is forbidden).
    /// Shadowing across scopes (inner scope redefines outer) is allowed.
    fn define_var(&mut self, name: &str, ty: Type) {
        self.define_var_with_span(name, ty, None);
    }

    /// Define a variable with a span for duplicate detection.
    fn define_var_with_span(&mut self, name: &str, ty: Type, span: Option<&Span>) {
        if let Some(scope) = self.scope_stack.last_mut() {
            if let Some(span) = span
                && scope.contains_key(name)
            {
                self.errors.push(TypeError {
                        message: format!(
                            "[E1501] Name '{}' is already defined in this scope. \
                             Redefinition in the same scope is not allowed. \
                             Hint: Use a different name, or define it in an inner scope (shadowing is allowed).",
                            name
                        ),
                        span: span.clone(),
                    });
                return;
            }
            scope.insert(name.to_string(), ty);
        }
    }

    /// Look up a variable type from the scope stack (innermost first).
    pub fn lookup_var(&self, name: &str) -> Option<Type> {
        for scope in self.scope_stack.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }

    /// Get all variable names and types visible in the current scope (for LSP completion).
    pub fn all_visible_vars(&self) -> Vec<(String, Type)> {
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // Walk from innermost to outermost, skip duplicates
        for scope in self.scope_stack.iter().rev() {
            for (name, ty) in scope {
                if seen.insert(name.clone()) {
                    result.push((name.clone(), ty.clone()));
                }
            }
        }
        result
    }

    /// Get all registered function names and their return types (for LSP completion).
    pub fn all_functions(&self) -> Vec<(String, Type)> {
        self.func_types
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Unwrap a mold type to get its inner value type.
    /// Used for `]=>` and `<=[` unmold operations.
    fn unmold_type(&self, ty: &Type) -> Type {
        match ty {
            // JSON unmolds to dynamic type (needs schema)
            Type::Json => Type::Unknown,
            // Molten is opaque — cannot unmold directly
            Type::Molten => Type::Unknown,
            // Generic mold types: extract the first type argument
            Type::Generic(name, args) => {
                match name.as_str() {
                    "Lax" | "Result" | "Async" | "Gorillax" | "RelaxedGorillax" => {
                        args.first().cloned().unwrap_or(Type::Unknown)
                    }
                    // Stream[T] unmolds to @[T] (List)
                    "Stream" => {
                        let inner = args.first().cloned().unwrap_or(Type::Unknown);
                        Type::List(Box::new(inner))
                    }
                    _ => {
                        // Custom mold types registered in the registry:
                        // extract the first type argument (filling type T)
                        if self.registry.mold_defs.contains_key(name.as_str()) {
                            args.first().cloned().unwrap_or(Type::Unknown)
                        } else {
                            Type::Unknown
                        }
                    }
                }
            }
            // Named type that is a registered mold: unmold to Unknown
            // (type parameter not instantiated, so we can't determine T)
            Type::Named(name) => {
                if self.registry.mold_defs.contains_key(name.as_str()) {
                    Type::Unknown
                } else {
                    // Non-mold named types pass through
                    ty.clone()
                }
            }
            // Unknown stays unknown
            Type::Unknown => Type::Unknown,
            // Non-mold types pass through (runtime will handle)
            _ => ty.clone(),
        }
    }

    /// Check an entire program. Collects type definitions first,
    /// then checks all statements.
    pub fn check_program(&mut self, program: &Program) {
        // First pass: register all type definitions and function signatures
        for stmt in &program.statements {
            self.register_types(stmt);
        }

        // Second pass: type-check statements
        for stmt in &program.statements {
            self.check_statement(stmt);
        }
    }

    /// Register type definitions from a statement (first pass).
    fn register_types(&mut self, stmt: &Statement) {
        match stmt {
            Statement::TypeDef(td) => {
                // E1501: Check for TypeDef name collision with existing types, functions, or molds
                let has_collision = self.registry.type_defs.contains_key(&td.name)
                    || self.func_types.contains_key(&td.name)
                    || self.registry.mold_defs.contains_key(&td.name);
                if has_collision {
                    self.errors.push(TypeError {
                        message: format!(
                            "[E1501] Name '{}' is already defined in this scope. \
                             Redefinition in the same scope is not allowed. \
                             Hint: Use a different name, or define it in an inner scope (shadowing is allowed).",
                            td.name
                        ),
                        span: td.span.clone(),
                    });
                }
                self.validate_class_like_fields("TypeDef", &td.name, &td.fields);
                let fields: Vec<(String, Type)> = td
                    .fields
                    .iter()
                    .filter(|f| !f.is_method)
                    .map(|f| {
                        let ty = f
                            .type_annotation
                            .as_ref()
                            .map(|t| self.registry.resolve_type(t))
                            .unwrap_or(Type::Unknown);
                        (f.name.clone(), ty)
                    })
                    .collect();
                self.registry.register_type(&td.name, fields);
            }
            Statement::MoldDef(md) => {
                self.validate_class_like_fields("MoldDef", &md.name, &md.fields);
                self.validate_mold_header_consistency(md);
                self.validate_mold_type_param_bindings(md);
                let type_params: Vec<String> =
                    md.type_params.iter().map(|tp| tp.name.clone()).collect();
                let fields: Vec<(String, Type)> = md
                    .fields
                    .iter()
                    .filter(|f| !f.is_method)
                    .map(|f| {
                        let ty = f
                            .type_annotation
                            .as_ref()
                            .map(|t| self.registry.resolve_type(t))
                            .unwrap_or(Type::Unknown);
                        (f.name.clone(), ty)
                    })
                    .collect();
                self.registry.register_mold(&md.name, type_params, fields);
                self.mold_header_args
                    .insert(md.name.clone(), md.mold_args.clone());
                self.mold_field_defs
                    .insert(md.name.clone(), md.fields.clone());
            }
            Statement::InheritanceDef(inh) => {
                self.validate_class_like_fields("InheritanceDef", &inh.child, &inh.fields);
                if self.registry.is_error_type(&inh.parent) {
                    let extra_fields: Vec<(String, Type)> = inh
                        .fields
                        .iter()
                        .filter(|f| !f.is_method)
                        .map(|f| {
                            let ty = f
                                .type_annotation
                                .as_ref()
                                .map(|t| self.registry.resolve_type(t))
                                .unwrap_or(Type::Unknown);
                            (f.name.clone(), ty)
                        })
                        .collect();
                    self.registry.register_error_type(&inh.child, extra_fields);
                } else {
                    let extra_fields: Vec<(String, Type)> = inh
                        .fields
                        .iter()
                        .filter(|f| !f.is_method)
                        .map(|f| {
                            let ty = f
                                .type_annotation
                                .as_ref()
                                .map(|t| self.registry.resolve_type(t))
                                .unwrap_or(Type::Unknown);
                            (f.name.clone(), ty)
                        })
                        .collect();
                    self.registry
                        .register_inheritance(&inh.parent, &inh.child, extra_fields);
                }
            }
            Statement::FuncDef(fd) => {
                // Register function return type for later lookup
                let ret_ty = fd
                    .return_type
                    .as_ref()
                    .map(|t| self.registry.resolve_type(t))
                    .unwrap_or(Type::Unknown);
                self.func_types.insert(fd.name.clone(), ret_ty);
                self.func_param_counts
                    .insert(fd.name.clone(), fd.params.len());
                // Register parameter types for partial application type inference
                let param_types: Vec<Type> = fd
                    .params
                    .iter()
                    .map(|p| {
                        p.type_annotation
                            .as_ref()
                            .map(|t| self.registry.resolve_type(t))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect();
                self.func_param_types.insert(fd.name.clone(), param_types);
            }
            Statement::Import(imp) => {
                // Core bundled package signatures (imported symbol path).
                if imp.path == "taida-lang/crypto" {
                    for sym in &imp.symbols {
                        if sym.name == "sha256" {
                            let local_name = sym.alias.as_ref().unwrap_or(&sym.name).clone();
                            self.func_types.insert(local_name.clone(), Type::Str);
                            self.func_param_counts.insert(local_name, 1);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Validate class-like definition fields (TypeDef / MoldDef / InheritanceDef).
    /// Non-method fields must have either a type annotation (`field: Type`)
    /// or a default value (`field <= value`).
    fn validate_class_like_fields(&mut self, kind: &str, def_name: &str, fields: &[FieldDef]) {
        for field in fields.iter().filter(|f| !f.is_method) {
            if field.type_annotation.is_none() && field.default_value.is_none() {
                self.errors.push(TypeError {
                    message: Self::binding_diag(
                        "E1400",
                        format!(
                            "{} '{}' field '{}' must declare either a type annotation (`{}: Type`) or a default value (`{} <= value`)",
                            kind, def_name, field.name, field.name, field.name
                        ),
                        "Declare fields as `name: Type` or `name <= default`; bare `name` is not allowed."
                    ),
                    span: field.span.clone(),
                });
            }
        }
    }

    fn resolve_mold_header_type(&self, ty: &TypeExpr, bound_types: &HashMap<String, Type>) -> Type {
        match ty {
            TypeExpr::Named(name) => bound_types
                .get(name)
                .cloned()
                .unwrap_or_else(|| self.registry.resolve_type(ty)),
            TypeExpr::BuchiPack(fields) => Type::BuchiPack(
                fields
                    .iter()
                    .map(|field| {
                        let field_ty = field
                            .type_annotation
                            .as_ref()
                            .map(|ty| self.resolve_mold_header_type(ty, bound_types))
                            .unwrap_or(Type::Unknown);
                        (field.name.clone(), field_ty)
                    })
                    .collect(),
            ),
            TypeExpr::List(inner) => {
                Type::List(Box::new(self.resolve_mold_header_type(inner, bound_types)))
            }
            TypeExpr::Generic(name, args) => Type::Generic(
                name.clone(),
                args.iter()
                    .map(|arg| self.resolve_mold_header_type(arg, bound_types))
                    .collect(),
            ),
            TypeExpr::Function(params, ret) => Type::Function(
                params
                    .iter()
                    .map(|param| self.resolve_mold_header_type(param, bound_types))
                    .collect(),
                Box::new(self.resolve_mold_header_type(ret, bound_types)),
            ),
        }
    }

    fn mold_header_type_compatible(&self, actual: &Type, expected: &Type) -> bool {
        match (actual, expected) {
            (_, Type::Unknown) | (Type::Unknown, _) => true,
            (
                Type::Function(actual_params, actual_ret),
                Type::Function(expected_params, expected_ret),
            ) => {
                actual_params.len() == expected_params.len()
                    && actual_params.iter().zip(expected_params.iter()).all(
                        |(actual_param, expected_param)| {
                            self.mold_header_type_compatible(actual_param, expected_param)
                                && self.mold_header_type_compatible(expected_param, actual_param)
                        },
                    )
                    && self.mold_header_type_compatible(actual_ret, expected_ret)
            }
            _ => self.registry.is_subtype_of(actual, expected),
        }
    }

    fn validate_mold_header_consistency(&mut self, md: &MoldDef) {
        let Some(name_args) = md.name_args.as_ref() else {
            return;
        };
        if name_args != &md.mold_args {
            self.errors.push(TypeError {
                message: Self::binding_diag(
                    "E1407",
                    format!(
                        "MoldDef '{}' must use the same header on both sides of `=>` when `Name[...]` is explicit",
                        md.name
                    ),
                    "Make `Name[...]` match `Mold[...]` exactly, or omit `Name[...]` entirely.",
                ),
                span: md.span.clone(),
            });
        }
    }

    /// Validate mold type parameter binding targets at definition time.
    /// `filling` always binds to the first type parameter.
    /// Additional type parameters must have corresponding non-default fields.
    fn validate_mold_type_param_bindings(&mut self, md: &MoldDef) {
        let additional_params: Vec<String> = md
            .type_params
            .iter()
            .skip(1)
            .map(|tp| tp.name.clone())
            .collect();
        if additional_params.is_empty() {
            return;
        }

        let positional_field_count = md
            .fields
            .iter()
            .filter(|f| !f.is_method && f.default_value.is_none() && f.name != "filling")
            .count();

        if additional_params.len() > positional_field_count {
            let unbound = additional_params[positional_field_count..].join(", ");
            self.errors.push(TypeError {
                message: Self::binding_diag(
                    "E1401",
                    format!(
                        "MoldDef '{}' has unbound type parameter(s): {}. \
additional type parameters must map to non-default fields after `filling`",
                        md.name, unbound
                    ),
                    "Add required non-default fields after `filling` so every extra type parameter has a binding target."
                ),
                span: md.span.clone(),
            });
        }
    }

    /// Validate custom mold instantiation binding rules for `[]` and `()`.
    fn validate_custom_mold_inst_bindings(
        &mut self,
        name: &str,
        type_args: &[Expr],
        fields: &[BuchiField],
        span: &Span,
    ) {
        let mold_fields = match self.mold_field_defs.get(name).cloned() {
            Some(f) => f,
            None => return,
        };

        let required_fields: Vec<String> = mold_fields
            .iter()
            .filter(|f| !f.is_method && f.default_value.is_none() && f.name != "filling")
            .map(|f| f.name.clone())
            .collect();
        let optional_fields: Vec<String> = mold_fields
            .iter()
            .filter(|f| !f.is_method && f.default_value.is_some())
            .map(|f| f.name.clone())
            .collect();

        // filling + non-default fields
        let required_positional = 1 + required_fields.len();
        if type_args.len() < required_positional {
            let missing_names: Vec<String> = std::iter::once("filling".to_string())
                .chain(required_fields.iter().cloned())
                .skip(type_args.len())
                .collect();
            self.errors.push(TypeError {
                message: Self::binding_diag(
                    "E1402",
                    format!(
                        "MoldInst '{}' requires {} positional `[]` argument(s), got {} (missing: {})",
                        name,
                        required_positional,
                        type_args.len(),
                        missing_names.join(", ")
                    ),
                    "Provide missing required values in `[]` order: `filling`, then non-default fields."
                ),
                span: span.clone(),
            });
        }

        if type_args.len() > required_positional {
            self.errors.push(TypeError {
                message: Self::binding_diag(
                    "E1403",
                    format!(
                        "MoldInst '{}' takes {} positional `[]` argument(s), got {}. \
defaulted fields must be provided via `()`",
                        name,
                        required_positional,
                        type_args.len()
                    ),
                    "Move optional/defaulted values from `[]` to named `()` options.",
                ),
                span: span.clone(),
            });
        }

        let required_set: std::collections::HashSet<String> = required_fields.into_iter().collect();
        let optional_set: std::collections::HashSet<String> = optional_fields.into_iter().collect();
        let mut seen = std::collections::HashSet::<String>::new();

        for field in fields {
            if !seen.insert(field.name.clone()) {
                self.errors.push(TypeError {
                    message: Self::binding_diag(
                        "E1404",
                        format!("MoldInst '{}' has duplicate option '{}'", name, field.name),
                        "Specify each named option in `()` at most once.",
                    ),
                    span: field.span.clone(),
                });
                continue;
            }

            if required_set.contains(&field.name) {
                self.errors.push(TypeError {
                    message: Self::binding_diag(
                        "E1405",
                        format!(
                            "MoldInst '{}' field '{}' must be passed via `[]`, not `()`",
                            name, field.name
                        ),
                        "Pass non-default fields as positional `[]` arguments in declaration order."
                    ),
                    span: field.span.clone(),
                });
            } else if !optional_set.contains(&field.name) {
                self.errors.push(TypeError {
                    message: Self::binding_diag(
                        "E1406",
                        format!(
                            "MoldInst '{}' has undefined option '{}' in `()`",
                            name, field.name
                        ),
                        "Use only fields declared with defaults as `()` options.",
                    ),
                    span: field.span.clone(),
                });
            }
        }
    }

    fn validate_mold_header_constraints(&mut self, name: &str, type_args: &[Expr], span: &Span) {
        let Some(header_args) = self.mold_header_args.get(name).cloned() else {
            return;
        };

        let mut bound_types = HashMap::<String, Type>::new();
        for (idx, (header_arg, actual_expr)) in header_args.iter().zip(type_args.iter()).enumerate()
        {
            let actual = self.infer_expr_type(actual_expr);
            match header_arg {
                MoldHeaderArg::TypeParam(tp) => {
                    if let Some(constraint) = &tp.constraint {
                        let expected = self.resolve_mold_header_type(constraint, &bound_types);
                        if !self.mold_header_type_compatible(&actual, &expected) {
                            self.errors.push(TypeError {
                                message: Self::binding_diag(
                                    "E1409",
                                    format!(
                                        "MoldInst '{}' positional `[]` argument {} violates constraint on '{}': expected {}, got {}",
                                        name,
                                        idx + 1,
                                        tp.name,
                                        expected,
                                        actual
                                    ),
                                    "Pass a value whose inferred type satisfies the constrained mold header.",
                                ),
                                span: span.clone(),
                            });
                        }
                    }
                    bound_types.insert(tp.name.clone(), actual);
                }
                MoldHeaderArg::Concrete(concrete) => {
                    let expected = self.resolve_mold_header_type(concrete, &bound_types);
                    if !self.mold_header_type_compatible(&actual, &expected) {
                        self.errors.push(TypeError {
                            message: Self::binding_diag(
                                "E1408",
                                format!(
                                    "MoldInst '{}' positional `[]` argument {} is fixed to {}, got {}",
                                    name,
                                    idx + 1,
                                    expected,
                                    actual
                                ),
                                "Pass a value whose inferred type matches the concrete mold header.",
                            ),
                            span: span.clone(),
                        });
                    }
                }
            }
        }
    }

    fn validate_function_param_defaults(&mut self, fd: &FuncDef, param_types: &[Type]) {
        let param_names: Vec<String> = fd.params.iter().map(|p| p.name.clone()).collect();

        for (i, param) in fd.params.iter().enumerate() {
            let ty = param_types.get(i).cloned().unwrap_or(Type::Unknown);

            if let Some(default_expr) = &param.default_value {
                let forbidden: HashSet<String> = param_names[i..].iter().cloned().collect();
                if let Some(illegal_ref) =
                    Self::find_forbidden_default_ref(default_expr, &forbidden)
                {
                    self.errors.push(TypeError {
                        message: format!(
                            "[E1302] Default value for parameter '{}' cannot reference '{}' (self or later parameter). Hint: Reference only earlier parameters in default expressions.",
                            param.name, illegal_ref
                        ),
                        span: param.span.clone(),
                    });
                }

                let default_ty = self.infer_expr_type(default_expr);
                if ty != Type::Unknown
                    && default_ty != Type::Unknown
                    && !self.registry.is_subtype_of(&default_ty, &ty)
                {
                    self.errors.push(TypeError {
                        message: format!(
                            "[E1303] Default value type mismatch for parameter '{}': expected {}, got {}. Hint: Make the default expression assignable to the parameter type.",
                            param.name, ty, default_ty
                        ),
                        span: param.span.clone(),
                    });
                }
            }

            self.define_var(&param.name, ty);
        }
    }

    /// Type-check a statement (second pass).
    fn check_statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Assignment(assign) => {
                let inferred = self.infer_expr_type(&assign.value);

                // If there's a type annotation, check compatibility
                if let Some(type_ann) = &assign.type_annotation {
                    let expected = self.registry.resolve_type(type_ann);
                    if !self.registry.is_subtype_of(&inferred, &expected)
                        && inferred != Type::Unknown
                    {
                        self.errors.push(TypeError {
                            message: format!(
                                "Type mismatch in assignment to '{}': expected {}, got {}",
                                assign.target, expected, inferred
                            ),
                            span: assign.span.clone(),
                        });
                    }
                    // Register with the annotated type
                    self.define_var_with_span(&assign.target, expected, Some(&assign.span));
                } else {
                    // @[] without type annotation is ambiguous — element type is unknown
                    if matches!(&inferred, Type::List(inner) if matches!(inner.as_ref(), Type::Unknown))
                        && matches!(&assign.value, Expr::ListLit(items, _) if items.is_empty())
                    {
                        self.errors.push(TypeError {
                                message: format!(
                                    "Empty list literal `@[]` requires a type annotation (e.g., `{}: @[Int] <= @[]`). Element type cannot be inferred.",
                                    assign.target
                                ),
                                span: assign.span.clone(),
                            });
                    }
                    // Register with the inferred type
                    self.define_var_with_span(&assign.target, inferred, Some(&assign.span));
                }
            }
            Statement::FuncDef(fd) => {
                // Register function as a variable (for first-class functions)
                let ret_ty = fd
                    .return_type
                    .as_ref()
                    .map(|t| self.registry.resolve_type(t))
                    .unwrap_or(Type::Unknown);
                let param_types: Vec<Type> = fd
                    .params
                    .iter()
                    .map(|p| {
                        p.type_annotation
                            .as_ref()
                            .map(|t| self.registry.resolve_type(t))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect();
                self.define_var_with_span(
                    &fd.name,
                    Type::Function(param_types.clone(), Box::new(ret_ty.clone())),
                    Some(&fd.span),
                );

                // Push new scope for function body
                self.push_scope();

                // Validate defaults left-to-right and register params in scope order.
                self.validate_function_param_defaults(fd, &param_types);

                // Check function body
                for body_stmt in &fd.body {
                    self.check_statement(body_stmt);
                }

                self.pop_scope();
            }
            Statement::Expr(expr) => {
                self.infer_expr_type(expr);
            }
            Statement::ErrorCeiling(ec) => {
                // Push scope for error handler
                self.push_scope();

                // Register the error parameter
                let err_ty = self.registry.resolve_type(&ec.error_type);
                self.define_var(&ec.error_param, err_ty);

                for body_stmt in &ec.handler_body {
                    self.check_statement(body_stmt);
                }

                self.pop_scope();
            }
            Statement::Import(imp) => {
                // Register imported symbols as Unknown
                // (We don't have cross-module type info yet)
                for sym in &imp.symbols {
                    let name = sym.alias.as_ref().unwrap_or(&sym.name);
                    self.define_var(name, Type::Unknown);
                }
            }
            Statement::UnmoldForward(uf) => {
                // `expr ]=> target` -- target gets the unmolded (inner) value
                let source_ty = self.infer_expr_type(&uf.source);
                let target_ty = self.unmold_type(&source_ty);
                self.define_var_with_span(&uf.target, target_ty, Some(&uf.span));
            }
            Statement::UnmoldBackward(ub) => {
                // `target <=[ expr`
                let source_ty = self.infer_expr_type(&ub.source);
                let target_ty = self.unmold_type(&source_ty);
                self.define_var_with_span(&ub.target, target_ty, Some(&ub.span));
            }
            // Type defs are handled in first pass
            _ => {}
        }
    }

    /// Infer the type of an expression.
    pub fn infer_expr_type(&mut self, expr: &Expr) -> Type {
        match expr {
            Expr::IntLit(_, _) => Type::Int,
            Expr::FloatLit(_, _) => Type::Float,
            Expr::StringLit(_, _) => Type::Str,
            Expr::TemplateLit(_, _) => Type::Str,
            Expr::BoolLit(_, _) => Type::Bool,
            Expr::Gorilla(_) => Type::Unit,
            Expr::Placeholder(_) => Type::Unknown,
            Expr::Hole(_) => Type::Unknown,

            Expr::Ident(name, _) => {
                // Look up variable in scope
                self.lookup_var(name).unwrap_or(Type::Unknown)
            }

            Expr::BuchiPack(fields, _) => {
                let field_types: Vec<(String, Type)> = fields
                    .iter()
                    .map(|f| {
                        let ty = self.infer_expr_type(&f.value);
                        (f.name.clone(), ty)
                    })
                    .collect();
                Type::BuchiPack(field_types)
            }

            Expr::ListLit(items, span) => {
                if items.is_empty() {
                    Type::List(Box::new(Type::Unknown))
                } else {
                    let first_type = self.infer_expr_type(&items[0]);
                    // リスト要素の同質性チェック (E0401)
                    // Int/Float 混在は Num に統一
                    let mut unified_type = first_type.clone();
                    for (i, item) in items.iter().enumerate().skip(1) {
                        let item_type = self.infer_expr_type(item);
                        if Self::contains_unknown(&item_type)
                            || Self::contains_unknown(&unified_type)
                        {
                            // Unknown を含む型は型推論未完了 — スキップ
                            // unified_type が Unknown で item_type が具体型なら更新
                            if unified_type == Type::Unknown && item_type != Type::Unknown {
                                unified_type = item_type;
                            }
                            continue;
                        }
                        // Int/Float の混在は Num に統一
                        if (unified_type == Type::Int
                            || unified_type == Type::Float
                            || unified_type == Type::Num)
                            && item_type.is_numeric()
                        {
                            if unified_type != item_type {
                                unified_type = Type::Num;
                            }
                            continue;
                        }
                        // BuchiPack 同士は構造的部分型なので許容
                        if matches!(unified_type, Type::BuchiPack(_))
                            && matches!(item_type, Type::BuchiPack(_))
                        {
                            continue;
                        }
                        // 型不一致
                        if item_type != unified_type {
                            self.errors.push(TypeError {
                                message: format!(
                                    "[E0401] リスト要素の型が不一致: 先頭要素は {} ですが、位置 {} の要素は {} です",
                                    first_type, i, item_type
                                ),
                                span: span.clone(),
                            });
                            break;
                        }
                    }
                    Type::List(Box::new(unified_type))
                }
            }

            Expr::BinaryOp(left, op, right, span) => {
                let left_type = self.infer_expr_type(left);
                let right_type = self.infer_expr_type(right);
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul => {
                        if left_type.is_numeric() && right_type.is_numeric() {
                            if matches!(left_type, Type::Float) || matches!(right_type, Type::Float)
                            {
                                Type::Float
                            } else {
                                Type::Int
                            }
                        } else if matches!(op, BinOp::Add)
                            && matches!(left_type, Type::Str)
                            && matches!(right_type, Type::Str)
                        {
                            Type::Str
                        } else if left_type == Type::Unknown || right_type == Type::Unknown {
                            Type::Unknown
                        } else {
                            self.errors.push(TypeError {
                                message: format!(
                                    "Cannot apply {:?} to {} and {}",
                                    op, left_type, right_type
                                ),
                                span: span.clone(),
                            });
                            Type::Unknown
                        }
                    }
                    BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::GtEq => Type::Bool,
                    BinOp::And | BinOp::Or => Type::Bool,
                    BinOp::Concat => Type::Str,
                }
            }

            Expr::UnaryOp(op, inner, _) => {
                let inner_type = self.infer_expr_type(inner);
                match op {
                    UnaryOp::Neg => {
                        if inner_type.is_numeric() || inner_type == Type::Unknown {
                            inner_type
                        } else {
                            Type::Unknown
                        }
                    }
                    UnaryOp::Not => Type::Bool,
                }
            }

            Expr::FuncCall(func, args, span) => {
                // C-5c: Reject old `_` partial application syntax in function call args.
                // Pipeline context (`data => f(_)`) is allowed — `_` refers to pipe value.
                if !self.in_pipeline {
                    for arg in args.iter() {
                        if let Expr::Placeholder(ph_span) = arg {
                            self.errors.push(TypeError {
                                message: "[E1502] Use empty slot syntax `f(5, )` instead of `f(5, _)` for partial application. \
                                     Hint: Remove the `_` and leave the argument position empty.".to_string(),
                                span: ph_span.clone(),
                            });
                        }
                    }
                }

                // C-5d: Reject partial application (Placeholder or Hole) in TypeDef/BuchiPack instantiation.
                // TypeDef calls look like FuncCall where callee is an uppercase Ident.
                if let Expr::Ident(callee_name, _) = func.as_ref()
                    && callee_name.chars().next().is_some_and(|c| c.is_uppercase())
                    && !self.func_types.contains_key(callee_name.as_str())
                {
                    // This is likely a TypeDef instantiation
                    for arg in args.iter() {
                        match arg {
                            Expr::Placeholder(ph_span) => {
                                self.errors.push(TypeError {
                                        message: "[E1503] Partial application is not supported for TypeDef/BuchiPack instantiation. \
                                             Hint: Provide all fields explicitly when creating a TypeDef instance.".to_string(),
                                        span: ph_span.clone(),
                                    });
                            }
                            Expr::Hole(h_span) => {
                                self.errors.push(TypeError {
                                        message: "[E1503] Partial application is not supported for TypeDef/BuchiPack instantiation. \
                                             Hint: Provide all fields explicitly when creating a TypeDef instance.".to_string(),
                                        span: h_span.clone(),
                                    });
                            }
                            _ => {}
                        }
                    }
                }

                // Count holes in the argument list
                let hole_count = args.iter().filter(|a| matches!(a, Expr::Hole(_))).count();

                // Try to resolve return type from function name
                if let Expr::Ident(name, _) = func.as_ref() {
                    // First check func_types (registered function return types)
                    if let Some(ret_ty) = self.func_types.get(name).cloned() {
                        if let Some(expected) = self.func_param_counts.get(name).copied() {
                            if args.len() > expected {
                                self.errors.push(TypeError {
                                    message: format!(
                                        "[E1301] Function '{}' takes at most {} argument(s), got {}. Hint: Remove extra arguments or update the function signature.",
                                        name, expected, args.len()
                                    ),
                                    span: span.clone(),
                                });
                            }
                            // Slot count (args.len()) must equal arity when holes are present
                            if hole_count > 0 && args.len() != expected {
                                self.errors.push(TypeError {
                                    message: format!(
                                        "[E1505] Partial application of '{}' requires exactly {} slot(s) (got {}). \
                                         Hint: Provide a value or empty slot for each parameter.",
                                        name, expected, args.len()
                                    ),
                                    span: span.clone(),
                                });
                            }
                        }
                        // E1506: Check argument types against registered parameter types
                        if let Some(param_types) = self.func_param_types.get(name).cloned() {
                            for (i, arg) in args.iter().enumerate() {
                                // Skip holes (partial application) and placeholders
                                if matches!(arg, Expr::Hole(_) | Expr::Placeholder(_)) {
                                    continue;
                                }
                                if let Some(expected_ty) = param_types.get(i) {
                                    if *expected_ty == Type::Unknown {
                                        continue;
                                    }
                                    let actual_ty = self.infer_expr_type(arg);
                                    if actual_ty == Type::Unknown {
                                        continue;
                                    }
                                    if !self.registry.is_subtype_of(&actual_ty, expected_ty) {
                                        self.errors.push(TypeError {
                                            message: format!(
                                                "[E1506] Argument {} of '{}' has type {}, expected {}. \
                                                 Hint: Pass a value of the correct type, or use an explicit conversion.",
                                                i + 1, name, actual_ty, expected_ty
                                            ),
                                            span: span.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        // If holes present, return a function type (partial application)
                        if hole_count > 0 {
                            // Use registered param types to infer concrete hole types
                            let registered_param_types = self.func_param_types.get(name);
                            let hole_param_types: Vec<Type> = args
                                .iter()
                                .enumerate()
                                .filter(|(_, a)| matches!(a, Expr::Hole(_)))
                                .map(|(i, _)| {
                                    registered_param_types
                                        .and_then(|pts| pts.get(i).cloned())
                                        .unwrap_or(Type::Unknown)
                                })
                                .collect();
                            return Type::Function(hole_param_types, Box::new(ret_ty));
                        }
                        return ret_ty;
                    }
                    // Check if variable holds a function type
                    if let Some(Type::Function(params, ret)) = self.lookup_var(name) {
                        if args.len() > params.len() {
                            self.errors.push(TypeError {
                                message: format!(
                                    "[E1301] Function value '{}' takes at most {} argument(s), got {}. Hint: Remove extra arguments or adjust the function type.",
                                    name, params.len(), args.len()
                                ),
                                span: span.clone(),
                            });
                        }
                        if hole_count > 0 && args.len() != params.len() {
                            self.errors.push(TypeError {
                                message: format!(
                                    "[E1505] Partial application of '{}' requires exactly {} slot(s) (got {}). \
                                     Hint: Provide a value or empty slot for each parameter.",
                                    name, params.len(), args.len()
                                ),
                                span: span.clone(),
                            });
                        }
                        // E1506: Check argument types against function parameter types
                        for (i, arg) in args.iter().enumerate() {
                            if matches!(arg, Expr::Hole(_) | Expr::Placeholder(_)) {
                                continue;
                            }
                            if let Some(expected_ty) = params.get(i) {
                                if *expected_ty == Type::Unknown {
                                    continue;
                                }
                                let actual_ty = self.infer_expr_type(arg);
                                if actual_ty == Type::Unknown {
                                    continue;
                                }
                                if !self.registry.is_subtype_of(&actual_ty, expected_ty) {
                                    self.errors.push(TypeError {
                                        message: format!(
                                            "[E1506] Argument {} of '{}' has type {}, expected {}. \
                                             Hint: Pass a value of the correct type, or use an explicit conversion.",
                                            i + 1, name, actual_ty, expected_ty
                                        ),
                                        span: span.clone(),
                                    });
                                }
                            }
                        }
                        if hole_count > 0 {
                            // Collect the types of the hole positions from the original param types
                            let hole_param_types: Vec<Type> = args
                                .iter()
                                .enumerate()
                                .filter(|(_, a)| matches!(a, Expr::Hole(_)))
                                .map(|(i, _)| params.get(i).cloned().unwrap_or(Type::Unknown))
                                .collect();
                            return Type::Function(hole_param_types, ret);
                        }
                        return *ret;
                    }
                    // Check if it's a known builtin
                    // E1507: Builtin arity check
                    // (name, min_args, max_args)
                    let builtin_arity: Option<(usize, usize)> = match name.as_str() {
                        "debug" => Some((1, 2)), // debug(value) or debug(label, value)
                        "toString" | "toStr" => Some((1, 1)),
                        "typeOf" | "typeof" => Some((1, 1)),
                        "jsonEncode" | "jsonPretty" => Some((1, 1)),
                        "nowMs" => Some((0, 0)),
                        "assert" => Some((1, 2)), // assert(cond) or assert(cond, msg)
                        "range" => Some((2, 3)),  // range(start, end) or range(start, end, step)
                        "enumerate" => Some((1, 1)),
                        "zip" => Some((2, 2)),
                        "hashMap" => Some((0, 1)),
                        "setOf" => Some((1, 1)),
                        "stdout" => Some((1, 1)),
                        "stderr" => Some((1, 1)),
                        "exit" => Some((1, 1)),
                        "stdin" => Some((1, 1)),
                        "argv" => Some((0, 0)),
                        "sleep" => Some((1, 1)),
                        _ => None,
                    };
                    if let Some((min_args, max_args)) = builtin_arity
                        && (args.len() < min_args || args.len() > max_args)
                    {
                        let arity_desc = if min_args == max_args {
                            format!("{}", min_args)
                        } else {
                            format!("{}-{}", min_args, max_args)
                        };
                        self.errors.push(TypeError {
                                message: format!(
                                    "[E1507] Builtin '{}' takes {} argument(s), got {}. \
                                     Hint: Check the function signature and provide the correct number of arguments.",
                                    name, arity_desc, args.len()
                                ),
                                span: span.clone(),
                            });
                    }
                    let base_ty = match name.as_str() {
                        // debug returns its argument (pass-through)
                        "debug" => {
                            if let Some(first_arg) = args.first() {
                                self.infer_expr_type(first_arg)
                            } else {
                                Type::Unit
                            }
                        }
                        "toString" | "toStr" => Type::Str,
                        "typeOf" | "typeof" => Type::Str,
                        "jsonEncode" | "jsonPretty" => Type::Str,
                        "nowMs" => Type::Int,
                        // Prelude functions
                        "assert" => Type::Unit,
                        "range" => Type::List(Box::new(Type::Int)),
                        "enumerate" => Type::List(Box::new(Type::Unknown)),
                        "zip" => Type::List(Box::new(Type::Unknown)),
                        "hashMap" => Type::Named("HashMap".to_string()),
                        "setOf" => Type::Named("Set".to_string()),
                        "stdout" | "stderr" | "exit" => Type::Unit,
                        "stdin" => Type::Str,
                        "argv" => Type::List(Box::new(Type::Str)),
                        "sleep" => Type::Generic("Async".to_string(), vec![Type::Unit]),
                        _ => Type::Unknown,
                    };
                    if hole_count > 0 {
                        let hole_param_types: Vec<Type> =
                            (0..hole_count).map(|_| Type::Unknown).collect();
                        return Type::Function(hole_param_types, Box::new(base_ty));
                    }
                    base_ty
                } else {
                    // Calling a non-ident expression (e.g. lambda call)
                    let func_type = self.infer_expr_type(func);
                    match func_type {
                        Type::Function(params, ret) => {
                            if args.len() > params.len() {
                                self.errors.push(TypeError {
                                    message: format!(
                                        "[E1301] Function call takes at most {} argument(s), got {}. Hint: Remove extra arguments or adjust the callee signature.",
                                        params.len(), args.len()
                                    ),
                                    span: span.clone(),
                                });
                            }
                            if hole_count > 0 && args.len() != params.len() {
                                self.errors.push(TypeError {
                                    message: format!(
                                        "[E1505] Partial application requires exactly {} slot(s) (got {}). \
                                         Hint: Provide a value or empty slot for each parameter.",
                                        params.len(), args.len()
                                    ),
                                    span: span.clone(),
                                });
                            }
                            if hole_count > 0 {
                                let hole_param_types: Vec<Type> = args
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, a)| matches!(a, Expr::Hole(_)))
                                    .map(|(i, _)| params.get(i).cloned().unwrap_or(Type::Unknown))
                                    .collect();
                                return Type::Function(hole_param_types, ret);
                            }
                            *ret
                        }
                        _ => Type::Unknown,
                    }
                }
            }

            Expr::MethodCall(obj, method, args, span) => {
                let obj_type = self.infer_expr_type(obj);
                // E1508: Method call argument count and type checking
                self.check_method_args(&obj_type, method, args, span);
                self.infer_method_return_type(&obj_type, method)
            }

            Expr::FieldAccess(obj, field, span) => {
                let obj_type = self.infer_expr_type(obj);
                match &obj_type {
                    Type::BuchiPack(fields) => {
                        if let Some((_, ty)) = fields.iter().find(|(name, _)| name == field) {
                            ty.clone()
                        } else {
                            self.errors.push(TypeError {
                                message: format!(
                                    "Field '{}' does not exist on type {}",
                                    field, obj_type
                                ),
                                span: span.clone(),
                            });
                            Type::Unknown
                        }
                    }
                    Type::Named(type_name) => {
                        // Look up field in registered type definition
                        if let Some(fields) = self.registry.get_type_fields(type_name) {
                            if let Some((_, ty)) = fields.iter().find(|(name, _)| name == field) {
                                ty.clone()
                            } else {
                                Type::Unknown
                            }
                        } else {
                            Type::Unknown
                        }
                    }
                    Type::Unknown => Type::Unknown,
                    _ => Type::Unknown,
                }
            }

            // IndexAccess removed in v0.5.0 — use .get(i) instead
            Expr::CondBranch(arms, _) => {
                // Infer type from the first arm's body (all arms should return same type)
                if let Some(first_arm) = arms.first() {
                    if let Some(last_expr) = first_arm.last_expr() {
                        self.infer_expr_type(last_expr)
                    } else {
                        Type::Unknown
                    }
                } else {
                    Type::Unknown
                }
            }

            Expr::Pipeline(exprs, _) => {
                // Pipeline: walk all expressions, set in_pipeline for non-first elements
                let old_in_pipeline = self.in_pipeline;
                let mut result_type = Type::Unknown;
                for (i, pipe_expr) in exprs.iter().enumerate() {
                    if i > 0 {
                        self.in_pipeline = true;
                    }
                    result_type = self.infer_expr_type(pipe_expr);
                }
                self.in_pipeline = old_in_pipeline;
                result_type
            }

            Expr::MoldInst(name, type_args, fields, mold_span) => {
                // C-5e: Reject Mold[_]() direct binding outside pipeline.
                // In pipeline (`data => Trim[_]()`), `_` refers to the pipe value — allowed.
                if !self.in_pipeline {
                    for arg in type_args.iter() {
                        if let Expr::Placeholder(ph_span) = arg {
                            self.errors.push(TypeError {
                                message: format!(
                                    "[E1504] `{}[_]()` cannot be used outside a pipeline. \
                                     The `_` placeholder in mold type arguments is only valid inside a pipeline expression (`data => {}[_]()`). \
                                     Hint: Pass a concrete value to the mold, e.g., `{}[value]()`.",
                                    name, name, name
                                ),
                                span: ph_span.clone(),
                            });
                        }
                    }
                }

                self.validate_custom_mold_inst_bindings(name, type_args, fields, mold_span);
                self.validate_mold_header_constraints(name, type_args, mold_span);
                match name.as_str() {
                    // JSON[raw, Schema]() returns Lax (wrapping the schema type)
                    "JSON" => Type::Generic("Lax".to_string(), vec![Type::Unknown]),
                    // Async[T] wraps a value
                    "Async" => Type::Generic(
                        "Async".to_string(),
                        vec![
                            type_args
                                .first()
                                .map(|a| self.infer_expr_type(a))
                                .unwrap_or(Type::Unknown),
                        ],
                    ),
                    // Cancel[async]() returns Async[T] (or Async[Unknown] fallback)
                    "Cancel" => {
                        let inner = type_args
                            .first()
                            .map(|a| self.infer_expr_type(a))
                            .map(|t| match t {
                                Type::Generic(name, args) if name == "Async" => {
                                    args.first().cloned().unwrap_or(Type::Unknown)
                                }
                                other => other,
                            })
                            .unwrap_or(Type::Unknown);
                        Type::Generic("Async".to_string(), vec![inner])
                    }
                    // Result[value, predicate] returns Result type
                    "Result" => Type::Generic(
                        "Result".to_string(),
                        vec![
                            type_args
                                .first()
                                .map(|a| self.infer_expr_type(a))
                                .unwrap_or(Type::Unknown),
                            Type::Unknown, // predicate type
                        ],
                    ),
                    // Lax[value]() returns Lax[T]
                    "Lax" => {
                        let inner = type_args
                            .first()
                            .map(|a| self.infer_expr_type(a))
                            .unwrap_or(Type::Unknown);
                        Type::Generic("Lax".to_string(), vec![inner])
                    }
                    // Div[x, y]() and Mod[x, y]() return Lax[Num]
                    "Div" | "Mod" => {
                        let inner = type_args
                            .first()
                            .map(|a| self.infer_expr_type(a))
                            .unwrap_or(Type::Num);
                        let inner = if inner.is_numeric() { inner } else { Type::Num };
                        Type::Generic("Lax".to_string(), vec![inner])
                    }
                    // Type conversion molds: Str[x]() -> Lax[Str], Int[x]() -> Lax[Int], etc.
                    "Str" => Type::Generic("Lax".to_string(), vec![Type::Str]),
                    "Int" => Type::Generic("Lax".to_string(), vec![Type::Int]),
                    "Float" => Type::Generic("Lax".to_string(), vec![Type::Float]),
                    "Bool" => Type::Generic("Lax".to_string(), vec![Type::Bool]),
                    "Bytes" => Type::Generic("Lax".to_string(), vec![Type::Bytes]),
                    "UInt8" => Type::Generic("Lax".to_string(), vec![Type::Int]),
                    "Char" => Type::Generic("Lax".to_string(), vec![Type::Str]),
                    "CodePoint" => Type::Generic("Lax".to_string(), vec![Type::Int]),
                    "Utf8Encode" => Type::Generic("Lax".to_string(), vec![Type::Bytes]),
                    "Utf8Decode" => Type::Generic("Lax".to_string(), vec![Type::Str]),
                    "U16BE" | "U16LE" | "U32BE" | "U32LE" => {
                        Type::Generic("Lax".to_string(), vec![Type::Bytes])
                    }
                    "U16BEDecode" | "U16LEDecode" | "U32BEDecode" | "U32LEDecode" => {
                        Type::Generic("Lax".to_string(), vec![Type::Int])
                    }
                    "BytesCursor" => Type::BuchiPack(vec![
                        ("bytes".to_string(), Type::Bytes),
                        ("offset".to_string(), Type::Int),
                        ("length".to_string(), Type::Int),
                    ]),
                    "BytesCursorRemaining" => Type::Int,
                    "BytesCursorTake" => Type::Generic(
                        "Lax".to_string(),
                        vec![Type::BuchiPack(vec![
                            ("value".to_string(), Type::Bytes),
                            (
                                "cursor".to_string(),
                                Type::BuchiPack(vec![
                                    ("bytes".to_string(), Type::Bytes),
                                    ("offset".to_string(), Type::Int),
                                    ("length".to_string(), Type::Int),
                                ]),
                            ),
                        ])],
                    ),
                    "BytesCursorU8" => Type::Generic(
                        "Lax".to_string(),
                        vec![Type::BuchiPack(vec![
                            ("value".to_string(), Type::Int),
                            (
                                "cursor".to_string(),
                                Type::BuchiPack(vec![
                                    ("bytes".to_string(), Type::Bytes),
                                    ("offset".to_string(), Type::Int),
                                    ("length".to_string(), Type::Int),
                                ]),
                            ),
                        ])],
                    ),
                    "BitAnd" | "BitOr" | "BitXor" | "BitNot" => Type::Int,
                    "ShiftL" | "ShiftR" | "ShiftRU" => {
                        Type::Generic("Lax".to_string(), vec![Type::Int])
                    }
                    "ToRadix" => Type::Generic("Lax".to_string(), vec![Type::Str]),
                    "ByteSet" => Type::Generic("Lax".to_string(), vec![Type::Bytes]),
                    "BytesToList" => Type::List(Box::new(Type::Int)),
                    // HOF molds return the appropriate type
                    // If input is Stream[T], output is also Stream[U]
                    "Map" | "Filter" | "Sort" | "Unique" | "Flatten" | "Reverse" | "Take"
                    | "TakeWhile" | "Drop" | "DropWhile" | "Append" | "Prepend" | "Zip"
                    | "Enumerate" => {
                        // These return a list or stream (same or transformed)
                        if let Some(first_arg) = type_args.first() {
                            let arg_type = self.infer_expr_type(first_arg);
                            if matches!(arg_type, Type::Generic(ref n, _) if n == "Stream") {
                                // Stream input: return Stream (lazy transform)
                                arg_type
                            } else if matches!(arg_type, Type::List(_)) {
                                arg_type
                            } else {
                                Type::List(Box::new(Type::Unknown))
                            }
                        } else {
                            Type::List(Box::new(Type::Unknown))
                        }
                    }
                    // Stream[value]() → Stream[T]
                    "Stream" => {
                        let inner = type_args
                            .first()
                            .map(|a| self.infer_expr_type(a))
                            .unwrap_or(Type::Unknown);
                        Type::Generic("Stream".to_string(), vec![inner])
                    }
                    // StreamFrom[list]() → Stream[T]
                    "StreamFrom" => {
                        if let Some(first_arg) = type_args.first() {
                            let arg_type = self.infer_expr_type(first_arg);
                            if let Type::List(inner) = arg_type {
                                Type::Generic("Stream".to_string(), vec![*inner])
                            } else {
                                Type::Generic("Stream".to_string(), vec![Type::Unknown])
                            }
                        } else {
                            Type::Generic("Stream".to_string(), vec![Type::Unknown])
                        }
                    }
                    "Fold" | "Foldr" | "Reduce" => {
                        // Returns the accumulator type (first arg)
                        if let Some(first_arg) = type_args.first() {
                            self.infer_expr_type(first_arg)
                        } else {
                            Type::Unknown
                        }
                    }
                    // String / Bytes operation molds
                    "Upper" | "Lower" | "Trim" | "Replace" | "CharAt" | "Repeat" | "Pad" => {
                        Type::Str
                    }
                    "Slice" => {
                        if let Some(first_arg) = type_args.first() {
                            let t = self.infer_expr_type(first_arg);
                            if t == Type::Bytes {
                                Type::Bytes
                            } else {
                                Type::Str
                            }
                        } else {
                            Type::Str
                        }
                    }
                    "Split" => Type::List(Box::new(Type::Str)),
                    // Number operation molds
                    "Abs" | "Clamp" => {
                        if let Some(first_arg) = type_args.first() {
                            let t = self.infer_expr_type(first_arg);
                            if t.is_numeric() { t } else { Type::Num }
                        } else {
                            Type::Num
                        }
                    }
                    "Floor" | "Ceil" | "Round" | "Truncate" => Type::Int,
                    "ToFixed" => Type::Str,
                    // List/Bytes operation molds
                    "Concat" => {
                        if let Some(first_arg) = type_args.first() {
                            let t = self.infer_expr_type(first_arg);
                            if t == Type::Bytes {
                                Type::Bytes
                            } else if matches!(t, Type::List(_)) || t == Type::Unknown {
                                t
                            } else {
                                Type::List(Box::new(Type::Unknown))
                            }
                        } else {
                            Type::List(Box::new(Type::Unknown))
                        }
                    }
                    "Join" => Type::Str,
                    "Sum" => Type::Num,
                    "Find" => Type::Generic("Lax".to_string(), vec![Type::Unknown]),
                    "FindIndex" | "Count" => Type::Int,
                    // Gorillax[value]() returns Gorillax[T]
                    "Gorillax" => {
                        let inner = type_args
                            .first()
                            .map(|a| self.infer_expr_type(a))
                            .unwrap_or(Type::Unknown);
                        Type::Generic("Gorillax".to_string(), vec![inner])
                    }
                    // Molten[]() returns Molten (no type arguments allowed)
                    "Molten" => {
                        if !type_args.is_empty() {
                            self.errors.push(TypeError {
                                message: "Molten takes no type arguments: Molten[]()".to_string(),
                                span: mold_span.clone(),
                            });
                        }
                        Type::Molten
                    }
                    // Cage[molten, F] where F: :Molten => :U → Gorillax[U]
                    // Cage requires Molten type as first argument
                    "Cage" => {
                        if let Some(first_arg) = type_args.first() {
                            let first_type = self.infer_expr_type(first_arg);
                            if first_type != Type::Molten && first_type != Type::Unknown {
                                self.errors.push(TypeError {
                                    message: format!(
                                        "Cage requires Molten type as first argument, got {}",
                                        first_type
                                    ),
                                    span: mold_span.clone(),
                                });
                            }
                        }
                        // Extract the return type U from the second argument (function F):
                        // - If F is a lambda `_ x = expr`, infer body type directly → Gorillax[U]
                        // - If F is a function reference, infer its type → Function(params, ret) → extract ret
                        // - Otherwise, fall back to Unknown (safe)
                        let inner = if type_args.len() >= 2 {
                            let second_arg = &type_args[1];
                            match second_arg {
                                Expr::Lambda(_params, body, _span) => {
                                    // Lambda: infer the body expression type directly
                                    self.infer_expr_type(body)
                                }
                                _ => {
                                    // Function reference or other expression:
                                    // infer its type, then extract return type if it's a Function type
                                    let fn_type = self.infer_expr_type(second_arg);
                                    match fn_type {
                                        Type::Function(_, ret) => *ret,
                                        _ => Type::Unknown,
                                    }
                                }
                            }
                        } else {
                            Type::Unknown
                        };
                        Type::Generic("Gorillax".to_string(), vec![inner])
                    }
                    _ => {
                        // Look up in mold definitions
                        if self.registry.mold_defs.contains_key(name) {
                            Type::Named(name.clone())
                        } else {
                            Type::Unknown
                        }
                    }
                }
            }

            Expr::Unmold(inner, _) => {
                // Unmolding a Mold[T] returns T
                let inner_type = self.infer_expr_type(inner);
                match &inner_type {
                    Type::Generic(name, args) => {
                        match name.as_str() {
                            "Lax" | "Result" | "Async" => {
                                // Return the first type argument (the wrapped value type)
                                args.first().cloned().unwrap_or(Type::Unknown)
                            }
                            "Stream" => {
                                // Stream[T] unmolds to @[T] (List)
                                let inner = args.first().cloned().unwrap_or(Type::Unknown);
                                Type::List(Box::new(inner))
                            }
                            _ => Type::Unknown,
                        }
                    }
                    _ => Type::Unknown,
                }
            }

            Expr::Lambda(params, body, _span) => {
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|p| {
                        p.type_annotation
                            .as_ref()
                            .map(|t| self.registry.resolve_type(t))
                            .unwrap_or(Type::Unknown)
                    })
                    .collect();
                // Try to infer return type from the body expression
                let ret_type = self.infer_expr_type(body);
                Type::Function(param_types, Box::new(ret_type))
            }

            Expr::TypeInst(name, _, _) => Type::Named(name.clone()),
            Expr::Throw(_, _) => Type::Unknown,
        }
    }
}

#[path = "checker_methods.rs"]
mod checker_methods;

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "checker_tests.rs"]
mod tests;
