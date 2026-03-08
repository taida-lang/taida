/// Hover information provider for Taida Lang LSP.
///
/// Provides type information and documentation when hovering over:
/// - Variables: type information
/// - Functions: signature (parameters + return type) + doc_comments
/// - Types: field list + doc_comments
/// - Molds: type parameters + doc_comments
/// - Expressions: inferred type
use tower_lsp::lsp_types::Position;

use crate::parser::{Expr, Statement, TypeExpr, parse};
use crate::types::TypeChecker;

/// Format a TypeExpr as a readable string for hover display.
fn format_type_expr(te: &TypeExpr) -> String {
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

/// Format doc_comments into a markdown string.
fn format_doc_comments(doc_comments: &[String]) -> String {
    if doc_comments.is_empty() {
        return String::new();
    }
    format!("\n\n---\n\n{}", doc_comments.join("\n"))
}

/// Get hover information for a position in the source.
/// Returns a markdown string with type info, or None if no info available.
pub fn get_hover_info(source: &str, position: Position) -> Option<String> {
    let (program, parse_errors) = parse(source);
    if !parse_errors.is_empty() {
        return None;
    }

    // Run type checker to populate scope info
    let mut checker = TypeChecker::new();
    checker.check_program(&program);

    // Find the identifier at the given position
    let target_line = position.line as usize + 1; // Span uses 1-based
    let target_col = position.character as usize + 1;

    // Walk the AST to find the identifier at this position
    for stmt in &program.statements {
        if let Some(info) =
            find_hover_in_statement(stmt, target_line, target_col, &checker, &program.statements)
        {
            return Some(info);
        }
    }

    None
}

/// Search a statement for hover-relevant nodes at the given position.
fn find_hover_in_statement(
    stmt: &Statement,
    line: usize,
    col: usize,
    checker: &TypeChecker,
    all_stmts: &[Statement],
) -> Option<String> {
    let _ = all_stmts;
    match stmt {
        Statement::Assignment(assign) => {
            // Check if cursor is on the variable name (target)
            if assign.span.line == line {
                // Check if cursor is on the value expression
                if let Some(info) = find_hover_in_expr(&assign.value, line, col, checker) {
                    return Some(info);
                }
                // Check if cursor is near the target name
                let var_type = checker.lookup_var(&assign.target);
                if let Some(ty) = var_type
                    && assign.span.column <= col
                    && col <= assign.span.column + assign.target.len()
                {
                    return Some(format!("```taida\n{}: {}\n```", assign.target, ty));
                }
            }
            None
        }
        Statement::FuncDef(fd) => {
            // Check if cursor is on the function name
            if fd.span.line == line {
                let ret_type = fd
                    .return_type
                    .as_ref()
                    .map(format_type_expr)
                    .unwrap_or_else(|| "Unknown".to_string());
                let params: Vec<String> = fd
                    .params
                    .iter()
                    .map(|p| {
                        if let Some(ann) = &p.type_annotation {
                            format!("{}: {}", p.name, format_type_expr(ann))
                        } else {
                            p.name.clone()
                        }
                    })
                    .collect();
                if fd.span.column <= col && col <= fd.span.column + fd.name.len() {
                    let doc = format_doc_comments(&fd.doc_comments);
                    return Some(format!(
                        "```taida\n{} {} => :{}\n```{}",
                        fd.name,
                        params.join(" "),
                        ret_type,
                        doc
                    ));
                }
            }
            // Search function body
            for body_stmt in &fd.body {
                if let Some(info) =
                    find_hover_in_statement(body_stmt, line, col, checker, all_stmts)
                {
                    return Some(info);
                }
            }
            None
        }
        Statement::TypeDef(td) => {
            // Check if cursor is on the type name
            if td.span.line == line
                && td.span.column <= col
                && col <= td.span.column + td.name.len()
            {
                let fields: Vec<String> = td
                    .fields
                    .iter()
                    .filter(|f| !f.is_method)
                    .map(|f| {
                        if let Some(ty) = &f.type_annotation {
                            format!("  {}: {}", f.name, format_type_expr(ty))
                        } else {
                            format!("  {}", f.name)
                        }
                    })
                    .collect();
                let doc = format_doc_comments(&td.doc_comments);
                return Some(format!(
                    "```taida\n{} = @(\n{}\n)\n```{}",
                    td.name,
                    fields.join(",\n"),
                    doc
                ));
            }
            None
        }
        Statement::MoldDef(md) => {
            // Check if cursor is on the mold name
            if md.span.line == line
                && md.span.column <= col
                && col <= md.span.column + md.name.len()
            {
                let type_params: Vec<String> =
                    md.type_params.iter().map(|tp| tp.name.clone()).collect();
                let fields: Vec<String> = md
                    .fields
                    .iter()
                    .filter(|f| !f.is_method)
                    .map(|f| {
                        if let Some(ty) = &f.type_annotation {
                            format!("  {}: {}", f.name, format_type_expr(ty))
                        } else {
                            format!("  {}", f.name)
                        }
                    })
                    .collect();
                let doc = format_doc_comments(&md.doc_comments);
                return Some(format!(
                    "```taida\nMold[{}] => {}[{}] = @(\n{}\n)\n```{}",
                    type_params.join(", "),
                    md.name,
                    type_params.join(", "),
                    fields.join(",\n"),
                    doc
                ));
            }
            None
        }
        Statement::InheritanceDef(inh) => {
            // Check if cursor is on the child type name
            if inh.span.line == line {
                // Check child name position (appears after "Parent => ")
                let fields: Vec<String> = inh
                    .fields
                    .iter()
                    .filter(|f| !f.is_method)
                    .map(|f| {
                        if let Some(ty) = &f.type_annotation {
                            format!("  {}: {}", f.name, format_type_expr(ty))
                        } else {
                            format!("  {}", f.name)
                        }
                    })
                    .collect();
                let parent_fields = checker
                    .registry
                    .get_type_fields(&inh.parent)
                    .unwrap_or_default();
                let parent_fields_str: Vec<String> = parent_fields
                    .iter()
                    .map(|(n, t)| format!("  {}: {} (inherited)", n, t))
                    .collect();

                let mut all_fields = parent_fields_str;
                all_fields.extend(fields);

                let doc = format_doc_comments(&inh.doc_comments);
                return Some(format!(
                    "```taida\n{} => {} = @(\n{}\n)\n```{}",
                    inh.parent,
                    inh.child,
                    all_fields.join(",\n"),
                    doc
                ));
            }
            None
        }
        Statement::Expr(expr) => find_hover_in_expr(expr, line, col, checker),
        Statement::ErrorCeiling(ec) => {
            for body_stmt in &ec.handler_body {
                if let Some(info) =
                    find_hover_in_statement(body_stmt, line, col, checker, all_stmts)
                {
                    return Some(info);
                }
            }
            None
        }
        _ => None,
    }
}

/// Search an expression for hover-relevant nodes at the given position.
fn find_hover_in_expr(
    expr: &Expr,
    line: usize,
    col: usize,
    checker: &TypeChecker,
) -> Option<String> {
    match expr {
        Expr::Ident(name, span) => {
            if span.line == line && span.column <= col && col < span.column + name.len() {
                let var_type = checker.lookup_var(name);
                if let Some(ty) = var_type {
                    return Some(format!("```taida\n{}: {}\n```", name, ty));
                }
            }
            None
        }
        Expr::MethodCall(obj, method, _args, span) => {
            if span.line == line {
                // If hovering on the method name, show method info
                let obj_type = {
                    let mut tc = TypeChecker::new();
                    tc.infer_expr_type(obj)
                };
                return Some(format!(
                    "```taida\n{}.{}\nReceiver type: {}\n```",
                    obj_type, method, obj_type
                ));
            }
            find_hover_in_expr(obj, line, col, checker)
        }
        Expr::FuncCall(func, args, span) => {
            if let Some(info) = find_hover_in_expr(func, line, col, checker) {
                return Some(info);
            }
            for arg in args {
                if let Some(info) = find_hover_in_expr(arg, line, col, checker) {
                    return Some(info);
                }
            }
            // Show return type on function name
            if span.line == line
                && let Expr::Ident(name, _) = func.as_ref()
            {
                let mut tc = TypeChecker::new();
                let ret = tc.infer_expr_type(expr);
                return Some(format!("```taida\n{}(...) => :{}\n```", name, ret));
            }
            None
        }
        Expr::BinaryOp(left, _op, right, _span) => {
            if let Some(info) = find_hover_in_expr(left, line, col, checker) {
                return Some(info);
            }
            find_hover_in_expr(right, line, col, checker)
        }
        Expr::FieldAccess(obj, field, span) => {
            if span.line == line {
                let obj_type = {
                    let mut tc = TypeChecker::new();
                    tc.infer_expr_type(obj)
                };
                // Try to get the field type from the registry
                let field_type = match &obj_type {
                    crate::types::Type::Named(name) => {
                        checker.registry.get_type_fields(name).and_then(|fields| {
                            fields
                                .iter()
                                .find(|(n, _)| n == field)
                                .map(|(_, t)| format!("{}", t))
                        })
                    }
                    crate::types::Type::BuchiPack(fields) => fields
                        .iter()
                        .find(|(n, _)| n == field)
                        .map(|(_, t)| format!("{}", t)),
                    _ => None,
                };
                let type_info = field_type.unwrap_or_else(|| format!("{}", obj_type));
                return Some(format!(
                    "```taida\n.{}: {}\nObject type: {}\n```",
                    field, type_info, obj_type
                ));
            }
            find_hover_in_expr(obj, line, col, checker)
        }
        Expr::MoldInst(name, _type_args, _fields, span) => {
            if span.line == line && span.column <= col && col < span.column + name.len() {
                // Look up mold definition
                if let Some((type_params, fields)) = checker.registry.mold_defs.get(name) {
                    let tp_str = type_params.join(", ");
                    let fields_str: Vec<String> = fields
                        .iter()
                        .map(|(n, t)| format!("{}: {}", n, t))
                        .collect();
                    return Some(format!(
                        "```taida\nMold[{}] => {}[{}] = @({})\n```",
                        tp_str,
                        name,
                        tp_str,
                        fields_str.join(", ")
                    ));
                }
                // Built-in mold
                return Some(format!("```taida\n{}[...]\n```", name));
            }
            None
        }
        Expr::TypeInst(name, _fields, span) => {
            if span.line == line
                && span.column <= col
                && col < span.column + name.len()
                && let Some(fields) = checker.registry.get_type_fields(name)
            {
                let fields_str: Vec<String> = fields
                    .iter()
                    .map(|(n, t)| format!("{}: {}", n, t))
                    .collect();
                return Some(format!(
                    "```taida\n{} = @({})\n```",
                    name,
                    fields_str.join(", ")
                ));
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hover_variable_type() {
        let source = "x <= 42";
        // x is at line 1, col 1 (1-based in Span)
        // LSP Position is 0-based
        let result = get_hover_info(
            source,
            Position {
                line: 0,
                character: 0,
            },
        );
        assert!(result.is_some(), "Should get hover info for variable x");
        let info = result.unwrap();
        assert!(info.contains("x"), "Should contain variable name");
        assert!(info.contains("Int"), "Should contain type Int");
    }

    #[test]
    fn test_hover_function_def() {
        let source = "add a b = a + b => :Int";
        let result = get_hover_info(
            source,
            Position {
                line: 0,
                character: 0,
            },
        );
        assert!(result.is_some(), "Should get hover info for function add");
        let info = result.unwrap();
        assert!(info.contains("add"), "Should contain function name");
        assert!(info.contains("Int"), "Should contain return type");
    }

    #[test]
    fn test_hover_type_def() {
        let source = "Person = @(name: Str, age: Int)";
        let result = get_hover_info(
            source,
            Position {
                line: 0,
                character: 0,
            },
        );
        assert!(result.is_some(), "Should get hover info for type Person");
        let info = result.unwrap();
        assert!(info.contains("Person"), "Should contain type name");
        assert!(info.contains("name"), "Should contain field name");
        assert!(info.contains("age"), "Should contain field age");
    }

    #[test]
    fn test_hover_with_doc_comments() {
        let source = "///@ A person type\nPerson = @(name: Str, age: Int)";
        let result = get_hover_info(
            source,
            Position {
                line: 1,
                character: 0,
            },
        );
        assert!(
            result.is_some(),
            "Should get hover info for documented type"
        );
        let info = result.unwrap();
        assert!(info.contains("Person"), "Should contain type name");
        assert!(info.contains("A person type"), "Should contain doc comment");
    }

    #[test]
    fn test_hover_no_info_on_whitespace() {
        let source = "x <= 42";
        // Position far beyond the statement
        let result = get_hover_info(
            source,
            Position {
                line: 5,
                character: 0,
            },
        );
        assert!(
            result.is_none(),
            "Should return None for position with no code"
        );
    }

    #[test]
    fn test_hover_parse_error_returns_none() {
        let source = "this is not valid taida @@@";
        let result = get_hover_info(
            source,
            Position {
                line: 0,
                character: 0,
            },
        );
        assert!(result.is_none(), "Should return None on parse error");
    }

    #[test]
    fn test_format_type_expr_named() {
        let te = TypeExpr::Named("Int".to_string());
        assert_eq!(format_type_expr(&te), "Int");
    }

    #[test]
    fn test_format_type_expr_list() {
        let te = TypeExpr::List(Box::new(TypeExpr::Named("Str".to_string())));
        assert_eq!(format_type_expr(&te), "@[Str]");
    }

    #[test]
    fn test_format_type_expr_generic() {
        let te = TypeExpr::Generic("Lax".to_string(), vec![TypeExpr::Named("Int".to_string())]);
        assert_eq!(format_type_expr(&te), "Lax[Int]");
    }
}
