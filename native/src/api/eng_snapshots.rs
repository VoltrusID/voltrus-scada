//! Data Manager › Config snapshots: the server's `/api/v1/config/snapshots`
//! routes (`src/api/config_snapshots.rs`), mirrored, plus what the section
//! derives from them off the UI thread: per-table row counts of a snapshot,
//! a readable content summary, and a diff summary (added, removed, changed
//! per table, with the changed fields of each row).
//!
//! A snapshot's `snapshot_data` is the whole configuration as one JSON
//! string (`{"devices": [...], "data_sources": [...], ...}`), so the list
//! keeps only its size and counts, never the text.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use anyhow::Result;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Method, VoltrusApi};

/// The list route.
pub const SNAPSHOTS: &str = "/api/v1/config/snapshots";

/// `GET /api/v1/config/snapshots/{id}`.
pub fn snapshot_path(id: i64) -> String {
    format!("{SNAPSHOTS}/{id}")
}

/// `GET /api/v1/config/snapshots/{from}/diff/{to}`.
pub fn diff_path(from: i64, to: i64) -> String {
    format!("{SNAPSHOTS}/{from}/diff/{to}")
}

/// `POST /api/v1/config/snapshots/{id}/rollback`.
pub fn rollback_path(id: i64) -> String {
    format!("{SNAPSHOTS}/{id}/rollback")
}

/// `models::ConfigSnapshot` as the server sends it.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConfigSnapshot {
    pub id: i64,
    pub label: Option<String>,
    pub description: Option<String>,
    /// The configuration tables as one JSON object, as text.
    pub snapshot_data: String,
    /// `manual`, `restore` (made before a rollback) or what else saved it.
    pub trigger: String,
    pub created_by: Option<String>,
    /// Unix seconds.
    pub created_at: i64,
}

/// `POST /api/v1/config/snapshots`: an empty label is sent as none.
pub fn create_body(label: &str) -> Value {
    let label = label.trim();
    if label.is_empty() {
        json!({ "label": null })
    } else {
        json!({ "label": label })
    }
}

/// One table of a snapshot and how many rows it held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCount {
    pub name: String,
    pub rows: usize,
}

/// A snapshot as the list shows it: everything but the data, plus the
/// data's size and row counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotSummary {
    pub id: i64,
    pub label: Option<String>,
    pub description: Option<String>,
    pub trigger: String,
    pub created_by: Option<String>,
    pub created_at: i64,
    /// Bytes of `snapshot_data`.
    pub bytes: usize,
    pub tables: Vec<TableCount>,
    /// Whether `snapshot_data` was not a JSON object.
    pub unreadable: bool,
}

/// The tables of `data` with their rows, in the server's table order
/// where it has one; `None` when it is not a JSON object.
fn tables(data: &str) -> Option<Vec<(String, Vec<Value>)>> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(data) else {
        return None;
    };
    let mut out: Vec<(String, Vec<Value>)> = map
        .into_iter()
        .map(|(name, rows)| match rows {
            Value::Array(rows) => (name, rows),
            _ => (name, Vec::new()),
        })
        .collect();
    out.sort_by(|a, b| table_rank(&a.0).cmp(&table_rank(&b.0)).then(a.0.cmp(&b.0)));
    Some(out)
}

/// The server snapshots these tables in this order
/// (`create_config_snapshot`); others come after, by name.
const TABLE_ORDER: [&str; 6] = [
    "devices",
    "data_sources",
    "flows",
    "alarms",
    "screens",
    "recipes",
];

fn table_rank(name: &str) -> usize {
    TABLE_ORDER
        .iter()
        .position(|known| *known == name)
        .unwrap_or(TABLE_ORDER.len())
}

impl SnapshotSummary {
    pub fn from_snapshot(snapshot: &ConfigSnapshot) -> Self {
        let parsed = tables(&snapshot.snapshot_data);
        Self {
            id: snapshot.id,
            label: snapshot.label.clone().filter(|label| !label.is_empty()),
            description: snapshot.description.clone().filter(|text| !text.is_empty()),
            trigger: snapshot.trigger.clone(),
            created_by: snapshot.created_by.clone().filter(|by| !by.is_empty()),
            created_at: snapshot.created_at,
            bytes: snapshot.snapshot_data.len(),
            unreadable: parsed.is_none(),
            tables: parsed
                .unwrap_or_default()
                .into_iter()
                .map(|(name, rows)| TableCount {
                    name,
                    rows: rows.len(),
                })
                .collect(),
        }
    }

