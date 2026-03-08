use super::lower::{LowerError, Lowering};
/// JSON schema resolution for the Taida native backend.
///
/// Contains `resolve_json_schema_descriptor`, `typedef_to_schema_descriptor`,
/// and `type_expr_to_schema_descriptor`.
///
/// These are `impl Lowering` methods split from lower.rs for maintainability.
use crate::parser::*;

impl Lowering {
    // -- JSON Schema Resolution (compile-time) --

    /// Resolve a JSON schema from an AST expression into a descriptor string
    /// for the C runtime. The schema expression is interpreted as a type descriptor:
    ///   - Ident("Int"/"Str"/"Float"/"Bool") -> primitive
    ///   - Ident("User") -> lookup TypeDef by name
    ///   - ListLit([Ident("Pilot")]) -> list schema
    ///
    /// Descriptor format:
    ///   "i" = Int, "f" = Float, "s" = Str, "b" = Bool
    ///   "T{TypeName|field1:desc,field2:desc,...}" = TypeDef
    ///   "L{desc}" = List of elements
    pub(crate) fn resolve_json_schema_descriptor(&self, expr: &Expr) -> Result<String, LowerError> {
        match expr {
            Expr::Ident(name, _) => match name.as_str() {
                "Int" => Ok("i".to_string()),
                "Str" => Ok("s".to_string()),
                "Float" => Ok("f".to_string()),
                "Bool" => Ok("b".to_string()),
                type_name => {
                    if let Some(field_types) = self.type_field_types.get(type_name) {
                        self.typedef_to_schema_descriptor(type_name, field_types)
                    } else {
                        Err(LowerError {
                            message: format!(
                                "Unknown schema type '{}' for JSON casting. Define it as a TypeDef first.",
                                type_name
                            ),
                        })
                    }
                }
            },
            // @[Schema] -- list type
            Expr::ListLit(items, _) => {
                if items.len() == 1 {
                    let elem_desc = self.resolve_json_schema_descriptor(&items[0])?;
                    Ok(format!("L{{{}}}", elem_desc))
                } else {
                    Err(LowerError {
                        message:
                            "List schema @[...] must have exactly one element type: @[TypeName]"
                                .to_string(),
                    })
                }
            }
            _ => Err(LowerError {
                message:
                    "JSON schema must be a type name (e.g., User, Int) or list type (e.g., @[User])"
                        .to_string(),
            }),
        }
    }

    /// Convert a TypeDef's field types to a schema descriptor string.
    fn typedef_to_schema_descriptor(
        &self,
        type_name: &str,
        field_types: &[(String, Option<crate::parser::TypeExpr>)],
    ) -> Result<String, LowerError> {
        let mut parts = Vec::new();
        for (name, type_ann) in field_types {
            let type_desc = match type_ann {
                Some(te) => self.type_expr_to_schema_descriptor(te)?,
                None => "s".to_string(), // default to Str
            };
            parts.push(format!("{}:{}", name, type_desc));
        }
        Ok(format!("T{{{}|{}}}", type_name, parts.join(",")))
    }

    /// Convert a TypeExpr to a schema descriptor string.
    fn type_expr_to_schema_descriptor(
        &self,
        type_expr: &crate::parser::TypeExpr,
    ) -> Result<String, LowerError> {
        match type_expr {
            crate::parser::TypeExpr::Named(name) => match name.as_str() {
                "Int" => Ok("i".to_string()),
                "Str" => Ok("s".to_string()),
                "Float" => Ok("f".to_string()),
                "Bool" => Ok("b".to_string()),
                other => {
                    if let Some(field_types) = self.type_field_types.get(other) {
                        self.typedef_to_schema_descriptor(other, field_types)
                    } else {
                        Ok("s".to_string()) // Unknown type defaults to Str
                    }
                }
            },
            crate::parser::TypeExpr::List(inner) => {
                let elem_desc = self.type_expr_to_schema_descriptor(inner)?;
                Ok(format!("L{{{}}}", elem_desc))
            }
            crate::parser::TypeExpr::BuchiPack(fields) => {
                // Inline buchi pack type @(field: Type, ...)
                let field_types: Vec<(String, Option<crate::parser::TypeExpr>)> = fields
                    .iter()
                    .filter(|f| !f.is_method)
                    .map(|f| (f.name.clone(), f.type_annotation.clone()))
                    .collect();
                self.typedef_to_schema_descriptor("BuchiPack", &field_types)
            }
            _ => Ok("s".to_string()), // fallback to Str
        }
    }
}
