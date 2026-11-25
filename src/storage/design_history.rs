//! Designer version history (docs/version-history.md): an append-only change
//! log of design objects, named versions of a screen, and the published copy
//! of each screen's 2D rows.
//!
//! * **Objects** are a screen's widgets and connections and the project-wide
//!   equipment types, equipment instances and faceplates. Every writer diffs
//!   the objects it may touch before and after its write, inside its own
//!   transaction, and appends one row per added, changed or removed object,
//!   all with one batch id ([`append`]). REST, MCP, project import, equipment
//!   propagation, snapshot rollback, revert and tag renames are logged alike.
//! * **Versions** are snapshots of a screen's draft rows with a name and a
//!   message; restoring one never replays the log.
//! * **Draft and published**: `scada_equipment`/`scada_connections` are the
//!   draft. Publishing copies a version into the `*_published` tables;
//!   operators read those, and a screen never published falls back to the
//!   draft.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Storage;
use super::settings_scada::{
    read_connections, read_connections_from, read_equipment, read_equipment_from,
};
use crate::models::{ScadaConnection, ScadaEquipment};

pub const WIDGET: &str = "widget";
pub const CONNECTION: &str = "connection";
pub const EQUIPMENT_TYPE: &str = "equipment_type";
pub const EQUIPMENT_INSTANCE: &str = "equipment_instance";
pub const FACEPLATE: &str = "faceplate";

/// Every object type, in display order.
pub const OBJECT_TYPES: &[&str] = &[
    WIDGET,
    CONNECTION,
    EQUIPMENT_TYPE,
    EQUIPMENT_INSTANCE,
    FACEPLATE,
];

/// Setting key of [`DesignSettings`].
pub const SETTINGS_KEY: &str = "design_history";

/// Operations whose consecutive changes of one object by one author (same
/// source) are squashed into one row within the squash window.
const SQUASHABLE: &[&str] = &["save"];

/// Who made a change, through what, and as part of which operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignActor {
    /// The user name, `api_key:<id>` for a key, `system` for the server.
    pub username: String,
    /// The channel: `user` (a session), `designer` / `native_designer` (a
    /// session through that editor), `api_key`, `mcp`, `copilot`, `flow`,
    /// `system`.
    pub source: String,
    /// What was done: `save`, `import`, `equipment`, `revert`, `rollback`,
    /// `tag_rename`, `wizard`, `faceplate`.
    pub operation: String,
    /// Free text shown with the batch (`Reverted to v3`).
    pub note: String,
}

impl DesignActor {
    pub fn new(
        username: impl Into<String>,
        source: impl Into<String>,
        operation: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            source: source.into(),
            operation: operation.into(),
            note: String::new(),
        }
    }

    /// A change the server makes on its own.
    pub fn system(operation: &str) -> Self {
        Self::new("system", "system", operation)
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    pub fn with_operation(mut self, operation: &str) -> Self {
        self.operation = operation.to_string();
        self
    }
}

/// The identity of one object.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ObjKey {
    /// The screen of a widget or connection, `""` for project objects.
    pub screen_id: String,
    pub object_type: String,
    pub object_id: String,
}

impl ObjKey {
    pub fn new(screen_id: &str, object_type: &str, object_id: &str) -> Self {
        Self {
            screen_id: screen_id.to_string(),
            object_type: object_type.to_string(),
            object_id: object_id.to_string(),
        }
    }
}

/// Objects by identity, each as its stored row in JSON.
pub type Objects = BTreeMap<ObjKey, Value>;

/// Which objects a writer may touch.
#[derive(Clone, Copy, Debug)]
pub enum Scope<'a> {
    /// The widgets and connections of one screen.
    Screen(&'a str),
    /// Every screen's widgets and connections, and the project objects.
    Project,
}

/// One object's change: `None` before is an add, `None` after a removal.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjChange {
    pub key: ObjKey,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// The batch a write appended.
#[derive(Clone, Debug, Serialize)]
pub struct BatchInfo {
    pub batch: String,
    /// Rows written (after squashing).
    pub changes: usize,
    /// Screens touched (`""` for project objects).
    pub screens: Vec<String>,
}

// ── Settings ──

/// Whether a publish needs a second person's approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    /// Publishing is one step.
    #[default]
    None,
    /// A publish becomes a request an admin other than the requester approves.
    Required,
}

/// Site settings of version history (setting `design_history`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DesignSettings {
    /// The two-stage flow: operators see published screens. Off, every reader
    /// sees the draft (as before phase 3) and publishing is refused.
    pub publishing: bool,
    pub approval: Approval,
    /// Consecutive saves of one object by one author within this many seconds
    /// become one row (0: never).
    pub squash_window_secs: u64,
    /// Rows older than this many days are squashed to one per object per hour
    /// (0: never).
    pub squash_after_days: u64,
    /// Rows older than this many days are deleted, except those a version
    /// points at (0: keep forever).
    pub keep_days: u64,
}

impl Default for DesignSettings {
    fn default() -> Self {
        Self {
            publishing: true,
            approval: Approval::None,
            squash_window_secs: 60,
            squash_after_days: 30,
            keep_days: 0,
        }
    }
}

pub fn read_settings(conn: &Connection) -> Result<DesignSettings> {
    let text: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [SETTINGS_KEY],
            |r| r.get(0),
        )
        .optional()?;
    Ok(text
        .and_then(|t| match serde_json::from_str(&t) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!("setting {SETTINGS_KEY} is not readable ({e}); using defaults");
                None
            }
        })
        .unwrap_or_default())
}

// ── Reading objects ──

fn row_value<T: Serialize>(row: &T) -> Result<Value> {
    serde_json::to_value(row).context("encode design object")
}

/// The objects of `scope` as they are stored now.
pub fn read_objects(conn: &Connection, scope: Scope<'_>) -> Result<Objects> {
    let mut out = Objects::new();
    let (eq, cn) = match scope {
        Scope::Screen(id) => (read_equipment(conn, id)?, read_connections(conn, id)?),
        Scope::Project => (
            read_equipment_from(conn, "scada_equipment", None)?,
            read_connections_from(conn, "scada_connections", None)?,
        ),
    };
    for e in eq {
        out.insert(
            ObjKey::new(&e.screen_id, WIDGET, &e.equipment_id),
            row_value(&e)?,
        );
    }
    for c in cn {
        out.insert(
            ObjKey::new(&c.screen_id, CONNECTION, &c.connection_id),
            row_value(&c)?,
        );
    }
    if let Scope::Project = scope {
        for t in super::equipment_types::read_types(conn, None)? {
            out.insert(ObjKey::new("", EQUIPMENT_TYPE, &t.id), row_value(&t)?);
        }
        for i in super::equipment_types::read_instances(conn, None, None)? {
            out.insert(ObjKey::new("", EQUIPMENT_INSTANCE, &i.name), row_value(&i)?);
        }
        for f in super::faceplates::load(conn, None)? {
            out.insert(ObjKey::new("", FACEPLATE, &f.id), row_value(&f)?);
        }
    }
    Ok(out)
}

