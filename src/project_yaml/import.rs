//! Project document -> database: plan (dry run) and apply.
//!
//! Both run the same code inside one SQLite transaction; a dry run simply never
//! writes and rolls back.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, Transaction, params_from_iter};
use serde_yaml_neo::{Mapping, Value};

use super::export::{self, Ctx, Row, entity_key, label};
use super::model::{
    Action, Change, Counts, ImportPlan, InvalidProject, Issue, Mode, ProjectDoc, Severity,
};
use super::norm::{self, canon, display, get, int, text, yaml_to_json_text, yaml_to_sql};
use super::secrets;
use super::spec::{self, Child, Field, Kind, Pk, Plain, Section, Stamp, Ty};
use super::validate;
use crate::storage::Storage;

/// Where `${ENV}` references are resolved from.
pub type Env = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub fn process_env() -> Env {
    Arc::new(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
}

/// What the import would do to the database, without writing anything.
pub async fn plan_import(storage: &Storage, doc: &ProjectDoc, mode: Mode) -> Result<ImportPlan> {
    plan_import_with_env(storage, doc, mode, process_env()).await
}

/// Import the project in one transaction. Nothing is written when any check
/// fails.
pub async fn apply_import(storage: &Storage, doc: &ProjectDoc, mode: Mode) -> Result<ImportPlan> {
    apply_import_with_env(storage, doc, mode, process_env()).await
}

pub async fn plan_import_with_env(
    storage: &Storage,
    doc: &ProjectDoc,
    mode: Mode,
    env: Env,
) -> Result<ImportPlan> {
    run_async(storage, doc, mode, false, env).await
}

pub async fn apply_import_with_env(
    storage: &Storage,
    doc: &ProjectDoc,
    mode: Mode,
    env: Env,
) -> Result<ImportPlan> {
    run_async(storage, doc, mode, true, env).await
}

async fn run_async(
    storage: &Storage,
    doc: &ProjectDoc,
    mode: Mode,
    write: bool,
    env: Env,
) -> Result<ImportPlan> {
    let doc = doc.clone();
    storage
        .with_conn(move |conn| run(conn, &doc, mode, write, &*env))
        .await
}

struct Work {
    sec: &'static Section,
    action: Action,
    entity: Mapping,
    existing: Option<Row>,
}

fn run(
    conn: &mut Connection,
    doc: &ProjectDoc,
    mode: Mode,
    write: bool,
    env: &(dyn Fn(&str) -> Option<String> + Send + Sync),
) -> Result<ImportPlan> {
    let tx = conn.transaction().context("begin transaction")?;
    let (snap, mut ctx) = export::read_all(&tx)?;

    let known = (mode == Mode::Merge).then(|| validate::Known::from_snapshot(&snap));
    let issues = validate::validate_doc(doc, known.as_ref(), false);
    if issues.iter().any(|i| i.severity == Severity::Error) {
        return Err(InvalidProject(issues).into());
    }

    let mut plan = ImportPlan {
        mode,
        dry_run: !write,
        totals: Counts::default(),
        sections: BTreeMap::new(),
        changes: Vec::new(),
        warnings: issues.iter().map(|i| i.to_string()).collect(),
    };
    let mut work: Vec<Work> = Vec::new();
    let mut secret_errors: Vec<Issue> = Vec::new();

    for sec in spec::SECTIONS {
        let existing = snap.get(sec.name).map(Vec::as_slice).unwrap_or(&[]);
        let by_identity: HashMap<(&str, usize), &Row> = existing
            .iter()
            .map(|r| ((r.key.as_str(), r.ord), r))
            .collect();
        let mut counts = Counts::default();

        let mut items: Vec<(String, Mapping)> = doc
            .entities(sec.name)
            .iter()
            .map(|e| (entity_key(sec, e), e.clone()))
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let mut seen: HashSet<(String, usize)> = HashSet::new();
        let mut prev: Option<String> = None;
        let mut ord = 0;
        for (key, mut entity) in items {
            if prev.as_deref() == Some(key.as_str()) {
                ord += 1;
            } else {
                ord = 0;
                prev = Some(key.clone());
            }
            let current = by_identity.get(&(key.as_str(), ord)).copied();
            seen.insert((key.clone(), ord));

            let fallback = current
                .map(|r| {
                    let mut e = r.entity.clone();
                    secrets::redact(sec.name, &r.label(), &mut e)
                })
                .unwrap_or_default();
            let missing = secrets::resolve(sec.name, &mut entity, env, &fallback);
            if !missing.is_empty() {
                secret_errors.push(Issue {
                    severity: Severity::Error,
                    at: format!("{}[{}]", sec.name, label(&key, ord)),
                    message: format!("environment variable(s) not set: {}", missing.join(", ")),
                });
            }

            match current {
                Some(row) => {
                    let changed = changed_fields(&row.entity, &entity);
                    if changed.is_empty() {
                        counts.unchanged += 1;
                    } else {
                        counts.update += 1;
                        plan.changes.push(Change {
                            section: sec.name.into(),
                            key: label(&key, ord),
                            action: Action::Update,
                            fields: changed,
                        });
                        work.push(Work {
                            sec,
                            action: Action::Update,
                            entity,
                            existing: Some(row.clone()),
                        });
                    }
                }
                None => {
                    counts.create += 1;
                    plan.changes.push(Change {
                        section: sec.name.into(),
                        key: label(&key, ord),
                        action: Action::Create,
                        fields: Vec::new(),
                    });
                    work.push(Work {
                        sec,
                        action: Action::Create,
                        entity,
                        existing: None,
                    });
                }
            }
        }

        if mode == Mode::Replace {
            for row in existing {
                if !seen.contains(&(row.key.clone(), row.ord)) {
                    counts.delete += 1;
                    plan.changes.push(Change {
                        section: sec.name.into(),
                        key: row.label(),
                        action: Action::Delete,
                        fields: Vec::new(),
                    });
                    work.push(Work {
                        sec,
                        action: Action::Delete,
                        entity: Mapping::new(),
                        existing: Some(row.clone()),
                    });
                }
            }
        }

        plan.totals.create += counts.create;
        plan.totals.update += counts.update;
        plan.totals.delete += counts.delete;
        plan.totals.unchanged += counts.unchanged;
        plan.sections.insert(sec.name.to_string(), counts);
    }

    if !secret_errors.is_empty() {
        return Err(InvalidProject(secret_errors).into());
    }

    if write {
        // Deletes first (children before parents), then creates and updates in
        // dependency order.
        for w in work.iter().rev().filter(|w| w.action == Action::Delete) {
            delete_entity(
                &tx,
                w.sec,
                w.existing.as_ref().context("missing row")?,
                &mut ctx,
            )
            .with_context(|| {
                format!(
                    "deleting {} {}",
                    w.sec.label,
                    w.existing.as_ref().map(Row::label).unwrap_or_default()
                )
            })?;
        }
        for w in work.iter().filter(|w| w.action != Action::Delete) {
            write_entity(&tx, w.sec, &w.entity, w.existing.as_ref(), &mut ctx, mode).with_context(
                || format!("writing {} {}", w.sec.label, entity_key(w.sec, &w.entity)),
            )?;
        }
        tx.commit().context("commit import")?;
    }
    Ok(plan)
}

/// Top-level fields that differ between two canonical entities.
fn changed_fields(existing: &Mapping, wanted: &Mapping) -> Vec<String> {
    let mut a = existing.clone();
    let mut b = wanted.clone();
    for k in spec::INPUT_ONLY {
        a.remove(*k);
        b.remove(*k);
    }
    let mut names: Vec<String> = a.keys().chain(b.keys()).map(display).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|n| a.get(n.as_str()) != b.get(n.as_str()))
        .collect()
}

