//! The operations pages' side of the REST API: queued commands, the shift
//! handover log and its automatic report, the audit log, and the telemetry
//! export. Typed mirrors of the server's answers, the paths with their
//! query strings, and the small rules the pages share with the server
//! (who may cancel, what counts as an @mention, how shifts fall).

use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use time::{Date, Month, PrimitiveDateTime, Time};

use super::{Alarm, Method, ServerError, VoltrusApi};
use crate::zone::LocalZone;

impl VoltrusApi {
    /// `GET path` as text, for the CSV downloads (`/api/v1/audit/export`,
    /// `/api/v1/data/export?format=csv`).
    pub async fn get_text(&self, path: &str) -> Result<String> {
        let resp = self.req(Method::GET, path)?.send().await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(ServerError::new(status, &body).into());
        }
        Ok(body)
    }

    /// `GET path` written to `file` as the server sent it; how many rows it
    /// held.
    pub async fn download(&self, path: &str, file: &Path, format: Format) -> Result<Saved> {
        let text = self.get_text(path).await?;
        let (rows, total) = format.count(&text);
        std::fs::write(file, &text)
            .map_err(|error| anyhow::anyhow!("could not write {}: {error}", file.display()))?;
        Ok(Saved {
            file: file.to_owned(),
            rows,
            total,
        })
    }
}

// ── Query strings ────────────────────────────────────────────────────────

/// `value` percent-encoded for a query string: everything but the
/// unreserved characters of RFC 3986.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(b"0123456789ABCDEF"[usize::from(byte >> 4)]));
            out.push(char::from(b"0123456789ABCDEF"[usize::from(byte & 0xf)]));
        }
    }
    out
}

/// `path` with the non-empty `params` as its query string.
fn with_query(path: &str, params: &[(&str, &str)]) -> String {
    let mut out = String::from(path);
    for (key, value) in params.iter().filter(|(_, value)| !value.is_empty()) {
        out.push(if out.contains('?') { '&' } else { '?' });
        out.push_str(key);
        out.push('=');
        out.push_str(&encode(value));
    }
    out
}

// ── Queued commands ──────────────────────────────────────────────────────

/// The statuses a command goes through (`models::CommandStatus`).
pub const COMMAND_STATUSES: [&str; 7] = [
    "pending",
    "queued",
    "sent",
    "acked",
    "failed",
    "expired",
    "cancelled",
];

/// Statuses of a command still on its way to the device.
pub const ACTIVE_STATUSES: [&str; 3] = ["pending", "queued", "sent"];

/// Mirrors `models::Command`, a row of `GET /api/v1/commands`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct QueuedCommand {
    pub id: String,
    pub data_source_id: i64,
    /// The numeric device id; `0` for a Modbus device (its id is text, see
    /// `device_ref`).
    pub device_id: i64,
    /// The device's id as its tag names it: a Modbus device's text id, or the
    /// numeric id as text. Empty on commands older than the field.
    pub device_ref: String,
    pub protocol: String,
    /// The value as the server stored it, e.g. `"42.5"`.
    pub value: String,
    pub status: String,
    /// `immediate`, `queued` or `scheduled`.
    pub policy: String,
    pub scheduled_at: Option<i64>,
    pub retry_count: i32,
    pub max_retries: i32,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub sent_at: Option<i64>,
    pub acked_at: Option<i64>,
    pub resolved_at: Option<i64>,
    pub created_by: String,
    pub readback_status: Option<String>,
    pub readback_value: Option<f64>,
}

impl QueuedCommand {
    /// The device as the console shows it: a Modbus device's text id, or `#`
    /// and the numeric id of the other protocols (and of commands older than
    /// `device_ref`).
    pub fn device_label(&self) -> String {
        if self.device_ref.is_empty() || self.device_ref == self.device_id.to_string() {
            format!("#{}", self.device_id)
        } else {
            self.device_ref.clone()
        }
    }

    /// Still on its way: pending, queued or sent.
    pub fn is_active(&self) -> bool {
        ACTIVE_STATUSES.contains(&self.status.as_str())
    }

    /// The server cancels only what has not been sent yet
    /// (`api/commands.rs::cancel_command`).
    pub fn is_cancellable(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "queued")
    }
}

/// Whether a user may cancel `command`: the cancel route is an operator
/// route (admin, demo, integration), and only unsent commands cancel.
pub fn may_cancel(operator: bool, command: &QueuedCommand) -> bool {
    operator && command.is_cancellable()
}

