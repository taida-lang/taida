/// std/json — JSON serialization/deserialization for Taida Lang.
///
/// Converts between Taida values and JSON strings.
/// Uses serde_json for parsing/generation.
///
/// JSON Molten Iron: JSON is an opaque primitive ("molten iron").
/// To use JSON data in Taida, it must be cast through a schema (TypeDef)
/// using `JSON[raw, Schema]()`. Direct manipulation of JSON values is prohibited.
use serde_json;

use crate::interpreter::value::Value;
use crate::parser::FieldDef;

/// Convert a serde_json::Value to a Taida Value (deep conversion).
/// Used only in tests; production code uses JSON[raw, Schema]() schema matching.
#[cfg(test)]
pub fn json_to_taida_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Str(String::new()), // No null in Taida
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Int(0)
            }
        }
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Array(arr) => Value::List(arr.iter().map(json_to_taida_value).collect()),
        serde_json::Value::Object(obj) => {
            let fields: Vec<(String, Value)> = obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_taida_value(v)))
                .collect();
            Value::BuchiPack(fields)
        }
    }
}

/// Convert a Taida Value to a serde_json::Value.
pub fn taida_value_to_json(val: &Value) -> serde_json::Value {
    match val {
        Value::Int(n) => serde_json::Value::Number(serde_json::Number::from(*n)),
        Value::Float(n) => {
            if let Some(num) = serde_json::Number::from_f64(*n) {
                serde_json::Value::Number(num)
            } else {
                serde_json::Value::Null
            }
        }
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::List(items) => {
            serde_json::Value::Array(items.iter().map(taida_value_to_json).collect())
        }
        Value::BuchiPack(fields) => {
            let mut map = serde_json::Map::new();
            for (field_name, field_val) in fields {
                // Skip __type field — it's internal metadata, not user data
                if field_name == "__type" {
                    continue;
                }
                map.insert(field_name.clone(), taida_value_to_json(field_val));
            }
            serde_json::Value::Object(map)
        }
        Value::Unit => serde_json::Value::Object(serde_json::Map::new()),
        Value::Json(j) => j.clone(),
        _ => serde_json::Value::Null,
    }
}

// ── JSON Molten Iron: Schema-based casting ─────────────────────────

/// Schema for JSON casting.
/// Describes the target type that JSON data should be cast into.
#[derive(Debug, Clone)]
pub enum JsonSchema {
    /// Primitive type: Int, Str, Float, Bool
    Primitive(PrimitiveType),
    /// TypeDef: named type with fields
    TypeDef(String, Vec<SchemaField>),
    /// List of a schema type: @[Schema]
    List(Box<JsonSchema>),
}

/// Primitive type for JSON schema matching.
#[derive(Debug, Clone)]
pub enum PrimitiveType {
    Int,
    Str,
    Float,
    Bool,
}

/// A field in a TypeDef schema.
#[derive(Debug, Clone)]
pub struct SchemaField {
    pub name: String,
    pub schema: JsonSchema,
}

/// Build a JsonSchema from a TypeDef's field definitions, resolving nested types.
/// `type_defs` maps type names to their field definitions.
pub fn build_schema_from_typedef(
    type_name: &str,
    fields: &[FieldDef],
    type_defs: &std::collections::HashMap<String, Vec<FieldDef>>,
) -> JsonSchema {
    let schema_fields: Vec<SchemaField> = fields
        .iter()
        .filter(|f| !f.is_method)
        .map(|f| {
            let schema = match &f.type_annotation {
                Some(type_expr) => type_expr_to_schema(type_expr, type_defs),
                None => JsonSchema::Primitive(PrimitiveType::Str), // default to Str
            };
            SchemaField {
                name: f.name.clone(),
                schema,
            }
        })
        .collect();
    JsonSchema::TypeDef(type_name.to_string(), schema_fields)
}