/// The project objects of one type (`faceplate`, ...), for a writer that only
/// touches those.
pub fn read_project_objects(conn: &Connection, object_type: &str) -> Result<Objects> {
    let mut out = Objects::new();
    match object_type {
        EQUIPMENT_TYPE => {
            for t in super::equipment_types::read_types(conn, None)? {
                out.insert(ObjKey::new("", EQUIPMENT_TYPE, &t.id), row_value(&t)?);
            }
        }
        EQUIPMENT_INSTANCE => {
            for i in super::equipment_types::read_instances(conn, None, None)? {
                out.insert(ObjKey::new("", EQUIPMENT_INSTANCE, &i.name), row_value(&i)?);
            }
        }
        FACEPLATE => {
            for f in super::faceplates::load(conn, None)? {
                out.insert(ObjKey::new("", FACEPLATE, &f.id), row_value(&f)?);
            }
        }
        _ => {}
    }
    Ok(out)
}

/// What changed from `before` to `after`, object by object.
pub fn diff_objects(before: &Objects, after: &Objects) -> Vec<ObjChange> {
    let mut out = Vec::new();
    for (k, b) in before {
        match after.get(k) {
            Some(a) if a == b => {}
            Some(a) => out.push(ObjChange {
                key: k.clone(),
                before: Some(b.clone()),
                after: Some(a.clone()),
            }),
            None => out.push(ObjChange {
                key: k.clone(),
                before: Some(b.clone()),
                after: None,
            }),
        }
    }
    for (k, a) in after {
        if !before.contains_key(k) {
            out.push(ObjChange {
                key: k.clone(),
                before: None,
                after: Some(a.clone()),
            });
        }
    }
    out
}

/// Run `write` and log what it changed in `scope` as one batch.
pub fn tracked<T>(
    conn: &Connection,
    scope: Scope<'_>,
    actor: &DesignActor,
    write: impl FnOnce() -> Result<T>,
) -> Result<(T, Option<BatchInfo>)> {
    let before = read_objects(conn, scope)?;
    let out = write()?;
    let after = read_objects(conn, scope)?;
    let batch = append(conn, diff_objects(&before, &after), actor)?;
    Ok((out, batch))
}

// ── Appending ──

fn kind_of(before: &Option<Value>, after: &Option<Value>) -> &'static str {
    match (before, after) {
        (None, _) => "add",
        (_, None) => "remove",
        _ => "update",
    }
}

fn new_batch_id(at_ms: i64) -> String {
    use ring::rand::SecureRandom;
    let mut r = [0u8; 4];
    let _ = ring::rand::SystemRandom::new().fill(&mut r);
    format!(
        "b{at_ms:x}{}",
        r.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

fn json_text(v: &Option<Value>) -> Option<String> {
    v.as_ref().map(Value::to_string)
}

fn parse_opt(text: Option<String>) -> Option<Value> {
    text.and_then(|t| serde_json::from_str(&t).ok())
}

/// Whether a version of `screen_id` was taken at or after `seq` (its rows
/// are never squashed away).
fn pinned(conn: &Connection, screen_id: &str, seq: i64) -> Result<bool> {
    if screen_id.is_empty() {
        return Ok(false);
    }
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM design_versions WHERE screen_id = ?1 AND upto_seq >= ?2)",
        params![screen_id, seq],
        |r| r.get(0),
    )?)
}

/// Append `changes` as one batch by `actor`. Nothing is written (and `None`
/// returned) when there is no change. A change of an object whose previous
/// row is by the same author, source and squashable operation, younger than
/// the squash window and not part of a version, replaces that row (first
/// `before`, new `after`).
pub fn append(
    conn: &Connection,
    changes: Vec<ObjChange>,
    actor: &DesignActor,
) -> Result<Option<BatchInfo>> {
    if changes.is_empty() {
        return Ok(None);
    }
    let settings = read_settings(conn)?;
    let at_ms = crate::timeutil::now_millis();
    let batch = new_batch_id(at_ms);
    let squash = settings.squash_window_secs > 0 && SQUASHABLE.contains(&actor.operation.as_str());
    let window_ms = (settings.squash_window_secs as i64).saturating_mul(1000);
    let mut written = 0;
    let mut screens = BTreeSet::new();
    for c in changes {
        let mut before = c.before;
        let mut squashed = 0i64;
        if squash {
            let last = conn
                .query_row(
                    "SELECT seq, before, username, source, operation, at_ms, squashed FROM design_changes
                     WHERE screen_id = ?1 AND object_type = ?2 AND object_id = ?3
                     ORDER BY seq DESC LIMIT 1",
                    params![c.key.screen_id, c.key.object_type, c.key.object_id],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, i64>(5)?,
                            r.get::<_, i64>(6)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((seq, b, user, source, op, at, sq)) = last
                && user == actor.username
                && source == actor.source
                && op == actor.operation
                && at_ms - at <= window_ms
                && !pinned(conn, &c.key.screen_id, seq)?
            {
                conn.execute("DELETE FROM design_changes WHERE seq = ?1", [seq])?;
                before = parse_opt(b);
                squashed = sq + 1;
            }
        }
        screens.insert(c.key.screen_id.clone());
        if before == c.after {
            continue;
        }
        conn.execute(
            "INSERT INTO design_changes (batch, screen_id, object_type, object_id, kind, before, after, username, source, operation, at_ms, squashed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                batch,
                c.key.screen_id,
                c.key.object_type,
                c.key.object_id,
                kind_of(&before, &c.after),
                json_text(&before),
                json_text(&c.after),
                actor.username,
                actor.source,
                actor.operation,
                at_ms,
                squashed,
            ],
        )
        .context("append design change")?;
        written += 1;
    }
    if written > 0 {
        conn.execute(
            "INSERT INTO design_batches (batch, username, source, operation, note, at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![batch, actor.username, actor.source, actor.operation, actor.note, at_ms],
        )
        .context("append design batch")?;
    }
    conn.execute(
        "DELETE FROM design_batches WHERE NOT EXISTS (SELECT 1 FROM design_changes c WHERE c.batch = design_batches.batch)
           AND batch IN (SELECT batch FROM design_batches ORDER BY at_ms DESC LIMIT 50)",
        [],
    )?;
    Ok((written > 0).then(|| BatchInfo {
        batch,
        changes: written,
        screens: screens.into_iter().collect(),
    }))
}

