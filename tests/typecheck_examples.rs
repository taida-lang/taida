/// Temporary integration test: verify all examples pass type checking.
/// This test ensures that integrating the type checker into main.rs
/// won't break any existing example files.
use std::fs;

fn check_file(path: &str) -> Vec<String> {
    let source = fs::read_to_string(path).unwrap();
    let (program, parse_errors) = taida::parser::parse(&source);
    if !parse_errors.is_empty() {
        return parse_errors
            .iter()
            .map(|e| format!("PARSE: {}", e))
            .collect();
    }
    let mut checker = taida::types::TypeChecker::new();
    checker.check_program(&program);
    checker.errors.iter().map(|e| format!("{}", e)).collect()
}

#[test]
fn test_all_examples_pass_typecheck() {
    let examples_dir = "examples";
    let mut failures = Vec::new();
    for entry in fs::read_dir(examples_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "td") {
            let errors = check_file(path.to_str().unwrap());
            if !errors.is_empty() {
                failures.push((path.display().to_string(), errors));
            }
        }
    }
    if !failures.is_empty() {
        let mut msg = String::new();
        for (file, errors) in &failures {
            msg.push_str(&format!("\n=== {} ===\n", file));
            for err in errors {
                msg.push_str(&format!("  {}\n", err));
            }
        }
        panic!("{} example files had type errors:{}", failures.len(), msg);
    }
}

// ── C-11b: docs 由来の否定例（checker がエラーを出すべきケース）──

fn check_source(source: &str) -> Vec<String> {
    let (program, parse_errors) = taida::parser::parse(source);
    if !parse_errors.is_empty() {
        return parse_errors
            .iter()
            .map(|e| format!("PARSE: {}", e))
            .collect();
    }
    let mut checker = taida::types::TypeChecker::new();
    checker.check_program(&program);
    checker.errors.iter().map(|e| format!("{}", e)).collect()
}

#[test]
fn test_negative_same_scope_redefinition() {
    // docs: same-scope redefinition is forbidden
    let errors = check_source("x <= 1\nx <= 2");
    assert!(
        errors.iter().any(|e| e.contains("[E1501]")),
        "Expected E1501, got: {:?}",
        errors
    );
}

#[test]
fn test_negative_function_overload() {
    // docs: function overloading is disallowed
    let errors = check_source("f x: Int =\n  x\n=> :Int\nf x: Str =\n  x\n=> :Str");
    assert!(
        errors.iter().any(|e| e.contains("[E1501]")),
        "Expected E1501, got: {:?}",
        errors
    );
}

#[test]
fn test_negative_old_placeholder_partial() {
    // docs: old `_` partial application is rejected
    let errors = check_source("add x y = x + y\n=> :Int\nresult <= add(5, _)");
    assert!(
        errors.iter().any(|e| e.contains("[E1502]")),
        "Expected E1502, got: {:?}",
        errors
    );
}

#[test]
fn test_negative_typedef_partial_application() {
    // docs: TypeDef partial application is not supported
    let errors = check_source("Point = @(x: Int, y: Int)\np <= Point(1, )");
    assert!(
        errors.iter().any(|e| e.contains("[E1503]")),
        "Expected E1503, got: {:?}",
        errors
    );
}

#[test]
fn test_negative_mold_placeholder_outside_pipeline() {
    // docs: Mold[_]() outside pipeline is rejected
    let errors = check_source("x <= Str[_]()");
    assert!(
        errors.iter().any(|e| e.contains("[E1504]")),
        "Expected E1504, got: {:?}",
        errors
    );
}