/// Convert a TypeExpr to a JsonSchema.
fn type_expr_to_schema(
    type_expr: &crate::parser::TypeExpr,
    type_defs: &std::collections::HashMap<String, Vec<FieldDef>>,
) -> JsonSchema {
    match type_expr {
        crate::parser::TypeExpr::Named(name) => match name.as_str() {
            "Int" => JsonSchema::Primitive(PrimitiveType::Int),
            "Str" => JsonSchema::Primitive(PrimitiveType::Str),
            "Float" => JsonSchema::Primitive(PrimitiveType::Float),
            "Bool" => JsonSchema::Primitive(PrimitiveType::Bool),
            other => {
                // Look up TypeDef
                if let Some(fields) = type_defs.get(other) {
                    build_schema_from_typedef(other, fields, type_defs)
                } else {
                    // Unknown type: default to Str
                    JsonSchema::Primitive(PrimitiveType::Str)
                }
            }
        },
        crate::parser::TypeExpr::List(inner) => {
            JsonSchema::List(Box::new(type_expr_to_schema(inner, type_defs)))
        }
        crate::parser::TypeExpr::BuchiPack(fields) => {
            // Inline buchi pack type: @(field: Type, ...)
            let schema_fields: Vec<SchemaField> = fields
                .iter()
                .filter(|f| !f.is_method)
                .map(|f| {
                    let schema = match &f.type_annotation {
                        Some(te) => type_expr_to_schema(te, type_defs),
                        None => JsonSchema::Primitive(PrimitiveType::Str),
                    };
                    SchemaField {
                        name: f.name.clone(),
                        schema,
                    }
                })
                .collect();
            JsonSchema::TypeDef("BuchiPack".to_string(), schema_fields)
        }
        _ => JsonSchema::Primitive(PrimitiveType::Str),
    }
}

/// Cast a JSON value into a Taida value according to a schema.
///
/// Schema matching rules:
/// 1. Field match: only schema fields are extracted, extras ignored
/// 2. Missing field: default value for the field's type
/// 3. Type mismatch: default value
/// 4. null: default value (null exclusion philosophy)
/// 5. Nested: recursive matching
/// 6. List: each element matched against element schema
pub fn json_to_typed_value(json: &serde_json::Value, schema: &JsonSchema) -> Value {
    match schema {
        JsonSchema::Primitive(prim) => json_to_primitive(json, prim),
        JsonSchema::TypeDef(type_name, schema_fields) => {
            match json {
                serde_json::Value::Object(obj) => {
                    let mut fields: Vec<(String, Value)> = Vec::new();
                    for sf in schema_fields {
                        let value = if let Some(json_val) = obj.get(&sf.name) {
                            if json_val.is_null() {
                                default_for_schema(&sf.schema)
                            } else {
                                json_to_typed_value(json_val, &sf.schema)
                            }
                        } else {
                            default_for_schema(&sf.schema)
                        };
                        fields.push((sf.name.clone(), value));
                    }
                    fields.push(("__type".to_string(), Value::Str(type_name.clone())));
                    Value::BuchiPack(fields)
                }
                serde_json::Value::Null => {
                    // null -> all defaults
                    let mut fields: Vec<(String, Value)> = Vec::new();
                    for sf in schema_fields {
                        fields.push((sf.name.clone(), default_for_schema(&sf.schema)));
                    }
                    fields.push(("__type".to_string(), Value::Str(type_name.clone())));
                    Value::BuchiPack(fields)
                }
                _ => {
                    // Non-object -> all defaults
                    let mut fields: Vec<(String, Value)> = Vec::new();
                    for sf in schema_fields {
                        fields.push((sf.name.clone(), default_for_schema(&sf.schema)));
                    }
                    fields.push(("__type".to_string(), Value::Str(type_name.clone())));
                    Value::BuchiPack(fields)
                }
            }
        }
        JsonSchema::List(elem_schema) => match json {
            serde_json::Value::Array(arr) => {
                let items: Vec<Value> = arr
                    .iter()
                    .map(|elem| json_to_typed_value(elem, elem_schema))
                    .collect();
                Value::List(items)
            }
            serde_json::Value::Null => Value::List(Vec::new()),
            _ => Value::List(Vec::new()),
        },
    }
}

