/// Shared formatting utilities for the LSP module.
///
/// Eliminates duplication of `format_type_expr`, `format_mold_header_*`,
/// and `format_doc_comments` between hover.rs and completion.rs.
use crate::parser::{MoldHeaderArg, TypeExpr};
use crate::types::Type;

/// Format a TypeExpr as a readable string for display.
pub fn format_type_expr(te: &TypeExpr) -> String {
    match te {
        TypeExpr::Named(name) => name.clone(),
        TypeExpr::BuchiPack(fields) => {
            let fs: Vec<String> = fields
                .iter()
                .map(|f| {
                    if let Some(ty) = &f.type_annotation {
                        format!("{}: {}", f.name, format_type_expr(ty))
                    } else {
                        f.name.clone()
                    }
                })
                .collect();
            format!("@({})", fs.join(", "))
        }
        TypeExpr::List(inner) => format!("@[{}]", format_type_expr(inner)),
        TypeExpr::Generic(name, params) => {
            let ps: Vec<String> = params.iter().map(format_type_expr).collect();
            format!("{}[{}]", name, ps.join(", "))
        }
        TypeExpr::Function(args, ret) => {
            let as_: Vec<String> = args.iter().map(format_type_expr).collect();
            format!("({}) => :{}", as_.join(", "), format_type_expr(ret))
        }
    }
}

/// Format a MoldHeaderArg for display.
pub fn format_mold_header_arg(arg: &MoldHeaderArg) -> String {
    match arg {
        MoldHeaderArg::TypeParam(tp) => match &tp.constraint {
            Some(constraint) => format!("{} <= :{}", tp.name, format_type_expr(constraint)),
            None => tp.name.clone(),
        },
        MoldHeaderArg::Concrete(ty) => format!(":{}", format_type_expr(ty)),
    }
}

/// Format the `[args]` suffix for a mold header.
pub fn format_mold_header_suffix(args: &[MoldHeaderArg]) -> String {
    if args.is_empty() {
        String::new()
    } else {
        format!(
            "[{}]",
            args.iter()
                .map(format_mold_header_arg)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Format a named mold header: `Name[args]`.
pub fn format_named_mold_header(name: &str, args: &[MoldHeaderArg]) -> String {
    format!("{}{}", name, format_mold_header_suffix(args))
}

/// Format registered type fields for display (indented, one per line).
pub fn format_registered_fields(fields: &[(String, Type)]) -> String {
    fields
        .iter()
        .map(|(name, ty)| format!("  {}: {}", name, ty))
        .collect::<Vec<_>>()
        .join(",\n")
}

/// Format registered type fields for inline display (comma-separated).
pub fn format_registry_fields_inline(fields: &[(String, Type)]) -> String {
    fields
        .iter()
        .map(|(name, ty)| format!("{}: {}", name, ty))
        .collect::<Vec<_>>()
        .join(", ")
}
