//! Database -> project document.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, Row as SqlRow};
use serde_yaml_neo::{Mapping, Value};

use super::model::{FORMAT, ProjectDoc};
use super::norm::{self, cell_to_yaml, display, int, json_to_yaml, text};
use super::secrets;
use super::spec::{self, Field, Kind, Plain, Section};
use crate::storage::Storage;

/// Lookups between database ids and natural keys, built once per read.
#[derive(Default)]
pub struct Ctx {
    pub tag_ref_by_id: HashMap<i64, String>,
    pub tag_id_by_ref: HashMap<String, i64>,
    pub script_name_by_id: HashMap<i64, String>,
    pub script_id_by_name: HashMap<String, i64>,
    pub sim_name_by_id: HashMap<i64, String>,
    pub sim_id_by_name: HashMap<String, i64>,
    pub warnings: Vec<String>,
}

/// An entity as stored, with its database handle and identity.
#[derive(Clone)]
pub struct Row {
    pub key: String,
    /// Position among entities sharing the same key (0 for the first).
    pub ord: usize,
    /// Unredacted canonical entity.
    pub entity: Mapping,
    /// Primary key of the row (`None` for sections that span several tables).
    pub handle: Option<Sql>,
}

impl Row {
    pub fn label(&self) -> String {
        label(&self.key, self.ord)
    }
}

pub fn label(key: &str, ord: usize) -> String {
    if ord == 0 {
        key.to_string()
    } else {
        format!("{key}#{}", ord + 1)
    }
}

pub type Snapshot = BTreeMap<&'static str, Vec<Row>>;

/// Identity of an entity inside its section.
pub fn entity_key(sec: &Section, e: &Mapping) -> String {
    match sec.kind {
        Kind::Settings => "settings".to_string(),
        Kind::Tags => match e.get("ref").and_then(Value::as_str) {
            Some(r) => r.to_string(),
            None => tag_base_ref(e),
        },
        _ => sec
            .key
            .iter()
            .map(|k| e.get(*k).map(display).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("/"),
    }
}

pub fn tag_base_ref(e: &Mapping) -> String {
    let device = e
        .get("device")
        .map(display)
        .unwrap_or_else(|| "default".to_string());
    format!("{device}/{}", e.get("key").map(display).unwrap_or_default())
}

/// Sort rows by key (stable, so database order breaks ties) and number
/// duplicates.
pub fn order_rows(rows: &mut [Row]) {
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    let mut prev: Option<String> = None;
    let mut n = 0;
    for r in rows.iter_mut() {
        if prev.as_deref() == Some(r.key.as_str()) {
            n += 1;
        } else {
            n = 0;
            prev = Some(r.key.clone());
        }
        r.ord = n;
    }
}

fn lenient_text(r: &SqlRow<'_>, i: usize) -> Result<String> {
    Ok(match r.get_ref(i)? {
        rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => {
            String::from_utf8_lossy(b).into_owned()
        }
        rusqlite::types::ValueRef::Integer(n) => n.to_string(),
        rusqlite::types::ValueRef::Real(f) => f.to_string(),
        rusqlite::types::ValueRef::Null => String::new(),
    })
}

fn load_ctx(conn: &Connection) -> Result<Ctx> {
    let mut ctx = Ctx::default();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut stmt = conn.prepare("SELECT id, device_id, key FROM data_sources ORDER BY id")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let base = format!("{}/{}", lenient_text(r, 1)?, lenient_text(r, 2)?);
        let n = counts.entry(base.clone()).or_insert(0);
        *n += 1;
        let reference = if *n == 1 { base } else { format!("{base}#{n}") };
        ctx.tag_id_by_ref.insert(reference.clone(), id);
        ctx.tag_ref_by_id.insert(id, reference);
    }
    let mut stmt = conn.prepare("SELECT id, name FROM scripts ORDER BY id")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let (id, name): (i64, String) = (r.get(0)?, lenient_text(r, 1)?);
        ctx.script_id_by_name.entry(name.clone()).or_insert(id);
        ctx.script_name_by_id.insert(id, name);
    }
    let mut stmt = conn.prepare("SELECT id, name FROM sim_profiles ORDER BY id")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let (id, name): (i64, String) = (r.get(0)?, lenient_text(r, 1)?);
        ctx.sim_id_by_name.entry(name.clone()).or_insert(id);
        ctx.sim_name_by_id.insert(id, name);
    }
    Ok(ctx)
}

