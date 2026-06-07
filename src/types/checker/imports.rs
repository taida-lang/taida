//! imports — methods split out of the TypeChecker impl.
//! Pure move from the parent module; behaviour unchanged.

use crate::net_surface::NET_HTTP_PROTOCOL_VARIANTS;
use crate::types::Type;

use super::TypeChecker;

impl TypeChecker {
    /// install pinned signatures for the interactive os
    /// variants. Idempotent — `register_os_import_symbol` delegates here
    /// for the same symbol names, so the import path remains a no-op
    /// overwrite with the identical `Gorillax[@(code: Int)]` shape.
    ///
    /// Captured `run` / `execShell` are intentionally left out: pinning
    /// them would change the non-interfering contract documented in
    /// `register_os_import_symbol` and tightening on the core-bundled
    /// path would silently affect every existing program that never
    /// imports `taida-lang/os`.
    pub(super) fn install_core_bundled_os_pins(&mut self) {
        self.pin_run_interactive_signature("runInteractive");
        self.pin_exec_shell_interactive_signature("execShellInteractive");
    }

    pub(super) fn pin_run_interactive_signature(&mut self, local_name: &str) {
        // runInteractive(program: Str, args: @[Str]) → Gorillax[@(code: Int)]
        let inner = Type::BuchiPack(vec![("code".to_string(), Type::Int)]);
        let ret = Type::Generic("Gorillax".to_string(), vec![inner]);
        self.func_types.insert(local_name.to_string(), ret);
        self.func_param_counts.insert(local_name.to_string(), 2);
        self.func_param_types.insert(
            local_name.to_string(),
            vec![Type::Str, Type::List(Box::new(Type::Str))],
        );
    }

    pub(super) fn pin_exec_shell_interactive_signature(&mut self, local_name: &str) {
        // execShellInteractive(command: Str) → Gorillax[@(code: Int)]
        let inner = Type::BuchiPack(vec![("code".to_string(), Type::Int)]);
        let ret = Type::Generic("Gorillax".to_string(), vec![inner]);
        self.func_types.insert(local_name.to_string(), ret);
        self.func_param_counts.insert(local_name.to_string(), 1);
        self.func_param_types
            .insert(local_name.to_string(), vec![Type::Str]);
    }

    pub(super) fn register_net_import_symbol(&mut self, symbol_name: &str, local_name: &str) {
        match symbol_name {
            "httpServe" => {
                self.net_http_serve_symbols.insert(local_name.to_string());
            }
            "HttpProtocol" => {
                self.registry.register_enum(
                    local_name,
                    NET_HTTP_PROTOCOL_VARIANTS
                        .iter()
                        .map(|variant| (*variant).to_string())
                        .collect(),
                );
                self.declared_header_arities
                    .insert(local_name.to_string(), 0);
                self.net_http_protocol_type_names
                    .insert(local_name.to_string());
            }
            _ => {}
        }
    }

    /// register typed signatures for `taida-lang/os` symbols that
    /// need compile-time Gorillax inner-shape pinning.
    ///
    /// Currently only the interactive variants are pinned, because
    /// their inner shape `@(code: Int)` is strictly narrower than the
    /// captured `run` / `execShell` form `@(stdout, stderr, code)` — and
    /// callers who reach for `.__value.stdout` on an interactive result
    /// must get a compile error rather than silent Unknown.
    ///
    /// The captured variants are intentionally left Unknown so we stay
    /// non-interfering with pre-existing callers (`run(...).__value.stdout`
    /// etc. must keep working). If/when we want to pin those too, add
    /// matches for "run" / "execShell" below.
    pub(super) fn register_os_import_symbol(&mut self, symbol_name: &str, local_name: &str) {
        match symbol_name {
            "runInteractive" => {
                // Delegates to the same helper used by the import-less path
                // (`install_core_bundled_os_pins`), so the pinned shape is
                // identical whether or not the user wrote
                // `>>> taida-lang/os => @(runInteractive)`. When the import
                // uses an alias (`runInteractive as foo`), this path also
                // installs the alias under the same pin.
                self.pin_run_interactive_signature(local_name);
            }
            "execShellInteractive" => {
                self.pin_exec_shell_interactive_signature(local_name);
            }
            _ => {
                // Other os symbols stay unregistered so the checker treats
                // them as Type::Unknown (pre-C19 behaviour, non-interfering).
            }
        }
    }

