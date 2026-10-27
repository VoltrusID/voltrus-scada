//! Parsing, defaults and canonical form of entities, plus the conversion
//! between YAML values and SQLite cells.

use anyhow::{Context, Result};
use rusqlite::types::{Value as Sql, ValueRef};
use serde_yaml_neo::{Mapping, Number, Value};

use super::spec::{Dv, Field, Req, Ty};

pub fn text(s: impl Into<String>) -> Value {
    Value::String(s.into())
}

pub fn int(i: i64) -> Value {
    Value::Number(Number::from(i))
}

pub fn real(f: f64) -> Value {
    Value::Number(Number::from(f))
}

pub fn as_str(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s),
        _ => None,
    }
}

/// Field value as text, for building keys and messages.
pub fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => serde_yaml_neo::to_string(other)
            .map(|s| s.trim_end().to_string())
            .unwrap_or_default(),
    }
}

/// Recursively order mapping keys so output never depends on how a JSON blob
/// happened to be written.
pub fn canon(v: Value) -> Value {
    match v {
        Value::Mapping(m) => {
            let mut entries: Vec<(Value, Value)> =
                m.into_iter().map(|(k, v)| (k, canon(v))).collect();
            entries.sort_by_key(|(k, _)| display(k));
            let mut out = Mapping::with_capacity(entries.len());
            for (k, v) in entries {
                out.insert(k, v);
            }
            Value::Mapping(out)
        }
        Value::Sequence(s) => Value::Sequence(s.into_iter().map(canon).collect()),
        Value::Tagged(t) => canon(t.value),
        other => other,
    }
}

pub fn json_to_yaml(j: &serde_json::Value) -> Result<Value> {
    Ok(canon(
        serde_yaml_neo::to_value(j).context("JSON value is not representable as YAML")?,
    ))
}

pub fn yaml_to_json(v: &Value) -> Result<serde_json::Value> {
    serde_json::to_value(v)
        .context("value is not representable as JSON (mapping keys must be strings)")
}

pub fn yaml_to_json_text(v: &Value) -> Result<String> {
    Ok(serde_json::to_string(&yaml_to_json(v)?)?)
}

pub fn dv_value(dv: Dv, ty: Ty) -> Value {
    match (dv, ty) {
        (Dv::S(s), _) => text(s),
        (Dv::I(i), Ty::Real) => real(i as f64),
        (Dv::I(i), _) => int(i),
        (Dv::R(r), _) => real(r),
        (Dv::B(b), _) => Value::Bool(b),
        (Dv::Arr, _) => Value::Sequence(Vec::new()),
        (Dv::Map, _) => Value::Mapping(Mapping::new()),
        (Dv::J(j), _) => serde_json::from_str::<serde_json::Value>(j)
            .ok()
            .and_then(|j| json_to_yaml(&j).ok())
            .unwrap_or(Value::Null),
    }
}

/// Default of a field, `None` for required/optional ones.
pub fn default_of(f: &Field) -> Option<Value> {
    match (f.req, f.ty) {
        (Req::Default(_), Ty::Obj(sub)) => {
            let mut errs = Vec::new();
            Some(Value::Mapping(normalize(
                sub,
                &Mapping::new(),
                "",
                &mut errs,
            )))
        }
        (Req::Default(dv), ty) => Some(dv_value(dv, ty)),
        _ => None,
    }
}

fn coerce(f: &Field, v: &Value, path: &str, errs: &mut Vec<String>) -> Option<Value> {
    let at = || format!("{path}{}{}", if path.is_empty() { "" } else { "." }, f.name);
    match f.ty {
        Ty::Text => match v {
            Value::String(_) => Some(v.clone()),
            Value::Number(n) => Some(text(n.to_string())),
            Value::Bool(b) => Some(text(b.to_string())),
            _ => {
                errs.push(format!("{}: expected text", at()));
                None
            }
        },
        Ty::Int => match v {
            Value::Number(n) if n.is_i64() => Some(v.clone()),
            Value::Number(n) if n.is_u64() => match i64::try_from(n.as_u64().unwrap_or(0)) {
                Ok(i) => Some(int(i)),
                Err(_) => {
                    errs.push(format!("{}: integer out of range", at()));
                    None
                }
            },
            _ => {
                errs.push(format!("{}: expected an integer", at()));
                None
            }
        },
        Ty::Real => match v {
            Value::Number(n) => Some(real(n.as_f64().unwrap_or(0.0))),
            _ => {
                errs.push(format!("{}: expected a number", at()));
                None
            }
        },
        Ty::Bool => match v {
            Value::Bool(_) => Some(v.clone()),
            _ => {
                errs.push(format!("{}: expected true or false", at()));
                None
            }
        },
        Ty::Json => {
            if let Err(e) = yaml_to_json(v) {
                errs.push(format!("{}: {e}", at()));
                return None;
            }
            Some(canon(v.clone()))
        }
        Ty::Obj(sub) => match v {
            Value::Mapping(m) => Some(Value::Mapping(normalize(sub, m, &at(), errs))),
            _ => {
                errs.push(format!("{}: expected a mapping", at()));
                None
            }
        },
        Ty::List(sub) => match v {
            Value::Sequence(items) => {
                let mut out = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    match item {
                        Value::Mapping(m) => out.push(Value::Mapping(normalize(
                            sub,
                            m,
                            &format!("{}[{i}]", at()),
                            errs,
                        ))),
                        _ => errs.push(format!("{}[{i}]: expected a mapping", at())),
                    }
                }
                Some(Value::Sequence(out))
            }
            _ => {
                errs.push(format!("{}: expected a list", at()));
                None
            }
        },
    }
}