/// Mirrors `models::CommandAuditEntry`, a row of
/// `GET /api/v1/commands/{id}/audit`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct CommandAudit {
    pub id: i64,
    pub command_id: String,
    pub old_status: String,
    pub new_status: String,
    pub changed_by: String,
    pub reason: Option<String>,
    pub created_at: i64,
}

/// `GET /api/v1/commands`, newest first: `status` (one of
/// [`COMMAND_STATUSES`], or all), a page of `limit` from `offset`.
pub fn commands_path(status: Option<&str>, limit: usize, offset: usize) -> String {
    with_query(
        "/api/v1/commands",
        &[
            ("status", status.unwrap_or("")),
            ("limit", &limit.to_string()),
            ("offset", &offset.to_string()),
        ],
    )
}

/// `GET /api/v1/commands/{id}/audit`.
pub fn command_audit_path(id: &str) -> String {
    format!("/api/v1/commands/{}/audit", encode(id))
}

/// `POST /api/v1/commands/{id}/cancel`.
pub fn cancel_path(id: &str) -> String {
    format!("/api/v1/commands/{}/cancel", encode(id))
}

/// The active commands, newest first, from one list per active status:
/// each list is short-lived (queued commands expire), so this misses none
/// that a single recent page would.
pub async fn active_commands(api: VoltrusApi) -> Result<Vec<QueuedCommand>> {
    let [pending, queued, sent] =
        ACTIVE_STATUSES.map(|status| commands_path(Some(status), ACTIVE_LIMIT, 0));
    let (pending, queued, sent) = futures_util::future::try_join3(
        api.get_json::<Vec<QueuedCommand>>(&pending),
        api.get_json::<Vec<QueuedCommand>>(&queued),
        api.get_json::<Vec<QueuedCommand>>(&sent),
    )
    .await?;
    Ok(merge_newest_first([pending, queued, sent]))
}

/// How many commands of one active status the Active list fetches.
pub const ACTIVE_LIMIT: usize = 200;

/// Several lists as one, newest first, each command once.
pub fn merge_newest_first<const N: usize>(lists: [Vec<QueuedCommand>; N]) -> Vec<QueuedCommand> {
    let mut all: Vec<QueuedCommand> = lists.into_iter().flatten().collect();
    all.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    all.dedup_by(|a, b| a.id == b.id);
    all
}

// ── Shift handover ───────────────────────────────────────────────────────

/// The server's shift periods (`api/shift_handover.rs`), in the day's order.
pub const SHIFT_PERIODS: [&str; 3] = ["morning", "afternoon", "night"];

/// Mirrors `models::ShiftHandover`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Handover {
    pub id: i64,
    pub author_id: i64,
    /// The author's username; `null` when the account no longer exists.
    pub author_username: Option<String>,
    /// `YYYY-MM-DD`.
    pub shift_date: String,
    pub shift_period: String,
    pub what_happened: String,
    pub whats_pending: String,
    pub what_to_watch: String,
    pub created_at: i64,
}

impl Handover {
    /// Who wrote the entry: the username, or `#id` when the account is gone.
    pub fn author(&self) -> String {
        match &self.author_username {
            Some(name) if !name.is_empty() => name.clone(),
            _ => format!("#{}", self.author_id),
        }
    }
}

/// `GET /api/v1/shift-handover` answers `{handovers, count}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct HandoverList {
    pub handovers: Vec<Handover>,
    pub count: usize,
}

/// The body of `POST /api/v1/shift-handover`
/// (`models::CreateShiftHandoverRequest`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct NewHandover {
    pub shift_date: String,
    pub shift_period: String,
    pub what_happened: String,
    pub whats_pending: String,
    pub what_to_watch: String,
}

impl NewHandover {
    /// Why the server or the log would not take this entry, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        if parse_date(&self.shift_date).is_none() {
            Some("The shift date reads YYYY-MM-DD")
        } else if !SHIFT_PERIODS.contains(&self.shift_period.as_str()) {
            Some("Pick the shift: morning, afternoon or night")
        } else if self.what_happened.trim().is_empty() {
            Some("Say what happened during the shift")
        } else {
            None
        }
    }
}

/// `POST /api/v1/shift-handover` answers `{id, ok}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Created {
    pub id: i64,
    pub ok: bool,
}