    pub(super) fn abi_request_fields() -> Vec<(String, Type)> {
        let pair_list = Self::abi_name_value_pair_list_type();
        vec![
            ("method".to_string(), Type::Str),
            ("path".to_string(), Type::Str),
            ("rawQuery".to_string(), Type::Str),
            ("query".to_string(), pair_list.clone()),
            ("headers".to_string(), pair_list),
            ("body".to_string(), Type::Bytes),
        ]
    }

    pub(super) fn abi_response_fields() -> Vec<(String, Type)> {
        let pair_list = Self::abi_name_value_pair_list_type();
        vec![
            ("status".to_string(), Type::Int),
            ("headers".to_string(), pair_list),
            ("body".to_string(), Type::Bytes),
        ]
    }

    pub(super) fn register_abi_type_symbol(&mut self, symbol_name: &str, local_name: &str) {
        match symbol_name {
            "WebRequest" => {
                self.registry
                    .register_type(local_name, Self::abi_request_fields());
                self.declared_concrete_type_names
                    .insert(local_name.to_string());
                self.declared_header_arities
                    .insert(local_name.to_string(), 0);
            }
            "WebResponse" => {
                self.registry
                    .register_type(local_name, Self::abi_response_fields());
                self.declared_concrete_type_names
                    .insert(local_name.to_string());
                self.declared_header_arities
                    .insert(local_name.to_string(), 0);
            }
            _ => {}
        }
    }

    pub(super) fn register_abi_imports(&mut self, symbols: &[crate::parser::ImportSymbol]) {
        let request_name = symbols
            .iter()
            .find(|sym| sym.name == "WebRequest")
            .map(|sym| sym.alias.as_deref().unwrap_or(sym.name.as_str()))
            .unwrap_or("WebRequest");
        let response_name = symbols
            .iter()
            .find(|sym| sym.name == "WebResponse")
            .map(|sym| sym.alias.as_deref().unwrap_or(sym.name.as_str()))
            .unwrap_or("WebResponse");

        for sym in symbols {
            let local_name = sym.alias.as_deref().unwrap_or(sym.name.as_str());
            self.register_abi_type_symbol(&sym.name, local_name);
        }

        let response_ty = Type::Named(response_name.to_string());
        for sym in symbols {
            let local_name = sym.alias.as_deref().unwrap_or(sym.name.as_str());
            match sym.name.as_str() {
                "text" => {
                    self.func_types
                        .insert(local_name.to_string(), response_ty.clone());
                    self.func_param_counts.insert(local_name.to_string(), 1);
                    self.func_param_types
                        .insert(local_name.to_string(), vec![Type::Str]);
                }
                "json" => {
                    self.func_types
                        .insert(local_name.to_string(), response_ty.clone());
                    self.func_param_counts.insert(local_name.to_string(), 1);
                    self.func_param_types
                        .insert(local_name.to_string(), vec![Type::Unknown]);
                }
                "bytes" => {
                    self.func_types
                        .insert(local_name.to_string(), response_ty.clone());
                    self.func_param_counts.insert(local_name.to_string(), 1);
                    self.func_param_types
                        .insert(local_name.to_string(), vec![Type::Bytes]);
                }
                "status" => {
                    self.func_types
                        .insert(local_name.to_string(), response_ty.clone());
                    self.func_param_counts.insert(local_name.to_string(), 2);
                    self.func_param_types
                        .insert(local_name.to_string(), vec![Type::Int, response_ty.clone()]);
                }
                "header" => {
                    self.func_types
                        .insert(local_name.to_string(), response_ty.clone());
                    self.func_param_counts.insert(local_name.to_string(), 3);
                    self.func_param_types.insert(
                        local_name.to_string(),
                        vec![Type::Str, Type::Str, response_ty.clone()],
                    );
                }
                "WebRequest" | "WebResponse" => {
                    let _ = request_name;
                }
                _ => {}
            }
        }
    }