// ── Writing ──

fn write_entity(
    tx: &Transaction<'_>,
    sec: &Section,
    entity: &Mapping,
    existing: Option<&Row>,
    ctx: &mut Ctx,
    mode: Mode,
) -> Result<()> {
    match &sec.kind {
        Kind::Plain(p) => write_plain(tx, sec, p, entity, existing, ctx),
        Kind::Tags => write_tag(tx, entity, existing, ctx),
        Kind::Screens => write_screen(tx, entity, existing, ctx),
        Kind::Settings => write_settings(tx, entity, mode == Mode::Replace),
    }
}

fn delete_entity(tx: &Transaction<'_>, sec: &Section, row: &Row, ctx: &mut Ctx) -> Result<()> {
    match &sec.kind {
        Kind::Plain(p) => delete_plain(tx, p, row, ctx),
        Kind::Tags => delete_tag(tx, row, ctx),
        Kind::Screens => delete_screen(tx, &row.key),
        Kind::Settings => write_settings(tx, &Mapping::new(), true),
    }
}

fn now(stamp: Stamp) -> Sql {
    match stamp {
        Stamp::Secs => Sql::Integer(crate::timeutil::now_secs()),
        Stamp::Iso => Sql::Text(crate::timeutil::now_iso()),
    }
}