/// Read every section from the database.
pub fn read_all(conn: &Connection) -> Result<(Snapshot, Ctx)> {
    let mut ctx = load_ctx(conn)?;
    let mut snap = Snapshot::new();
    for sec in spec::SECTIONS {
        let mut rows = match &sec.kind {
            Kind::Plain(p) => read_plain(conn, sec, p, &mut ctx),
            Kind::Tags => read_tags(conn, sec, &mut ctx),
            Kind::Screens => read_screens(conn, sec, &mut ctx),
            Kind::Settings => read_settings(conn, sec, &mut ctx),
        }
        .with_context(|| format!("reading {}", sec.name))?;
        order_rows(&mut rows);
        snap.insert(sec.name, rows);
    }
    Ok((snap, ctx))
}

fn columns(fields: &[Field]) -> Vec<&Field> {
    fields.iter().filter(|f| f.is_column()).collect()
}

fn select_list(cols: &[&Field]) -> String {
    cols.iter()
        .map(|f| format!("\"{}\"", f.col))
        .collect::<Vec<_>>()
        .join(", ")
}

fn read_cols(r: &SqlRow<'_>, start: usize, cols: &[&Field], ctx: &mut Ctx) -> Result<Mapping> {
    let mut m = Mapping::new();
    for (i, f) in cols.iter().enumerate() {
        if let Some(v) = cell_to_yaml(r.get_ref(start + i)?, f.ty, f.col, &mut ctx.warnings) {
            m.insert(text(f.name), v);
        }
    }
    Ok(m)
}

/// Normalise a freshly read entity, turning schema violations into warnings.
fn finish(sec: &Section, raw: Mapping, handle: Option<Sql>, ctx: &mut Ctx) -> Option<Row> {
    let mut errs = Vec::new();
    let entity = norm::normalize(sec.fields, &raw, "", &mut errs);
    if !errs.is_empty() {
        ctx.warnings.push(format!(
            "skipped a {} that does not fit the project schema: {}",
            sec.label,
            errs.join("; ")
        ));
        return None;
    }
    Some(Row {
        key: entity_key(sec, &entity),
        ord: 0,
        entity,
        handle,
    })
}

fn read_plain(conn: &Connection, sec: &Section, p: &Plain, ctx: &mut Ctx) -> Result<Vec<Row>> {
    let cols = columns(sec.fields);
    let id_sel = match p.pk {
        spec::Pk::Auto => "id".to_string(),
        spec::Pk::Text => "id".to_string(),
    };
    let sql = format!(
        "SELECT {id_sel}, {} FROM \"{}\" ORDER BY rowid",
        select_list(&cols),
        p.table
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let handle = match p.pk {
            spec::Pk::Auto => Sql::Integer(r.get(0)?),
            spec::Pk::Text => Sql::Text(lenient_text(r, 0)?),
        };
        let mut raw = read_cols(r, 1, &cols, ctx)?;
        for child in p.children {
            let Some(list) = sec.fields.iter().find(|f| f.name == child.yaml) else {
                continue;
            };
            let spec::Ty::List(sub) = list.ty else {
                continue;
            };
            let ccols = columns(sub);
            let csql = format!(
                "SELECT {} FROM \"{}\" WHERE \"{}\" = ?1 ORDER BY {}",
                select_list(&ccols),
                child.table,
                child.fk,
                child.order
            );
            let mut cstmt = conn.prepare_cached(&csql)?;
            let mut crows = cstmt.query([&handle])?;
            let mut items = Vec::new();
            while let Some(cr) = crows.next()? {
                items.push(Value::Mapping(read_cols(cr, 0, &ccols, ctx)?));
            }
            if !items.is_empty() {
                raw.insert(text(child.yaml), Value::Sequence(items));
            }
        }
        if let Some(h) = p.hook {
            (h.export)(&mut raw, ctx);
        }
        if let Some(row) = finish(sec, raw, Some(handle), ctx) {
            out.push(row);
        }
    }
    Ok(out)
}

