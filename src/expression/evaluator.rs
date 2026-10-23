use serde_json::Value;
use std::collections::HashMap;

use super::parser::{BinOp, CompareOp, Expr, Parser};
use super::token::tokenize;

pub fn evaluate(input: &str, values: &HashMap<String, f64>) -> Result<f64, String> {
    let tokens = tokenize(input)?;
    let mut parser = Parser::new(tokens);
    let expr = parser.parse()?;
    eval_expr(&expr, values)
}

fn eval_expr(expr: &Expr, values: &HashMap<String, f64>) -> Result<f64, String> {
    match expr {
        Expr::Number(n) => Ok(*n),
        Expr::Register(name) => values
            .get(name)
            .copied()
            .ok_or_else(|| format!("Unknown register: '{name}'")),
        Expr::StringLit(_) => Err("String literal not supported in numeric evaluation".into()),
        Expr::JsonPath(_) => Err("JSON path not supported in numeric evaluation".into()),
        Expr::Func(name, args) => match name.as_str() {
            "sqrt" => {
                if args.len() != 1 {
                    return Err(format!("{name} expects 1 argument"));
                }
                let v = eval_expr(&args[0], values)?;
                Ok(v.sqrt())
            }
            "abs" => {
                if args.len() != 1 {
                    return Err(format!("{name} expects 1 argument"));
                }
                let v = eval_expr(&args[0], values)?;
                Ok(v.abs())
            }
            "round" => {
                if args.len() != 1 {
                    return Err(format!("{name} expects 1 argument"));
                }
                let v = eval_expr(&args[0], values)?;
                Ok(v.round())
            }
            "clamp" => {
                if args.len() != 3 {
                    return Err(format!("{name} expects 3 arguments"));
                }
                let v = eval_expr(&args[0], values)?;
                let min = eval_expr(&args[1], values)?;
                let max = eval_expr(&args[2], values)?;
                Ok(v.clamp(min, max))
            }
            "avg" => {
                if args.is_empty() {
                    return Err("avg expects at least 1 argument".into());
                }
                let sum: f64 = args
                    .iter()
                    .map(|a| eval_expr(a, values))
                    .collect::<Result<Vec<_>, _>>()?
                    .iter()
                    .sum();
                Ok(sum / args.len() as f64)
            }
            "if" => {
                if args.len() != 3 {
                    return Err("if expects 3 arguments".into());
                }
                let cond = eval_expr(&args[0], values)?;
                if cond != 0.0 {
                    eval_expr(&args[1], values)
                } else {
                    eval_expr(&args[2], values)
                }
            }
            _ => Err(format!("Unknown function: '{name}'")),
        },
        Expr::BinOp { op, left, right } => {
            let l = eval_expr(left, values)?;
            let r = eval_expr(right, values)?;
            Ok(match op {
                BinOp::Add => l + r,
                BinOp::Sub => l - r,
                BinOp::Mul => l * r,
                BinOp::Div => {
                    if r == 0.0 {
                        return Err("Division by zero".into());
                    }
                    l / r
                }
            })
        }
        Expr::UnaryMinus(inner) => Ok(-eval_expr(inner, values)?),
        Expr::Compare { .. } => Err("Comparison not supported in numeric evaluation".into()),
        Expr::LogicalAnd(_, _) | Expr::LogicalOr(_, _) => {
            Err("Logical operation not supported in numeric evaluation".into())
        }
    }
}

pub fn evaluate_json(input: &str, context: &Value) -> Result<Value, String> {
    let tokens = tokenize(input)?;
    let mut parser = Parser::new(tokens);
    let expr = parser.parse()?;
    eval_expr_json(&expr, context)
}