fn q(name: &str) -> String {
    format!("\"{name}\"")
}

fn column_values(fields: &[Field], entity: &Mapping) -> Result<Vec<(&'static str, Sql)>> {
    let mut out = Vec::new();
    for f in fields.iter().filter(|f| f.is_column()) {
        out.push((f.col, yaml_to_sql(f, entity.get(f.name))?));
    }
    Ok(out)
}

fn write_plain(
    tx: &Transaction<'_>,
    sec: &Section,
    p: &Plain,
    entity: &Mapping,
    existing: Option<&Row>,
    ctx: &mut Ctx,
) -> Result<()> {
    let mut entity = entity.clone();
    if let Some(h) = p.hook {
        (h.import)(&mut entity, ctx)?;
    }
    let mut vals = column_values(sec.fields, &entity)?;
    let handle: Sql = match existing {
        Some(row) => {
            let handle = row.handle.clone().context("existing row has no handle")?;
            let mut sets: Vec<(&str, Sql)> = vals
                .into_iter()
                .filter(|(c, _)| !(matches!(p.pk, Pk::Text) && *c == "id"))
                .collect();
            if let Some(u) = p.updated {
                sets.push((u, now(p.stamp)));
            }
            sets.extend(derived(p.table, &entity));
            let assigns: Vec<String> = sets
                .iter()
                .enumerate()
                .map(|(i, (c, _))| format!("{} = ?{}", q(c), i + 1))
                .collect();
            let sql = format!(
                "UPDATE {} SET {} WHERE id = ?{}",
                q(p.table),
                assigns.join(", "),
                sets.len() + 1
            );
            let mut args: Vec<Sql> = sets.into_iter().map(|(_, v)| v).collect();
            args.push(handle.clone());
            tx.execute(&sql, params_from_iter(args))?;
            handle
        }
        None => {
            if let Some(c) = p.created {
                vals.push((c, now(p.stamp)));
            }
            if let Some(u) = p.updated {
                vals.push((u, now(p.stamp)));
            }
            vals.extend(derived(p.table, &entity));
            let names: Vec<String> = vals.iter().map(|(c, _)| q(c)).collect();
            let marks: Vec<String> = (1..=vals.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({})",
                q(p.table),
                names.join(", "),
                marks.join(", ")
            );
            let pk_text = vals
                .iter()
                .find(|(c, _)| *c == "id")
                .map(|(_, v)| v.clone());
            tx.execute(&sql, params_from_iter(vals.into_iter().map(|(_, v)| v)))?;
            match p.pk {
                Pk::Auto => Sql::Integer(tx.last_insert_rowid()),
                Pk::Text => pk_text.context("text primary key missing")?,
            }
        }
    };

    remember_named(p.table, &handle, &entity, ctx);

    for child in p.children {
        let wanted = rows_of(&entity, child.yaml);
        let current = existing
            .map(|r| rows_of(&r.entity, child.yaml))
            .unwrap_or_default();
        if existing.is_some() && wanted == current {
            continue;
        }
        write_children(tx, sec, p, child, &handle, &wanted)?;
    }
    Ok(())
}

/// Later sections name scripts and simulation profiles; keep the lookups current.
fn remember_named(table: &str, handle: &Sql, entity: &Mapping, ctx: &mut Ctx) {
    let Sql::Integer(id) = handle else { return };
    let name = norm::get_str(entity, "name");
    match table {
        "scripts" => {
            if let Some(old) = ctx.script_name_by_id.insert(*id, name.clone())
                && old != name
            {
                ctx.script_id_by_name.remove(&old);
            }
            ctx.script_id_by_name.insert(name, *id);
        }
        "sim_profiles" => {
            if let Some(old) = ctx.sim_name_by_id.insert(*id, name.clone())
                && old != name
            {
                ctx.sim_id_by_name.remove(&old);
            }
            ctx.sim_id_by_name.insert(name, *id);
        }
        _ => {}
    }
}