/// Parse one mapping against a field list: reject unknown fields, check
/// types, fill defaults and emit the canonical form (spec field order, lean
/// defaults omitted).
pub fn normalize(fields: &[Field], input: &Mapping, path: &str, errs: &mut Vec<String>) -> Mapping {
    for k in input.keys() {
        match k {
            Value::String(name) if fields.iter().any(|f| f.name == name) => {}
            Value::String(name) => errs.push(format!(
                "{}: unknown field '{name}'",
                if path.is_empty() { "(entity)" } else { path }
            )),
            other => errs.push(format!("{path}: non-text key {}", display(other))),
        }
    }
    let mut out = Mapping::new();
    for f in fields {
        let raw = input.get(f.name).filter(|v| !v.is_null());
        let value = match raw {
            Some(v) => coerce(f, v, path, errs),
            None => match f.req {
                Req::Required => {
                    errs.push(format!(
                        "{}{}{}: missing required field",
                        path,
                        if path.is_empty() { "" } else { "." },
                        f.name
                    ));
                    None
                }
                Req::Optional => None,
                Req::Default(_) => default_of(f),
            },
        };
        let Some(value) = value else { continue };
        if f.lean && default_of(f).as_ref() == Some(&value) {
            continue;
        }
        if matches!(f.req, Req::Optional)
            && let Value::Mapping(m) = &value
            && m.is_empty()
            && matches!(f.ty, Ty::Obj(_))
        {
            continue;
        }
        out.insert(text(f.name), value);
    }
    out
}

/// Value of `f` in `m`, falling back to the field default (or null).
pub fn get(m: &Mapping, f: &Field) -> Value {
    m.get(f.name)
        .cloned()
        .or_else(|| default_of(f))
        .unwrap_or(Value::Null)
}

pub fn get_str(m: &Mapping, name: &str) -> String {
    m.get(name).map(display).unwrap_or_default()
}

// ── SQL cells ──

/// Read one cell into a YAML value; `None` for SQL NULL. Lenient about the
/// storage class because SQLite columns are loosely typed.
pub fn cell_to_yaml(
    c: ValueRef<'_>,
    ty: Ty,
    name: &str,
    warnings: &mut Vec<String>,
) -> Option<Value> {
    if matches!(c, ValueRef::Null) {
        return None;
    }
    let as_text = || match c {
        ValueRef::Text(b) | ValueRef::Blob(b) => String::from_utf8_lossy(b).into_owned(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(r) => r.to_string(),
        ValueRef::Null => String::new(),
    };
    Some(match ty {
        Ty::Text => text(as_text()),
        Ty::Int => match c {
            ValueRef::Integer(i) => int(i),
            ValueRef::Real(r) => int(r as i64),
            _ => int(as_text().trim().parse().unwrap_or(0)),
        },
        Ty::Real => match c {
            ValueRef::Integer(i) => real(i as f64),
            ValueRef::Real(r) => real(r),
            _ => real(as_text().trim().parse().unwrap_or(0.0)),
        },
        Ty::Bool => match c {
            ValueRef::Integer(i) => Value::Bool(i != 0),
            ValueRef::Real(r) => Value::Bool(r != 0.0),
            _ => Value::Bool(matches!(as_text().trim(), "1" | "true" | "TRUE" | "True")),
        },
        Ty::Json => {
            let raw = as_text();
            match serde_json::from_str::<serde_json::Value>(&raw)
                .map_err(anyhow::Error::from)
                .and_then(|j| json_to_yaml(&j))
            {
                Ok(v) => v,
                Err(_) => {
                    warnings.push(format!(
                        "column '{name}' holds text that is not valid JSON; exported as a string"
                    ));
                    text(raw)
                }
            }
        }
        Ty::Obj(_) | Ty::List(_) => return None,
    })
}

/// Convert a canonical entity value into the SQL value for one column.
pub fn yaml_to_sql(f: &Field, v: Option<&Value>) -> Result<Sql> {
    let v = match v {
        Some(Value::Null) | None => match default_of(f) {
            Some(d) => d,
            None => return Ok(Sql::Null),
        },
        Some(v) => v.clone(),
    };
    Ok(match (f.ty, &v) {
        (Ty::Text, Value::String(s)) => Sql::Text(s.clone()),
        (Ty::Int, Value::Number(n)) => Sql::Integer(n.as_i64().context("expected integer")?),
        (Ty::Real, Value::Number(n)) => Sql::Real(n.as_f64().context("expected number")?),
        (Ty::Bool, Value::Bool(b)) => Sql::Integer(*b as i64),
        (Ty::Json, v) => Sql::Text(yaml_to_json_text(v)?),
        _ => anyhow::bail!("field '{}' holds a value of the wrong type", f.name),
    })
}