/// Rewrite the numeric ids flow nodes use into natural keys.
pub fn flow_export(raw: &mut Mapping, ctx: &mut Ctx) {
    let Some(Value::Sequence(nodes)) = raw.get_mut("nodes") else {
        return;
    };
    for node in nodes {
        let Value::Mapping(n) = node else { continue };
        let kind = n
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match kind.as_str() {
            "write_out" => {
                if let Some(id) = n.get("data_source_id").and_then(Value::as_i64) {
                    match ctx.tag_ref_by_id.get(&id) {
                        Some(r) => {
                            n.remove("data_source_id");
                            n.insert(text("tag"), text(r.clone()));
                        }
                        None => ctx
                            .warnings
                            .push(format!("flow node refers to missing tag id {id}")),
                    }
                }
            }
            "script" => {
                if let Some(id) = n.get("script_id").and_then(Value::as_i64) {
                    match ctx.script_name_by_id.get(&id) {
                        Some(name) => {
                            n.remove("script_id");
                            n.insert(text("script"), text(name.clone()));
                        }
                        None => ctx
                            .warnings
                            .push(format!("flow node refers to missing script id {id}")),
                    }
                }
            }
            _ => {}
        }
    }
}

// ── Tags ──

const TAG_SELECT: &str = "SELECT id, key, device_id, name, unit, color, decimals, min_val, max_val, \
    base_val, sort_order, chart_type, source_type, source_config, path, equipment_id, sim_value, \
    warn_val, crit_val, on_delay_secs, off_delay_secs, writable, write_min, write_max, \
    write_max_rate, write_requires_reason, write_deadband FROM data_sources ORDER BY id";

