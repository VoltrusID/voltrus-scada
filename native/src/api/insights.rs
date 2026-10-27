//! The Data Manager's Insights group on the REST API: OEE (metrics,
//! downtime events, shift schedules), analytics (bucketed aggregates and a
//! period comparison) and recipes (saved device values, loaded through the
//! command queue). Typed mirrors of the server's answers in its real shapes
//! (`models/operations.rs`, `analytics/mod.rs`), the paths, the request
//! bodies with the checks the server makes, and the arithmetic the
//! sections draw from.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::ops::{QueuedCommand, encode};

/// `path` with the non-empty `params` as its query string.
fn with_query(path: &str, params: &[(&str, String)]) -> String {
    let mut out = String::from(path);
    for (key, value) in params.iter().filter(|(_, value)| !value.is_empty()) {
        out.push(if out.contains('?') { '&' } else { '?' });
        out.push_str(key);
        out.push('=');
        out.push_str(&encode(value));
    }
    out
}

/// `2h 5m`, `5m`, `45s`, `1d 3h`: a duration as the web client writes it
/// (`OeePanel.tsx::formatDuration`), with days for the week's range.
pub fn duration_label(secs: i64) -> String {
    let secs = secs.max(0);
    let (days, hours, minutes) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{secs}s")
    }
}

// ── OEE ──────────────────────────────────────────────────────────────────

/// The OEE section's ranges, as the web client offers them.
pub const OEE_RANGES: [(&str, i64); 3] = [("8h", 8 * 3600), ("24h", 24 * 3600), ("7d", 7 * 86_400)];

/// The machine states the server accepts for a downtime event
/// (`api/oee.rs::create_downtime`).
pub const DOWNTIME_STATES: [&str; 5] = ["idle", "faulted", "setup", "maintenance", "running"];

pub const DOWNTIME_PATH: &str = "/api/v1/oee/downtime";
pub const SHIFTS_PATH: &str = "/api/v1/oee/shifts";

/// Mirrors `models::OeeMetrics`, `GET /api/v1/oee/metrics`. Performance and
/// quality are 1.0 until the server learns production counts
/// (`storage/oee.rs::calculate_oee`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct OeeMetrics {
    pub availability: f64,
    pub performance: f64,
    pub quality: f64,
    pub oee: f64,
    pub total_running_secs: i64,
    pub total_planned_secs: i64,
    pub downtime_count: i64,
}

/// How an OEE figure rates: world class from 85 %, typical from 65 %
/// (the web gauge's green, amber and red).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Good,
    Fair,
    Poor,
}

pub fn band(value: f64) -> Band {
    if value >= 0.85 {
        Band::Good
    } else if value >= 0.65 {
        Band::Fair
    } else {
        Band::Poor
    }
}

/// A fraction as a whole percentage, as the gauges show it.
#[allow(clippy::cast_possible_truncation)]
pub fn percent(value: f64) -> i64 {
    if value.is_finite() {
        (value * 100.0).round() as i64
    } else {
        0
    }
}

/// Mirrors `models::DowntimeEvent`, a row of `GET /api/v1/oee/downtime`
/// (newest first) and the answer to `POST /api/v1/oee/downtime`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct DowntimeEvent {
    pub id: i64,
    pub device_id: String,
    pub state: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_secs: Option<i64>,
    pub reason: Option<String>,
}

impl DowntimeEvent {
    pub fn is_open(&self) -> bool {
        self.ended_at.is_none()
    }

    /// How long it lasted, or has lasted by `now` while open (the server
    /// counts an open event up to now the same way).
    pub fn duration_at(&self, now: i64) -> i64 {
        self.duration_secs
            .unwrap_or_else(|| (now - self.started_at).max(0))
    }
}

