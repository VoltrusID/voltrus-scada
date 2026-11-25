//! Tag references inside the property bindings of SCADA widgets.
//!
//! The Designer stores bindings in a widget's `config.bindings`, keyed by
//! property: `{"kind":"tag","tag":"<ref>"}`, `{"kind":"expr","expr":"<src>"}`
//! or `{"kind":"map","expr":"<src>","cases":[...],"else":...}`. Tags are named
//! by project ref (`device/key`, `#n` for duplicates, as in the export). In an
//! expression a ref is written `{ref}`; inside the braces a backslash escapes
//! the next character (`\}`, `\\`), and braces inside string literals are text.
//! This mirrors the client's lexer (`client/src/designer/expr.ts`).

use serde_json::Value;

/// Key of the bindings object in a widget's config.
pub const BINDINGS: &str = "bindings";

/// A `{ref}` occurrence: byte range of the whole `{...}` and the unescaped, trimmed ref.
struct Occurrence {
    start: usize,
    end: usize,
    reference: String,
}

fn scan(src: &str) -> Result<Vec<Occurrence>, String> {
    let mut out = Vec::new();
    let mut it = src.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        match c {
            '"' | '\'' => {
                let mut closed = false;
                while let Some((_, d)) = it.next() {
                    if d == '\\' {
                        it.next();
                    } else if d == c {
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return Err("unterminated string".into());
                }
            }
            '{' => {
                let mut reference = String::new();
                let mut end = None;
                while let Some((j, d)) = it.next() {
                    if d == '\\' {
                        if let Some((_, e)) = it.next() {
                            reference.push(e);
                        }
                    } else if d == '}' {
                        end = Some(j + 1);
                        break;
                    } else {
                        reference.push(d);
                    }
                }
                let Some(end) = end else {
                    return Err("missing } after tag reference".into());
                };
                let reference = reference.trim().to_string();
                if reference.is_empty() {
                    return Err("empty tag reference".into());
                }
                out.push(Occurrence {
                    start: i,
                    end,
                    reference,
                });
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The tag refs an expression reads, in order of appearance.
pub fn expr_refs(src: &str) -> Result<Vec<String>, String> {
    Ok(scan(src)?.into_iter().map(|o| o.reference).collect())
}

/// `{ref}` with `\` and `}` escaped.
pub fn quote_ref(reference: &str) -> String {
    let mut s = String::with_capacity(reference.len() + 2);
    s.push('{');
    for c in reference.chars() {
        if c == '\\' || c == '}' {
            s.push('\\');
        }
        s.push(c);
    }
    s.push('}');
    s
}

/// The expression with every ref for which `f` gives a new one replaced.
/// `None` when nothing changed (or the expression cannot be scanned).
pub fn rewrite_expr(src: &str, f: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let occ = scan(src).ok()?;
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    let mut changed = false;
    for o in occ {
        if let Some(new) = f(&o.reference).filter(|n| *n != o.reference) {
            out.push_str(&src[at..o.start]);
            out.push_str(&quote_ref(&new));
            at = o.end;
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    out.push_str(&src[at..]);
    Some(out)
}

/// The refs one binding reads, or what is wrong with it.
pub fn binding_refs(binding: &Value) -> Result<Vec<String>, String> {
    let Some(b) = binding.as_object() else {
        return Err("a binding must be an object".into());
    };
    match b.get("kind").and_then(Value::as_str) {
        Some("tag") => match b.get("tag").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => Ok(vec![t.to_string()]),
            _ => Err("a tag binding needs `tag`".into()),
        },
        Some(kind @ ("expr" | "map")) => match b.get("expr").and_then(Value::as_str) {
            Some(e) => expr_refs(e),
            None => Err(format!("a {kind} binding needs `expr`")),
        },
        Some(other) => Err(format!(
            "unknown binding kind '{other}' (allowed: tag, expr, map)"
        )),
        None => Err("a binding needs `kind`".into()),
    }
}

/// Every binding of a widget config: (property, its refs or its problem).
/// Empty when the config has no bindings; a `bindings` that is not an object
/// is reported under the property name `""`.
pub fn config_binding_refs(config: &Value) -> Vec<(String, Result<Vec<String>, String>)> {
    match config.get(BINDINGS) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(m)) => m
            .iter()
            .map(|(prop, b)| (prop.clone(), binding_refs(b)))
            .collect(),
        Some(_) => vec![(String::new(), Err("`bindings` must be an object".into()))],
    }
}

/// Rewrite the refs of every binding in a widget config. True when changed.
pub fn rewrite_config(config: &mut Value, f: &dyn Fn(&str) -> Option<String>) -> bool {
    let Some(Value::Object(m)) = config.get_mut(BINDINGS) else {
        return false;
    };
    let mut changed = false;
    for b in m.values_mut() {
        let Some(b) = b.as_object_mut() else { continue };
        match b.get("kind").and_then(Value::as_str) {
            Some("tag") => {
                if let Some(new) = b
                    .get("tag")
                    .and_then(Value::as_str)
                    .and_then(|t| f(t).filter(|n| n != t))
                {
                    b.insert("tag".into(), Value::String(new));
                    changed = true;
                }
            }
            Some("expr" | "map") => {
                if let Some(new) = b
                    .get("expr")
                    .and_then(Value::as_str)
                    .and_then(|e| rewrite_expr(e, f))
                {
                    b.insert("expr".into(), Value::String(new));
                    changed = true;
                }
            }
            _ => {}
        }
    }
    changed
}

/// After tags were renamed (key or device changed): rewrite every binding
/// ref whose tag kept its id but whose ref text changed, in all SCADA widget
/// configs. `before` is [`super::export::tag_refs`] taken before the change.
/// Returns the number of widgets rewritten.
pub fn rewrite_after_rename(
    conn: &rusqlite::Connection,
    before: &[(i64, String)],
) -> anyhow::Result<usize> {
    let after: std::collections::HashMap<i64, String> =
        super::export::tag_refs(conn)?.into_iter().collect();
    let renamed: std::collections::HashMap<&str, &str> = before
        .iter()
        .filter_map(|(id, old)| {
            after
                .get(id)
                .filter(|new| *new != old)
                .map(|new| (old.as_str(), new.as_str()))
        })
        .collect();
    if renamed.is_empty() {
        return Ok(0);
    }
    let f = |r: &str| renamed.get(r).map(|n| n.to_string());
    use crate::storage::design_history as h;
    // The draft is logged in the design history; the published copy and the
    // saved versions follow too (a rename is a ref fix-up, not a design
    // change), so operators' screens keep their bindings and an older
    // version published or restored later binds the renamed tags.
    let mut updates: Vec<(&str, String, String, String)> = Vec::new();
    for table in ["scada_equipment", "scada_equipment_published"] {
        let rows: Vec<(String, String, String)> = {
            let Ok(mut stmt) = conn.prepare(&format!(
                "SELECT screen_id, equipment_id, config FROM {table} WHERE config LIKE '%\"bindings\"%'"
            )) else {
                continue;
            };
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?
        };
        for (screen, equipment, config) in rows {
            let Ok(mut v) = serde_json::from_str::<Value>(&config) else {
                continue;
            };
            if rewrite_config(&mut v, &f) {
                updates.push((table, screen, equipment, v.to_string()));
            }
        }
    }
    // Only the screens whose draft changes are read for the history.
    let screens: std::collections::BTreeSet<String> = updates
        .iter()
        .filter(|u| u.0 == "scada_equipment")
        .map(|u| u.1.clone())
        .collect();
    let mut before_objects = h::Objects::new();
    for screen in &screens {
        before_objects.extend(h::read_objects(conn, h::Scope::Screen(screen))?);
    }
    let mut n = 0;
    for (table, screen, equipment, config) in &updates {
        conn.execute(
            &format!("UPDATE {table} SET config = ?1 WHERE screen_id = ?2 AND equipment_id = ?3"),
            rusqlite::params![config, screen, equipment],
        )?;
        if *table == "scada_equipment" {
            n += 1;
        }
    }
    let versions: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, snapshot FROM design_versions WHERE snapshot LIKE '%bindings%'")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    for (id, text) in versions {
        let Ok(mut snap) = serde_json::from_str::<h::LayoutSnapshot>(&text) else {
            continue;
        };
        let mut changed = false;
        for e in &mut snap.equipment {
            let Ok(mut v) = serde_json::from_str::<Value>(&e.config) else {
                continue;
            };
            if rewrite_config(&mut v, &f) {
                e.config = v.to_string();
                changed = true;
            }
        }
        if changed {
            conn.execute(
                "UPDATE design_versions SET snapshot = ?2 WHERE id = ?1",
                rusqlite::params![id, serde_json::to_string(&snap)?],
            )?;
        }
    }
    if n > 0 {
        let mut after_objects = h::Objects::new();
        for screen in &screens {
            after_objects.extend(h::read_objects(conn, h::Scope::Screen(screen))?);
        }
        let note = renamed
            .iter()
            .map(|(a, b)| format!("{a} → {b}"))
            .collect::<Vec<_>>()
            .join(", ");
        h::append(
            conn,
            h::diff_objects(&before_objects, &after_objects),
            &h::DesignActor::system("tag_rename").with_note(note),
        )?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn refs_of_expressions() {
        assert_eq!(
            expr_refs(r#"{wtp/a} > 1 ? "x{not/a/ref}" : text({ wtp/b }, 1)"#).unwrap(),
            vec!["wtp/a", "wtp/b"]
        );
        assert_eq!(
            expr_refs(r"{d/we\}ird} + {d/back\\slash}").unwrap(),
            vec!["d/we}ird", r"d/back\slash"]
        );
        assert_eq!(
            expr_refs(r#"'it\'s {no}' + {d/k#2}"#).unwrap(),
            vec!["d/k#2"]
        );
        assert!(expr_refs("{d/a").is_err());
        assert!(expr_refs("{ }").is_err());
        assert!(expr_refs("\"open").is_err());
    }

    #[test]
    fn rewriting_escapes_the_new_ref() {
        let f = |r: &str| (r == "d/old").then(|| "d/n}w".to_string());
        assert_eq!(
            rewrite_expr(r#"{d/old} + "{d/old}" + { d/old } + {d/other}"#, &f).unwrap(),
            r#"{d/n\}w} + "{d/old}" + {d/n\}w} + {d/other}"#
        );
        assert_eq!(rewrite_expr("{d/other}", &f), None);
        assert_eq!(expr_refs(r"{d/n\}w}").unwrap(), vec!["d/n}w"]);
    }

    #[test]
    fn bindings_of_a_config() {
        let mut c = json!({"bindings": {
            "visible": {"kind": "tag", "tag": "d/old"},
            "label": {"kind": "expr", "expr": "text({d/old}, 1)"},
            "color": {"kind": "map", "expr": "{d/old}", "cases": [], "else": "#fff"},
            "bad": {"kind": "loop"},
        }});
        let refs = config_binding_refs(&c);
        assert_eq!(refs.len(), 4);
        assert!(refs.iter().any(|(p, r)| p == "bad" && r.is_err()));
        let f = |r: &str| (r == "d/old").then(|| "d/new".to_string());
        assert!(rewrite_config(&mut c, &f));
        assert_eq!(c["bindings"]["visible"]["tag"], "d/new");
        assert_eq!(c["bindings"]["label"]["expr"], "text({d/new}, 1)");
        assert_eq!(c["bindings"]["color"]["expr"], "{d/new}");
        assert!(!rewrite_config(&mut c, &f));
        assert!(config_binding_refs(&json!({})).is_empty());
        assert!(config_binding_refs(&json!({"bindings": []}))[0].1.is_err());
    }
}
