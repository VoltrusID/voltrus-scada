//! What a collaboration write checks (docs/collaboration.md#validation): the
//! objects that changed since the last write must be rows a REST write or a
//! project import would accept. Only what a change introduces is held
//! against it: a binding to a tag deleted long ago does not stop someone
//! from moving the widget. Without a [`Ctx`] only the shape is checked
//! (what each update gets as it arrives); references are checked when the
//! edits are written.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;
use voltrus_collab::document::{ConnectionRow, ObjectKind, Rows, WidgetRow};

use crate::project_yaml::binding_refs::config_binding_refs;
use crate::project_yaml::export::{FACEPLATE_MAP, TAG_KEYS, tag_refs};

/// The largest `config`, bytes (JSON text).
pub const MAX_CONFIG: usize = 64 * 1024;
/// Coordinates and sizes stay within this.
pub const MAX_COORD: f64 = 100_000.0;
/// Most waypoints of a connection.
pub const MAX_WAYPOINTS: usize = 1000;

/// What exists, for reference checks.
#[derive(Debug, Default, Clone)]
pub struct Ctx {
    pub tag_ids: HashSet<i64>,
    pub tag_refs: HashSet<String>,
    pub instances: HashSet<String>,
    pub faceplates: HashSet<String>,
}

impl Ctx {
    pub fn load(conn: &rusqlite::Connection) -> anyhow::Result<Ctx> {
        let mut ctx = Ctx::default();
        for (id, r) in tag_refs(conn)? {
            ctx.tag_ids.insert(id);
            ctx.tag_refs.insert(r);
        }
        let mut stmt = conn.prepare("SELECT name FROM equipment_instances")?;
        ctx.instances = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt = conn.prepare("SELECT id FROM faceplates")?;
        ctx.faceplates = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ctx)
    }
}

/// One object that may not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub kind: ObjectKind,
    pub id: String,
    pub reason: String,
    /// Remove it rather than put it back (a connection whose end is gone).
    pub remove: bool,
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
}

fn kind_ok(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= 64
        && kind
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.:".contains(c))
}

fn coord_ok(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORD
}