fn read_tags(conn: &Connection, sec: &Section, ctx: &mut Ctx) -> Result<Vec<Row>> {
    use spec::Ty::{Bool, Int, Json, Real, Text};
    let mut stmt = conn.prepare(TAG_SELECT)?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let mut cell = |i: usize, ty: spec::Ty, name: &str| -> Result<Option<Value>> {
            Ok(cell_to_yaml(r.get_ref(i)?, ty, name, &mut ctx.warnings))
        };
        let mut m = Mapping::new();
        let put = |m: &mut Mapping, k: &str, v: Option<Value>| {
            if let Some(v) = v {
                m.insert(text(k), v);
            }
        };
        put(&mut m, "key", cell(1, Text, "key")?);
        put(&mut m, "device", cell(2, Text, "device_id")?);
        put(&mut m, "name", cell(3, Text, "name")?);
        put(&mut m, "unit", cell(4, Text, "unit")?);
        put(&mut m, "color", cell(5, Text, "color")?);
        put(&mut m, "decimals", cell(6, Int, "decimals")?);
        put(&mut m, "min", cell(7, Real, "min_val")?);
        put(&mut m, "max", cell(8, Real, "max_val")?);
        put(&mut m, "base", cell(9, Real, "base_val")?);
        put(&mut m, "sort_order", cell(10, Int, "sort_order")?);
        put(&mut m, "chart_type", cell(11, Text, "chart_type")?);
        put(&mut m, "source_type", cell(12, Text, "source_type")?);
        let mut source_config = cell(13, Json, "source_config")?;
        put(&mut m, "path", cell(14, Text, "path")?);
        put(&mut m, "equipment_id", cell(15, Text, "equipment_id")?);
        put(&mut m, "sim_value", cell(16, Real, "sim_value")?);

        // Alarm thresholds: columns plus the keys the alarm evaluator reads
        // out of source_config.
        let mut alarm = Mapping::new();
        put(&mut alarm, "warn", cell(17, Real, "warn_val")?);
        put(&mut alarm, "crit", cell(18, Real, "crit_val")?);
        put(&mut alarm, "on_delay_secs", cell(19, Int, "on_delay_secs")?);
        put(
            &mut alarm,
            "off_delay_secs",
            cell(20, Int, "off_delay_secs")?,
        );
        if let Some(Value::Mapping(cfg)) = source_config.as_mut() {
            for k in ["low_warn", "low_crit", "deadband"] {
                if cfg.get(k).is_some_and(Value::is_number) {
                    alarm.insert(text(k), cfg.remove(k).unwrap_or(Value::Null));
                }
            }
        }
        put(&mut m, "source_config", source_config);
        put(&mut m, "alarm", Some(Value::Mapping(alarm)));

        let mut write = Mapping::new();
        put(&mut write, "writable", cell(21, Bool, "writable")?);
        put(&mut write, "min", cell(22, Real, "write_min")?);
        put(&mut write, "max", cell(23, Real, "write_max")?);
        put(&mut write, "max_rate", cell(24, Int, "write_max_rate")?);
        put(
            &mut write,
            "requires_reason",
            cell(25, Bool, "write_requires_reason")?,
        );
        put(&mut write, "deadband", cell(26, Real, "write_deadband")?);
        put(&mut m, "write", Some(Value::Mapping(write)));

        let reference = ctx.tag_ref_by_id.get(&id).cloned().unwrap_or_default();
        if reference != tag_base_ref(&m) {
            m.insert(text("ref"), text(reference));
        }
        if let Some(row) = finish(sec, m, Some(Sql::Integer(id)), ctx) {
            out.push(row);
        }
    }
    Ok(out)
}

// ── Screens ──

type ByScreen = BTreeMap<String, Vec<Mapping>>;

fn group(
    conn: &Connection,
    sql: &str,
    mut f: impl FnMut(&SqlRow<'_>) -> Result<Option<Mapping>>,
) -> Result<ByScreen> {
    let mut out: ByScreen = BTreeMap::new();
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let screen = lenient_text(r, 0)?;
        if let Some(m) = f(r)? {
            out.entry(screen).or_default().push(m);
        }
    }
    Ok(out)
}