fn eval_expr_json(expr: &Expr, context: &Value) -> Result<Value, String> {
    match expr {
        Expr::Number(n) => Ok(Value::Number(
            serde_json::Number::from_f64(*n).unwrap_or_else(|| serde_json::Number::from(0)),
        )),
        Expr::StringLit(s) => Ok(Value::String(s.clone())),
        Expr::Register(name) => resolve_json_path(context, std::slice::from_ref(name)),
        Expr::JsonPath(path) => resolve_json_path(context, path),
        Expr::Func(name, args) => match name.as_str() {
            "sqrt" => {
                if args.len() != 1 {
                    return Err(format!("{name} expects 1 argument"));
                }
                let v = to_f64(&eval_expr_json(&args[0], context)?)?;
                Ok(Value::Number(
                    serde_json::Number::from_f64(v.sqrt())
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ))
            }
            "abs" => {
                if args.len() != 1 {
                    return Err(format!("{name} expects 1 argument"));
                }
                let v = to_f64(&eval_expr_json(&args[0], context)?)?;
                Ok(Value::Number(
                    serde_json::Number::from_f64(v.abs())
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ))
            }
            "round" => {
                if args.len() != 1 {
                    return Err(format!("{name} expects 1 argument"));
                }
                let v = to_f64(&eval_expr_json(&args[0], context)?)?;
                Ok(Value::Number(
                    serde_json::Number::from_f64(v.round())
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ))
            }
            "clamp" => {
                if args.len() != 3 {
                    return Err(format!("{name} expects 3 arguments"));
                }
                let v = to_f64(&eval_expr_json(&args[0], context)?)?;
                let min = to_f64(&eval_expr_json(&args[1], context)?)?;
                let max = to_f64(&eval_expr_json(&args[2], context)?)?;
                Ok(Value::Number(
                    serde_json::Number::from_f64(v.clamp(min, max))
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ))
            }
            "avg" => {
                if args.is_empty() {
                    return Err("avg expects at least 1 argument".into());
                }
                let sum: f64 = args
                    .iter()
                    .map(|a| eval_expr_json(a, context).and_then(|v| to_f64(&v)))
                    .collect::<Result<Vec<_>, _>>()?
                    .iter()
                    .sum();
                Ok(Value::Number(
                    serde_json::Number::from_f64(sum / args.len() as f64)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ))
            }
            "if" => {
                if args.len() != 3 {
                    return Err("if expects 3 arguments".into());
                }
                let cond = to_bool(&eval_expr_json(&args[0], context)?);
                if cond {
                    eval_expr_json(&args[1], context)
                } else {
                    eval_expr_json(&args[2], context)
                }
            }
            _ => Err(format!("Unknown function: '{name}'")),
        },
        Expr::BinOp { op, left, right } => {
            let l = to_f64(&eval_expr_json(left, context)?)?;
            let r = to_f64(&eval_expr_json(right, context)?)?;
            let result = match op {
                BinOp::Add => l + r,
                BinOp::Sub => l - r,
                BinOp::Mul => l * r,
                BinOp::Div => {
                    if r == 0.0 {
                        return Err("Division by zero".into());
                    }
                    l / r
                }
            };
            Ok(Value::Number(
                serde_json::Number::from_f64(result).unwrap_or_else(|| serde_json::Number::from(0)),
            ))
        }
        Expr::UnaryMinus(inner) => {
            let v = to_f64(&eval_expr_json(inner, context)?)?;
            Ok(Value::Number(
                serde_json::Number::from_f64(-v).unwrap_or_else(|| serde_json::Number::from(0)),
            ))
        }
        Expr::Compare { op, left, right } => {
            let l = eval_expr_json(left, context)?;
            let r = eval_expr_json(right, context)?;
            Ok(Value::Bool(compare_values(*op, &l, &r)))
        }
        Expr::LogicalAnd(left, right) => {
            let l = to_bool(&eval_expr_json(left, context)?);
            if !l {
                return Ok(Value::Bool(false));
            }
            let r = to_bool(&eval_expr_json(right, context)?);
            Ok(Value::Bool(r))
        }
        Expr::LogicalOr(left, right) => {
            let l = to_bool(&eval_expr_json(left, context)?);
            if l {
                return Ok(Value::Bool(true));
            }
            let r = to_bool(&eval_expr_json(right, context)?);
            Ok(Value::Bool(r))
        }
    }
}

fn resolve_json_path(context: &Value, path: &[String]) -> Result<Value, String> {
    let mut current = context;
    for segment in path {
        match current {
            Value::Object(map) => {
                current = map
                    .get(segment)
                    .ok_or_else(|| format!("Path segment '{}' not found", segment))?;
            }
            _ => {
                return Err(format!(
                    "Cannot resolve path segment '{}' in non-object",
                    segment
                ));
            }
        }
    }
    Ok(current.clone())
}

fn to_f64(value: &Value) -> Result<f64, String> {
    match value {
        Value::Number(n) => n.as_f64().ok_or_else(|| "Invalid number".into()),
        Value::String(s) => s
            .parse()
            .map_err(|_| format!("Cannot convert string '{}' to number", s)),
        _ => Err(format!("Cannot convert {:?} to number", value)),
    }
}

fn to_bool(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|v| v != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn compare_values(op: CompareOp, left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Null, Value::Null) => matches!(op, CompareOp::Eq),
        (Value::Null, _) | (_, Value::Null) => false,
        (Value::Number(ln), Value::Number(rn)) => {
            let l = ln.as_f64().unwrap_or(0.0);
            let r = rn.as_f64().unwrap_or(0.0);
            match op {
                CompareOp::Gt => l > r,
                CompareOp::Lt => l < r,
                CompareOp::Gte => l >= r,
                CompareOp::Lte => l <= r,
                CompareOp::Eq => (l - r).abs() < f64::EPSILON,
            }
        }
        (Value::String(ls), Value::String(rs)) => match op {
            CompareOp::Gt => ls > rs,
            CompareOp::Lt => ls < rs,
            CompareOp::Gte => ls >= rs,
            CompareOp::Lte => ls <= rs,
            CompareOp::Eq => ls == rs,
        },
        _ => false,
    }
}