/// `GET /api/v1/shift-handover`, filtered by date and period when given.
pub fn handovers_path(date: Option<&str>, period: Option<&str>) -> String {
    with_query(
        "/api/v1/shift-handover",
        &[
            ("shift_date", date.unwrap_or("")),
            ("shift_period", period.unwrap_or("")),
        ],
    )
}

pub const HANDOVER_PATH: &str = "/api/v1/shift-handover";
pub const SHIFT_REPORT_PATH: &str = "/api/v1/shift-handover/auto-report";

/// Mirrors `models::Annotation`, as the shift report lists them.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Annotation {
    pub id: i64,
    pub tag_id: String,
    pub annotation_text: String,
    pub data_timestamp: i64,
    pub author_id: i64,
    pub created_at: i64,
}

/// Mirrors `models::SecurityEvent`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SecurityEvent {
    pub id: i64,
    pub event_type: String,
    pub severity: String,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    pub username: Option<String>,
    pub details: Option<String>,
    pub created_at: i64,
}

/// `GET /api/v1/shift-handover/auto-report`: the shift in progress (as the
/// server's UTC clock places it, a night shift on the date it started) and
/// what happened in it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ShiftReport {
    pub shift_date: String,
    pub shift_period: String,
    /// The shift's `[start, end)` in Unix seconds.
    pub period_start: Option<i64>,
    pub period_end: Option<i64>,
    pub alarms: Vec<Alarm>,
    pub annotations: Vec<Annotation>,
    pub handovers: Vec<Handover>,
    pub security_events: Vec<SecurityEvent>,
}

/// The shift a UTC hour falls in, by the server's rule: morning 06–14,
/// afternoon 14–22, night 22–06.
pub fn shift_of_hour(hour: u8) -> &'static str {
    match hour {
        6..14 => "morning",
        14..22 => "afternoon",
        _ => "night",
    }
}

/// The hours a shift spans, `(start, end)`; night ends the next morning.
pub fn shift_hours(period: &str) -> Option<(u8, u8)> {
    match period {
        "morning" => Some((6, 14)),
        "afternoon" => Some((14, 22)),
        "night" => Some((22, 6)),
        _ => None,
    }
}

/// The ranges of `text` that are @mentions, by the server's rule
/// (`models::parse_mentions`): `@` not right after a word character (so not
/// an email address), then one or more of `[A-Za-z0-9_]`. The ranges cover
/// the `@` too.
pub fn mention_spans(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut spans = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'@' && (at == 0 || !word(bytes[at - 1])) {
            let mut end = at + 1;
            while end < bytes.len() && word(bytes[end]) {
                end += 1;
            }
            if end > at + 1 {
                spans.push(at..end);
            }
            at = end;
        } else {
            at += 1;
        }
    }
    spans
}

// ── Audit log ────────────────────────────────────────────────────────────

/// Mirrors `models::AuditLogEntry`. `created_at` is Unix seconds.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AuditEntry {
    pub id: i64,
    pub action: String,
    pub username: String,
    pub resource: String,
    pub details: Option<String>,
    pub created_at: i64,
}

/// `GET /api/v1/audit` answers one page and the totals.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AuditPage {
    pub entries: Vec<AuditEntry>,
    pub total: i64,
    pub page: u32,
    pub limit: u32,
    pub total_pages: u32,
}

/// The audit log's sortable columns (the server sorts by these three).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AuditSort {
    #[default]
    Time,
    Action,
    User,
}

impl AuditSort {
    pub const ALL: [Self; 3] = [Self::Time, Self::Action, Self::User];

    pub fn param(self) -> &'static str {
        match self {
            Self::Time => "created_at",
            Self::Action => "action",
            Self::User => "username",
        }
    }

    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Time => Self::Action,
            Self::Action => Self::User,
            Self::User => Self::Time,
        }
    }
}

/// What the audit log page asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditQuery {
    /// Matched anywhere in the action, user, resource and details.
    pub search: String,
    /// Exact action, e.g. `login`.
    pub action: String,
    /// Exact username.
    pub username: String,
    /// Unix seconds, inclusive.
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// From 1.
    pub page: u32,
    pub limit: u32,
    pub sort: AuditSort,
    pub descending: bool,
}

impl Default for AuditQuery {
    fn default() -> Self {
        Self {
            search: String::new(),
            action: String::new(),
            username: String::new(),
            from: None,
            to: None,
            page: 1,
            limit: AUDIT_PAGE,
            sort: AuditSort::Time,
            descending: true,
        }
    }
}

/// Audit entries on a page.
pub const AUDIT_PAGE: u32 = 50;