fn read_screens(conn: &Connection, sec: &Section, ctx: &mut Ctx) -> Result<Vec<Row>> {
    use spec::Ty::{Json, Real, Text};

    // Per-screen child tables, grouped by screen id.
    let mut membership: ByScreen = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT screen_id, data_source_id, sort_order FROM screen_data_sources ORDER BY screen_id, sort_order, data_source_id",
        )?;
        let mut rows = stmt.query([])?;
        let mut tmp: Vec<(String, String, i64)> = Vec::new();
        while let Some(r) = rows.next()? {
            let (screen, id, so): (String, i64, i64) = (lenient_text(r, 0)?, r.get(1)?, r.get(2)?);
            match ctx.tag_ref_by_id.get(&id) {
                Some(reference) => tmp.push((screen, reference.clone(), so)),
                None => ctx.warnings.push(format!(
                    "screen '{screen}' lists missing tag id {id}; left out of the export"
                )),
            }
        }
        tmp.sort_by(|a, b| (&a.0, a.2, &a.1).cmp(&(&b.0, b.2, &b.1)));
        for (screen, reference, so) in tmp {
            let mut m = Mapping::new();
            m.insert(text("ref"), text(reference));
            m.insert(text("sort_order"), int(so));
            membership.entry(screen).or_default().push(m);
        }
    }

    let mut layouts: BTreeMap<String, Mapping> = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT screen_id, breakpoint, layout FROM screen_layouts ORDER BY screen_id, breakpoint",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let screen = lenient_text(r, 0)?;
            let bp = lenient_text(r, 1)?;
            let mut v = cell_to_yaml(r.get_ref(2)?, Json, "layout", &mut ctx.warnings)
                .unwrap_or(Value::Sequence(Vec::new()));
            if let Value::Sequence(items) = &mut v {
                for item in items {
                    if let Value::Mapping(m) = item
                        && let Some(id) =
                            m.get("i").map(display).and_then(|s| s.parse::<i64>().ok())
                        && let Some(reference) = ctx.tag_ref_by_id.get(&id)
                    {
                        m.remove("i");
                        m.insert(text("tag"), text(reference.clone()));
                    }
                }
            }
            layouts
                .entry(screen)
                .or_default()
                .insert(text(bp), norm::canon(v));
        }
    }

    let mut canvas: ByScreen = BTreeMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT screen_id, data_source_id, x, y, width, height FROM canvas_layouts")?;
        let mut rows = stmt.query([])?;
        let mut tmp: Vec<(String, String, Mapping)> = Vec::new();
        while let Some(r) = rows.next()? {
            let screen = lenient_text(r, 0)?;
            let id: i64 = r.get(1)?;
            let Some(reference) = ctx.tag_ref_by_id.get(&id).cloned() else {
                ctx.warnings.push(format!(
                    "canvas of screen '{screen}' places missing tag id {id}; left out of the export"
                ));
                continue;
            };
            let mut m = Mapping::new();
            m.insert(text("tag"), text(reference.clone()));
            for (i, k) in [(2, "x"), (3, "y"), (4, "width"), (5, "height")] {
                if let Some(v) = cell_to_yaml(r.get_ref(i)?, Real, k, &mut ctx.warnings) {
                    m.insert(text(k), v);
                }
            }
            tmp.push((screen, reference, m));
        }
        tmp.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        for (screen, _, m) in tmp {
            canvas.entry(screen).or_default().push(m);
        }
    }

    let mut twin_layouts = group(
        conn,
        "SELECT screen_id, equipment_id, equipment_type, equipment_name, position_x, position_z, rotation_y FROM twin_layouts ORDER BY screen_id, equipment_id",
        |r| {
            let mut m = Mapping::new();
            let mut w = Vec::new();
            for (i, k, ty) in [
                (1, "equipment_id", Text),
                (2, "equipment_type", Text),
                (3, "equipment_name", Text),
                (4, "position_x", Real),
                (5, "position_z", Real),
                (6, "rotation_y", Real),
            ] {
                if let Some(v) = cell_to_yaml(r.get_ref(i)?, ty, k, &mut w) {
                    m.insert(text(k), v);
                }
            }
            Ok(Some(m))
        },
    )?;
    let mut twin_connections = group(
        conn,
        "SELECT screen_id, from_id, to_id FROM twin_connections ORDER BY screen_id, from_id, to_id",
        |r| {
            let mut m = Mapping::new();
            m.insert(text("from_id"), text(lenient_text(r, 1)?));
            m.insert(text("to_id"), text(lenient_text(r, 2)?));
            Ok(Some(m))
        },
    )?;

    let mut scada_eq: ByScreen = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT screen_id, equipment_id, equipment_type, equipment_name, x, y, width, height, rotation, sensor_bindings, config FROM scada_equipment ORDER BY screen_id, equipment_id",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let screen = lenient_text(r, 0)?;
            let mut m = Mapping::new();
            for (i, k, ty) in [
                (1, "equipment_id", Text),
                (2, "equipment_type", Text),
                (3, "equipment_name", Text),
                (4, "x", Real),
                (5, "y", Real),
                (6, "width", Real),
                (7, "height", Real),
                (8, "rotation", Real),
            ] {
                if let Some(v) = cell_to_yaml(r.get_ref(i)?, ty, k, &mut ctx.warnings) {
                    m.insert(text(k), v);
                }
            }
            if let Some(Value::Sequence(bindings)) =
                cell_to_yaml(r.get_ref(9)?, Json, "sensor_bindings", &mut ctx.warnings)
            {
                let mut refs = Vec::new();
                for b in bindings {
                    match b.as_i64().and_then(|id| ctx.tag_ref_by_id.get(&id)) {
                        Some(reference) => refs.push(text(reference.clone())),
                        None => ctx.warnings.push(format!(
                            "SCADA widget '{}' on screen '{screen}' is bound to a missing tag ({}); binding left out",
                            m.get("equipment_id").map(display).unwrap_or_default(),
                            display(&b)
                        )),
                    }
                }
                m.insert(text("sensors"), Value::Sequence(refs));
            }
            if let Some(v) = cell_to_yaml(r.get_ref(10)?, Json, "config", &mut ctx.warnings) {
                m.insert(text("config"), v);
            }
            scada_eq.entry(screen).or_default().push(m);
        }
    }
    let mut scada_conn = group(
        conn,
        "SELECT screen_id, connection_id, from_equipment, to_equipment, connection_type, waypoints FROM scada_connections ORDER BY screen_id, connection_id",
        |r| {
            let mut m = Mapping::new();
            let mut w = Vec::new();
            for (i, k, ty) in [
                (1, "connection_id", Text),
                (2, "from_equipment", Text),
                (3, "to_equipment", Text),
                (4, "connection_type", Text),
                (5, "waypoints", Json),
            ] {
                if let Some(v) = cell_to_yaml(r.get_ref(i)?, ty, k, &mut w) {
                    m.insert(text(k), v);
                }
            }
            Ok(Some(m))
        },
    )?;

    // Camera position of the 3D twin lives in the settings table.
    let mut cameras: BTreeMap<String, Value> = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT key, value FROM settings WHERE key LIKE 'twin\\_camera:%' ESCAPE '\\'",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let key = lenient_text(r, 0)?;
            let screen = key.trim_start_matches("twin_camera:").to_string();
            match serde_json::from_str::<serde_json::Value>(&lenient_text(r, 1)?)
                .map_err(anyhow::Error::from)
                .and_then(|j| json_to_yaml(&j))
            {
                Ok(v) => {
                    cameras.insert(screen, v);
                }
                Err(_) => ctx.warnings.push(format!(
                    "camera setting of screen '{screen}' is not valid JSON; left out"
                )),
            }
        }
    }

    // Screens that have a devices row.
    struct Dev {
        name: String,
        kind: String,
        sort_order: i64,
        sim_mode: bool,
        sim_profile: Option<i64>,
        sim_config: Option<Value>,
    }
    let mut devices: BTreeMap<String, Dev> = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, name, type, sort_order, simulation_mode, sim_profile_id, sim_config FROM devices",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let id = lenient_text(r, 0)?;
            let sim_config = cell_to_yaml(r.get_ref(6)?, Json, "sim_config", &mut ctx.warnings);
            let sim_profile = match r.get_ref(5)? {
                rusqlite::types::ValueRef::Integer(i) => Some(i),
                _ => None,
            };
            devices.insert(
                id,
                Dev {
                    name: lenient_text(r, 1)?,
                    kind: lenient_text(r, 2)?,
                    sort_order: match r.get_ref(3)? {
                        rusqlite::types::ValueRef::Integer(i) => i,
                        _ => 0,
                    },
                    sim_mode: matches!(r.get_ref(4)?, rusqlite::types::ValueRef::Integer(i) if i != 0),
                    sim_profile,
                    sim_config,
                },
            );
        }
    }

    let mut ids: Vec<String> = devices.keys().cloned().collect();
    for k in membership
        .keys()
        .chain(layouts.keys())
        .chain(canvas.keys())
        .chain(twin_layouts.keys())
        .chain(twin_connections.keys())
        .chain(scada_eq.keys())
        .chain(scada_conn.keys())
        .chain(cameras.keys())
    {
        ids.push(k.clone());
    }
    ids.sort();
    ids.dedup();

    let mut out = Vec::new();
    for id in ids {
        let mut m = Mapping::new();
        m.insert(text("id"), text(id.clone()));
        if let Some(d) = devices.get(&id) {
            m.insert(text("name"), text(d.name.clone()));
            m.insert(text("type"), text(d.kind.clone()));
            m.insert(text("sort_order"), int(d.sort_order));
            m.insert(text("simulation_mode"), Value::Bool(d.sim_mode));
            if let Some(pid) = d.sim_profile {
                match ctx.sim_name_by_id.get(&pid) {
                    Some(name) => {
                        m.insert(text("sim_profile"), text(name.clone()));
                    }
                    None => ctx.warnings.push(format!(
                        "screen '{id}' uses missing simulation profile id {pid}"
                    )),
                }
            }
            if let Some(c) = &d.sim_config {
                m.insert(text("sim_config"), c.clone());
            }
        }
        if let Some(t) = membership.remove(&id) {
            m.insert(
                text("tags"),
                Value::Sequence(t.into_iter().map(Value::Mapping).collect()),
            );
        }
        if let Some(l) = layouts.remove(&id) {
            m.insert(text("layouts"), Value::Mapping(l));
        }
        if let Some(c) = canvas.remove(&id) {
            m.insert(
                text("canvas"),
                Value::Sequence(c.into_iter().map(Value::Mapping).collect()),
            );
        }
        let mut twin = Mapping::new();
        if let Some(c) = cameras.remove(&id) {
            twin.insert(text("camera"), c);
        }
        if let Some(l) = twin_layouts.remove(&id) {
            twin.insert(
                text("layouts"),
                Value::Sequence(l.into_iter().map(Value::Mapping).collect()),
            );
        }
        if let Some(c) = twin_connections.remove(&id) {
            twin.insert(
                text("connections"),
                Value::Sequence(c.into_iter().map(Value::Mapping).collect()),
            );
        }
        if !twin.is_empty() {
            m.insert(text("twin"), Value::Mapping(twin));
        }
        let mut scada = Mapping::new();
        if let Some(e) = scada_eq.remove(&id) {
            scada.insert(
                text("equipment"),
                Value::Sequence(e.into_iter().map(Value::Mapping).collect()),
            );
        }
        if let Some(c) = scada_conn.remove(&id) {
            scada.insert(
                text("connections"),
                Value::Sequence(c.into_iter().map(Value::Mapping).collect()),
            );
        }
        if !scada.is_empty() {
            m.insert(text("scada"), Value::Mapping(scada));
        }
        if let Some(row) = finish(sec, m, Some(Sql::Text(id)), ctx) {
            out.push(row);
        }
    }
    Ok(out)
}

