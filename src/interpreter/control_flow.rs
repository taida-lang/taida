use super::eval::{Interpreter, RuntimeError, Signal};
use super::value::Value;
/// Control flow evaluation for the Taida interpreter.
///
/// Contains `eval_pipeline_step`, `eval_binary_op`, and `eval_cond_branch`.
///
/// These are `impl Interpreter` methods split from eval.rs for maintainability.
use crate::parser::*;

impl Interpreter {
    /// Evaluate a single pipeline step, applying the current value.
    ///
    /// Pipeline semantics:
    /// - FuncCall with placeholders: substitute placeholders with current value
    /// - FuncCall without placeholders: pass current as first argument
    /// - MethodCall: evaluate with current as _ in scope
    /// - Any other expression: evaluate with _ bound to current in scope
    pub(crate) fn eval_pipeline_step(
        &mut self,
        step: &Expr,
        current: Value,
    ) -> Result<Signal, RuntimeError> {
        match step {
            Expr::FuncCall(callee, args, span) => {
                // Check if any arg is a Placeholder
                let has_placeholder = args.iter().any(|a| matches!(a, Expr::Placeholder(_)));

                if has_placeholder {
                    // Replace placeholders with current value in the args,
                    // then evaluate the call directly with the substituted args.
                    // We bind current to a temporary variable to avoid value_to_expr issues.
                    self.env.push_scope();
                    let pipe_val_name = "__pipe_current__";
                    self.env.define_force(pipe_val_name, current);

                    let mut new_args: Vec<Expr> = Vec::new();
                    let dummy_span = crate::lexer::Span::new(0, 0, 0, 0);
                    for arg in args {
                        if matches!(arg, Expr::Placeholder(_)) {
                            new_args
                                .push(Expr::Ident(pipe_val_name.to_string(), dummy_span.clone()));
                        } else {
                            new_args.push(arg.clone());
                        }
                    }

                    let new_call = Expr::FuncCall(callee.clone(), new_args, span.clone());
                    let result = self.eval_expr(&new_call);
                    self.env.pop_scope();
                    result
                } else {
                    // No placeholder — pass current as first argument
                    self.env.push_scope();
                    let pipe_val_name = "__pipe_current__";
                    self.env.define_force(pipe_val_name, current);

                    let dummy_span = crate::lexer::Span::new(0, 0, 0, 0);
                    let mut new_args = vec![Expr::Ident(pipe_val_name.to_string(), dummy_span)];
                    new_args.extend(args.iter().cloned());
                    let new_call = Expr::FuncCall(callee.clone(), new_args, span.clone());
                    let result = self.eval_expr(&new_call);
                    self.env.pop_scope();
                    result
                }
            }
            Expr::MethodCall(obj, method, args, span) => {
                // For method calls in pipeline, check for placeholder in obj
                if matches!(obj.as_ref(), Expr::Placeholder(_)) {
                    // Replace obj placeholder with current
                    self.env.push_scope();
                    let pipe_val_name = "__pipe_current__";
                    self.env.define_force(pipe_val_name, current);
                    let dummy_span = crate::lexer::Span::new(0, 0, 0, 0);
                    let new_call = Expr::MethodCall(
                        Box::new(Expr::Ident(pipe_val_name.to_string(), dummy_span)),
                        method.clone(),
                        args.clone(),
                        span.clone(),
                    );
                    let result = self.eval_expr(&new_call);
                    self.env.pop_scope();
                    result
                } else {
                    // Fallback: evaluate with _ bound to current
                    self.env.push_scope();
                    self.env.define_force("_", current);
                    let result = self.eval_expr(step);
                    self.env.pop_scope();
                    result
                }
            }
            Expr::MoldInst(name, type_args, fields, span) => {
                // MoldInst in pipeline: replace _ placeholders in type_args with current value
                let has_placeholder = type_args.iter().any(|a| matches!(a, Expr::Placeholder(_)));

                if has_placeholder {
                    self.env.push_scope();
                    let pipe_val_name = "__pipe_current__";
                    self.env.define_force(pipe_val_name, current);

                    let dummy_span = crate::lexer::Span::new(0, 0, 0, 0);
                    let new_type_args: Vec<Expr> = type_args
                        .iter()
                        .map(|arg| {
                            if matches!(arg, Expr::Placeholder(_)) {
                                Expr::Ident(pipe_val_name.to_string(), dummy_span.clone())
                            } else {
                                arg.clone()
                            }
                        })
                        .collect();

                    let new_expr =
                        Expr::MoldInst(name.clone(), new_type_args, fields.clone(), span.clone());
                    let result = self.eval_expr(&new_expr);
                    self.env.pop_scope();
                    result
                } else {
                    // No placeholder — insert current as first type arg
                    self.env.push_scope();
                    let pipe_val_name = "__pipe_current__";
                    self.env.define_force(pipe_val_name, current);

                    let dummy_span = crate::lexer::Span::new(0, 0, 0, 0);
                    let mut new_type_args =
                        vec![Expr::Ident(pipe_val_name.to_string(), dummy_span)];
                    new_type_args.extend(type_args.iter().cloned());

                    let new_expr =
                        Expr::MoldInst(name.clone(), new_type_args, fields.clone(), span.clone());
                    let result = self.eval_expr(&new_expr);
                    self.env.pop_scope();
                    result
                }
            }
            Expr::Ident(_, _) => {
                // Identifier in pipeline: evaluate it, and if it's a function, call it with current
                let step_val = match self.eval_expr(step)? {
                    Signal::Value(v) => v,
                    other => return Ok(other),
                };
                match step_val {
                    Value::Function(ref func) => {
                        // Call the function with current as the only argument
                        let args = vec![current];
                        self.call_function_with_values(func, &args)
                            .map(Signal::Value)
                    }
                    _ => {
                        // Not a function — just return the evaluated value
                        Ok(Signal::Value(step_val))
                    }
                }
            }
            _ => {
                // General case: bind _ to current and evaluate
                self.env.push_scope();
                self.env.define_force("_", current);
                let result = self.eval_expr(step);
                self.env.pop_scope();
                result
            }
        }
    }