/// Convert a JSON value to a primitive Taida value.
///
/// Philosophy I: "null/undefined の完全排除 — 全ての型にデフォルト値を保証"
/// Parse failures and type mismatches silently fall back to the type's default
/// value (0, 0.0, "", false). This is intentional per Taida's null-exclusion
/// philosophy, though it means parse errors are indistinguishable from legitimate
/// zero/empty values.
fn json_to_primitive(json: &serde_json::Value, prim: &PrimitiveType) -> Value {
    match prim {
        PrimitiveType::Int => match json {
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else if let Some(f) = n.as_f64() {
                    Value::Int(f as i64)
                } else {
                    Value::Int(0)
                }
            }
            serde_json::Value::String(s) => {
                s.parse::<i64>().map(Value::Int).unwrap_or(Value::Int(0))
            }
            serde_json::Value::Bool(b) => Value::Int(if *b { 1 } else { 0 }),
            _ => Value::Int(0),
        },
        PrimitiveType::Float => match json {
            serde_json::Value::Number(n) => Value::Float(n.as_f64().unwrap_or(0.0)),
            serde_json::Value::String(s) => s
                .parse::<f64>()
                .map(Value::Float)
                .unwrap_or(Value::Float(0.0)),
            serde_json::Value::Bool(b) => Value::Float(if *b { 1.0 } else { 0.0 }),
            _ => Value::Float(0.0),
        },
        PrimitiveType::Str => match json {
            serde_json::Value::String(s) => Value::Str(s.clone()),
            serde_json::Value::Number(n) => Value::Str(format!("{}", n)),
            serde_json::Value::Bool(b) => Value::Str(format!("{}", b)),
            serde_json::Value::Null => Value::Str(String::new()),
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                Value::Str(serde_json::to_string(json).unwrap_or_default())
            }
        },
        PrimitiveType::Bool => match json {
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => Value::Bool(n.as_f64().is_some_and(|f| f != 0.0)),
            serde_json::Value::String(s) => Value::Bool(!s.is_empty()),
            serde_json::Value::Null => Value::Bool(false),
            _ => Value::Bool(false),
        },
    }
}

/// Get the default value for a schema type.
pub fn default_for_schema(schema: &JsonSchema) -> Value {
    match schema {
        JsonSchema::Primitive(PrimitiveType::Int) => Value::Int(0),
        JsonSchema::Primitive(PrimitiveType::Float) => Value::Float(0.0),
        JsonSchema::Primitive(PrimitiveType::Str) => Value::Str(String::new()),
        JsonSchema::Primitive(PrimitiveType::Bool) => Value::Bool(false),
        JsonSchema::TypeDef(type_name, fields) => {
            let mut result_fields: Vec<(String, Value)> = fields
                .iter()
                .map(|f| (f.name.clone(), default_for_schema(&f.schema)))
                .collect();
            result_fields.push(("__type".to_string(), Value::Str(type_name.clone())));
            Value::BuchiPack(result_fields)
        }
        JsonSchema::List(_) => Value::List(Vec::new()),
    }
}

/// jsonEncode(value) -> Str
/// Converts a Taida value to a compact JSON string.
pub fn stdlib_json_encode(args: &[Value]) -> Result<Value, String> {
    let val = args.first().unwrap_or(&Value::Unit);
    match val {
        Value::Json(j) => Ok(Value::Str(serde_json::to_string(j).unwrap_or_default())),
        _ => {
            let json = taida_value_to_json(val);
            Ok(Value::Str(serde_json::to_string(&json).unwrap_or_default()))
        }
    }
}