/// `Some(Conflict)` when the screen moved on from `base`.
pub fn check_base(
    conn: &Connection,
    screen_id: &str,
    base: &super::LayoutBase,
) -> Result<Option<super::LayoutSave>> {
    super::settings_scada::check_base(conn, screen_id, base)
}

/// The highest change-log `seq` touching a screen (0: none yet).
pub fn head_seq(conn: &Connection, screen_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM design_changes WHERE screen_id = ?1",
        [screen_id],
        |r| r.get(0),
    )?)
}

/// The batches touching a screen after `seq`, oldest first, each with the
/// objects it changed (for a 409: who changed what since you loaded).
/// The seq before a screen's last `n` changes (0 when it has fewer).
pub fn recent_floor(conn: &Connection, screen_id: &str, n: i64) -> Result<i64> {
    let floor: Option<i64> = conn.query_row(
        "SELECT MIN(seq) FROM (SELECT seq FROM design_changes WHERE screen_id = ?1 ORDER BY seq DESC LIMIT ?2)",
        params![screen_id, n],
        |r| r.get(0),
    )?;
    Ok(floor.map(|f| f - 1).unwrap_or(0))
}

pub fn batches_since(conn: &Connection, screen_id: &str, seq: i64) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT c.batch, c.username, c.source, c.operation, COALESCE(b.note, ''), c.at_ms, c.object_type, c.object_id, c.kind, c.seq
         FROM design_changes c LEFT JOIN design_batches b ON b.batch = c.batch
         WHERE c.screen_id = ?1 AND c.seq > ?2 ORDER BY c.seq LIMIT 2000",
    )?;
    let mut out: Vec<Value> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut rows = stmt.query(params![screen_id, seq])?;
    while let Some(r) = rows.next()? {
        let batch: String = r.get(0)?;
        let i = *index.entry(batch.clone()).or_insert_with(|| {
            out.push(json!({
                "batch": batch,
                "username": r.get::<_, String>(1).unwrap_or_default(),
                "source": r.get::<_, String>(2).unwrap_or_default(),
                "operation": r.get::<_, String>(3).unwrap_or_default(),
                "note": r.get::<_, String>(4).unwrap_or_default(),
                "at_ms": r.get::<_, i64>(5).unwrap_or_default(),
                "objects": [],
            }));
            out.len() - 1
        });
        if let Some(Value::Array(objects)) = out[i].get_mut("objects") {
            objects.push(json!({
                "object_type": r.get::<_, String>(6)?,
                "object_id": r.get::<_, String>(7)?,
                "kind": r.get::<_, String>(8)?,
                "seq": r.get::<_, i64>(9)?,
            }));
        }
    }
    Ok(out)
}

// ── Reading the log ──

/// One row of the change log.
#[derive(Debug, Clone, Serialize)]
pub struct ChangeRow {
    pub seq: i64,
    pub batch: String,
    pub screen_id: String,
    pub object_type: String,
    pub object_id: String,
    pub kind: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub username: String,
    pub source: String,
    pub operation: String,
    pub note: String,
    pub at_ms: i64,
    /// How many earlier rows were squashed into this one.
    pub squashed: i64,
}

/// Filter of [`list_changes`].
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChangeFilter {
    /// A screen, or `""` for project objects; `None`: everything.
    #[serde(default)]
    pub screen_id: Option<String>,
    #[serde(default)]
    pub object_type: Option<String>,
    #[serde(default)]
    pub object_id: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub batch: Option<String>,
    /// Only rows older than this seq (paging, newest first).
    #[serde(default)]
    pub before_seq: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

fn map_change(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChangeRow> {
    Ok(ChangeRow {
        seq: r.get(0)?,
        batch: r.get(1)?,
        screen_id: r.get(2)?,
        object_type: r.get(3)?,
        object_id: r.get(4)?,
        kind: r.get(5)?,
        before: parse_opt(r.get(6)?),
        after: parse_opt(r.get(7)?),
        username: r.get(8)?,
        source: r.get(9)?,
        operation: r.get(10)?,
        note: r.get(11)?,
        at_ms: r.get(12)?,
        squashed: r.get(13)?,
    })
}

const CHANGE_COLUMNS: &str = "c.seq, c.batch, c.screen_id, c.object_type, c.object_id, c.kind, c.before, c.after, c.username, c.source, c.operation, COALESCE(b.note, ''), c.at_ms, c.squashed";

/// Change-log rows, newest first (at most 500).
pub fn list_changes(conn: &Connection, f: &ChangeFilter) -> Result<Vec<ChangeRow>> {
    let limit = f.limit.unwrap_or(100).clamp(1, 500);
    let sql = format!(
        "SELECT {CHANGE_COLUMNS} FROM design_changes c LEFT JOIN design_batches b ON b.batch = c.batch
         WHERE (?1 IS NULL OR c.screen_id = ?1) AND (?2 IS NULL OR c.object_type = ?2)
           AND (?3 IS NULL OR c.object_id = ?3) AND (?4 IS NULL OR c.username = ?4)
           AND (?5 IS NULL OR c.batch = ?5) AND (?6 IS NULL OR c.seq < ?6)
         ORDER BY c.seq DESC LIMIT ?7"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![
                f.screen_id,
                f.object_type,
                f.object_id,
                f.username,
                f.batch,
                f.before_seq,
                limit
            ],
            map_change,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Per object of a screen (present or removed): its last change.
pub fn blame(conn: &Connection, screen_id: &str) -> Result<Vec<Value>> {
    let sql = format!(
        "SELECT {CHANGE_COLUMNS} FROM design_changes c LEFT JOIN design_batches b ON b.batch = c.batch
         WHERE c.seq IN (SELECT MAX(seq) FROM design_changes WHERE screen_id = ?1 GROUP BY object_type, object_id)
         ORDER BY c.object_type, c.object_id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([screen_id], map_change)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for r in rows {
        let first: Option<(String, i64)> = conn
            .query_row(
                "SELECT username, at_ms FROM design_changes WHERE screen_id = ?1 AND object_type = ?2 AND object_id = ?3 AND kind = 'add' ORDER BY seq DESC LIMIT 1",
                params![screen_id, r.object_type, r.object_id],
                |x| Ok((x.get(0)?, x.get(1)?)),
            )
            .optional()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM design_changes WHERE screen_id = ?1 AND object_type = ?2 AND object_id = ?3",
            params![screen_id, r.object_type, r.object_id],
            |x| x.get(0),
        )?;
        out.push(json!({
            "object_type": r.object_type,
            "object_id": r.object_id,
            "exists": r.after.is_some(),
            "seq": r.seq,
            "batch": r.batch,
            "kind": r.kind,
            "username": r.username,
            "source": r.source,
            "operation": r.operation,
            "note": r.note,
            "at_ms": r.at_ms,
            "changes": count,
            "created_by": first.as_ref().map(|f| f.0.clone()),
            "created_at_ms": first.map(|f| f.1),
        }));
    }
    Ok(out)
}

// ── Layout snapshots ──

/// A screen's 2D rows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutSnapshot {
    pub equipment: Vec<ScadaEquipment>,
    pub connections: Vec<ScadaConnection>,
}

impl PartialEq for ScadaEquipment {
    fn eq(&self, o: &Self) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(o).ok()
    }
}

impl PartialEq for ScadaConnection {
    fn eq(&self, o: &Self) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(o).ok()
    }
}