impl AuditQuery {
    fn filters(&self) -> [(&'static str, String); 5] {
        [
            ("search", self.search.trim().to_owned()),
            ("action", self.action.trim().to_owned()),
            ("username", self.username.trim().to_owned()),
            (
                "from",
                self.from.map(|at| at.to_string()).unwrap_or_default(),
            ),
            ("to", self.to.map(|at| at.to_string()).unwrap_or_default()),
        ]
    }

    /// `GET /api/v1/audit` for this page.
    pub fn path(&self) -> String {
        let filters = self.filters();
        let mut params: Vec<(&str, &str)> = filters
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        let (page, limit) = (self.page.max(1).to_string(), self.limit.to_string());
        params.extend([
            ("page", page.as_str()),
            ("limit", limit.as_str()),
            ("sort", self.sort.param()),
            ("order", if self.descending { "desc" } else { "asc" }),
        ]);
        with_query("/api/v1/audit", &params)
    }

    /// `GET /api/v1/audit/export` with the same filters: every matching
    /// entry as CSV, oldest first.
    pub fn export_path(&self) -> String {
        let filters = self.filters();
        let params: Vec<(&str, &str)> = filters
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        with_query("/api/v1/audit/export", &params)
    }
}

// ── Telemetry export ─────────────────────────────────────────────────────

/// A download's file format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Json,
    Csv,
}

impl Format {
    pub fn param(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
        }
    }

    /// Rows in a downloaded body, and the server's total where it says.
    pub fn count(self, text: &str) -> (usize, Option<i64>) {
        match self {
            Self::Json => serde_json::from_str::<ExportAnswer>(text).map_or((0, None), |answer| {
                (answer.data.len(), Some(answer.meta.total))
            }),
            Self::Csv => (csv_records(text).saturating_sub(1), None),
        }
    }
}

/// Records in CSV text: line ends outside quotes, plus a last line without
/// one.
pub fn csv_records(text: &str) -> usize {
    let mut quoted = false;
    let mut records = 0;
    let mut open = false;
    for character in text.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                open = true;
            }
            '\n' if !quoted => {
                records += 1;
                open = false;
            }
            '\r' => {}
            _ => open = true,
        }
    }
    records + usize::from(open)
}

/// The server's aggregations (`api/data/helpers.rs`); none is raw rows.
pub const AGGREGATIONS: [&str; 5] = ["avg", "min", "max", "count", "sum"];
/// The windows an aggregation takes.
pub const INTERVALS: [&str; 5] = ["1m", "5m", "15m", "1h", "1d"];
/// The server's cap on rows per export.
pub const EXPORT_MAX: usize = 10_000;

/// What `GET /api/v1/data/export` is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportQuery {
    /// Unix seconds.
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub device: String,
    pub metric: String,
    /// One of [`AGGREGATIONS`]; `None` for raw rows.
    pub aggregation: Option<&'static str>,
    /// One of [`INTERVALS`]; sent only with an aggregation.
    pub interval: &'static str,
    pub format: Format,
    pub limit: usize,
}

impl Default for ExportQuery {
    fn default() -> Self {
        Self {
            from: None,
            to: None,
            device: String::new(),
            metric: String::new(),
            aggregation: None,
            interval: "5m",
            format: Format::Json,
            limit: 1000,
        }
    }
}

impl ExportQuery {
    pub fn path(&self) -> String {
        let (from, to, limit) = (
            self.from.map(|at| at.to_string()).unwrap_or_default(),
            self.to.map(|at| at.to_string()).unwrap_or_default(),
            self.limit.clamp(1, EXPORT_MAX).to_string(),
        );
        with_query(
            "/api/v1/data/export",
            &[
                ("from", from.as_str()),
                ("to", to.as_str()),
                ("device", self.device.trim()),
                ("metric", self.metric.trim()),
                ("aggregation", self.aggregation.unwrap_or("")),
                (
                    "interval",
                    if self.aggregation.is_some() {
                        self.interval
                    } else {
                        ""
                    },
                ),
                ("format", self.format.param()),
                ("limit", limit.as_str()),
            ],
        )
    }
}

/// `GET /api/v1/data/export?format=json` (`models::ExportResponse`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ExportAnswer {
    pub data: Vec<ExportRow>,
    pub meta: ExportMeta,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ExportRow {
    /// Unix seconds.
    pub timestamp: i64,
    pub device: String,
    pub metric: String,
    pub value: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ExportMeta {
    /// Matching rows on the server, before the limit.
    pub total: i64,
    pub limit: usize,
    pub offset: usize,
    pub aggregation: String,
    pub interval: String,
}

/// A download written to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub file: PathBuf,
    pub rows: usize,
    /// The server's count of matching rows, when the format says.
    pub total: Option<i64>,
}