    pub(super) fn register_imported_function_signature(
        &mut self,
        fd: &crate::parser::FuncDef,
        local_name: &str,
        type_aliases: &std::collections::HashMap<&str, &str>,
    ) {
        let ret_ty = fd
            .return_type
            .as_ref()
            .map(|ty| self.resolve_imported_type_expr(ty, type_aliases))
            .unwrap_or(Type::Unknown);
        let param_types: Vec<Type> = fd
            .params
            .iter()
            .map(|param| {
                param
                    .type_annotation
                    .as_ref()
                    .map(|ty| self.resolve_imported_type_expr(ty, type_aliases))
                    .unwrap_or(Type::Unknown)
            })
            .collect();

        self.func_types.insert(local_name.to_string(), ret_ty);
        self.func_param_counts
            .insert(local_name.to_string(), fd.params.len());
        self.func_param_types
            .insert(local_name.to_string(), param_types);

        if !fd.type_params.is_empty() {
            let aliased = Self::alias_imported_func_def(fd, local_name, type_aliases);
            self.generic_func_defs
                .insert(local_name.to_string(), aliased);
        }
    }

    pub(super) fn alias_imported_func_def(
        fd: &crate::parser::FuncDef,
        local_name: &str,
        type_aliases: &std::collections::HashMap<&str, &str>,
    ) -> crate::parser::FuncDef {
        let mut aliased = fd.clone();
        aliased.name = local_name.to_string();
        for type_param in &mut aliased.type_params {
            if let Some(constraint) = &type_param.constraint {
                type_param.constraint =
                    Some(Self::alias_imported_type_expr(constraint, type_aliases));
            }
        }
        for param in &mut aliased.params {
            if let Some(type_annotation) = &param.type_annotation {
                param.type_annotation = Some(Self::alias_imported_type_expr(
                    type_annotation,
                    type_aliases,
                ));
            }
        }
        if let Some(return_type) = &aliased.return_type {
            aliased.return_type = Some(Self::alias_imported_type_expr(return_type, type_aliases));
        }
        aliased
    }

    pub(super) fn alias_imported_type_expr(
        ty: &crate::parser::TypeExpr,
        type_aliases: &std::collections::HashMap<&str, &str>,
    ) -> crate::parser::TypeExpr {
        use crate::parser::TypeExpr;

        match ty {
            TypeExpr::Named(name) => TypeExpr::Named(
                type_aliases
                    .get(name.as_str())
                    .copied()
                    .unwrap_or(name.as_str())
                    .to_string(),
            ),
            TypeExpr::BuchiPack(fields) => TypeExpr::BuchiPack(
                fields
                    .iter()
                    .map(|field| {
                        let mut field = field.clone();
                        if let Some(type_annotation) = &field.type_annotation {
                            field.type_annotation = Some(Self::alias_imported_type_expr(
                                type_annotation,
                                type_aliases,
                            ));
                        }
                        field
                    })
                    .collect(),
            ),
            TypeExpr::List(inner) => TypeExpr::List(Box::new(Self::alias_imported_type_expr(
                inner,
                type_aliases,
            ))),
            TypeExpr::Generic(name, args) => TypeExpr::Generic(
                type_aliases
                    .get(name.as_str())
                    .copied()
                    .unwrap_or(name.as_str())
                    .to_string(),
                args.iter()
                    .map(|arg| Self::alias_imported_type_expr(arg, type_aliases))
                    .collect(),
            ),
            TypeExpr::Function(params, ret) => TypeExpr::Function(
                params
                    .iter()
                    .map(|param| Self::alias_imported_type_expr(param, type_aliases))
                    .collect(),
                Box::new(Self::alias_imported_type_expr(ret, type_aliases)),
            ),
        }
    }
}