pub fn read_draft(conn: &Connection, screen_id: &str) -> Result<LayoutSnapshot> {
    Ok(LayoutSnapshot {
        equipment: read_equipment(conn, screen_id)?,
        connections: read_connections(conn, screen_id)?,
    })
}

/// The published state of a screen: `None` when it was never published or
/// publishing is off (readers then see the draft).
pub fn read_published(conn: &Connection, screen_id: &str) -> Result<Option<LayoutSnapshot>> {
    if !read_settings(conn)?.publishing || published_state(conn, screen_id)?.is_none() {
        return Ok(None);
    }
    Ok(Some(LayoutSnapshot {
        equipment: read_equipment_from(conn, "scada_equipment_published", Some(screen_id))?,
        connections: read_connections_from(conn, "scada_connections_published", Some(screen_id))?,
    }))
}

/// The screen's published version: `(version_id, upto_seq, at_ms, by)`.
pub fn published_state(
    conn: &Connection,
    screen_id: &str,
) -> Result<Option<(i64, i64, i64, String)>> {
    Ok(conn
        .query_row(
            "SELECT version_id, upto_seq, published_at_ms, published_by FROM design_screen_state WHERE screen_id = ?1",
            [screen_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?)
}

/// The screen as it was right after change `seq` (after the whole batch it
/// belongs to): the draft with every later change of the screen undone,
/// newest first. Exact while the rows after
/// `seq` are not squashed across it (versions pin their rows).
pub fn state_at_seq(conn: &Connection, screen_id: &str, seq: i64) -> Result<LayoutSnapshot> {
    // A batch is one write: a seq inside it means the state after the whole
    // batch, so a connection and the widgets it was saved with come together.
    let seq: i64 = conn
        .query_row(
            "SELECT MAX(seq) FROM design_changes WHERE screen_id = ?1
               AND batch = (SELECT batch FROM design_changes WHERE seq = ?2)",
            params![screen_id, seq],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten()
        .unwrap_or(seq);
    let draft = read_draft(conn, screen_id)?;
    let mut eq: BTreeMap<String, ScadaEquipment> = draft
        .equipment
        .into_iter()
        .map(|e| (e.equipment_id.clone(), e))
        .collect();
    let mut cn: BTreeMap<String, ScadaConnection> = draft
        .connections
        .into_iter()
        .map(|c| (c.connection_id.clone(), c))
        .collect();
    let mut stmt = conn.prepare(
        "SELECT object_type, object_id, before FROM design_changes
         WHERE screen_id = ?1 AND seq > ?2 AND object_type IN ('widget', 'connection') ORDER BY seq DESC",
    )?;
    let mut rows = stmt.query(params![screen_id, seq])?;
    while let Some(r) = rows.next()? {
        let (ty, id, before): (String, String, Option<String>) = (r.get(0)?, r.get(1)?, r.get(2)?);
        let before = before.and_then(|b| serde_json::from_str::<Value>(&b).ok());
        if ty == WIDGET {
            match before.and_then(|b| serde_json::from_value(b).ok()) {
                Some(e) => {
                    eq.insert(id, e);
                }
                None => {
                    eq.remove(&id);
                }
            }
        } else {
            match before.and_then(|b| serde_json::from_value(b).ok()) {
                Some(c) => {
                    cn.insert(id, c);
                }
                None => {
                    cn.remove(&id);
                }
            }
        }
    }
    Ok(LayoutSnapshot {
        equipment: eq.into_values().collect(),
        connections: cn.into_values().collect(),
    })
}

// ── Versions ──

/// A named version of a screen.
#[derive(Debug, Clone, Serialize)]
pub struct Version {
    pub id: i64,
    pub screen_id: String,
    pub name: String,
    pub message: String,
    /// The screen as of this change.
    pub upto_seq: i64,
    pub username: String,
    pub at_ms: i64,
    /// `saved`, `pending` (a publish waiting for approval), `published`
    /// (published at least once), `rejected`.
    pub status: String,
    pub requested_by: Option<String>,
    pub reviewed_by: Option<String>,
    pub review_note: Option<String>,
    pub reviewed_at_ms: Option<i64>,
    pub published_at_ms: Option<i64>,
    pub published_by: Option<String>,
    /// Id of the electronic signature recorded by the edition, if any.
    pub signature: Option<String>,
    /// What operators see now.
    pub current: bool,
    pub widgets: usize,
    pub connections: usize,
}

const VERSION_COLUMNS: &str = "id, screen_id, name, message, upto_seq, username, at_ms, status, requested_by, reviewed_by, review_note, reviewed_at_ms, published_at_ms, published_by, signature, snapshot";

fn map_version(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Version, String)> {
    Ok((
        Version {
            id: r.get(0)?,
            screen_id: r.get(1)?,
            name: r.get(2)?,
            message: r.get(3)?,
            upto_seq: r.get(4)?,
            username: r.get(5)?,
            at_ms: r.get(6)?,
            status: r.get(7)?,
            requested_by: r.get(8)?,
            reviewed_by: r.get(9)?,
            review_note: r.get(10)?,
            reviewed_at_ms: r.get(11)?,
            published_at_ms: r.get(12)?,
            published_by: r.get(13)?,
            signature: r.get(14)?,
            current: false,
            widgets: 0,
            connections: 0,
        },
        r.get(15)?,
    ))
}

fn finish_version(
    conn: &Connection,
    (mut v, snap): (Version, String),
) -> Result<(Version, LayoutSnapshot)> {
    let snapshot: LayoutSnapshot = serde_json::from_str(&snap).context("version snapshot")?;
    v.widgets = snapshot.equipment.len();
    v.connections = snapshot.connections.len();
    v.current = published_state(conn, &v.screen_id)?.is_some_and(|p| p.0 == v.id);
    Ok((v, snapshot))
}

/// A screen's versions, newest first.
pub fn list_versions(conn: &Connection, screen_id: &str) -> Result<Vec<Version>> {
    let sql = format!(
        "SELECT {VERSION_COLUMNS} FROM design_versions WHERE screen_id = ?1 ORDER BY id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([screen_id], map_version)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|r| finish_version(conn, r).map(|(v, _)| v))
        .collect()
}

/// A version by id (digits) or name.
pub fn get_version(
    conn: &Connection,
    screen_id: &str,
    reference: &str,
) -> Result<Option<(Version, LayoutSnapshot)>> {
    let sql = format!(
        "SELECT {VERSION_COLUMNS} FROM design_versions WHERE screen_id = ?1 AND (name = ?2 OR CAST(id AS TEXT) = ?2) ORDER BY (name = ?2) DESC LIMIT 1"
    );
    let row = conn
        .query_row(&sql, params![screen_id, reference], map_version)
        .optional()?;
    row.map(|r| finish_version(conn, r)).transpose()
}

/// The next free automatic name (`v1`, `v2`, ...).
fn next_name(conn: &Connection, screen_id: &str) -> Result<String> {
    let mut n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM design_versions WHERE screen_id = ?1",
        [screen_id],
        |r| r.get(0),
    )?;
    loop {
        n += 1;
        let name = format!("v{n}");
        let taken: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM design_versions WHERE screen_id = ?1 AND name = ?2)",
            params![screen_id, name],
            |r| r.get(0),
        )?;
        if !taken {
            return Ok(name);
        }
    }
}

/// Why a version could not be created.
#[derive(Debug)]
pub struct NameTaken(pub String);

impl std::fmt::Display for NameTaken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a version named '{}' exists already", self.0)
    }
}