/// Downtime by machine state over `events`: state, seconds, events;
/// longest first. What availability lost, and to what.
pub fn downtime_by_state(events: &[DowntimeEvent], now: i64) -> Vec<(String, i64, usize)> {
    let mut by: Vec<(String, i64, usize)> = Vec::new();
    for event in events {
        let secs = event.duration_at(now);
        match by.iter_mut().find(|(state, _, _)| *state == event.state) {
            Some(entry) => {
                entry.1 += secs;
                entry.2 += 1;
            }
            None => by.push((event.state.clone(), secs, 1)),
        }
    }
    by.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    by
}

/// `GET /api/v1/oee/metrics` for `device` over `[from, to]` (Unix seconds).
pub fn oee_metrics_path(device: &str, from: i64, to: i64) -> String {
    with_query(
        "/api/v1/oee/metrics",
        &[
            ("device_id", device.to_owned()),
            ("from", from.to_string()),
            ("to", to.to_string()),
        ],
    )
}

/// `GET /api/v1/oee/downtime`: events that started in `[from, to]`, of
/// one device or of every device.
pub fn downtime_path(device: Option<&str>, from: i64, to: i64) -> String {
    with_query(
        DOWNTIME_PATH,
        &[
            ("device_id", device.unwrap_or("").to_owned()),
            ("from", from.to_string()),
            ("to", to.to_string()),
        ],
    )
}

/// `PUT /api/v1/oee/downtime/{id}/close`.
pub fn close_downtime_path(id: i64) -> String {
    format!("{DOWNTIME_PATH}/{id}/close")
}

/// `DELETE /api/v1/oee/shifts/{id}`.
pub fn shift_path(id: i64) -> String {
    format!("{SHIFTS_PATH}/{id}")
}

/// The body of `POST /api/v1/oee/downtime`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewDowntime {
    pub device_id: String,
    pub state: String,
}

impl NewDowntime {
    /// The checks `create_downtime` makes: a device, a known state.
    pub fn new(device: &str, state: &str) -> Result<Self, String> {
        let device_id = device.trim();
        if device_id.is_empty() {
            return Err("Choose the device that stopped".into());
        }
        let state = state.trim().to_lowercase();
        if !DOWNTIME_STATES.contains(&state.as_str()) {
            return Err(format!(
                "The state is one of {}",
                DOWNTIME_STATES.join(", ")
            ));
        }
        Ok(Self {
            device_id: device_id.to_owned(),
            state,
        })
    }
}

/// The body of `PUT /api/v1/oee/downtime/{id}/close`; no reason is sent as
/// `null`, as the web client leaves it out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CloseDowntime {
    pub reason: Option<String>,
}

impl CloseDowntime {
    pub fn new(reason: &str) -> Self {
        let reason = reason.trim();
        Self {
            reason: (!reason.is_empty()).then(|| reason.to_owned()),
        }
    }
}

/// The days of a shift's `days_mask`: bit 0 is Monday, bit 6 Sunday.
pub const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
/// Monday to Friday, the web form's default.
pub const WEEKDAYS: u32 = 0b001_1111;
pub const EVERY_DAY: u32 = 0b111_1111;

/// `Mon–Fri`, `Every day`, `Sat, Sun`, `No days`: a `days_mask` read aloud.
pub fn days_label(mask: u32) -> String {
    match mask & EVERY_DAY {
        0 => "No days".into(),
        EVERY_DAY => "Every day".into(),
        WEEKDAYS => "Mon–Fri".into(),
        mask => DAYS
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1 << bit) != 0)
            .map(|(_, day)| *day)
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// Mirrors `models::ShiftConfig`, a row of `GET /api/v1/oee/shifts` and
/// the answer to `POST /api/v1/oee/shifts`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ShiftConfig {
    pub id: i64,
    pub name: String,
    pub start_hour: u32,
    pub start_minute: u32,
    pub duration_hours: u32,
    pub days_mask: u32,
    pub created_at: i64,
}