    /// The label, else the description, else `Snapshot #id` (the web row
    /// shows the label or the description).
    pub fn title(&self) -> String {
        self.label
            .clone()
            .or_else(|| self.description.clone())
            .unwrap_or_else(|| format!("Snapshot #{}", self.id))
    }

    /// Rows over every table.
    pub fn total_rows(&self) -> usize {
        self.tables.iter().map(|table| table.rows).sum()
    }

    /// `12 devices · 40 data_sources · …`, empty tables left out.
    pub fn counts_line(&self) -> String {
        let parts: Vec<String> = self
            .tables
            .iter()
            .filter(|table| table.rows > 0)
            .map(|table| format!("{} {}", table.rows, table.name))
            .collect();
        if parts.is_empty() {
            "empty".to_owned()
        } else {
            parts.join(" · ")
        }
    }

    /// Whether `query` (lower case) is in its label, description, trigger,
    /// author or `#id`.
    pub fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let has = |text: &Option<String>| {
            text.as_deref()
                .is_some_and(|text| text.to_lowercase().contains(query))
        };
        has(&self.label)
            || has(&self.description)
            || has(&self.created_by)
            || self.trigger.to_lowercase().contains(query)
            || format!("#{}", self.id).contains(query)
    }
}

/// `1.2 KB`, `3.4 MB`.
#[allow(clippy::cast_precision_loss)]
pub fn size_label(bytes: usize) -> String {
    let bytes = bytes as f64;
    if bytes < 1024.0 {
        format!("{bytes} B")
    } else if bytes < 1024.0 * 1024.0 {
        format!("{:.1} KB", bytes / 1024.0)
    } else {
        format!("{:.1} MB", bytes / (1024.0 * 1024.0))
    }
}

/// A row's name for people: its `name`, `key`, `label` or `title`, with
/// its id; else just `#id`.
pub fn row_label(row: &Value) -> String {
    let id = row.get("id").map(id_text);
    let name = ["name", "key", "label", "title", "source_key"]
        .iter()
        .find_map(|field| row.get(*field).and_then(Value::as_str))
        .filter(|name| !name.is_empty());
    match (name, id) {
        (Some(name), Some(id)) => format!("{name} (#{id})"),
        (Some(name), None) => name.to_owned(),
        (None, Some(id)) => format!("#{id}"),
        (None, None) => "row without id".to_owned(),
    }
}

/// An id as text: numbers plain, strings without quotes.
fn id_text(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The server's changed-row id is `serde_json::Value::to_string()` of the
/// row's id, so a string id arrives quoted: `"\"abc\""`. Strip the quotes.
pub fn display_id(id: &str) -> String {
    serde_json::from_str::<Value>(id).map_or_else(|_| id.to_owned(), |value| id_text(&value))
}

/// One table of a snapshot with every row's label, for the detail view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableContent {
    pub name: String,
    pub rows: Vec<String>,
}

/// One snapshot opened: its summary and the content of each table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotDetail {
    pub summary: SnapshotSummary,
    pub tables: Vec<TableContent>,
}

impl SnapshotDetail {
    pub fn from_snapshot(snapshot: &ConfigSnapshot) -> Self {
        Self {
            summary: SnapshotSummary::from_snapshot(snapshot),
            tables: tables(&snapshot.snapshot_data)
                .unwrap_or_default()
                .into_iter()
                .map(|(name, rows)| TableContent {
                    name,
                    rows: rows.iter().map(row_label).collect(),
                })
                .collect(),
        }
    }
}

// ── Diff ─────────────────────────────────────────────────────────────────

/// A changed row as `compute_diff` sends it.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ChangedRow {
    /// `Value::to_string()` of the row's id (see [`display_id`]).
    pub id: String,
    pub from: Value,
    pub to: Value,
}

/// One table of `GET …/{from}/diff/{to}`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct TableDiff {
    pub added: Vec<Value>,
    pub removed: Vec<Value>,
    pub changed: Vec<ChangedRow>,
}

/// The diff answer: tables that differ, by name (unchanged tables are
/// left out; `{}` when nothing differs).
pub type ConfigDiff = BTreeMap<String, TableDiff>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Removed,
    Changed,
}