impl std::error::Error for NameTaken {}

/// Record the current draft of a screen as a version.
pub fn create_version(
    conn: &Connection,
    screen_id: &str,
    name: Option<&str>,
    message: &str,
    username: &str,
    status: &str,
) -> Result<Version> {
    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => {
            let taken: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM design_versions WHERE screen_id = ?1 AND name = ?2)",
                params![screen_id, n],
                |r| r.get(0),
            )?;
            if taken {
                return Err(NameTaken(n.to_string()).into());
            }
            n.to_string()
        }
        None => next_name(conn, screen_id)?,
    };
    let snapshot = read_draft(conn, screen_id)?;
    let upto = head_seq(conn, screen_id)?;
    let requested_by = (status == "pending").then_some(username);
    conn.execute(
        "INSERT INTO design_versions (screen_id, name, message, upto_seq, snapshot, username, at_ms, status, requested_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            screen_id,
            name,
            message,
            upto,
            serde_json::to_string(&snapshot)?,
            username,
            crate::timeutil::now_millis(),
            status,
            requested_by,
        ],
    )?;
    let id = conn.last_insert_rowid();
    Ok(get_version(conn, screen_id, &id.to_string())?
        .context("version just created")?
        .0)
}

/// Make version `version_id` what operators see.
pub fn publish_version(
    conn: &Connection,
    screen_id: &str,
    version_id: i64,
    by: &str,
    signature: Option<&str>,
) -> Result<Version> {
    let (v, snap) =
        get_version(conn, screen_id, &version_id.to_string())?.context("no such version")?;
    conn.execute(
        "DELETE FROM scada_equipment_published WHERE screen_id = ?1",
        [screen_id],
    )?;
    conn.execute(
        "DELETE FROM scada_connections_published WHERE screen_id = ?1",
        [screen_id],
    )?;
    for e in &snap.equipment {
        conn.execute(
            "INSERT INTO scada_equipment_published (screen_id, equipment_id, equipment_type, equipment_name, x, y, width, height, rotation, sensor_bindings, config)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![screen_id, e.equipment_id, e.equipment_type, e.equipment_name, e.x, e.y, e.width, e.height, e.rotation, e.sensor_bindings, e.config],
        )?;
    }
    for c in &snap.connections {
        conn.execute(
            "INSERT INTO scada_connections_published (screen_id, connection_id, from_equipment, to_equipment, waypoints, connection_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![screen_id, c.connection_id, c.from_equipment, c.to_equipment, c.waypoints, c.connection_type],
        )?;
    }
    let now = crate::timeutil::now_millis();
    conn.execute(
        "INSERT INTO design_screen_state (screen_id, version_id, upto_seq, published_at_ms, published_by) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(screen_id) DO UPDATE SET version_id = excluded.version_id, upto_seq = excluded.upto_seq,
           published_at_ms = excluded.published_at_ms, published_by = excluded.published_by",
        params![screen_id, v.id, v.upto_seq, now, by],
    )?;
    conn.execute(
        "UPDATE design_versions SET status = 'published', published_at_ms = ?2, published_by = ?3, signature = COALESCE(?4, signature) WHERE id = ?1",
        params![v.id, now, by, signature],
    )?;
    Ok(get_version(conn, screen_id, &v.id.to_string())?
        .context("version just published")?
        .0)
}

/// Record the review of a pending version (approve or reject).
pub fn review_version(
    conn: &Connection,
    version_id: i64,
    status: &str,
    by: &str,
    note: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE design_versions SET status = ?2, reviewed_by = ?3, review_note = ?4, reviewed_at_ms = ?5 WHERE id = ?1",
        params![version_id, status, by, note, crate::timeutil::now_millis()],
    )?;
    Ok(())
}

/// Forget a deleted screen's published copy (its log and versions stay).
pub fn forget_published(conn: &Connection, screen_id: &str) -> Result<()> {
    for t in [
        "scada_equipment_published",
        "scada_connections_published",
        "design_screen_state",
    ] {
        conn.execute(
            &format!("DELETE FROM {t} WHERE screen_id = ?1"),
            [screen_id],
        )?;
    }
    Ok(())
}

/// Status of a screen for the Designer's title bar.
pub fn screen_status(conn: &Connection, screen_id: &str) -> Result<Value> {
    let settings = read_settings(conn)?;
    let seq = head_seq(conn, screen_id)?;
    let published = published_state(conn, screen_id)?;
    let (published_json, since) = match &published {
        Some((vid, upto, at, by)) => {
            let name: Option<String> = conn
                .query_row(
                    "SELECT name FROM design_versions WHERE id = ?1",
                    [vid],
                    |r| r.get(0),
                )
                .optional()?;
            let draft = read_draft(conn, screen_id)?;
            let pub_rows = LayoutSnapshot {
                equipment: read_equipment_from(conn, "scada_equipment_published", Some(screen_id))?,
                connections: read_connections_from(
                    conn,
                    "scada_connections_published",
                    Some(screen_id),
                )?,
            };
            let differs = draft != pub_rows;
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM design_changes WHERE screen_id = ?1 AND seq > ?2",
                params![screen_id, upto],
                |r| r.get(0),
            )?;
            (
                json!({"version_id": vid, "name": name, "upto_seq": upto, "at_ms": at, "by": by, "draft_differs": differs}),
                n,
            )
        }
        None => (Value::Null, 0),
    };
    let last = list_versions(conn, screen_id)?.into_iter().next();
    let since_version = match &last {
        Some(v) => conn.query_row(
            "SELECT COUNT(*) FROM design_changes WHERE screen_id = ?1 AND seq > ?2",
            params![screen_id, v.upto_seq],
            |r| r.get::<_, i64>(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(*) FROM design_changes WHERE screen_id = ?1",
            [screen_id],
            |r| r.get::<_, i64>(0),
        )?,
    };
    let pending: Vec<Version> = list_versions(conn, screen_id)?
        .into_iter()
        .filter(|v| v.status == "pending")
        .collect();
    Ok(json!({
        "screen_id": screen_id,
        "seq": seq,
        "revision": super::settings_scada::layout_revision(conn, screen_id)?,
        "publishing": settings.publishing,
        "approval": settings.approval,
        "published": published_json,
        "changes_since_published": since,
        "last_version": last,
        "changes_since_version": since_version,
        "pending": pending,
    }))
}