impl ShiftConfig {
    /// `22:00–06:00 +1`: start, end, and whether it ends the next day.
    pub fn window_label(&self) -> String {
        let start = self.start_hour * 60 + self.start_minute;
        let end = start + self.duration_hours * 60;
        let next_day = if end >= 24 * 60 { " +1" } else { "" };
        let end = end % (24 * 60);
        format!(
            "{:02}:{:02}–{:02}:{:02}{next_day}",
            self.start_hour,
            self.start_minute,
            end / 60,
            end % 60
        )
    }
}

/// The body of `POST /api/v1/oee/shifts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewShift {
    pub name: String,
    pub start_hour: u32,
    pub start_minute: u32,
    pub duration_hours: u32,
    pub days_mask: u32,
}

impl NewShift {
    /// The form's text as a shift, with the checks `create_shift` makes:
    /// a name, a start `HH:MM` (or `H`), 1–24 hours, a 7-day mask.
    pub fn parse(name: &str, start: &str, hours: &str, days_mask: u32) -> Result<Self, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("The shift needs a name".into());
        }
        let start = start.trim();
        let (hour, minute) = start.split_once(':').unwrap_or((start, "0"));
        let start_hour: u32 = hour
            .trim()
            .parse()
            .map_err(|_| format!("The start is a time like 06:00, not “{start}”"))?;
        let start_minute: u32 = minute
            .trim()
            .parse()
            .map_err(|_| format!("The start is a time like 06:00, not “{start}”"))?;
        if start_hour > 23 {
            return Err("The start hour is 0–23".into());
        }
        if start_minute > 59 {
            return Err("The start minute is 0–59".into());
        }
        let duration_hours: u32 = hours
            .trim()
            .parse()
            .map_err(|_| "The length is a whole number of hours".to_owned())?;
        if !(1..=24).contains(&duration_hours) {
            return Err("A shift lasts 1–24 hours".into());
        }
        if days_mask > EVERY_DAY {
            return Err("The days are Monday to Sunday".into());
        }
        if days_mask == 0 {
            return Err("Pick at least one day".into());
        }
        Ok(Self {
            name: name.to_owned(),
            start_hour,
            start_minute,
            duration_hours,
            days_mask,
        })
    }
}

// ── Analytics ────────────────────────────────────────────────────────────

/// The analytics ranges and their bucket size in seconds, as the web
/// client pairs them (`AnalyticsPanel.tsx`).
pub const ANALYTICS_RANGES: [(&str, i64, i64); 5] = [
    ("1h", 3600, 60),
    ("6h", 21_600, 300),
    ("24h", 86_400, 3600),
    ("7d", 604_800, 21_600),
    ("30d", 2_592_000, 86_400),
];

/// The smallest bucket the server makes (`analytics::aggregate_metrics`
/// clamps below a minute).
pub const MIN_INTERVAL: i64 = 60;

/// Mirrors `analytics::AggregatedMetric`. The server reports `stddev` as 0
/// for buckets (`storage/oee.rs::aggregate_raw`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AggregatedMetric {
    pub metric: String,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub stddev: f64,
    pub count: u64,
    pub first_ts: i64,
    pub last_ts: i64,
}

/// One bucket of `GET /api/v1/analytics/aggregate`: its start (Unix
/// seconds) and its figures; the server sends `[ts, {…}]` pairs.
pub type Bucket = (i64, AggregatedMetric);

/// Mirrors `analytics::ComparisonResult`, `GET /api/v1/analytics/compare`:
/// `current` is period 1, `previous` period 2, and the change is the
/// current average against the previous one.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ComparisonResult {
    pub metric: String,
    pub current: AggregatedMetric,
    pub previous: AggregatedMetric,
    pub change_percent: f64,
}

/// `GET /api/v1/analytics/aggregate`.
pub fn aggregate_path(device: &str, metric: &str, from: i64, to: i64, interval: i64) -> String {
    with_query(
        "/api/v1/analytics/aggregate",
        &[
            ("device_id", device.to_owned()),
            ("metric", metric.to_owned()),
            ("from", from.to_string()),
            ("to", to.to_string()),
            ("interval", interval.to_string()),
        ],
    )
}