    /// Evaluate a binary operation.
    pub(crate) fn eval_binary_op(
        &self,
        left: &Value,
        op: &BinOp,
        right: &Value,
    ) -> Result<Signal, RuntimeError> {
        match op {
            BinOp::Add => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Signal::Value(Value::Int(a + b))),
                (Value::Float(a), Value::Float(b)) => Ok(Signal::Value(Value::Float(a + b))),
                (Value::Int(a), Value::Float(b)) => Ok(Signal::Value(Value::Float(*a as f64 + b))),
                (Value::Float(a), Value::Int(b)) => Ok(Signal::Value(Value::Float(a + *b as f64))),
                (Value::Str(a), Value::Str(b)) => {
                    // String concatenation with + between two strings
                    Ok(Signal::Value(Value::Str(format!("{}{}", a, b))))
                }
                _ => Err(RuntimeError {
                    message: format!("Cannot add {} and {}", left, right),
                }),
            },
            BinOp::Sub => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Signal::Value(Value::Int(a - b))),
                (Value::Float(a), Value::Float(b)) => Ok(Signal::Value(Value::Float(a - b))),
                (Value::Int(a), Value::Float(b)) => Ok(Signal::Value(Value::Float(*a as f64 - b))),
                (Value::Float(a), Value::Int(b)) => Ok(Signal::Value(Value::Float(a - *b as f64))),
                _ => Err(RuntimeError {
                    message: format!("Cannot subtract {} and {}", left, right),
                }),
            },
            BinOp::Mul => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Signal::Value(Value::Int(a * b))),
                (Value::Float(a), Value::Float(b)) => Ok(Signal::Value(Value::Float(a * b))),
                (Value::Int(a), Value::Float(b)) => Ok(Signal::Value(Value::Float(*a as f64 * b))),
                (Value::Float(a), Value::Int(b)) => Ok(Signal::Value(Value::Float(a * *b as f64))),
                _ => Err(RuntimeError {
                    message: format!("Cannot multiply {} and {}", left, right),
                }),
            },
            // BinOp::Div and BinOp::Mod removed — use Div[x, y]() and Mod[x, y]() molds
            BinOp::Eq => Ok(Signal::Value(Value::Bool(left == right))),
            BinOp::NotEq => Ok(Signal::Value(Value::Bool(left != right))),
            BinOp::Lt => Ok(Signal::Value(Value::Bool(left < right))),
            BinOp::Gt => Ok(Signal::Value(Value::Bool(left > right))),
            BinOp::GtEq => Ok(Signal::Value(Value::Bool(left >= right))),
            BinOp::And => Ok(Signal::Value(Value::Bool(
                left.is_truthy() && right.is_truthy(),
            ))),
            BinOp::Or => Ok(Signal::Value(Value::Bool(
                left.is_truthy() || right.is_truthy(),
            ))),
            BinOp::Concat => {
                let left_str = left.to_display_string();
                let right_str = right.to_display_string();
                Ok(Signal::Value(Value::Str(format!(
                    "{}{}",
                    left_str, right_str
                ))))
            }
        }
    }

    /// Evaluate a condition branch.
    pub(crate) fn eval_cond_branch(&mut self, arms: &[CondArm]) -> Result<Signal, RuntimeError> {
        for arm in arms {
            match &arm.condition {
                Some(cond) => {
                    let cond_val = match self.eval_expr(cond)? {
                        Signal::Value(v) => v,
                        other => return Ok(other),
                    };
                    if cond_val.is_truthy() {
                        return self.eval_cond_arm_body(&arm.body);
                    }
                }
                None => {
                    // Default case (| _ |>)
                    return self.eval_cond_arm_body(&arm.body);
                }
            }
        }
        // No branch matched — return Unit
        Ok(Signal::Value(Value::Unit))
    }

    /// Evaluate a condition arm body (Vec<Statement>).
    /// Returns the value of the last expression statement.
    fn eval_cond_arm_body(&mut self, body: &[Statement]) -> Result<Signal, RuntimeError> {
        self.eval_statements(body)
    }
}