// ── Diff ──

fn parse_text_json(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| v.clone()),
        _ => v.clone(),
    }
}

/// Changed field names between two rows (`config.<key>` inside config).
fn changed_fields(a: &Value, b: &Value) -> Vec<String> {
    let (Some(a), Some(b)) = (a.as_object(), b.as_object()) else {
        return vec![];
    };
    let mut keys: BTreeSet<&String> = a.keys().collect();
    keys.extend(b.keys());
    let mut out = Vec::new();
    for k in keys {
        let (x, y) = (
            a.get(k).unwrap_or(&Value::Null),
            b.get(k).unwrap_or(&Value::Null),
        );
        if x == y {
            continue;
        }
        if k == "config" || k == "sensor_bindings" || k == "waypoints" {
            let (px, py) = (parse_text_json(x), parse_text_json(y));
            if px == py {
                continue;
            }
            if let (Some(ox), Some(oy)) = (px.as_object(), py.as_object()) {
                let mut inner: BTreeSet<&String> = ox.keys().collect();
                inner.extend(oy.keys());
                for ik in inner {
                    if ox.get(ik) != oy.get(ik) {
                        out.push(format!("{k}.{ik}"));
                    }
                }
                continue;
            }
        }
        out.push(k.clone());
    }
    out
}

fn diff_rows<T: Serialize>(from: &[T], to: &[T], id: fn(&T) -> &str) -> Result<Value> {
    let a: BTreeMap<&str, Value> = from
        .iter()
        .map(|r| Ok((id(r), serde_json::to_value(r)?)))
        .collect::<Result<_>>()?;
    let b: BTreeMap<&str, Value> = to
        .iter()
        .map(|r| Ok((id(r), serde_json::to_value(r)?)))
        .collect::<Result<_>>()?;
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for (k, v) in &b {
        match a.get(k) {
            None => added.push(v.clone()),
            Some(old) if old != v => changed.push(json!({
                "id": k, "fields": changed_fields(old, v), "before": old, "after": v,
            })),
            _ => {}
        }
    }
    for (k, v) in &a {
        if !b.contains_key(k) {
            removed.push(v.clone());
        }
    }
    Ok(json!({"added": added, "removed": removed, "changed": changed}))
}

/// Per object: added, removed, and changed with the changed field names.
pub fn diff_layouts(from: &LayoutSnapshot, to: &LayoutSnapshot) -> Result<Value> {
    let widgets = diff_rows(&from.equipment, &to.equipment, |e| e.equipment_id.as_str())?;
    let connections = diff_rows(&from.connections, &to.connections, |c| {
        c.connection_id.as_str()
    })?;
    let n = |v: &Value, k: &str| v[k].as_array().map(Vec::len).unwrap_or(0);
    let summary = json!({
        "added": n(&widgets, "added") + n(&connections, "added"),
        "removed": n(&widgets, "removed") + n(&connections, "removed"),
        "changed": n(&widgets, "changed") + n(&connections, "changed"),
    });
    Ok(json!({"widgets": widgets, "connections": connections, "summary": summary}))
}

/// A screen's rows as project YAML (`screens[].scada`), tags named by ref
/// like the exporter does, for a readable structural diff.
pub fn layout_yaml(conn: &Connection, snap: &LayoutSnapshot) -> Result<String> {
    let refs: HashMap<i64, String> = crate::project_yaml::export::tag_refs(conn)?
        .into_iter()
        .collect();
    let by_ref = |v: &Value| -> Value {
        match v.as_i64().and_then(|id| refs.get(&id)) {
            Some(r) => Value::String(r.clone()),
            None => v.clone(),
        }
    };
    let mut equipment = Vec::new();
    for e in &snap.equipment {
        let mut m = serde_json::Map::new();
        m.insert("equipment_id".into(), json!(e.equipment_id));
        m.insert("equipment_type".into(), json!(e.equipment_type));
        if !e.equipment_name.is_empty() {
            m.insert("equipment_name".into(), json!(e.equipment_name));
        }
        for (k, v) in [
            ("x", e.x),
            ("y", e.y),
            ("width", e.width),
            ("height", e.height),
            ("rotation", e.rotation),
        ] {
            m.insert(k.into(), json!(v));
        }
        let sensors: Vec<Value> = serde_json::from_str::<Vec<Value>>(&e.sensor_bindings)
            .unwrap_or_default()
            .iter()
            .map(by_ref)
            .collect();
        if !sensors.is_empty() {
            m.insert("sensors".into(), Value::Array(sensors));
        }
        let mut config: Value = serde_json::from_str(&e.config).unwrap_or(json!({}));
        if let Some(c) = config.as_object_mut() {
            for key in crate::project_yaml::export::TAG_KEYS {
                if let Some(v) = c.get(*key).cloned() {
                    c.insert((*key).to_string(), by_ref(&v));
                }
            }
            if let Some(Value::Object(map)) = c.get_mut(crate::project_yaml::export::FACEPLATE_MAP)
            {
                for v in map.values_mut() {
                    *v = by_ref(v);
                }
            }
        }
        if config.as_object().is_some_and(|c| !c.is_empty()) {
            m.insert("config".into(), config);
        }
        equipment.push(Value::Object(m));
    }
    let connections: Vec<Value> = snap
        .connections
        .iter()
        .map(|c| {
            json!({
                "connection_id": c.connection_id,
                "from_equipment": c.from_equipment,
                "to_equipment": c.to_equipment,
                "connection_type": c.connection_type,
                "waypoints": serde_json::from_str::<Value>(&c.waypoints).unwrap_or(json!([])),
            })
        })
        .collect();
    let doc = json!({"scada": {"equipment": equipment, "connections": connections}});
    serde_yaml_neo::to_string(&doc).context("encode YAML")
}

// ── Resolving states ──