/// `GET /api/v1/analytics/compare`: `current` as period 1, `previous` as
/// period 2.
pub fn compare_path(
    device: &str,
    metric: &str,
    current: (i64, i64),
    previous: (i64, i64),
) -> String {
    with_query(
        "/api/v1/analytics/compare",
        &[
            ("device_id", device.to_owned()),
            ("metric", metric.to_owned()),
            ("period1_from", current.0.to_string()),
            ("period1_to", current.1.to_string()),
            ("period2_from", previous.0.to_string()),
            ("period2_to", previous.1.to_string()),
        ],
    )
}

/// The range split in two for a comparison: the recent half is current,
/// the older half previous. The server's periods include both ends, so the
/// older half stops one second before the recent one starts and no sample
/// counts twice.
pub fn halves(from: i64, to: i64) -> ((i64, i64), (i64, i64)) {
    let mid = from + (to - from) / 2;
    ((mid, to), (from, mid - 1))
}

/// The whole range's figures from its buckets (the web's stat cards): the
/// lowest, the highest, the sample-weighted average.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overall {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub samples: u64,
    pub buckets: usize,
}

#[allow(clippy::cast_precision_loss)]
pub fn overall(buckets: &[Bucket]) -> Option<Overall> {
    if buckets.is_empty() {
        return None;
    }
    let (mut min, mut max, mut sum, mut samples) = (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0u64);
    for (_, bucket) in buckets {
        min = min.min(bucket.min);
        max = max.max(bucket.max);
        sum += bucket.avg * bucket.count as f64;
        samples += bucket.count;
    }
    Some(Overall {
        min,
        max,
        avg: if samples > 0 {
            sum / samples as f64
        } else {
            0.0
        },
        samples,
        buckets: buckets.len(),
    })
}

/// The bucket sizes a drill-down steps through.
const LADDER: [i64; 7] = [60, 300, 900, 3600, 21_600, 86_400, 604_800];

/// The bucket size for a span: the largest step of the ladder giving at
/// least a dozen buckets, never below the server's minute.
pub fn interval_for(span_secs: i64) -> i64 {
    LADDER
        .iter()
        .rev()
        .copied()
        .find(|step| span_secs / step >= 12)
        .unwrap_or(MIN_INTERVAL)
}

/// Drilling into a bucket starting at `start` of `interval` seconds: the
/// bucket's span with finer buckets, or `None` when it is already the
/// finest the server makes.
pub fn drill(start: i64, interval: i64) -> Option<(i64, i64, i64)> {
    (interval > MIN_INTERVAL).then(|| {
        let finer = interval_for(interval).min(interval / 2).max(MIN_INTERVAL);
        (start, start + interval, finer)
    })
}

/// The buckets as the web client's CSV export writes them.
pub fn buckets_csv(buckets: &[Bucket], metric: &str) -> String {
    let mut out = String::from("timestamp,min,max,avg,count,metric\n");
    for (at, bucket) in buckets {
        out.push_str(&format!(
            "{at},{},{},{},{},{metric}\n",
            bucket.min, bucket.max, bucket.avg, bucket.count
        ));
    }
    out
}

// ── Recipes ──────────────────────────────────────────────────────────────

pub const RECIPES_PATH: &str = "/api/v1/recipes";

/// Mirrors `models::Recipe`, a row of `GET /api/v1/recipes`. `values` is
/// JSON text: register name to value, the device's values when saved.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Recipe {
    pub id: i64,
    pub name: String,
    pub category: Option<String>,
    pub device_id: String,
    pub values: String,
    pub version: i32,
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Recipe {
    /// The saved values by register name, in name order; what is not a
    /// number is left out (the server's load does the same).
    pub fn values(&self) -> Vec<(String, f64)> {
        let map: HashMap<String, serde_json::Value> =
            serde_json::from_str(&self.values).unwrap_or_default();
        let mut values: Vec<(String, f64)> = map
            .into_iter()
            .filter_map(|(name, value)| Some((name, value.as_f64()?)))
            .collect();
        values.sort_by(|a, b| a.0.cmp(&b.0));
        values
    }

    /// The category, if it has one ("" reads as none).
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref().filter(|text| !text.is_empty())
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref().filter(|text| !text.is_empty())
    }
}