impl Change {
    pub fn sign(self) -> &'static str {
        match self {
            Self::Added => "+",
            Self::Removed => "−",
            Self::Changed => "~",
        }
    }
}

/// One row of the diff summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffItem {
    pub change: Change,
    pub label: String,
    /// For a changed row, the fields whose values differ.
    pub fields: Vec<String>,
}

/// One table of the diff summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffTable {
    pub name: String,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub items: Vec<DiffItem>,
}

/// The diff of two snapshots as the section shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffSummary {
    pub from: i64,
    pub to: i64,
    pub tables: Vec<DiffTable>,
}

impl DiffSummary {
    pub fn new(from: i64, to: i64, diff: &ConfigDiff) -> Self {
        let mut tables: Vec<DiffTable> = diff
            .iter()
            .map(|(name, table)| {
                let mut items = Vec::new();
                items.extend(table.added.iter().map(|row| DiffItem {
                    change: Change::Added,
                    label: row_label(row),
                    fields: Vec::new(),
                }));
                items.extend(table.removed.iter().map(|row| DiffItem {
                    change: Change::Removed,
                    label: row_label(row),
                    fields: Vec::new(),
                }));
                items.extend(table.changed.iter().map(|row| {
                    let label = row_label(&row.to);
                    DiffItem {
                        change: Change::Changed,
                        label: if label == "row without id" {
                            format!("#{}", display_id(&row.id))
                        } else {
                            label
                        },
                        fields: changed_fields(&row.from, &row.to),
                    }
                }));
                DiffTable {
                    name: name.clone(),
                    added: table.added.len(),
                    removed: table.removed.len(),
                    changed: table.changed.len(),
                    items,
                }
            })
            .filter(|table| !table.items.is_empty())
            .collect();
        tables.sort_by(|a, b| {
            table_rank(&a.name)
                .cmp(&table_rank(&b.name))
                .then(a.name.cmp(&b.name))
        });
        Self { from, to, tables }
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// Added, removed and changed rows over every table.
    pub fn totals(&self) -> (usize, usize, usize) {
        self.tables.iter().fold((0, 0, 0), |(a, r, c), table| {
            (a + table.added, r + table.removed, c + table.changed)
        })
    }
}

/// The fields of two rows whose values differ (either side's), sorted.
pub fn changed_fields(from: &Value, to: &Value) -> Vec<String> {
    let (Some(from), Some(to)) = (from.as_object(), to.as_object()) else {
        return if from == to {
            Vec::new()
        } else {
            vec!["value".to_owned()]
        };
    };
    let mut fields: Vec<String> = from
        .keys()
        .chain(to.keys())
        .filter(|key| from.get(*key) != to.get(*key))
        .cloned()
        .collect();
    fields.sort_unstable();
    fields.dedup();
    fields
}

// ── The list ─────────────────────────────────────────────────────────────

/// What the list sorts by (the header cells, and S).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    /// Newest first by default, as the server lists them.
    #[default]
    Created,
    Label,
    Trigger,
    Size,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Label => "label",
            Self::Trigger => "trigger",
            Self::Size => "size",
        }
    }

    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Created => Self::Label,
            Self::Label => Self::Trigger,
            Self::Trigger => Self::Size,
            Self::Size => Self::Created,
        }
    }
}

/// Positions in `list` of the snapshots matching `query`, sorted by `key`
/// (created: newest first unless `reversed`; the others ascending unless
/// `reversed`).
pub fn visible_rows(
    list: &[SnapshotSummary],
    query: &str,
    key: SortKey,
    reversed: bool,
) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut rows: Vec<usize> = (0..list.len())
        .filter(|at| list[*at].matches(&query))
        .collect();
    rows.sort_by(|a, b| {
        let (a, b) = (&list[*a], &list[*b]);
        let order = match key {
            SortKey::Created => b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)),
            SortKey::Label => a.title().to_lowercase().cmp(&b.title().to_lowercase()),
            SortKey::Trigger => a.trigger.cmp(&b.trigger),
            SortKey::Size => a.bytes.cmp(&b.bytes),
        };
        let order = if order == Ordering::Equal {
            b.id.cmp(&a.id)
        } else {
            order
        };
        if reversed { order.reverse() } else { order }
    });
    rows
}

/// Whether the diff can run: both ends picked.
pub fn diff_ready(from: Option<i64>, to: Option<i64>) -> Option<(i64, i64)> {
    Some((from?, to?))
}