/// Columns the server computes from other fields when it saves an entity
/// through its API; the project does not carry them.
fn derived(table: &str, entity: &Mapping) -> Vec<(&'static str, Sql)> {
    let opt = |s: Option<String>| s.map(Sql::Text).unwrap_or(Sql::Null);
    match table {
        "scripts" => vec![(
            "syntax_error",
            opt(crate::script_engine::validate_syntax(&norm::get_str(
                entity, "source",
            ))),
        )],
        "custom_metrics" => {
            let formula = norm::get_str(entity, "formula");
            let refs = crate::analytics::custom::extract_tag_refs(&formula);
            vec![
                (
                    "tag_refs",
                    Sql::Text(serde_json::to_string(&refs).unwrap_or_else(|_| "[]".into())),
                ),
                (
                    "validation_error",
                    opt(crate::analytics::custom::validate_formula(&formula).err()),
                ),
            ]
        }
        _ => Vec::new(),
    }
}

fn rows_of(entity: &Mapping, name: &str) -> Vec<Mapping> {
    match entity.get(name) {
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(|v| v.as_mapping().cloned())
            .collect(),
        _ => Vec::new(),
    }
}

fn write_children(
    tx: &Transaction<'_>,
    sec: &Section,
    p: &Plain,
    child: &Child,
    parent: &Sql,
    rows: &[Mapping],
) -> Result<()> {
    tx.execute(
        &format!("DELETE FROM {} WHERE {} = ?1", q(child.table), q(child.fk)),
        [parent],
    )?;
    let Some(Ty::List(sub)) = sec
        .fields
        .iter()
        .find(|f| f.name == child.yaml)
        .map(|f| f.ty)
    else {
        bail!("no field list for child '{}'", child.yaml);
    };
    let _ = p;
    for row in rows {
        let mut vals = column_values(sub, row)?;
        vals.push((child.fk, parent.clone()));
        if child.created {
            vals.push(("created_at", Sql::Integer(crate::timeutil::now_secs())));
        }
        let names: Vec<String> = vals.iter().map(|(c, _)| q(c)).collect();
        let marks: Vec<String> = (1..=vals.len()).map(|i| format!("?{i}")).collect();
        tx.execute(
            &format!(
                "INSERT INTO {} ({}) VALUES ({})",
                q(child.table),
                names.join(", "),
                marks.join(", ")
            ),
            params_from_iter(vals.into_iter().map(|(_, v)| v)),
        )?;
    }
    Ok(())
}

fn delete_plain(tx: &Transaction<'_>, p: &Plain, row: &Row, ctx: &mut Ctx) -> Result<()> {
    let handle = row.handle.clone().context("existing row has no handle")?;
    if let Sql::Integer(id) = &handle {
        match p.table {
            "scripts" => {
                if let Some(n) = ctx.script_name_by_id.remove(id) {
                    ctx.script_id_by_name.remove(&n);
                }
            }
            "sim_profiles" => {
                if let Some(n) = ctx.sim_name_by_id.remove(id) {
                    ctx.sim_id_by_name.remove(&n);
                }
            }
            _ => {}
        }
    }
    for child in p.children {
        tx.execute(
            &format!("DELETE FROM {} WHERE {} = ?1", q(child.table), q(child.fk)),
            [&handle],
        )?;
    }
    match p.table {
        "scripts" => {
            tx.execute(
                "DELETE FROM script_executions WHERE script_id = ?1",
                [&handle],
            )?;
        }
        "sim_profiles" => {
            tx.execute(
                "UPDATE devices SET sim_profile_id = NULL WHERE sim_profile_id = ?1",
                [&handle],
            )?;
        }
        _ => {}
    }
    tx.execute(
        &format!("DELETE FROM {} WHERE id = ?1", q(p.table)),
        [&handle],
    )?;
    Ok(())
}

