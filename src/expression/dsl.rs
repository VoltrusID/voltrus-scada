//! Declarative transform DSL — a zero-cost filter/mutate language for the common
//! "emit when X, set Y" case, evaluated natively in Rust (no JS engine, no worker
//! thread, no JSON marshal beyond `Value` clones).
//!
//! A program is a list of statements, one per line (`#` comments allowed):
//!
//! ```text
//! when <expr>          # keep the record iff <expr> is truthy (drop otherwise)
//! set <path> = <expr>  # mutate a field (dotted JSON path, e.g. `status.alarm`)
//! drop                 # drop unconditionally
//! ```
//!
//! Expressions reuse the crate `expression::evaluate_json` evaluator (JSON paths,
//! comparisons, arithmetic), e.g.:
//!   `metrics.temp > 80 || metrics.vib > 5.0`
//!   `metrics.temp * 1.8 + 32`
//!
//! `set` evaluates its RHS against the *original* record, then writes the dotted
//! path on a (cloned, mutated) copy — mirrors opex-flow's `dsl.rs` semantics.

use serde_json::Value;

use super::evaluator::evaluate_json;

/// Run a DSL program against a record.
///
/// `Ok(None)` = dropped (filter rejected / explicit `drop`);
/// `Ok(Some)` = (possibly mutated) record;
/// `Err` = parse or evaluation error (caller applies on-error policy).
pub fn run_dsl(src: &str, rec: &Value) -> Result<Option<Value>, String> {
    let prog = parse(src)?;
    if let Some(cond) = &prog.filter
        && !truthy(&evaluate_json(cond, rec)?)
    {
        return Ok(None);
    }
    if prog.drop {
        return Ok(None);
    }
    if prog.sets.is_empty() {
        return Ok(Some(rec.clone()));
    }
    let mut out = rec.clone();
    for (path, expr) in &prog.sets {
        let v = evaluate_json(expr, rec)?;
        set_path(&mut out, path, v);
    }
    Ok(Some(out))
}

#[derive(Debug, Default)]
struct Program {
    filter: Option<String>,
    sets: Vec<(String, String)>,
    drop: bool,
}

/// Parse DSL source into a [`Program`].
fn parse(src: &str) -> Result<Program, String> {
    let mut prog = Program::default();
    for (i, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (kw, rest) = split_kw(line);
        match kw {
            "when" => {
                let e = rest.trim();
                if e.is_empty() {
                    return Err(format!("line {}: `when` needs an expression", i + 1));
                }
                if prog.filter.is_some() {
                    return Err(format!("line {}: multiple `when` clauses", i + 1));
                }
                prog.filter = Some(e.to_string());
            }
            "set" => {
                let e = rest.trim();
                let eq = e
                    .find('=')
                    .ok_or_else(|| format!("line {}: `set` needs `=`", i + 1))?;
                let lhs = e[..eq].trim();
                let rhs = e[eq + 1..].trim();
                // Reject `==`, `>=`, `<=`, `!=`, `=>` that a naive find('=') would catch.
                if lhs.is_empty()
                    || matches!(lhs.as_bytes().last(), Some(b'>' | b'<' | b'!' | b'='))
                {
                    return Err(format!("line {}: invalid `set` target", i + 1));
                }
                if rhs.is_empty() {
                    return Err(format!("line {}: `set` needs an expression", i + 1));
                }
                prog.sets.push((lhs.to_string(), rhs.to_string()));
            }
            "drop" => {
                if !rest.trim().is_empty() {
                    return Err(format!("line {}: `drop` takes no arguments", i + 1));
                }
                prog.drop = true;
            }
            other => {
                return Err(format!(
                    "line {}: unknown statement `{other}` (expected when/set/drop)",
                    i + 1
                ));
            }
        }
    }
    Ok(prog)
}

/// Split `kw rest...` on the first run of whitespace.
fn split_kw(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(idx) => (&line[..idx], line[idx..].trim_start()),
        None => (line, ""),
    }
}

/// Truthiness matching the flow-engine `is_truthy` rule.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Write `value` into the dotted `path` of `rec`, creating intermediate objects.
/// Non-object intermediates are overwritten with an object (opex-flow semantics).
fn set_path(rec: &mut Value, path: &str, value: Value) {
    let segs: Vec<&str> = path.split('.').collect();
    if segs.is_empty() {
        return;
    }
    if !rec.is_object() {
        *rec = Value::Object(serde_json::Map::new());
    }
    let mut cur = rec;
    for seg in &segs[..segs.len() - 1] {
        if seg.is_empty() {
            return;
        }
        if !cur[seg].is_object() {
            cur[seg] = Value::Object(serde_json::Map::new());
        }
        cur = &mut cur[seg];
    }
    if let Some(last) = segs.last()
        && !last.is_empty()
    {
        cur[last] = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn passthrough_when_no_statements() {
        let rec = json!({ "a": 1 });
        assert_eq!(run_dsl("", &rec).unwrap(), Some(json!({ "a": 1 })));
    }

    #[test]
    fn when_keeps_on_truthy_drops_on_falsy() {
        let rec = json!({ "temp": 90 });
        assert_eq!(
            run_dsl("when temp > 80", &rec).unwrap(),
            Some(json!({ "temp": 90 }))
        );
        let cool = json!({ "temp": 10 });
        assert_eq!(run_dsl("when temp > 80", &cool).unwrap(), None);
    }

    #[test]
    fn set_mutates_field() {
        let rec = json!({ "temp": 100 });
        let out = run_dsl("set status.alarm = \"high\"", &rec)
            .unwrap()
            .unwrap();
        assert_eq!(out["status"]["alarm"], "high");
        assert_eq!(out["temp"], 100); // original field retained
    }

    #[test]
    fn drop_unconditional() {
        assert_eq!(run_dsl("drop", &json!({})).unwrap(), None);
    }

    #[test]
    fn combined_when_and_set() {
        let rec = json!({ "temp": 90 });
        let out = run_dsl("when temp > 80\nset status.alarm = \"high\"", &rec)
            .unwrap()
            .unwrap();
        assert_eq!(out["status"]["alarm"], "high");
        // filtered out → None
        assert_eq!(
            run_dsl(
                "when temp > 80\nset status.alarm = \"high\"",
                &json!({ "temp": 1 })
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn parse_rejects_unknown_statement() {
        assert!(parse("frobnicate x").is_err());
    }

    #[test]
    fn parse_rejects_double_when() {
        assert!(parse("when a\nwhen b").is_err());
    }
}