/// A state of a screen named in a request: `draft`, `published`, `seq:<n>`,
/// or a version id or name.
pub fn resolve_state(
    conn: &Connection,
    screen_id: &str,
    spec: &str,
) -> Result<Option<(Value, LayoutSnapshot)>> {
    match spec {
        "draft" | "" => Ok(Some((
            json!({"kind": "draft", "seq": head_seq(conn, screen_id)?}),
            read_draft(conn, screen_id)?,
        ))),
        "published" => {
            let snap = match published_state(conn, screen_id)? {
                Some(_) => LayoutSnapshot {
                    equipment: read_equipment_from(
                        conn,
                        "scada_equipment_published",
                        Some(screen_id),
                    )?,
                    connections: read_connections_from(
                        conn,
                        "scada_connections_published",
                        Some(screen_id),
                    )?,
                },
                None => read_draft(conn, screen_id)?,
            };
            Ok(Some((json!({"kind": "published"}), snap)))
        }
        s if s.starts_with("seq:") => {
            let Ok(seq) = s[4..].parse::<i64>() else {
                return Ok(None);
            };
            Ok(Some((
                json!({"kind": "seq", "seq": seq}),
                state_at_seq(conn, screen_id, seq)?,
            )))
        }
        s => Ok(get_version(conn, screen_id, s)?
            .map(|(v, snap)| (json!({"kind": "version", "version": v}), snap))),
    }
}

// ── Revert ──

/// One object to put back, and where to take its state from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevertObject {
    /// `widget` (default) or `connection`.
    #[serde(default)]
    pub object_type: Option<String>,
    #[serde(alias = "object")]
    pub object_id: String,
    /// The object as it was right after this change.
    #[serde(default)]
    pub to_seq: Option<i64>,
    /// ... or as it is in this version (id or name), `published` or `draft`.
    #[serde(default)]
    pub to: Option<String>,
}

/// What a revert did, or why it did not.
#[derive(Debug, Clone, PartialEq)]
pub enum RevertOutcome {
    Done {
        batch: Option<String>,
        seq: i64,
        revision: String,
        /// Connections removed because a widget they end on was removed.
        removed_connections: Vec<String>,
        /// Restored widgets whose group has no other member any more.
        ungrouped: Vec<String>,
    },
    /// A connection would end on widgets that do not exist; restore them too.
    Dangling {
        connection: String,
        missing: Vec<String>,
    },
    /// An object or a state named in the request does not exist.
    NotFound(String),
    /// `base_seq`/`base_revision` is stale.
    Conflict(super::LayoutSave),
}

fn connection_ends(c: &ScadaConnection) -> [&str; 2] {
    [c.from_equipment.as_str(), c.to_equipment.as_str()]
}

/// Revert one or several objects of a screen as one batch, following the
/// rules of docs/version-history.md (dangling connections, groups).
pub fn revert_objects(
    conn: &Connection,
    screen_id: &str,
    objects: &[RevertObject],
    base: &super::LayoutBase,
    actor: &DesignActor,
) -> Result<RevertOutcome> {
    if let Some(c) = super::settings_scada::check_base(conn, screen_id, base)? {
        return Ok(RevertOutcome::Conflict(c));
    }
    let draft = read_draft(conn, screen_id)?;
    let mut eq: BTreeMap<String, ScadaEquipment> = draft
        .equipment
        .into_iter()
        .map(|e| (e.equipment_id.clone(), e))
        .collect();
    let mut cn: BTreeMap<String, ScadaConnection> = draft
        .connections
        .into_iter()
        .map(|c| (c.connection_id.clone(), c))
        .collect();
    let mut states: HashMap<String, LayoutSnapshot> = HashMap::new();
    let mut removed_widgets: BTreeSet<String> = BTreeSet::new();
    let mut restored_widgets: BTreeSet<String> = BTreeSet::new();
    let mut restored_connections: BTreeSet<String> = BTreeSet::new();
    let mut explicit_connections: BTreeSet<String> = BTreeSet::new();
    for o in objects {
        let spec = match (&o.to_seq, &o.to) {
            (Some(s), _) => format!("seq:{s}"),
            (None, Some(t)) => t.clone(),
            (None, None) => {
                return Ok(RevertOutcome::NotFound(format!(
                    "object '{}': give to_seq or to",
                    o.object_id
                )));
            }
        };
        if !states.contains_key(&spec) {
            match resolve_state(conn, screen_id, &spec)? {
                Some((_, snap)) => {
                    states.insert(spec.clone(), snap);
                }
                None => return Ok(RevertOutcome::NotFound(format!("no state '{spec}'"))),
            }
        }
        let state = &states[&spec];
        let ty = o.object_type.as_deref().unwrap_or(WIDGET);
        match ty {
            WIDGET => match state
                .equipment
                .iter()
                .find(|e| e.equipment_id == o.object_id)
            {
                Some(e) => {
                    eq.insert(o.object_id.clone(), e.clone());
                    restored_widgets.insert(o.object_id.clone());
                }
                None => {
                    eq.remove(&o.object_id);
                    removed_widgets.insert(o.object_id.clone());
                }
            },
            CONNECTION => {
                explicit_connections.insert(o.object_id.clone());
                match state
                    .connections
                    .iter()
                    .find(|c| c.connection_id == o.object_id)
                {
                    Some(c) => {
                        cn.insert(o.object_id.clone(), c.clone());
                        restored_connections.insert(o.object_id.clone());
                    }
                    None => {
                        cn.remove(&o.object_id);
                    }
                }
            }
            other => {
                return Ok(RevertOutcome::NotFound(format!(
                    "object type '{other}' is not part of a screen"
                )));
            }
        }
    }
    // A removed widget takes the connections that end on it along.
    let mut removed_connections = Vec::new();
    cn.retain(|id, c| {
        let dangling = connection_ends(c)
            .iter()
            .any(|e| removed_widgets.contains(*e));
        if dangling && !explicit_connections.contains(id) {
            removed_connections.push(id.clone());
            false
        } else {
            true
        }
    });
    // A restored connection needs both ends.
    for id in &restored_connections {
        let Some(c) = cn.get(id) else { continue };
        let missing: Vec<String> = connection_ends(c)
            .iter()
            .filter(|e| !eq.contains_key(**e))
            .map(|e| e.to_string())
            .collect();
        if !missing.is_empty() {
            return Ok(RevertOutcome::Dangling {
                connection: id.clone(),
                missing,
            });
        }
    }
    // A restored widget whose group has no other member is restored ungrouped.
    let mut ungrouped = Vec::new();
    for id in &restored_widgets {
        let Some(group) = eq.get(id).and_then(|e| {
            serde_json::from_str::<Value>(&e.config)
                .ok()?
                .get("group")?
                .as_str()
                .map(str::to_string)
        }) else {
            continue;
        };
        let others = eq.iter().any(|(oid, e)| {
            oid != id
                && serde_json::from_str::<Value>(&e.config)
                    .ok()
                    .and_then(|c| c.get("group").and_then(Value::as_str).map(|g| g == group))
                    .unwrap_or(false)
        });
        if !others && let Some(e) = eq.get_mut(id) {
            let mut config: Value = serde_json::from_str(&e.config).unwrap_or(json!({}));
            if let Some(m) = config.as_object_mut() {
                m.remove("group");
            }
            e.config = config.to_string();
            ungrouped.push(id.clone());
        }
    }
    let batch = super::settings_scada::write_layout_rows(
        conn,
        screen_id,
        Some(eq.into_values().collect()),
        Some(cn.into_values().collect()),
        actor,
    )?;
    Ok(RevertOutcome::Done {
        batch: batch.map(|b| b.batch),
        seq: head_seq(conn, screen_id)?,
        revision: super::settings_scada::layout_revision(conn, screen_id)?,
        removed_connections,
        ungrouped,
    })
}