/// A tag id as rows hold it (a number, or a numeric string).
fn tag_id(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Tag ids a widget row names: sensors, the tag keys, the faceplate map.
fn tag_ids_of(sensors: &Value, config: &serde_json::Map<String, Value>) -> Vec<i64> {
    let mut out: Vec<i64> = sensors
        .as_array()
        .map(|a| a.iter().filter_map(tag_id).collect())
        .unwrap_or_default();
    for k in TAG_KEYS {
        if let Some(id) = config.get(*k).and_then(tag_id) {
            out.push(id);
        }
    }
    if let Some(Value::Object(m)) = config.get(FACEPLATE_MAP) {
        out.extend(m.values().filter_map(tag_id));
    }
    out
}

fn binding_refs(config: &Value) -> Vec<String> {
    config_binding_refs(config)
        .into_iter()
        .filter_map(|(_, r)| r.ok())
        .flatten()
        .collect()
}

/// Whether a widget row may be written (`prev`: as it was last written).
/// `ctx: None` checks its shape only.
pub fn check_widget(
    w: &WidgetRow,
    prev: Option<&WidgetRow>,
    ctx: Option<&Ctx>,
) -> Result<(), String> {
    if !id_ok(&w.id) {
        return Err("bad widget id".into());
    }
    if !kind_ok(&w.kind) {
        return Err(format!(
            "bad widget type '{}'",
            w.kind.chars().take(64).collect::<String>()
        ));
    }
    if w.name.chars().count() > 256 {
        return Err("name over 256 characters".into());
    }
    if ![w.x, w.y, w.rotation].into_iter().all(coord_ok) {
        return Err("position or rotation is not a number within the stage".into());
    }
    if ![w.width, w.height]
        .into_iter()
        .all(|v| coord_ok(v) && v >= 0.0)
    {
        return Err("size is not a positive number within the stage".into());
    }
    if w.config.len() > MAX_CONFIG {
        return Err(format!("config over {} KB", MAX_CONFIG / 1024));
    }
    let config: Value =
        serde_json::from_str(&w.config).map_err(|_| "config is not JSON".to_string())?;
    let Value::Object(cfg) = &config else {
        return Err("config is not an object".into());
    };
    let sensors: Value =
        serde_json::from_str(&w.sensors).map_err(|_| "tag bindings are not JSON".to_string())?;
    if !sensors.is_array() {
        return Err("tag bindings are not a list".into());
    }
    // The phone layout override (phase 5): only a changed one is checked.
    let prev_mobile = prev.and_then(|p| {
        serde_json::from_str::<Value>(&p.config)
            .ok()
            .and_then(|c| c.get(voltrus_collab::mobile::KEY).cloned())
    });
    if cfg.get(voltrus_collab::mobile::KEY) != prev_mobile.as_ref() {
        voltrus_collab::mobile::check_config(cfg)?;
    }
    let Some(ctx) = ctx else {
        for (prop, refs) in config_binding_refs(&config) {
            if let Err(e) = refs {
                return Err(if prop.is_empty() {
                    format!("bindings: {e}")
                } else {
                    format!("binding of {prop}: {e}")
                });
            }
        }
        return Ok(());
    };
    let prev_parsed = prev.map(|p| {
        (
            serde_json::from_str::<Value>(&p.config).unwrap_or(Value::Null),
            serde_json::from_str::<Value>(&p.sensors).unwrap_or(Value::Null),
        )
    });
    // Tag ids: only new ones must exist.
    let old_ids: HashSet<i64> = match &prev_parsed {
        Some((Value::Object(pc), ps)) => tag_ids_of(ps, pc).into_iter().collect(),
        _ => HashSet::new(),
    };
    for id in tag_ids_of(&sensors, cfg) {
        if !old_ids.contains(&id) && !ctx.tag_ids.contains(&id) {
            return Err(format!("tag {id} does not exist"));
        }
    }
    // Bindings: well formed; refs they add must exist.
    let old_refs: HashSet<String> = prev_parsed
        .as_ref()
        .map(|(pc, _)| binding_refs(pc).into_iter().collect())
        .unwrap_or_default();
    for (prop, refs) in config_binding_refs(&config) {
        match refs {
            Err(e) => {
                let at = if prop.is_empty() {
                    "bindings".to_string()
                } else {
                    format!("binding of {prop}")
                };
                return Err(format!("{at}: {e}"));
            }
            Ok(refs) => {
                for r in refs {
                    if !old_refs.contains(&r) && !ctx.tag_refs.contains(&r) {
                        return Err(format!("binding of {prop} reads unknown tag '{r}'"));
                    }
                }
            }
        }
    }
    let prev_cfg = prev_parsed.as_ref().map(|(c, _)| c);
    let unchanged = |key: &str| prev_cfg.and_then(|c| c.get(key)) == cfg.get(key);
    if let Some(name) = cfg.get("equipment").filter(|_| !unchanged("equipment")) {
        match name.as_str() {
            Some(n) if ctx.instances.contains(n) => {}
            Some(n) => return Err(format!("equipment instance '{n}' does not exist")),
            None => return Err("equipment is not a name".into()),
        }
    }
    if let Some(fp) = cfg.get("faceplate").filter(|_| !unchanged("faceplate")) {
        match fp {
            Value::Null => {}
            Value::String(f) if f.is_empty() || ctx.faceplates.contains(f) => {}
            other => return Err(format!("faceplate {other} does not exist")),
        }
    }
    Ok(())
}

/// Whether a connection row may be written (its shape).
pub fn check_connection(c: &ConnectionRow) -> Result<(), String> {
    if !id_ok(&c.id) {
        return Err("bad connection id".into());
    }
    if !kind_ok(&c.kind) || c.kind.len() > 32 {
        return Err(format!(
            "bad connection type '{}'",
            c.kind.chars().take(32).collect::<String>()
        ));
    }
    let wp: Value =
        serde_json::from_str(&c.waypoints).map_err(|_| "waypoints are not JSON".to_string())?;
    let Some(points) = wp.as_array() else {
        return Err("waypoints are not a list".into());
    };
    if points.len() > MAX_WAYPOINTS {
        return Err(format!("more than {MAX_WAYPOINTS} waypoints"));
    }
    let point_ok = |p: &Value| {
        p.as_array()
            .is_some_and(|xy| xy.len() == 2 && xy.iter().all(|v| v.as_f64().is_some_and(coord_ok)))
    };
    if !points.iter().all(point_ok) {
        return Err("a waypoint is not [x, y]".into());
    }
    Ok(())
}

/// The problems of `next` against `prev` (the rows last written): changed
/// widgets and connections that fail, and connections whose ends are gone.
pub fn check(prev: &Rows, next: &Rows, ctx: Option<&Ctx>) -> Vec<Problem> {
    let mut out = Vec::new();
    let prev_w: BTreeMap<&str, &WidgetRow> =
        prev.widgets.iter().map(|w| (w.id.as_str(), w)).collect();
    let next_w: BTreeMap<&str, &WidgetRow> =
        next.widgets.iter().map(|w| (w.id.as_str(), w)).collect();
    for w in &next.widgets {
        let before = prev_w.get(w.id.as_str()).copied();
        if before == Some(w) {
            continue;
        }
        if let Err(reason) = check_widget(w, before, ctx) {
            out.push(Problem {
                kind: ObjectKind::Widget,
                id: w.id.clone(),
                reason,
                remove: before.is_none(),
            });
        }
    }
    // A widget that is put back or removed may leave its connections dangling
    // too: their ends are checked against the widgets that pass.
    let failing: HashSet<&str> = out
        .iter()
        .filter(|p| p.remove)
        .map(|p| p.id.as_str())
        .collect();
    let ends: BTreeMap<&str, &WidgetRow> = next_w
        .into_iter()
        .filter(|(id, _)| !failing.contains(id))
        .collect();
    let prev_c: BTreeMap<&str, &ConnectionRow> = prev
        .connections
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();
    for c in &next.connections {
        let missing: Vec<&str> = [c.from.as_str(), c.to.as_str()]
            .into_iter()
            .filter(|e| !ends.contains_key(e))
            .collect();
        if !missing.is_empty() {
            out.push(Problem {
                kind: ObjectKind::Connection,
                id: c.id.clone(),
                reason: format!(
                    "it ends on {}, which no longer exists",
                    missing.join(" and ")
                ),
                remove: true,
            });
            continue;
        }
        let before = prev_c.get(c.id.as_str()).copied();
        if before == Some(c) {
            continue;
        }
        if let Err(reason) = check_connection(c) {
            out.push(Problem {
                kind: ObjectKind::Connection,
                id: c.id.clone(),
                reason,
                remove: before.is_none(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(id: &str, config: &str) -> WidgetRow {
        WidgetRow {
            id: id.into(),
            kind: "symbol".into(),
            name: String::new(),
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rotation: 0.0,
            sensors: "[1]".into(),
            config: config.into(),
        }
    }

    fn c(id: &str, from: &str, to: &str) -> ConnectionRow {
        ConnectionRow {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            kind: "flow".into(),
            waypoints: "[]".into(),
        }
    }

    fn ctx() -> Ctx {
        Ctx {
            tag_ids: [1, 2].into_iter().collect(),
            tag_refs: ["d/a".to_string()].into_iter().collect(),
            instances: ["P-101".to_string()].into_iter().collect(),
            faceplates: ["fp1".to_string()].into_iter().collect(),
        }
    }

    #[test]
    fn valid_changes_pass() {
        let prev = Rows {
            widgets: vec![w("a", "{}")],
            connections: vec![],
        };
        let next = Rows {
            widgets: vec![
                w(
                    "a",
                    r#"{"bindings":{"label":{"kind":"tag","tag":"d/a"}},"equipment":"P-101","faceplate":"fp1"}"#,
                ),
                w("b", "{}"),
            ],
            connections: vec![c("c", "a", "b")],
        };
        assert_eq!(check(&prev, &next, Some(&ctx())), vec![]);
    }

    #[test]
    fn bad_values_are_named() {
        let prev = Rows::default();
        let mut huge = w("h", "{}");
        huge.x = f64::NAN;
        let cases = [
            (
                w("u", r#"{"bindings":{"label":{"kind":"tag","tag":"d/zz"}}}"#),
                "unknown tag 'd/zz'",
            ),
            (
                w("k", r#"{"bindings":{"label":{"kind":"nope"}}}"#),
                "unknown binding kind",
            ),
            (w("t", "[1]"), "config is not an object"),
            (w("e", r#"{"equipment":"P-999"}"#), "P-999"),
            (huge, "within the stage"),
        ];
        for (row, want) in cases {
            let got = check(
                &prev,
                &Rows {
                    widgets: vec![row],
                    connections: vec![],
                },
                Some(&ctx()),
            );
            assert_eq!(got.len(), 1, "{want}");
            assert!(got[0].reason.contains(want), "{} vs {want}", got[0].reason);
            assert!(got[0].remove);
        }
        let mut s = w("s", "{}");
        s.sensors = "[77]".into();
        assert!(
            check(
                &prev,
                &Rows {
                    widgets: vec![s],
                    connections: vec![]
                },
                Some(&ctx())
            )[0]
            .reason
            .contains("tag 77")
        );
    }

    #[test]
    fn only_what_a_change_adds_counts() {
        // A widget bound to a tag that was deleted since: moving it is fine.
        let old = w(
            "a",
            r#"{"bindings":{"label":{"kind":"tag","tag":"d/gone"}}}"#,
        );
        let mut moved = old.clone();
        moved.x = 50.0;
        let prev = Rows {
            widgets: vec![old],
            connections: vec![],
        };
        let next = Rows {
            widgets: vec![moved],
            connections: vec![],
        };
        assert!(check(&prev, &next, Some(&ctx())).is_empty());
    }

    #[test]
    fn dangling_connections_are_removed() {
        let prev = Rows {
            widgets: vec![w("a", "{}"), w("b", "{}")],
            connections: vec![c("c", "a", "b")],
        };
        let next = Rows {
            widgets: vec![w("a", "{}")],
            connections: vec![c("c", "a", "b")],
        };
        let p = check(&prev, &next, Some(&ctx()));
        assert_eq!(p.len(), 1);
        assert!(p[0].remove && p[0].kind == ObjectKind::Connection);
        // A connection to a new widget that fails goes with it.
        let next = Rows {
            widgets: vec![w("a", "{}"), w("b", "{}"), w("x", "[]")],
            connections: vec![c("c", "a", "b"), c("d", "a", "x")],
        };
        let p = check(&prev, &next, Some(&ctx()));
        assert_eq!(
            p.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["x", "d"]
        );
    }

    #[test]
    fn without_a_context_only_the_shape_counts() {
        let prev = Rows::default();
        let unknown = w("u", r#"{"bindings":{"label":{"kind":"tag","tag":"d/zz"}}}"#);
        let next = Rows {
            widgets: vec![unknown],
            connections: vec![],
        };
        assert!(check(&prev, &next, None).is_empty());
        let next = Rows {
            widgets: vec![w("k", r#"{"bindings":{"label":{"kind":"nope"}}}"#)],
            connections: vec![],
        };
        assert_eq!(check(&prev, &next, None).len(), 1);
    }

    #[test]
    fn the_phone_layout_override_is_checked_when_it_changes() {
        let prev = Rows {
            widgets: vec![w("a", "{}")],
            connections: vec![],
        };
        let ok = Rows {
            widgets: vec![w("a", r#"{"mobile":{"hide":true,"order":2,"span":1}}"#)],
            connections: vec![],
        };
        assert!(check(&prev, &ok, Some(&ctx())).is_empty());
        assert!(check(&prev, &ok, None).is_empty());
        for (config, want) in [
            (r#"{"mobile":{"span":3}}"#, "mobile.span"),
            (r#"{"mobile":{"hide":"yes"}}"#, "mobile.hide"),
            (r#"{"mobile":[1]}"#, "not an object"),
            (r#"{"mobile":{"script":"x"}}"#, "not a known key"),
        ] {
            let bad = Rows {
                widgets: vec![w("a", config)],
                connections: vec![],
            };
            for c in [Some(ctx()), None] {
                let p = check(&prev, &bad, c.as_ref());
                assert_eq!(p.len(), 1, "{config}");
                assert!(p[0].reason.contains(want), "{} vs {want}", p[0].reason);
                assert!(!p[0].remove, "an existing widget is put back, not removed");
            }
        }
        // An override written before (not by this change) does not stop a move.
        let old = w("a", r#"{"mobile":{"span":7}}"#);
        let mut moved = old.clone();
        moved.x = 30.0;
        let prev = Rows {
            widgets: vec![old],
            connections: vec![],
        };
        let next = Rows {
            widgets: vec![moved],
            connections: vec![],
        };
        assert!(check(&prev, &next, Some(&ctx())).is_empty());
    }
}
