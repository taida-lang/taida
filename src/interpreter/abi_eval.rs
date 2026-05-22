use super::eval::{Interpreter, RuntimeError, Signal};
use super::value::Value;
use crate::parser::Expr;

const ABI_SYMBOLS: &[&str] = &["text", "json", "bytes", "status", "header"];

pub(crate) fn abi_symbols() -> &'static [&'static str] {
    ABI_SYMBOLS
}

impl Interpreter {
    pub(crate) fn try_abi_func(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<Option<Signal>, RuntimeError> {
        let original_name = match self.env.get(name) {
            Some(Value::Str(tag)) if tag.starts_with("__abi_builtin_") => {
                tag["__abi_builtin_".len()..].to_string()
            }
            _ => return Ok(None),
        };

        match original_name.as_str() {
            "text" => {
                if args.len() != 1 {
                    return Err(RuntimeError {
                        message: format!("text requires exactly 1 argument, got {}", args.len()),
                    });
                }
                let body = self.eval_str_arg(&args[0], "text")?;
                Ok(Some(Signal::Value(abi_response(
                    200,
                    vec![(
                        "content-type".to_string(),
                        "text/plain; charset=utf-8".to_string(),
                    )],
                    body.into_bytes(),
                ))))
            }
            "json" => {
                if args.len() != 1 {
                    return Err(RuntimeError {
                        message: format!("json requires exactly 1 argument, got {}", args.len()),
                    });
                }
                let value = match self.eval_expr(&args[0])? {
                    Signal::Value(v) => v,
                    other => return Ok(Some(other)),
                };
                let json = match &value {
                    Value::Json(j) => j.clone(),
                    _ => crate::interpreter::json::taida_value_to_json_with_enum_defs(
                        &value,
                        &self.enum_defs,
                    ),
                };
                let body = serde_json::to_vec(&json).unwrap_or_default();
                Ok(Some(Signal::Value(abi_response(
                    200,
                    vec![("content-type".to_string(), "application/json".to_string())],
                    body,
                ))))
            }
            "bytes" => {
                if args.len() != 1 {
                    return Err(RuntimeError {
                        message: format!("bytes requires exactly 1 argument, got {}", args.len()),
                    });
                }
                let value = match self.eval_expr(&args[0])? {
                    Signal::Value(v) => v,
                    other => return Ok(Some(other)),
                };
                let body = match value {
                    Value::Bytes(bytes) => Value::bytes_take(bytes),
                    Value::Str(s) => Value::str_take(s).into_bytes(),
                    other => other.to_display_string().into_bytes(),
                };
                Ok(Some(Signal::Value(abi_response(
                    200,
                    vec![(
                        "content-type".to_string(),
                        "application/octet-stream".to_string(),
                    )],
                    body,
                ))))
            }
            "status" => {
                if args.len() != 2 {
                    return Err(RuntimeError {
                        message: format!("status requires exactly 2 arguments, got {}", args.len()),
                    });
                }
                let code = match self.eval_expr(&args[0])? {
                    Signal::Value(Value::Int(n)) => n,
                    Signal::Value(other) => {
                        return Err(RuntimeError {
                            message: format!(
                                "status: first argument must be Int, got {}",
                                Self::type_name_of(&other)
                            ),
                        });
                    }
                    other => return Ok(Some(other)),
                };
                let response = match self.eval_expr(&args[1])? {
                    Signal::Value(v) => v,
                    other => return Ok(Some(other)),
                };
                Ok(Some(Signal::Value(with_response_field(
                    response,
                    "status",
                    Value::Int(code),
                ))))
            }
            "header" => {
                if args.len() != 3 {
                    return Err(RuntimeError {
                        message: format!("header requires exactly 3 arguments, got {}", args.len()),
                    });
                }
                let key = self.eval_str_arg(&args[0], "header")?;
                let value = self.eval_str_arg(&args[1], "header")?;
                let response = match self.eval_expr(&args[2])? {
                    Signal::Value(v) => v,
                    other => return Ok(Some(other)),
                };
                Ok(Some(Signal::Value(add_response_header(
                    response, key, value,
                ))))
            }
            _ => Ok(None),
        }
    }

    fn eval_str_arg(&mut self, arg: &Expr, func: &str) -> Result<String, RuntimeError> {
        match self.eval_expr(arg)? {
            Signal::Value(Value::Str(s)) => Ok(Value::str_take(s)),
            Signal::Value(other) => Err(RuntimeError {
                message: format!(
                    "{}: argument must be Str, got {}",
                    func,
                    Self::type_name_of(&other)
                ),
            }),
            other => Err(RuntimeError {
                message: format!("{}: unexpected control signal {:?}", func, other),
            }),
        }
    }
}

fn abi_response(status: i64, headers: Vec<(String, String)>, body: Vec<u8>) -> Value {
    Value::pack(vec![
        ("status".to_string(), Value::Int(status)),
        ("headers".to_string(), header_map(headers)),
        ("body".to_string(), Value::bytes(body)),
    ])
}

fn header_map(headers: Vec<(String, String)>) -> Value {
    let entries = headers
        .into_iter()
        .map(|(key, value)| {
            Value::pack(vec![
                ("key".to_string(), Value::str(key)),
                ("value".to_string(), Value::str(value)),
            ])
        })
        .collect();
    Value::pack(vec![
        ("__entries".to_string(), Value::list(entries)),
        ("__type".to_string(), Value::str("HashMap".to_string())),
    ])
}

fn with_response_field(response: Value, field_name: &str, field_value: Value) -> Value {
    match response {
        Value::BuchiPack(fields) => {
            let mut out = Vec::with_capacity(fields.len().max(3));
            let mut replaced = false;
            for (name, value) in fields.iter() {
                if name == field_name {
                    out.push((name.clone(), field_value.clone()));
                    replaced = true;
                } else {
                    out.push((name.clone(), value.clone()));
                }
            }
            if !replaced {
                out.push((field_name.to_string(), field_value));
            }
            Value::pack(out)
        }
        _ => response,
    }
}

fn add_response_header(response: Value, key: String, value: String) -> Value {
    match response {
        Value::BuchiPack(fields) => {
            let mut out = Vec::with_capacity(fields.len().max(3));
            let mut replaced = false;
            for (name, field_value) in fields.iter() {
                if name == "headers" {
                    out.push((name.clone(), append_header(field_value, &key, &value)));
                    replaced = true;
                } else {
                    out.push((name.clone(), field_value.clone()));
                }
            }
            if !replaced {
                out.push((
                    "headers".to_string(),
                    header_map(vec![(key.clone(), value.clone())]),
                ));
            }
            Value::pack(out)
        }
        _ => response,
    }
}

fn append_header(headers: &Value, key: &str, value: &str) -> Value {
    if let Value::BuchiPack(fields) = headers
        && let Some(Value::List(entries)) = fields
            .iter()
            .find(|(name, _)| name == "__entries")
            .map(|(_, value)| value)
    {
        let mut next = entries.as_ref().clone();
        next.push(Value::pack(vec![
            ("key".to_string(), Value::str(key.to_string())),
            ("value".to_string(), Value::str(value.to_string())),
        ]));
        return Value::pack(vec![
            ("__entries".to_string(), Value::list(next)),
            ("__type".to_string(), Value::str("HashMap".to_string())),
        ]);
    }
    header_map(vec![(key.to_string(), value.to_string())])
}