/// The recipes' categories, each once, in order.
pub fn categories(recipes: &[Recipe]) -> Vec<String> {
    recipes
        .iter()
        .filter_map(|recipe| recipe.category().map(str::to_owned))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// `GET /api/v1/recipes`, of one device or of all.
pub fn recipes_path(device: Option<&str>) -> String {
    with_query(
        RECIPES_PATH,
        &[("device_id", device.unwrap_or("").to_owned())],
    )
}

/// `GET`, `PUT` or `DELETE /api/v1/recipes/{id}`.
pub fn recipe_path(id: i64) -> String {
    format!("{RECIPES_PATH}/{id}")
}

/// `POST /api/v1/recipes/{id}/load`.
pub fn load_path(id: i64) -> String {
    format!("{RECIPES_PATH}/{id}/load")
}

/// `GET /api/v1/commands/{id}`: one command of a recipe load.
pub fn command_path(id: &str) -> String {
    format!("/api/v1/commands/{}", encode(id))
}

/// The body of `POST /api/v1/recipes`: the server saves the device's
/// current values under this name ("save current as recipe").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewRecipe {
    pub name: String,
    pub device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn optional(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

impl NewRecipe {
    pub fn new(
        name: &str,
        device: &str,
        category: &str,
        description: &str,
    ) -> Result<Self, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("The recipe needs a name".into());
        }
        let device = device.trim();
        if device.is_empty() {
            return Err("Choose the device whose values it keeps".into());
        }
        Ok(Self {
            name: name.to_owned(),
            device_id: device.to_owned(),
            category: optional(category),
            description: optional(description),
        })
    }
}

/// The body of `PUT /api/v1/recipes/{id}`. The server keeps a field it is
/// sent as `null` (`COALESCE`), so a cleared category or description goes
/// as `""`, which every client reads as none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecipeUpdate {
    pub name: String,
    pub category: String,
    pub description: String,
}

impl RecipeUpdate {
    pub fn new(name: &str, category: &str, description: &str) -> Result<Self, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("The recipe needs a name".into());
        }
        Ok(Self {
            name: name.to_owned(),
            category: category.trim().to_owned(),
            description: description.trim().to_owned(),
        })
    }
}

/// The answer to `POST /api/v1/recipes/{id}/load`: one queued command per
/// value the server found a data source for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RecipeLoad {
    pub ok: bool,
    pub command_ids: Vec<String>,
    pub message: String,
}

/// A command status no longer changes.
pub fn is_terminal(status: &str) -> bool {
    matches!(status, "acked" | "failed" | "expired" | "cancelled")
}

/// Where a recipe load stands: commands acknowledged, failed, and whether
/// every command is done (as `RecipeLoadStatus.tsx` counts).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadTally {
    pub total: usize,
    pub acked: usize,
    pub failed: usize,
    pub done: bool,
}