// ── Calls ────────────────────────────────────────────────────────────────

impl VoltrusApi {
    /// `GET /api/v1/config/snapshots` (the newest 50), summarised.
    pub async fn config_snapshots(&self) -> Result<Vec<SnapshotSummary>> {
        let list: Vec<ConfigSnapshot> = self.get_json(SNAPSHOTS).await?;
        Ok(list.iter().map(SnapshotSummary::from_snapshot).collect())
    }

    /// `GET /api/v1/config/snapshots/{id}`, with its content summary.
    pub async fn config_snapshot(&self, id: i64) -> Result<SnapshotDetail> {
        let snapshot: ConfigSnapshot = self.get_json(&snapshot_path(id)).await?;
        Ok(SnapshotDetail::from_snapshot(&snapshot))
    }

    /// `POST /api/v1/config/snapshots` with `label` (none when empty).
    pub async fn create_config_snapshot(&self, label: &str) -> Result<SnapshotSummary> {
        let snapshot: ConfigSnapshot = self
            .send_json(Method::POST, SNAPSHOTS, &create_body(label))
            .await?;
        Ok(SnapshotSummary::from_snapshot(&snapshot))
    }

    /// `GET /api/v1/config/snapshots/{from}/diff/{to}`, summarised.
    pub async fn diff_config_snapshots(&self, from: i64, to: i64) -> Result<DiffSummary> {
        let diff: ConfigDiff = self.get_json(&diff_path(from, to)).await?;
        Ok(DiffSummary::new(from, to, &diff))
    }