/// Restore a whole screen to a state (a snapshot restore, consistent by
/// construction).
pub fn revert_screen(
    conn: &Connection,
    screen_id: &str,
    spec: &str,
    base: &super::LayoutBase,
    actor: &DesignActor,
) -> Result<RevertOutcome> {
    if let Some(c) = super::settings_scada::check_base(conn, screen_id, base)? {
        return Ok(RevertOutcome::Conflict(c));
    }
    let Some((_, snap)) = resolve_state(conn, screen_id, spec)? else {
        return Ok(RevertOutcome::NotFound(format!("no state '{spec}'")));
    };
    let batch = super::settings_scada::write_layout_rows(
        conn,
        screen_id,
        Some(snap.equipment),
        Some(snap.connections),
        actor,
    )?;
    Ok(RevertOutcome::Done {
        batch: batch.map(|b| b.batch),
        seq: head_seq(conn, screen_id)?,
        revision: super::settings_scada::layout_revision(conn, screen_id)?,
        removed_connections: vec![],
        ungrouped: vec![],
    })
}

// ── Retention ──

/// What a maintenance pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MaintenanceReport {
    /// Rows merged into another row.
    pub squashed: usize,
    /// Rows deleted by `keep_days`.
    pub deleted: usize,
}

/// Squash rows older than `squash_after_days` to one per object per hour
/// (never across a version's `upto_seq`), then delete rows older than
/// `keep_days` that no version points at.
pub fn maintain(conn: &Connection, now_ms: i64) -> Result<MaintenanceReport> {
    let s = read_settings(conn)?;
    let mut report = MaintenanceReport::default();
    let day = 86_400_000i64;
    if s.squash_after_days > 0 {
        let cutoff = now_ms - s.squash_after_days as i64 * day;
        let mut pins: HashMap<String, Vec<i64>> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT screen_id, upto_seq FROM design_versions")?;
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                pins.entry(r.get(0)?).or_default().push(r.get(1)?);
            }
        }
        type Row = (i64, ObjKey, i64, Option<String>, Option<String>, i64);
        let rows: Vec<Row> = {
            let mut stmt = conn.prepare(
                "SELECT seq, screen_id, object_type, object_id, at_ms, before, after, squashed FROM design_changes
                 WHERE at_ms < ?1 ORDER BY screen_id, object_type, object_id, seq",
            )?;
            stmt.query_map([cutoff], |r| {
                Ok((
                    r.get(0)?,
                    ObjKey {
                        screen_id: r.get(1)?,
                        object_type: r.get(2)?,
                        object_id: r.get(3)?,
                    },
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            })?
            .collect::<Result<_, _>>()?
        };
        let mut groups: BTreeMap<(ObjKey, i64, usize), Vec<&Row>> = BTreeMap::new();
        for row in &rows {
            let below = pins
                .get(&row.1.screen_id)
                .map(|p| p.iter().filter(|u| **u < row.0).count())
                .unwrap_or(0);
            groups
                .entry((row.1.clone(), row.2 / 3_600_000, below))
                .or_default()
                .push(row);
        }
        for rows in groups.values().filter(|g| g.len() > 1) {
            let first = rows[0];
            let last = rows[rows.len() - 1];
            let before = parse_opt(first.3.clone());
            let after = parse_opt(last.4.clone());
            let merged: i64 = rows.iter().map(|r| r.5 + 1).sum::<i64>() - 1;
            for r in &rows[..rows.len() - 1] {
                conn.execute("DELETE FROM design_changes WHERE seq = ?1", [r.0])?;
                report.squashed += 1;
            }
            if before == after {
                conn.execute("DELETE FROM design_changes WHERE seq = ?1", [last.0])?;
                report.squashed += 1;
            } else {
                conn.execute(
                    "UPDATE design_changes SET before = ?2, kind = ?3, squashed = ?4 WHERE seq = ?1",
                    params![last.0, json_text(&before), kind_of(&before, &after), merged],
                )?;
            }
        }
    }
    if s.keep_days > 0 {
        let cutoff = now_ms - s.keep_days as i64 * day;
        report.deleted = conn.execute(
            "DELETE FROM design_changes WHERE at_ms < ?1 AND seq NOT IN (SELECT upto_seq FROM design_versions)",
            [cutoff],
        )?;
    }
    conn.execute(
        "DELETE FROM design_batches WHERE NOT EXISTS (SELECT 1 FROM design_changes c WHERE c.batch = design_batches.batch)",
        [],
    )?;
    Ok(report)
}

// ── Storage wrappers ──

impl Storage {
    pub async fn design_settings(&self) -> Result<DesignSettings> {
        self.with_conn(|c| read_settings(c)).await
    }

    pub async fn save_design_settings(&self, s: &DesignSettings) -> Result<()> {
        let text = serde_json::to_string(s)?;
        self.upsert_setting(SETTINGS_KEY, &text).await
    }

    /// Run a history read or write on the shared connection in one
    /// transaction (committed when `f` succeeds).
    pub async fn design_tx<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        self.with_conn(move |conn| {
            let tx = conn.transaction()?;
            let out = f(&tx)?;
            tx.commit()?;
            Ok(out)
        })
        .await
    }

    pub async fn design_maintain(&self) -> Result<MaintenanceReport> {
        let now = crate::timeutil::now_millis();
        self.design_tx(move |c| maintain(c, now)).await
    }
}

/// Squash and prune the change log once a day (the first pass ten minutes
/// after start).
pub async fn run_maintenance_task(storage: Storage) {
    tokio::time::sleep(std::time::Duration::from_secs(600)).await;
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(86_400));
    loop {
        tick.tick().await;
        match storage.design_maintain().await {
            Ok(r) if r.squashed > 0 || r.deleted > 0 => {
                tracing::info!(
                    "design history: squashed {} rows, deleted {}",
                    r.squashed,
                    r.deleted
                )
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("design history maintenance failed: {e:#}"),
        }
    }
}