pub fn tally(ids: &[String], known: &HashMap<String, QueuedCommand>) -> LoadTally {
    let commands: Vec<&QueuedCommand> = ids.iter().filter_map(|id| known.get(id)).collect();
    LoadTally {
        total: ids.len(),
        acked: commands.iter().filter(|c| c.status == "acked").count(),
        failed: commands.iter().filter(|c| c.status == "failed").count(),
        done: commands.len() == ids.len() && commands.iter().all(|c| is_terminal(&c.status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oee_answers_decode_in_the_servers_shapes() {
        // models::OeeMetrics as get_oee_metrics answers.
        let metrics: OeeMetrics = serde_json::from_str(
            r#"{"availability": 0.875, "performance": 1.0, "quality": 1.0, "oee": 0.875,
                "total_running_secs": 25200, "total_planned_secs": 28800, "downtime_count": 2}"#,
        )
        .unwrap();
        assert_eq!(percent(metrics.oee), 88);
        assert_eq!(band(metrics.oee), Band::Good);
        assert_eq!(band(0.7), Band::Fair);
        assert_eq!(band(0.2), Band::Poor);
        assert_eq!(
            metrics.total_planned_secs - metrics.total_running_secs,
            3600
        );

        // models::DowntimeEvent rows, one closed and one open.
        let events: Vec<DowntimeEvent> = serde_json::from_str(
            r#"[{"id": 9, "device_id": "sma_inv_1", "state": "faulted", "started_at": 1000,
                 "ended_at": null, "duration_secs": null, "reason": null},
                {"id": 8, "device_id": "sma_inv_1", "state": "idle", "started_at": 100,
                 "ended_at": 700, "duration_secs": 600, "reason": "Grid outage"}]"#,
        )
        .unwrap();
        assert!(events[0].is_open() && !events[1].is_open());
        assert_eq!(events[0].duration_at(1300), 300);
        assert_eq!(events[1].duration_at(99_999), 600);
        assert_eq!(
            downtime_by_state(&events, 1300),
            [("idle".to_owned(), 600, 1), ("faulted".to_owned(), 300, 1)]
        );

        // models::ShiftConfig as create_shift answers (201).
        let shift: ShiftConfig = serde_json::from_str(
            r#"{"id": 3, "name": "Night", "start_hour": 22, "start_minute": 0,
                "duration_hours": 8, "days_mask": 31, "created_at": 1761390000}"#,
        )
        .unwrap();
        assert_eq!(shift.window_label(), "22:00–06:00 +1");
        assert_eq!(days_label(shift.days_mask), "Mon–Fri");
    }

    #[test]
    fn oee_paths_and_bodies_are_what_the_server_reads() {
        assert_eq!(
            oee_metrics_path("sma inv/1", 10, 20),
            "/api/v1/oee/metrics?device_id=sma%20inv%2F1&from=10&to=20"
        );
        assert_eq!(
            downtime_path(None, 10, 20),
            "/api/v1/oee/downtime?from=10&to=20"
        );
        assert_eq!(close_downtime_path(4), "/api/v1/oee/downtime/4/close");
        assert_eq!(shift_path(5), "/api/v1/oee/shifts/5");

        let open = NewDowntime::new(" sma_inv_1 ", "Faulted").unwrap();
        assert_eq!(
            serde_json::to_value(&open).unwrap(),
            serde_json::json!({"device_id": "sma_inv_1", "state": "faulted"})
        );
        assert!(NewDowntime::new("", "idle").is_err());
        assert!(NewDowntime::new("d", "broken").is_err());
        assert_eq!(
            serde_json::to_value(CloseDowntime::new("  ")).unwrap(),
            serde_json::json!({"reason": null})
        );
        assert_eq!(
            serde_json::to_value(CloseDowntime::new(" Bearing ")).unwrap(),
            serde_json::json!({"reason": "Bearing"})
        );
    }

    #[test]
    fn a_shift_form_is_checked_as_the_server_checks_it() {
        let shift = NewShift::parse(" Morning ", "06:30", "8", WEEKDAYS).unwrap();
        assert_eq!(
            serde_json::to_value(&shift).unwrap(),
            serde_json::json!({"name": "Morning", "start_hour": 6, "start_minute": 30,
                               "duration_hours": 8, "days_mask": 31})
        );
        assert_eq!(NewShift::parse("A", "6", "8", 1).unwrap().start_minute, 0);
        assert!(NewShift::parse(" ", "06:00", "8", 1).is_err(), "a name");
        assert!(NewShift::parse("A", "24:00", "8", 1).is_err(), "hour 0–23");
        assert!(
            NewShift::parse("A", "06:60", "8", 1).is_err(),
            "minute 0–59"
        );
        assert!(NewShift::parse("A", "six", "8", 1).is_err());
        assert!(NewShift::parse("A", "06:00", "0", 1).is_err(), "1–24 h");
        assert!(NewShift::parse("A", "06:00", "25", 1).is_err(), "1–24 h");
        assert!(NewShift::parse("A", "06:00", "8", 0).is_err(), "a day");
        assert!(NewShift::parse("A", "06:00", "8", 128).is_err(), "7 days");
        assert_eq!(days_label(EVERY_DAY), "Every day");
        assert_eq!(days_label(0b110_0000), "Sat, Sun");
        assert_eq!(days_label(0b000_0101), "Mon, Wed");
        let day = ShiftConfig {
            start_hour: 6,
            duration_hours: 8,
            ..ShiftConfig::default()
        };
        assert_eq!(day.window_label(), "06:00–14:00");
    }

    #[test]
    fn durations_read_like_the_web() {
        assert_eq!(duration_label(45), "45s");
        assert_eq!(duration_label(300), "5m");
        assert_eq!(duration_label(7500), "2h 5m");
        assert_eq!(duration_label(90_000), "1d 1h");
        assert_eq!(duration_label(-5), "0s");
    }

    #[test]
    fn analytics_answers_decode_and_add_up() {
        // aggregate_metrics' Vec<(i64, AggregatedMetric)> as JSON pairs.
        let buckets: Vec<Bucket> = serde_json::from_str(
            r#"[[1761386400, {"metric": "ac_power", "min": 10.0, "max": 30.0, "avg": 20.0,
                 "stddev": 0.0, "count": 3, "first_ts": 1761386401, "last_ts": 1761389999}],
                [1761390000, {"metric": "ac_power", "min": 5.0, "max": 50.0, "avg": 40.0,
                 "stddev": 0.0, "count": 1, "first_ts": 1761390001, "last_ts": 1761390001}]]"#,
        )
        .unwrap();
        assert_eq!(buckets[1].0, 1_761_390_000);
        let all = overall(&buckets).unwrap();
        assert_eq!(
            (all.min, all.max, all.samples, all.buckets),
            (5.0, 50.0, 4, 2)
        );
        assert!((all.avg - 25.0).abs() < 1e-9, "weighted by samples");
        assert!(overall(&[]).is_none());
        assert_eq!(
            buckets_csv(&buckets[..1], "ac_power"),
            "timestamp,min,max,avg,count,metric\n1761386400,10,30,20,3,ac_power\n"
        );

        // analytics::ComparisonResult.
        let compared: ComparisonResult = serde_json::from_str(
            r#"{"metric": "ac_power",
                "current": {"metric": "ac_power", "min": 1.0, "max": 9.0, "avg": 5.5, "stddev": 1.2,
                            "count": 40, "first_ts": 200, "last_ts": 300},
                "previous": {"metric": "ac_power", "min": 1.0, "max": 8.0, "avg": 5.0, "stddev": 1.0,
                             "count": 38, "first_ts": 100, "last_ts": 199},
                "change_percent": 10.0}"#,
        )
        .unwrap();
        assert_eq!(compared.current.count, 40);
        assert_eq!(compared.change_percent, 10.0);
    }

    #[test]
    fn analytics_paths_split_and_drill() {
        assert_eq!(
            aggregate_path("sma_inv_1", "ac_power", 0, 3600, 60),
            "/api/v1/analytics/aggregate?device_id=sma_inv_1&metric=ac_power&from=0&to=3600&interval=60"
        );
        let (current, previous) = halves(0, 100);
        assert_eq!(
            (current, previous),
            ((50, 100), (0, 49)),
            "recent half first"
        );
        assert_eq!(
            compare_path("d", "m", current, previous),
            "/api/v1/analytics/compare?device_id=d&metric=m&period1_from=50&period1_to=100&period2_from=0&period2_to=49"
        );
        assert_eq!(interval_for(86_400), 3600);
        assert_eq!(interval_for(3600), 300);
        assert_eq!(interval_for(600), 60);
        assert_eq!(drill(7200, 3600), Some((7200, 10_800, 300)));
        assert_eq!(drill(0, 300), Some((0, 300, 60)));
        assert_eq!(drill(0, 60), None, "a minute is the finest bucket");
        assert_eq!(drill(0, 86_400), Some((0, 86_400, 3600)));
    }

    #[test]
    fn recipes_decode_and_their_bodies_follow_the_server() {
        // models::Recipe as list_recipes answers.
        let recipes: Vec<Recipe> = serde_json::from_str(
            r#"[{"id": 4, "name": "Summer", "category": "Seasonal", "device_id": "sma_inv_1",
                 "values": "{\"dc_power\":61000.5,\"ac_power\":59000,\"label\":\"x\"}",
                 "version": 2, "description": null, "created_at": 1761300000, "updated_at": 1761390000},
                {"id": 5, "name": "Base", "category": "", "device_id": "pm5560", "values": "{}",
                 "version": 1, "description": "", "created_at": 1, "updated_at": 2}]"#,
        )
        .unwrap();
        assert_eq!(
            recipes[0].values(),
            [
                ("ac_power".to_owned(), 59000.0),
                ("dc_power".to_owned(), 61000.5)
            ]
        );
        assert_eq!(recipes[1].category(), None, "an empty category is none");
        assert_eq!(recipes[1].description(), None);
        assert_eq!(categories(&recipes), ["Seasonal"]);

        assert_eq!(recipes_path(None), "/api/v1/recipes");
        assert_eq!(
            recipes_path(Some("sma_inv_1")),
            "/api/v1/recipes?device_id=sma_inv_1"
        );
        assert_eq!(load_path(4), "/api/v1/recipes/4/load");
        assert_eq!(command_path("cmd_ab/1"), "/api/v1/commands/cmd_ab%2F1");

        let new = NewRecipe::new(" Winter ", "sma_inv_1", " ", "Low sun").unwrap();
        assert_eq!(
            serde_json::to_value(&new).unwrap(),
            serde_json::json!({"name": "Winter", "device_id": "sma_inv_1", "description": "Low sun"})
        );
        assert!(NewRecipe::new("", "d", "", "").is_err());
        assert!(NewRecipe::new("a", " ", "", "").is_err());
        assert_eq!(
            serde_json::to_value(RecipeUpdate::new("W", "", " cleared? ").unwrap()).unwrap(),
            serde_json::json!({"name": "W", "category": "", "description": "cleared?"})
        );
        assert!(RecipeUpdate::new(" ", "", "").is_err());

        // load_recipe's answer.
        let load: RecipeLoad = serde_json::from_str(
            r#"{"ok": true, "command_ids": ["cmd_a", "cmd_b"], "message": "Recipe load queued: 2 commands"}"#,
        )
        .unwrap();
        assert_eq!(load.command_ids, ["cmd_a", "cmd_b"]);
    }

    #[test]
    fn a_load_is_done_when_every_command_is() {
        let ids = vec!["a".to_owned(), "b".to_owned()];
        let command = |id: &str, status: &str| QueuedCommand {
            id: id.into(),
            status: status.into(),
            ..QueuedCommand::default()
        };
        let mut known = HashMap::new();
        known.insert("a".to_owned(), command("a", "acked"));
        assert!(!tally(&ids, &known).done, "b is not known yet");
        known.insert("b".to_owned(), command("b", "sent"));
        assert!(!tally(&ids, &known).done);
        known.insert("b".to_owned(), command("b", "failed"));
        assert_eq!(
            tally(&ids, &known),
            LoadTally {
                total: 2,
                acked: 1,
                failed: 1,
                done: true
            }
        );
        assert!(is_terminal("expired") && !is_terminal("queued"));
    }
}