/// Replace natural keys inside flow nodes by the ids the engine reads.
pub fn flow_import(entity: &mut Mapping, ctx: &Ctx) -> Result<()> {
    let Some(Value::Sequence(nodes)) = entity.get_mut("nodes") else {
        return Ok(());
    };
    for node in nodes {
        let Value::Mapping(n) = node else { continue };
        if let Some(r) = n.get("tag").and_then(Value::as_str).map(str::to_string) {
            let id = ctx
                .tag_id_by_ref
                .get(&r)
                .with_context(|| format!("flow node refers to unknown tag '{r}'"))?;
            n.remove("tag");
            n.insert(text("data_source_id"), int(*id));
        }
        if let Some(s) = n.get("script").and_then(Value::as_str).map(str::to_string) {
            let id = ctx
                .script_id_by_name
                .get(&s)
                .with_context(|| format!("flow node refers to unknown script '{s}'"))?;
            n.remove("script");
            n.insert(text("script_id"), int(*id));
        }
    }
    Ok(())
}

// ── Tags ──

fn sub(entity: &Mapping, name: &str) -> Mapping {
    entity
        .get(name)
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default()
}

fn field<'a>(fields: &'a [Field], name: &str) -> &'a Field {
    fields.iter().find(|f| f.name == name).expect("known field")
}

fn tag_values(entity: &Mapping) -> Result<Vec<(&'static str, Sql)>> {
    let f = |n: &str| field(spec::TAG_FIELDS, n);
    let alarm = sub(entity, "alarm");
    let write = sub(entity, "write");
    let sql = |fl: &Field, m: &Mapping| yaml_to_sql(fl, m.get(fl.name));

    // Thresholds that live inside source_config next to the tag's own keys.
    let mut cfg = match get(entity, f("source_config")) {
        Value::Mapping(m) => m,
        _ => Mapping::new(),
    };
    for k in ["low_warn", "low_crit", "deadband"] {
        if let Some(v) = alarm.get(k) {
            cfg.insert(text(k), v.clone());
        }
    }
    let source_config = Sql::Text(yaml_to_json_text(&canon(Value::Mapping(cfg)))?);

    let a = |n: &str| field(spec::TAG_ALARM, n);
    let w = |n: &str| field(spec::TAG_WRITE, n);
    Ok(vec![
        ("key", sql(f("key"), entity)?),
        ("device_id", sql(f("device"), entity)?),
        ("name", sql(f("name"), entity)?),
        ("unit", sql(f("unit"), entity)?),
        ("color", sql(f("color"), entity)?),
        ("decimals", sql(f("decimals"), entity)?),
        ("min_val", sql(f("min"), entity)?),
        ("max_val", sql(f("max"), entity)?),
        ("base_val", sql(f("base"), entity)?),
        ("sort_order", sql(f("sort_order"), entity)?),
        ("chart_type", sql(f("chart_type"), entity)?),
        ("source_type", sql(f("source_type"), entity)?),
        ("source_config", source_config),
        ("path", sql(f("path"), entity)?),
        ("equipment_id", sql(f("equipment_id"), entity)?),
        ("sim_value", sql(f("sim_value"), entity)?),
        ("warn_val", sql(a("warn"), &alarm)?),
        ("crit_val", sql(a("crit"), &alarm)?),
        ("on_delay_secs", sql(a("on_delay_secs"), &alarm)?),
        ("off_delay_secs", sql(a("off_delay_secs"), &alarm)?),
        ("writable", sql(w("writable"), &write)?),
        ("write_min", sql(w("min"), &write)?),
        ("write_max", sql(w("max"), &write)?),
        ("write_max_rate", sql(w("max_rate"), &write)?),
        ("write_requires_reason", sql(w("requires_reason"), &write)?),
        ("write_deadband", sql(w("deadband"), &write)?),
    ])
}