// ── Settings singleton ──

pub const SETTING_KEYS: &[&str] = &["alarm_cooldown_secs", "naming_template", "naming_regex"];
pub const OEE_PREFIX: &str = "oee_threshold_";

fn read_settings(conn: &Connection, sec: &Section, ctx: &mut Ctx) -> Result<Vec<Row>> {
    let mut m = Mapping::new();
    let mut naming = Mapping::new();
    let mut oee = Mapping::new();
    let mut stmt = conn.prepare(
        "SELECT key, value FROM settings WHERE key IN ('alarm_cooldown_secs','naming_template','naming_regex') OR key LIKE 'oee\\_threshold\\_%' ESCAPE '\\' ORDER BY key",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let (key, value) = (lenient_text(r, 0)?, lenient_text(r, 1)?);
        match key.as_str() {
            "alarm_cooldown_secs" => match value.trim().parse::<i64>() {
                Ok(n) => {
                    m.insert(text("alarm_cooldown_secs"), int(n));
                }
                Err(_) => ctx
                    .warnings
                    .push("setting alarm_cooldown_secs is not an integer; left out".into()),
            },
            "naming_template" => {
                naming.insert(text("template"), text(value));
            }
            "naming_regex" => {
                naming.insert(text("regex"), text(value));
            }
            k if k.starts_with(OEE_PREFIX) => {
                let device = k.trim_start_matches(OEE_PREFIX).to_string();
                match serde_json::from_str::<serde_json::Value>(&value)
                    .map_err(anyhow::Error::from)
                    .and_then(|j| json_to_yaml(&j))
                {
                    Ok(v) => {
                        oee.insert(text(device), v);
                    }
                    Err(_) => ctx.warnings.push(format!(
                        "OEE threshold of '{device}' is not valid JSON; left out"
                    )),
                }
            }
            _ => {}
        }
    }
    drop(rows);
    drop(stmt);
    if !naming.is_empty() {
        m.insert(text("naming_convention"), Value::Mapping(naming));
    }
    if !oee.is_empty() {
        m.insert(text("oee_thresholds"), norm::canon(Value::Mapping(oee)));
    }
    let thresholds = conn.query_row(
        "SELECT failed_login_threshold, failed_login_window_secs, suspicious_401_threshold, suspicious_401_window_secs, rate_limit_threshold, rate_limit_window_secs FROM security_thresholds WHERE id = 1",
        [],
        |r| {
            let mut out = Vec::new();
            for i in 0..6 {
                out.push(r.get::<_, i64>(i)?);
            }
            Ok(out)
        },
    );
    if let Ok(vals) = thresholds {
        let mut t = Mapping::new();
        for (f, v) in spec::SECURITY_THRESHOLDS.iter().zip(vals) {
            t.insert(text(f.name), int(v));
        }
        m.insert(text("security_thresholds"), Value::Mapping(t));
    }
    if let Ok(cfg) = conn.query_row(
        "SELECT config FROM compliance_config WHERE id = 1",
        [],
        |r| r.get::<_, String>(0),
    ) {
        match serde_json::from_str::<serde_json::Value>(&cfg)
            .map_err(anyhow::Error::from)
            .and_then(|j| json_to_yaml(&j))
        {
            Ok(v) => {
                m.insert(text("compliance"), v);
            }
            Err(_) => ctx
                .warnings
                .push("compliance configuration is not valid JSON; left out".into()),
        }
    }
    let mut out = Vec::new();
    if let Some(row) = finish(sec, m, None, ctx)
        && !row.entity.is_empty()
    {
        out.push(row);
    }
    Ok(out)
}