/// jsonPretty(value) -> Str
/// Converts a Taida value to a pretty-printed JSON string.
pub fn stdlib_json_pretty(args: &[Value]) -> Result<Value, String> {
    let val = args.first().unwrap_or(&Value::Unit);
    let json = taida_value_to_json(val);
    Ok(Value::Str(
        serde_json::to_string_pretty(&json).unwrap_or_default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_encode_buchi_pack() {
        let pack = Value::BuchiPack(vec![
            ("name".to_string(), Value::Str("Alice".to_string())),
            ("age".to_string(), Value::Int(30)),
        ]);
        let result = stdlib_json_encode(&[pack]).unwrap();
        let Value::Str(s) = result else {
            unreachable!("Expected Str from json_encode");
        };
        assert!(s.contains("\"name\":\"Alice\""));
        assert!(s.contains("\"age\":30"));
    }

    #[test]
    fn test_json_pretty() {
        let pack = Value::BuchiPack(vec![("x".to_string(), Value::Int(1))]);
        let result = stdlib_json_pretty(&[pack]).unwrap();
        let Value::Str(s) = result else {
            unreachable!("Expected Str from json_pretty");
        };
        assert!(s.contains('\n')); // Pretty printed should have newlines
        assert!(s.contains("\"x\": 1"));
    }

    #[test]
    fn test_json_encode_accepts_json_value() {
        let json = Value::Json(serde_json::json!({"x": 42}));
        let result = stdlib_json_encode(&[json]).unwrap();
        let Value::Str(s) = result else {
            unreachable!("Expected Str from json_encode");
        };
        assert!(s.contains("\"x\":42") || s.contains("\"x\": 42"));
    }

    #[test]
    fn test_json_to_taida_value_deep_conversion() {
        let json = serde_json::json!({
            "name": "Alice",
            "scores": [100, 95],
            "active": true,
            "data": null
        });
        let result = json_to_taida_value(&json);
        let Value::BuchiPack(fields) = result else {
            unreachable!("Expected BuchiPack from json_to_taida_value");
        };
        assert!(
            fields
                .iter()
                .any(|(k, v)| k == "name" && *v == Value::Str("Alice".to_string()))
        );
        assert!(
            fields
                .iter()
                .any(|(k, v)| k == "data" && *v == Value::Str(String::new()))
        );
    }

    #[test]
    fn test_taida_value_to_json_with_json_value() {
        let json_val = serde_json::json!({"a": 1});
        let val = Value::Json(json_val.clone());
        let result = taida_value_to_json(&val);
        assert_eq!(result, json_val);
    }

    // ── Molten Iron: schema-based casting tests ─────────────

    #[test]
    fn test_schema_primitive_int() {
        let schema = JsonSchema::Primitive(PrimitiveType::Int);
        assert_eq!(
            json_to_typed_value(&serde_json::json!(42), &schema),
            Value::Int(42)
        );
        assert_eq!(
            json_to_typed_value(&serde_json::json!("not a number"), &schema),
            Value::Int(0)
        );
        assert_eq!(
            json_to_typed_value(&serde_json::json!(null), &schema),
            Value::Int(0)
        );
    }

    #[test]
    fn test_schema_primitive_str() {
        let schema = JsonSchema::Primitive(PrimitiveType::Str);
        assert_eq!(
            json_to_typed_value(&serde_json::json!("hello"), &schema),
            Value::Str("hello".to_string())
        );
        assert_eq!(
            json_to_typed_value(&serde_json::json!(null), &schema),
            Value::Str(String::new())
        );
    }

    #[test]
    fn test_schema_typedef_basic() {
        let schema = JsonSchema::TypeDef(
            "User".to_string(),
            vec![
                SchemaField {
                    name: "name".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Str),
                },
                SchemaField {
                    name: "age".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Int),
                },
            ],
        );
        let json = serde_json::json!({"name": "Asuka", "age": 14, "extra": "ignored"});
        let result = json_to_typed_value(&json, &schema);
        let Value::BuchiPack(fields) = result else {
            unreachable!("Expected BuchiPack from schema typedef");
        };
        assert!(
            fields
                .iter()
                .any(|(k, v)| k == "name" && *v == Value::Str("Asuka".to_string()))
        );
        assert!(
            fields
                .iter()
                .any(|(k, v)| k == "age" && *v == Value::Int(14))
        );
        assert!(
            fields
                .iter()
                .any(|(k, v)| k == "__type" && *v == Value::Str("User".to_string()))
        );
        // "extra" should NOT be present
        assert!(!fields.iter().any(|(k, _)| k == "extra"));
    }

    #[test]
    fn test_schema_missing_fields_get_defaults() {
        let schema = JsonSchema::TypeDef(
            "User".to_string(),
            vec![
                SchemaField {
                    name: "name".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Str),
                },
                SchemaField {
                    name: "age".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Int),
                },
                SchemaField {
                    name: "email".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Str),
                },
            ],
        );
        let json = serde_json::json!({"name": "Asuka"});
        let result = json_to_typed_value(&json, &schema);
        let Value::BuchiPack(fields) = result else {
            unreachable!("Expected BuchiPack from schema missing fields");
        };
        assert_eq!(
            fields.iter().find(|(k, _)| k == "age").unwrap().1,
            Value::Int(0)
        );
        assert_eq!(
            fields.iter().find(|(k, _)| k == "email").unwrap().1,
            Value::Str(String::new())
        );
    }

    #[test]
    fn test_schema_type_mismatch_defaults() {
        let schema = JsonSchema::TypeDef(
            "User".to_string(),
            vec![SchemaField {
                name: "age".to_string(),
                schema: JsonSchema::Primitive(PrimitiveType::Int),
            }],
        );
        let json = serde_json::json!({"age": "not a number"});
        let result = json_to_typed_value(&json, &schema);
        let Value::BuchiPack(fields) = result else {
            unreachable!("Expected BuchiPack from schema type mismatch");
        };
        assert_eq!(
            fields.iter().find(|(k, _)| k == "age").unwrap().1,
            Value::Int(0)
        );
    }

    #[test]
    fn test_schema_null_to_defaults() {
        let schema = JsonSchema::TypeDef(
            "User".to_string(),
            vec![
                SchemaField {
                    name: "name".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Str),
                },
                SchemaField {
                    name: "age".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Int),
                },
            ],
        );
        let json = serde_json::json!({"name": null, "age": null});
        let result = json_to_typed_value(&json, &schema);
        let Value::BuchiPack(fields) = result else {
            unreachable!("Expected BuchiPack from schema null defaults");
        };
        assert_eq!(
            fields.iter().find(|(k, _)| k == "name").unwrap().1,
            Value::Str(String::new())
        );
        assert_eq!(
            fields.iter().find(|(k, _)| k == "age").unwrap().1,
            Value::Int(0)
        );
    }

    #[test]
    fn test_schema_nested_typedef() {
        let schema = JsonSchema::TypeDef(
            "User".to_string(),
            vec![
                SchemaField {
                    name: "name".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Str),
                },
                SchemaField {
                    name: "address".to_string(),
                    schema: JsonSchema::TypeDef(
                        "Address".to_string(),
                        vec![
                            SchemaField {
                                name: "city".to_string(),
                                schema: JsonSchema::Primitive(PrimitiveType::Str),
                            },
                            SchemaField {
                                name: "zip".to_string(),
                                schema: JsonSchema::Primitive(PrimitiveType::Str),
                            },
                        ],
                    ),
                },
            ],
        );
        let json =
            serde_json::json!({"name": "Asuka", "address": {"city": "Tokyo-3", "zip": "999"}});
        let result = json_to_typed_value(&json, &schema);
        let Value::BuchiPack(fields) = result else {
            unreachable!("Expected BuchiPack from nested schema");
        };
        let Some((_, Value::BuchiPack(addr_fields))) =
            fields.iter().find(|(k, _)| k == "address")
        else {
            unreachable!("Expected address to be BuchiPack");
        };
        assert!(
            addr_fields
                .iter()
                .any(|(k, v)| k == "city" && *v == Value::Str("Tokyo-3".to_string()))
        );
    }

    #[test]
    fn test_schema_list() {
        let schema = JsonSchema::List(Box::new(JsonSchema::TypeDef(
            "Pilot".to_string(),
            vec![
                SchemaField {
                    name: "name".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Str),
                },
                SchemaField {
                    name: "syncRate".to_string(),
                    schema: JsonSchema::Primitive(PrimitiveType::Int),
                },
            ],
        )));
        let json = serde_json::json!([
            {"name": "Asuka", "syncRate": 78},
            {"name": "Rei", "syncRate": 65}
        ]);
        let result = json_to_typed_value(&json, &schema);
        let Value::List(items) = result else {
            unreachable!("Expected List from schema list");
        };
        assert_eq!(items.len(), 2);
        if let Value::BuchiPack(fields) = &items[0] {
            assert!(
                fields
                    .iter()
                    .any(|(k, v)| k == "name" && *v == Value::Str("Asuka".to_string()))
            );
        }
    }

    #[test]
    fn test_default_for_schema() {
        assert_eq!(
            default_for_schema(&JsonSchema::Primitive(PrimitiveType::Int)),
            Value::Int(0)
        );
        assert_eq!(
            default_for_schema(&JsonSchema::Primitive(PrimitiveType::Str)),
            Value::Str(String::new())
        );
        assert_eq!(
            default_for_schema(&JsonSchema::List(Box::new(JsonSchema::Primitive(
                PrimitiveType::Int
            )))),
            Value::List(Vec::new())
        );
    }
}