fn write_tag(
    tx: &Transaction<'_>,
    entity: &Mapping,
    existing: Option<&Row>,
    ctx: &mut Ctx,
) -> Result<()> {
    let mut vals = tag_values(entity)?;
    let reference = entity_key(spec::section("tags").expect("tags section"), entity);
    match existing {
        Some(row) => {
            let Some(Sql::Integer(id)) = row.handle.clone() else {
                bail!("existing tag has no id");
            };
            let assigns: Vec<String> = vals
                .iter()
                .enumerate()
                .map(|(i, (c, _))| format!("{} = ?{}", q(c), i + 1))
                .collect();
            let n = vals.len();
            let mut args: Vec<Sql> = vals.into_iter().map(|(_, v)| v).collect();
            args.push(Sql::Integer(id));
            tx.execute(
                &format!(
                    "UPDATE data_sources SET {} WHERE id = ?{}",
                    assigns.join(", "),
                    n + 1
                ),
                params_from_iter(args),
            )?;
            // Keep both lookup directions in step if the ref was renamed in place.
            ctx.tag_id_by_ref.insert(reference.clone(), id);
            ctx.tag_ref_by_id.insert(id, reference);
        }
        None => {
            if let Some(hint) = entity.get("id_hint").and_then(Value::as_i64) {
                let taken: bool = tx
                    .query_row("SELECT 1 FROM data_sources WHERE id = ?1", [hint], |_| {
                        Ok(true)
                    })
                    .unwrap_or(false);
                if !taken {
                    vals.push(("id", Sql::Integer(hint)));
                }
            }
            let names: Vec<String> = vals.iter().map(|(c, _)| q(c)).collect();
            let marks: Vec<String> = (1..=vals.len()).map(|i| format!("?{i}")).collect();
            tx.execute(
                &format!(
                    "INSERT INTO data_sources ({}) VALUES ({})",
                    names.join(", "),
                    marks.join(", ")
                ),
                params_from_iter(vals.into_iter().map(|(_, v)| v)),
            )?;
            let id = tx.last_insert_rowid();
            ctx.tag_id_by_ref.insert(reference.clone(), id);
            ctx.tag_ref_by_id.insert(id, reference);
        }
    }
    Ok(())
}

fn delete_tag(tx: &Transaction<'_>, row: &Row, ctx: &mut Ctx) -> Result<()> {
    let Some(Sql::Integer(id)) = row.handle.clone() else {
        bail!("existing tag has no id");
    };
    tx.execute(
        "DELETE FROM screen_data_sources WHERE data_source_id = ?1",
        [id],
    )?;
    tx.execute("DELETE FROM canvas_layouts WHERE data_source_id = ?1", [id])?;
    tx.execute("DELETE FROM data_sources WHERE id = ?1", [id])?;
    if let Some(r) = ctx.tag_ref_by_id.remove(&id) {
        ctx.tag_id_by_ref.remove(&r);
    }
    Ok(())
}

// ── Screens ──

const SCREEN_TABLES: &[&str] = &[
    "screen_data_sources",
    "screen_layouts",
    "canvas_layouts",
    "twin_layouts",
    "twin_connections",
    "scada_equipment",
    "scada_connections",
];

fn delete_screen(tx: &Transaction<'_>, id: &str) -> Result<()> {
    for t in SCREEN_TABLES {
        tx.execute(&format!("DELETE FROM {t} WHERE screen_id = ?1"), [id])?;
    }
    tx.execute("DELETE FROM devices WHERE id = ?1", [id])?;
    tx.execute(
        "DELETE FROM settings WHERE key = ?1",
        [format!("twin_camera:{id}")],
    )?;
    Ok(())
}

fn tag_id(ctx: &Ctx, reference: &str, what: &str) -> Result<i64> {
    ctx.tag_id_by_ref
        .get(reference)
        .copied()
        .with_context(|| format!("{what} refers to unknown tag '{reference}'"))
}