// ── Public entry points ──

/// ISO-8601 UTC timestamp for the manifest. Honours `SOURCE_DATE_EPOCH` so
/// exports can be made reproducible.
pub fn generated_at() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or_else(crate::timeutil::now_secs);
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Build a document from a snapshot, replacing secrets with `${ENV}` references.
pub fn doc_from_snapshot(snap: &Snapshot, warnings: Vec<String>) -> ProjectDoc {
    let mut doc = ProjectDoc {
        format: FORMAT.to_string(),
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        generated_at: generated_at(),
        sections: BTreeMap::new(),
        warnings,
    };
    for sec in spec::SECTIONS {
        let rows = snap.get(sec.name).map(Vec::as_slice).unwrap_or(&[]);
        let mut items = Vec::with_capacity(rows.len());
        for r in rows {
            let mut e = r.entity.clone();
            secrets::redact(sec.name, &r.label(), &mut e);
            items.push(e);
        }
        doc.sections.insert(sec.name.to_string(), items);
    }
    doc
}

/// Read the whole configuration of the database as a project document.
///
/// Runtime data (telemetry, alarm history, audit logs, users, API keys,
/// sessions, licence activation) is not part of a project, and every secret is
/// replaced by a `${ENV}` reference.
pub async fn export_project_yaml(storage: &Storage) -> Result<ProjectDoc> {
    storage
        .with_conn(|conn| {
            let tx = conn.transaction()?;
            let (snap, ctx) = read_all(&tx)?;
            Ok(doc_from_snapshot(&snap, ctx.warnings))
        })
        .await
}