    /// `POST /api/v1/config/snapshots/{id}/rollback` (204). The server
    /// first saves the current configuration as a `restore` snapshot.
    pub async fn rollback_config_snapshot(&self, id: i64) -> Result<()> {
        self.send_empty::<IgnoredAny>(Method::POST, &rollback_path(id))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot as `GET /api/v1/config/snapshots` sends it.
    const LIST: &str = r#"[{
        "id": 7, "label": "Before flow refactor", "description": "Manual snapshot",
        "snapshot_data": "{\"recipes\":[],\"data_sources\":[{\"id\":1,\"key\":\"ac_power\",\"name\":\"AC power\"},{\"id\":2,\"key\":\"dc_power\",\"name\":\"DC power\"}],\"devices\":[{\"id\":3,\"name\":\"Inverter A\"}],\"flows\":[],\"alarms\":[],\"screens\":[{\"id\":\"plts_1\",\"title\":\"PLTS\"}]}",
        "trigger": "manual", "created_by": null, "created_at": 1761390000
    }, {
        "id": 6, "label": null, "description": "Auto-snapshot before rollback to snapshot #2",
        "snapshot_data": "not json", "trigger": "restore", "created_by": "admin",
        "created_at": 1761380000
    }]"#;

    fn list() -> Vec<SnapshotSummary> {
        let list: Vec<ConfigSnapshot> = serde_json::from_str(LIST).unwrap();
        list.iter().map(SnapshotSummary::from_snapshot).collect()
    }

    #[test]
    fn a_snapshot_decodes_and_summarises_its_tables_in_server_order() {
        let list = list();
        let first = &list[0];
        assert_eq!(first.id, 7);
        assert_eq!(first.title(), "Before flow refactor");
        let names: Vec<&str> = first.tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "devices",
                "data_sources",
                "flows",
                "alarms",
                "screens",
                "recipes"
            ]
        );
        assert_eq!(first.total_rows(), 4);
        assert_eq!(
            first.counts_line(),
            "1 devices · 2 data_sources · 1 screens"
        );
        assert!(!first.unreadable);
        assert!(first.bytes > 100);
        // No label: the description; data that is not JSON is flagged.
        assert_eq!(
            list[1].title(),
            "Auto-snapshot before rollback to snapshot #2"
        );
        assert!(list[1].unreadable);
        assert_eq!(list[1].counts_line(), "empty");
        assert_eq!(list[1].created_by.as_deref(), Some("admin"));
    }

    #[test]
    fn the_detail_names_every_row() {
        let snapshot: Vec<ConfigSnapshot> = serde_json::from_str(LIST).unwrap();
        let detail = SnapshotDetail::from_snapshot(&snapshot[0]);
        let sources = detail
            .tables
            .iter()
            .find(|t| t.name == "data_sources")
            .unwrap();
        assert_eq!(sources.rows, ["AC power (#1)", "DC power (#2)"]);
        let screens = detail.tables.iter().find(|t| t.name == "screens").unwrap();
        assert_eq!(screens.rows, ["PLTS (#plts_1)"]);
        assert_eq!(row_label(&json!({"id": 9})), "#9");
        assert_eq!(row_label(&json!({"other": 1})), "row without id");
    }

    #[test]
    fn the_create_body_sends_no_label_when_empty() {
        assert_eq!(create_body("  "), json!({ "label": null }));
        assert_eq!(create_body(" Before X "), json!({ "label": "Before X" }));
        // The server's `CreateSnapshotRequest` is `{label: Option<String>}`.
        assert_eq!(
            create_body("x")
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["label"]
        );
    }

    #[test]
    fn the_diff_decodes_as_compute_diff_sends_it_and_summarises() {
        // `compute_diff`: tables with changes only; changed ids are
        // `Value::to_string()` (strings quoted).
        let body = r#"{
            "data_sources": {
                "added": [{"id": 5, "key": "irradiance", "name": "Irradiance"}],
                "removed": [{"id": 2, "key": "dc_power", "name": "DC power"}],
                "changed": [{"id": "1",
                    "from": {"id": 1, "key": "ac_power", "name": "AC power", "warn_val": 80.0},
                    "to": {"id": 1, "key": "ac_power", "name": "AC power", "warn_val": 85.0, "unit": "kW"}}]
            },
            "screens": {
                "added": [], "removed": [],
                "changed": [{"id": "\"plts_1\"", "from": {"id": "plts_1", "title": "PLTS"},
                             "to": {"id": "plts_1", "title": "PLTS 1"}}]
            },
            "devices": {"added": [{"id": 4, "name": "Meter"}], "removed": [], "changed": []}
        }"#;
        let diff: ConfigDiff = serde_json::from_str(body).unwrap();
        let summary = DiffSummary::new(3, 7, &diff);
        let names: Vec<&str> = summary.tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["devices", "data_sources", "screens"]);
        assert_eq!(summary.totals(), (2, 1, 2));
        let sources = &summary.tables[1];
        assert_eq!((sources.added, sources.removed, sources.changed), (1, 1, 1));
        assert_eq!(sources.items[0].change, Change::Added);
        assert_eq!(sources.items[0].label, "Irradiance (#5)");
        assert_eq!(sources.items[1].change, Change::Removed);
        assert_eq!(sources.items[2].fields, ["unit", "warn_val"]);
        assert_eq!(summary.tables[2].items[0].fields, ["title"]);
        assert_eq!(display_id("\"plts_1\""), "plts_1");
        assert_eq!(display_id("12"), "12");
        // Nothing differs: `{}`.
        let none: ConfigDiff = serde_json::from_str("{}").unwrap();
        assert!(DiffSummary::new(1, 1, &none).is_empty());
    }

    #[test]
    fn the_list_searches_and_sorts() {
        let list = list();
        assert_eq!(visible_rows(&list, "", SortKey::Created, false), [0, 1]);
        assert_eq!(visible_rows(&list, "", SortKey::Created, true), [1, 0]);
        assert_eq!(visible_rows(&list, "RESTORE", SortKey::Created, false), [1]);
        assert_eq!(visible_rows(&list, "#7", SortKey::Created, false), [0]);
        assert_eq!(visible_rows(&list, "admin", SortKey::Created, false), [1]);
        assert_eq!(visible_rows(&list, "", SortKey::Size, false), [1, 0]);
        assert_eq!(visible_rows(&list, "", SortKey::Label, false), [1, 0]);
        assert_eq!(SortKey::Size.next(), SortKey::Created);
        assert!(visible_rows(&list, "nothing like it", SortKey::Label, false).is_empty());
    }

    #[test]
    fn small_helpers() {
        assert_eq!(size_label(512), "512 B");
        assert_eq!(size_label(2048), "2.0 KB");
        assert_eq!(size_label(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(diff_ready(Some(1), None), None);
        assert_eq!(diff_ready(Some(1), Some(2)), Some((1, 2)));
        assert_eq!(rollback_path(4), "/api/v1/config/snapshots/4/rollback");
        assert_eq!(diff_path(1, 2), "/api/v1/config/snapshots/1/diff/2");
        assert_eq!(changed_fields(&json!(1), &json!(2)), ["value"]);
    }
}