fn write_screen(
    tx: &Transaction<'_>,
    entity: &Mapping,
    _existing: Option<&Row>,
    ctx: &mut Ctx,
) -> Result<()> {
    let id = norm::get_str(entity, "id");
    delete_screen(tx, &id)?;
    let f = |n: &str| field(spec::SCREEN_FIELDS, n);

    if entity.get("name").is_some() {
        let sim_profile = match entity.get("sim_profile").and_then(Value::as_str) {
            Some(name) => Sql::Integer(*ctx.sim_id_by_name.get(name).with_context(|| {
                format!("screen '{id}' uses unknown simulation profile '{name}'")
            })?),
            None => Sql::Null,
        };
        tx.execute(
            "INSERT INTO devices (id, name, type, sort_order, simulation_mode, sim_profile_id, sim_config) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params_from_iter(vec![
                Sql::Text(id.clone()),
                yaml_to_sql(f("name"), entity.get("name"))?,
                yaml_to_sql(f("type"), entity.get("type"))?,
                yaml_to_sql(f("sort_order"), entity.get("sort_order"))?,
                yaml_to_sql(f("simulation_mode"), entity.get("simulation_mode"))?,
                sim_profile,
                yaml_to_sql(f("sim_config"), entity.get("sim_config"))?,
            ]),
        )?;
    }

    for item in rows_of(entity, "tags") {
        let reference = norm::get_str(&item, "ref");
        let tid = tag_id(ctx, &reference, &format!("screen '{id}'"))?;
        tx.execute(
            "INSERT INTO screen_data_sources (screen_id, data_source_id, sort_order) VALUES (?1,?2,?3)",
            params_from_iter(vec![
                Sql::Text(id.clone()),
                Sql::Integer(tid),
                yaml_to_sql(field(spec::SCREEN_TAG, "sort_order"), item.get("sort_order"))?,
            ]),
        )?;
    }

    if let Some(Value::Mapping(layouts)) = entity.get("layouts") {
        for (bp, items) in layouts {
            let mut items = items.clone();
            if let Value::Sequence(list) = &mut items {
                for item in list {
                    let Value::Mapping(m) = item else { continue };
                    if let Some(r) = m.get("tag").and_then(Value::as_str).map(str::to_string) {
                        let tid = tag_id(ctx, &r, &format!("layout of screen '{id}'"))?;
                        m.remove("tag");
                        m.insert(text("i"), text(tid.to_string()));
                    }
                }
            }
            tx.execute(
                "INSERT INTO screen_layouts (screen_id, breakpoint, layout) VALUES (?1,?2,?3)",
                params_from_iter(vec![
                    Sql::Text(id.clone()),
                    Sql::Text(display(bp)),
                    Sql::Text(yaml_to_json_text(&canon(items))?),
                ]),
            )?;
        }
    }

    for item in rows_of(entity, "canvas") {
        let reference = norm::get_str(&item, "tag");
        let tid = tag_id(ctx, &reference, &format!("canvas of screen '{id}'"))?;
        let mut args = vec![Sql::Text(id.clone()), Sql::Integer(tid)];
        for n in ["x", "y", "width", "height"] {
            args.push(yaml_to_sql(field(spec::SCREEN_CANVAS, n), item.get(n))?);
        }
        tx.execute(
            "INSERT INTO canvas_layouts (screen_id, data_source_id, x, y, width, height) VALUES (?1,?2,?3,?4,?5,?6)",
            params_from_iter(args),
        )?;
    }

    let twin = sub(entity, "twin");
    for item in rows_of(&twin, "layouts") {
        let mut args = vec![Sql::Text(id.clone())];
        for n in [
            "equipment_id",
            "equipment_type",
            "equipment_name",
            "position_x",
            "position_z",
            "rotation_y",
        ] {
            args.push(yaml_to_sql(field(spec::TWIN_LAYOUT, n), item.get(n))?);
        }
        tx.execute(
            "INSERT INTO twin_layouts (screen_id, equipment_id, equipment_type, equipment_name, position_x, position_z, rotation_y) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params_from_iter(args),
        )?;
    }
    for item in rows_of(&twin, "connections") {
        tx.execute(
            "INSERT INTO twin_connections (screen_id, from_id, to_id) VALUES (?1,?2,?3)",
            params_from_iter(vec![
                Sql::Text(id.clone()),
                Sql::Text(norm::get_str(&item, "from_id")),
                Sql::Text(norm::get_str(&item, "to_id")),
            ]),
        )?;
    }
    if let Some(camera) = twin.get("camera") {
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params_from_iter(vec![
                Sql::Text(format!("twin_camera:{id}")),
                Sql::Text(yaml_to_json_text(camera)?),
            ]),
        )?;
    }

    let scada = sub(entity, "scada");
    for item in rows_of(&scada, "equipment") {
        let mut bindings = Vec::new();
        if let Some(Value::Sequence(refs)) = item.get("sensors") {
            for r in refs {
                bindings.push(serde_json::Value::from(tag_id(
                    ctx,
                    &display(r),
                    &format!("SCADA widget of screen '{id}'"),
                )?));
            }
        }
        let mut args = vec![Sql::Text(id.clone())];
        for n in [
            "equipment_id",
            "equipment_type",
            "equipment_name",
            "x",
            "y",
            "width",
            "height",
            "rotation",
        ] {
            args.push(yaml_to_sql(field(spec::SCADA_EQUIPMENT, n), item.get(n))?);
        }
        args.push(Sql::Text(serde_json::to_string(&bindings)?));
        args.push(yaml_to_sql(
            field(spec::SCADA_EQUIPMENT, "config"),
            item.get("config"),
        )?);
        tx.execute(
            "INSERT INTO scada_equipment (screen_id, equipment_id, equipment_type, equipment_name, x, y, width, height, rotation, sensor_bindings, config) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params_from_iter(args),
        )?;
    }
    for item in rows_of(&scada, "connections") {
        let mut args = vec![Sql::Text(id.clone())];
        for n in [
            "connection_id",
            "from_equipment",
            "to_equipment",
            "connection_type",
            "waypoints",
        ] {
            args.push(yaml_to_sql(field(spec::SCADA_CONNECTION, n), item.get(n))?);
        }
        tx.execute(
            "INSERT INTO scada_connections (screen_id, connection_id, from_equipment, to_equipment, connection_type, waypoints) VALUES (?1,?2,?3,?4,?5,?6)",
            params_from_iter(args),
        )?;
    }
    Ok(())
}