// ── Dates as operators type them ─────────────────────────────────────────

/// `YYYY-MM-DD`.
pub fn parse_date(text: &str) -> Option<Date> {
    let mut parts = text.trim().splitn(3, '-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()
}

pub fn format_date(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

/// `YYYY-MM-DD`, `YYYY-MM-DD HH:MM` or `YYYY-MM-DD HH:MM:SS` (a `T` also
/// separates) on `zone`'s clock, as Unix seconds. A bare date is its
/// first second, or with `end_of_day` its last. The zone's offset is the one
/// in force at that wall time; a time the clock skipped reads as after the
/// change, one it showed twice as the first pass.
pub fn parse_local(text: &str, zone: &LocalZone, end_of_day: bool) -> Option<i64> {
    let text = text.trim();
    let (date, clock) = match text.split_once([' ', 'T']) {
        Some((date, clock)) => (date, Some(clock.trim())),
        None => (text, None),
    };
    let date = parse_date(date)?;
    let time = match clock {
        Some(clock) => {
            let mut parts = clock.splitn(3, ':');
            let hour: u8 = parts.next()?.parse().ok()?;
            let minute: u8 = parts.next()?.parse().ok()?;
            let second: u8 = match parts.next() {
                Some(second) => second.parse().ok()?,
                None => 0,
            };
            Time::from_hms(hour, minute, second).ok()?
        }
        None if end_of_day => Time::from_hms(23, 59, 59).ok()?,
        None => Time::MIDNIGHT,
    };
    zone.unix_from_local(PrimitiveDateTime::new(date, time))
}

/// Unix seconds as `YYYY-MM-DD HH:MM` on `zone`'s clock at that instant.
pub fn format_local(unix_secs: i64, zone: &LocalZone) -> String {
    zone.at(unix_secs).map_or_else(
        || "—".to_owned(),
        |at| {
            format!(
                "{} {:02}:{:02}",
                format_date(at.date()),
                at.hour(),
                at.minute()
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_row_decodes_as_the_server_serializes_it() {
        // `models::Command` as `GET /api/v1/commands` sends it: the readback
        // fields are skipped while empty, `readback_attempts` always comes.
        let body = r#"[{
            "id": "k2j9x0a7q1w3e5r7t9y1u", "data_source_id": 12, "device_id": 3,
            "protocol": "modbus", "value": "42.5", "status": "queued",
            "policy": "scheduled", "scheduled_at": 1761400000, "retry_count": 1,
            "max_retries": 3, "error_message": null, "created_at": 1761390000,
            "sent_at": null, "acked_at": null, "resolved_at": null,
            "created_by": "operator1", "readback_attempts": 0
        }, {
            "id": "b", "data_source_id": 4, "device_id": 1, "protocol": "s7",
            "value": "1", "status": "acked", "policy": "immediate",
            "scheduled_at": null, "retry_count": 0, "max_retries": 3,
            "error_message": null, "created_at": 1761380000, "sent_at": 1761380001,
            "acked_at": 1761380002, "resolved_at": 1761380002, "created_by": "admin",
            "readback_status": "match", "readback_value": 1.0,
            "readback_attempts": 1, "readback_ts": "2026-10-25T10:00:00Z"
        }]"#;
        let list: Vec<QueuedCommand> = serde_json::from_str(body).unwrap();
        assert_eq!(list[0].scheduled_at, Some(1_761_400_000));
        assert_eq!((list[0].retry_count, list[0].max_retries), (1, 3));
        assert!(list[0].is_active() && list[0].is_cancellable());
        assert_eq!(list[1].readback_value, Some(1.0));
        assert!(!list[1].is_active() && !list[1].is_cancellable());
    }

    #[test]
    fn commands_name_their_device_by_text_id_or_number() {
        let command = |device_id: i64, device_ref: &str| QueuedCommand {
            device_id,
            device_ref: device_ref.into(),
            ..QueuedCommand::default()
        };
        assert_eq!(command(0, "inverter-1").device_label(), "inverter-1");
        assert_eq!(command(3, "3").device_label(), "#3");
        assert_eq!(command(3, "").device_label(), "#3");
    }

    #[test]
    fn only_operators_cancel_and_only_unsent_commands() {
        let command = |status: &str| QueuedCommand {
            status: status.into(),
            ..QueuedCommand::default()
        };
        assert!(may_cancel(true, &command("pending")));
        assert!(may_cancel(true, &command("queued")));
        // The server refuses a sent command (`cancel_command`), so the
        // console offers no cancel for it.
        assert!(!may_cancel(true, &command("sent")));
        assert!(!may_cancel(true, &command("acked")));
        // A viewer (role `viewer`/`operator` on the server) cannot reach the
        // operator route at all.
        assert!(!may_cancel(false, &command("pending")));
    }

    #[test]
    fn command_audit_rows_decode() {
        let body = r#"[{"id": 7, "command_id": "abc", "old_status": "pending",
            "new_status": "cancelled", "changed_by": "admin",
            "reason": "Cancelled by user", "created_at": 1761390100}]"#;
        let audit: Vec<CommandAudit> = serde_json::from_str(body).unwrap();
        assert_eq!(audit[0].new_status, "cancelled");
        assert_eq!(audit[0].reason.as_deref(), Some("Cancelled by user"));
    }

    #[test]
    fn command_paths_carry_status_and_page() {
        assert_eq!(
            commands_path(None, 20, 40),
            "/api/v1/commands?limit=20&offset=40"
        );
        assert_eq!(
            commands_path(Some("failed"), 20, 0),
            "/api/v1/commands?status=failed&limit=20&offset=0"
        );
        assert_eq!(cancel_path("a b"), "/api/v1/commands/a%20b/cancel");
        assert_eq!(command_audit_path("x1"), "/api/v1/commands/x1/audit");
    }

    #[test]
    fn active_lists_merge_newest_first_without_repeats() {
        let command = |id: &str, at: i64| QueuedCommand {
            id: id.into(),
            created_at: at,
            ..QueuedCommand::default()
        };
        let merged = merge_newest_first([
            vec![command("a", 10), command("c", 30)],
            vec![command("b", 20)],
            vec![command("c", 30)],
        ]);
        let ids: Vec<&str> = merged.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["c", "b", "a"]);
    }

    #[test]
    fn the_handover_list_and_the_shift_report_decode() {
        let list = r#"{"handovers": [{"id": 3, "author_id": 1,
            "shift_date": "2026-10-25", "shift_period": "night",
            "what_happened": "Inverter 2 tripped, @maria reset it",
            "whats_pending": "", "what_to_watch": "DC string B",
            "created_at": 1761390000}], "count": 1}"#;
        let list: HandoverList = serde_json::from_str(list).unwrap();
        assert_eq!(list.count, 1);
        assert_eq!(list.handovers[0].shift_period, "night");
        assert_eq!(
            list.handovers[0].author(),
            "#1",
            "an older server sends no name"
        );
        let named: Handover =
            serde_json::from_str(r#"{"id": 4, "author_id": 2, "author_username": "maria"}"#)
                .unwrap();
        assert_eq!(named.author(), "maria");

        let report = r#"{
            "shift_date": "2026-10-25", "shift_period": "morning",
            "alarms": [{"id": 9, "source_key": "ac_current_l1", "source_name": "AC current L1",
                "severity": "critical", "priority": "high", "value": 86.4, "threshold": 85.0,
                "message": "AC current L1 above 85", "triggered_at": 1761380000,
                "resolved_at": null, "acknowledged_at": null, "acknowledged_by": null,
                "cooldown_until": null, "shelved_until": null, "shelved_by": null,
                "shelved_reason": null, "deadband": 0.0, "state": "active", "alarm_type": "high"}],
            "alarm_count": 1,
            "annotations": [{"id": 1, "tag_id": "irradiance", "annotation_text": "Cloud front",
                "data_timestamp": 1761381000, "author_id": 2, "created_at": 1761381100,
                "updated_at": 1761381100}],
            "annotation_count": 1,
            "handovers": [], "handover_count": 0,
            "security_events": [{"id": 4, "event_type": "login_failed", "severity": "warning",
                "source_ip": "10.0.0.8", "user_agent": null, "username": "guest",
                "details": "bad password", "created_at": 1761382000}],
            "security_event_count": 1
        }"#;
        let report: ShiftReport = serde_json::from_str(report).unwrap();
        assert_eq!(report.alarms[0].severity, "critical");
        assert_eq!(report.annotations[0].annotation_text, "Cloud front");
        assert_eq!(report.security_events[0].username.as_deref(), Some("guest"));
        assert!(report.handovers.is_empty());
    }

    #[test]
    fn a_new_entry_needs_a_date_a_shift_and_what_happened() {
        let mut entry = NewHandover {
            shift_date: "2026-10-25".into(),
            shift_period: "night".into(),
            what_happened: "  ".into(),
            ..NewHandover::default()
        };
        assert_eq!(entry.problem(), Some("Say what happened during the shift"));
        entry.what_happened = "Quiet".into();
        assert_eq!(entry.problem(), None);
        entry.shift_period = "evening".into();
        assert!(entry.problem().is_some());
        entry.shift_period = "morning".into();
        entry.shift_date = "2026-13-01".into();
        assert_eq!(entry.problem(), Some("The shift date reads YYYY-MM-DD"));
        let body = serde_json::to_value(NewHandover {
            shift_date: "2026-10-25".into(),
            shift_period: "morning".into(),
            what_happened: "a".into(),
            ..NewHandover::default()
        })
        .unwrap();
        assert_eq!(body["what_to_watch"], "");
        assert_eq!(body["shift_period"], "morning");
    }

    #[test]
    fn handover_paths_filter_only_what_is_set() {
        assert_eq!(handovers_path(None, None), "/api/v1/shift-handover");
        assert_eq!(
            handovers_path(Some("2026-10-25"), Some("night")),
            "/api/v1/shift-handover?shift_date=2026-10-25&shift_period=night"
        );
        assert_eq!(
            handovers_path(None, Some("morning")),
            "/api/v1/shift-handover?shift_period=morning"
        );
    }

    #[test]
    fn mentions_follow_the_servers_rule() {
        let text = "Ask @maria_2 and @Bob. Mail ops@plant.id, not @ alone; @x";
        let names: Vec<&str> = mention_spans(text)
            .into_iter()
            .map(|span| &text[span])
            .collect();
        assert_eq!(names, ["@maria_2", "@Bob", "@x"]);
        assert!(mention_spans("").is_empty());
        // Multibyte text around a mention keeps byte ranges on boundaries.
        let text = "Café @andré";
        let spans = mention_spans(text);
        assert_eq!(&text[spans[0].clone()], "@andr");
    }

    #[test]
    fn shifts_fall_like_the_server_places_them() {
        assert_eq!(shift_of_hour(5), "night");
        assert_eq!(shift_of_hour(6), "morning");
        assert_eq!(shift_of_hour(14), "afternoon");
        assert_eq!(shift_of_hour(22), "night");
    }

    #[test]
    fn shift_reports_carry_the_servers_window() {
        let report: ShiftReport = serde_json::from_str(
            r#"{"shift_date": "2026-05-03", "shift_period": "night",
                "period_start": 1777845600, "period_end": 1777874400,
                "alarms": [], "annotations": [], "handovers": [], "security_events": []}"#,
        )
        .unwrap();
        assert_eq!(
            report.period_end.unwrap() - report.period_start.unwrap(),
            8 * 3600
        );
        assert_eq!(
            report.period_start,
            Some(1_777_845_600),
            "22:00 UTC on 3 May 2026"
        );
    }

    #[test]
    fn audit_pages_decode_and_paths_keep_filters_and_sort() {
        let body = r#"{"entries": [{"id": 5, "action": "login", "username": "admin",
            "resource": "auth", "details": null, "created_at": 1761390000}],
            "total": 51, "page": 2, "limit": 50, "total_pages": 2}"#;
        let page: AuditPage = serde_json::from_str(body).unwrap();
        assert_eq!((page.total, page.total_pages), (51, 2));
        assert_eq!(page.entries[0].details, None);

        let mut query = AuditQuery::default();
        assert_eq!(
            query.path(),
            "/api/v1/audit?page=1&limit=50&sort=created_at&order=desc"
        );
        query.search = " tag write ".into();
        query.username = "op&1".into();
        query.from = Some(1_761_350_400);
        query.page = 3;
        query.sort = AuditSort::User;
        query.descending = false;
        assert_eq!(
            query.path(),
            "/api/v1/audit?search=tag%20write&username=op%261&from=1761350400&page=3&limit=50&sort=username&order=asc"
        );
        assert_eq!(
            query.export_path(),
            "/api/v1/audit/export?search=tag%20write&username=op%261&from=1761350400"
        );
        assert_eq!(AuditSort::User.next(), AuditSort::Time);
    }

    #[test]
    fn export_paths_send_an_interval_only_with_an_aggregation() {
        let mut query = ExportQuery::default();
        assert_eq!(query.path(), "/api/v1/data/export?format=json&limit=1000");
        query.interval = "1h";
        assert!(!query.path().contains("interval"));
        query.aggregation = Some("avg");
        query.device = "inverter-1".into();
        query.from = Some(100);
        query.to = Some(200);
        query.format = Format::Csv;
        query.limit = 50_000;
        assert_eq!(
            query.path(),
            "/api/v1/data/export?from=100&to=200&device=inverter-1&aggregation=avg&interval=1h&format=csv&limit=10000"
        );
    }

    #[test]
    fn downloads_count_their_rows() {
        let json = r#"{"data": [{"timestamp": 1761390000, "device": "inv1",
            "metric": "ac_power", "value": 512.5}], "meta": {"total": 1200,
            "limit": 1, "offset": 0, "aggregation": "raw", "interval": "none"}}"#;
        assert_eq!(Format::Json.count(json), (1, Some(1200)));
        let csv = "timestamp,device,metric,value\n1,a,b,2\n3,a,b,4\n";
        assert_eq!(Format::Csv.count(csv), (2, None));
        // A quoted field may hold a line break; the last line may lack one.
        let audit = "id,action,username,resource,details,created_at\n1,login,admin,auth,\"two\nlines\",5\n2,x,y,z,,6";
        assert_eq!(Format::Csv.count(audit), (2, None));
        assert_eq!(Format::Csv.count(""), (0, None));
    }

    #[test]
    fn local_dates_read_on_the_operators_clock() {
        let jakarta = &LocalZone::named("Asia/Jakarta").unwrap();
        // 2026-10-25 00:00 in Jakarta is 2026-10-24 17:00 UTC.
        assert_eq!(
            parse_local("2026-10-25", jakarta, false),
            Some(1_792_861_200)
        );
        assert_eq!(
            parse_local("2026-10-25", jakarta, true),
            Some(1_792_861_200 + 86_399)
        );
        assert_eq!(
            parse_local("2026-10-25 14:30", jakarta, false),
            Some(1_792_913_400)
        );
        assert_eq!(
            parse_local("2026-10-25T14:30:15", jakarta, false),
            Some(1_792_913_415)
        );
        assert_eq!(parse_local("25/10/2026", jakarta, false), None);
        assert_eq!(parse_local("2026-02-30", jakarta, false), None);
        assert_eq!(format_local(1_792_913_400, jakarta), "2026-10-25 14:30");
        assert_eq!(format_date(parse_date(" 2026-1-5 ").unwrap()), "2026-01-05");
    }

    #[test]
    fn local_times_follow_the_daylight_saving_change() {
        let york = &LocalZone::named("America/New_York").unwrap();
        // An alarm at noon on each side of 2026-11-01's change.
        let before = 1_793_462_400; // 2026-10-31 12:00 EDT
        let after = 1_793_638_800; // 2026-11-02 12:00 EST
        assert_eq!(format_local(before, york), "2026-10-31 12:00");
        assert_eq!(format_local(after, york), "2026-11-02 12:00");
        assert_eq!(parse_local("2026-10-31 12:00", york, false), Some(before));
        assert_eq!(parse_local("2026-11-02 12:00", york, false), Some(after));
        // A range over the change reads each end on its own offset: a day
        // on the wall is 25 hours that Sunday.
        let day = parse_local("2026-11-01", york, false).unwrap();
        let end = parse_local("2026-11-01", york, true).unwrap();
        assert_eq!(end - day, 25 * 3600 - 1);
        assert_eq!(format_local(day, york), "2026-11-01 00:00");
        assert_eq!(format_local(end, york), "2026-11-01 23:59");
        // The hour the clocks skip reads as after the change.
        let skipped = parse_local("2026-03-08 02:30", york, false).unwrap();
        assert_eq!(format_local(skipped, york), "2026-03-08 03:30");
        // A night shift, 22:00 to 06:00 on the wall.
        let night = |from: &str, to: &str| {
            parse_local(to, york, false).unwrap() - parse_local(from, york, false).unwrap()
        };
        assert_eq!(night("2026-10-31 22:00", "2026-11-01 06:00"), 9 * 3600);
        assert_eq!(night("2026-03-07 22:00", "2026-03-08 06:00"), 7 * 3600);
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(encode("x y&z=1/é"), "x%20y%26z%3D1%2F%C3%A9");
    }
}