// ── Settings ──

fn upsert_setting(tx: &Transaction<'_>, key: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// Write the settings singleton. With `reset_missing`, whitelisted settings the
/// project does not mention go back to their defaults (replace mode).
fn write_settings(tx: &Transaction<'_>, entity: &Mapping, reset_missing: bool) -> Result<()> {
    let delete = |key: &str| -> Result<()> {
        tx.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    };

    match entity.get("alarm_cooldown_secs").and_then(Value::as_i64) {
        Some(n) => upsert_setting(tx, "alarm_cooldown_secs", &n.to_string())?,
        None if reset_missing => delete("alarm_cooldown_secs")?,
        None => {}
    }
    let naming = sub(entity, "naming_convention");
    for (field, key) in [("template", "naming_template"), ("regex", "naming_regex")] {
        match naming.get(field).and_then(Value::as_str) {
            Some(v) => upsert_setting(tx, key, v)?,
            None if reset_missing => delete(key)?,
            None => {}
        }
    }

    let oee = sub(entity, "oee_thresholds");
    if reset_missing {
        let keep: HashSet<String> = oee
            .keys()
            .map(|k| format!("{}{}", export::OEE_PREFIX, display(k)))
            .collect();
        let mut stmt = tx
            .prepare("SELECT key FROM settings WHERE key LIKE 'oee\\_threshold\\_%' ESCAPE '\\'")?;
        let keys: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?;
        drop(stmt);
        for k in keys.into_iter().filter(|k| !keep.contains(k)) {
            delete(&k)?;
        }
    }
    for (device, v) in &oee {
        upsert_setting(
            tx,
            &format!("{}{}", export::OEE_PREFIX, display(device)),
            &yaml_to_json_text(v)?,
        )?;
    }

    let has_thresholds = entity.get("security_thresholds").is_some();
    if has_thresholds || reset_missing {
        let t = sub(entity, "security_thresholds");
        let mut args = vec![Sql::Integer(1)];
        for f in spec::SECURITY_THRESHOLDS {
            args.push(yaml_to_sql(f, t.get(f.name))?);
        }
        tx.execute(
            "INSERT OR REPLACE INTO security_thresholds (id, failed_login_threshold, failed_login_window_secs, suspicious_401_threshold, suspicious_401_window_secs, rate_limit_threshold, rate_limit_window_secs) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params_from_iter(args),
        )?;
    }

    match entity.get("compliance") {
        Some(v) => {
            tx.execute(
                "INSERT INTO compliance_config (id, config) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET config = ?1",
                [yaml_to_json_text(v)?],
            )?;
        }
        None if reset_missing => {
            tx.execute("DELETE FROM compliance_config WHERE id = 1", [])?;
        }
        None => {}
    }
    Ok(())
}
