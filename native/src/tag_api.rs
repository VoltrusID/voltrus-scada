//! Tag writes and write policies, the tag tree and the naming convention,
//! chart annotations and the tag detail's history: the server's shapes for
//! them, and the rules the console applies before it sends anything. No UI
//! here, so all of it is tested on its own.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

use crate::api::HistorySeries;

// ── Writes ────────────────────────────────────────────────────────────────

/// How a written value reaches the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// Over the WebSocket, at once: admin only, and the server checks only
    /// the tag's `writable` flag (no limits, no reason, no command record).
    Direct,
    /// `POST /api/v1/tags/{id}/write` with `policy: immediate`: checked
    /// against the write policy and recorded as a command.
    Immediate,
    /// The same with `policy: queued`: the command queue sends it later.
    Queued,
}

impl WriteMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "Direct",
            Self::Immediate => "Immediate",
            Self::Queued => "Queued",
        }
    }

    /// The REST `policy` field; `None` for a WebSocket write.
    pub fn policy(self) -> Option<&'static str> {
        match self {
            Self::Direct => None,
            Self::Immediate => Some("immediate"),
            Self::Queued => Some("queued"),
        }
    }

    /// The modes a user may pick: operators (admin, demo, integration)
    /// write through REST; only an admin also writes over the WebSocket.
    pub fn allowed(admin: bool, operator: bool) -> &'static [Self] {
        if admin {
            &[Self::Direct, Self::Immediate, Self::Queued]
        } else if operator {
            &[Self::Immediate, Self::Queued]
        } else {
            &[]
        }
    }
}

/// Mirrors `voltrus::models::WritePolicy`, the answer of `GET
/// /api/v1/data-sources/{id}/write-policy`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WritePolicy {
    pub writable: bool,
    pub write_min: Option<f64>,
    pub write_max: Option<f64>,
    /// Writes per minute at most.
    pub write_max_rate: Option<i64>,
    pub write_requires_reason: bool,
    /// A write this close to the current value is refused.
    pub write_deadband: Option<f64>,
}

/// The body of `POST /api/v1/tags/{id}/write`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WriteRequest {
    pub value: f64,
    pub policy: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The server's `202 Accepted` answer to a REST write.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct WriteAccepted {
    pub command_id: String,
    pub status: String,
    pub data_source_id: i64,
    pub value: f64,
    pub created_at: i64,
}

/// The value to write, or why it may not go: the checks the server makes,
/// against the policy when the console knows it.
pub fn check_write(
    text: &str,
    reason: &str,
    mode: WriteMode,
    policy: Option<&WritePolicy>,
) -> Result<f64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter a value".into());
    }
    let value = text
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("“{text}” is not a number"))?;
    let Some(policy) = policy else {
        return Ok(value);
    };
    if !policy.writable {
        return Err("The tag's write policy does not allow writes".into());
    }
    if let Some(min) = policy.write_min
        && value < min
    {
        return Err(format!("Below the minimum, {min}"));
    }
    if let Some(max) = policy.write_max
        && value > max
    {
        return Err(format!("Above the maximum, {max}"));
    }
    if policy.write_requires_reason {
        if mode == WriteMode::Direct {
            return Err("This tag needs a reason; a direct write carries none".into());
        }
        if reason.trim().is_empty() {
            return Err("This tag needs a reason for every write".into());
        }
    }
    Ok(value)
}

/// The REST write for `value`, or `None` for a direct (WebSocket) write.
pub fn write_request(value: f64, reason: &str, mode: WriteMode) -> Option<WriteRequest> {
    let reason = reason.trim();
    Some(WriteRequest {
        value,
        policy: mode.policy()?,
        reason: (!reason.is_empty()).then(|| reason.to_owned()),
    })
}

/// An optional number from a form field: empty is `None`.
fn optional<T: std::str::FromStr>(label: &str, text: &str) -> Result<Option<T>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse()
        .map(Some)
        .map_err(|_| format!("{label}: “{text}” is not a number"))
}

/// A number limit from a form field: empty is `None`; `nan` and `inf`
/// parse as floats but are no limit, so they are refused like any other
/// non-number.
fn finite(label: &str, text: &str) -> Result<Option<f64>, String> {
    match optional::<f64>(label, text)? {
        Some(value) if !value.is_finite() => {
            Err(format!("{label}: “{}” is not a number", text.trim()))
        }
        value => Ok(value),
    }
}

/// The body of `PATCH /api/v1/data-sources/{id}/write-policy`: the server
/// changes only the settings it carries, and `Some(None)` (sent as `null`)
/// clears a limit.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PolicyPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_min: Option<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_max: Option<Option<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_max_rate: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_requires_reason: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_deadband: Option<Option<f64>>,
}

impl PolicyPatch {
    /// Nothing to change.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The whole policy once the server applied the patch: `before` with
    /// the patch's settings, or, with `before` unknown, the patch alone
    /// when it sets all six.
    pub fn applied(&self, before: Option<&WritePolicy>) -> Option<WritePolicy> {
        Some(match before {
            Some(before) => WritePolicy {
                writable: self.writable.unwrap_or(before.writable),
                write_min: self.write_min.unwrap_or(before.write_min),
                write_max: self.write_max.unwrap_or(before.write_max),
                write_max_rate: self.write_max_rate.unwrap_or(before.write_max_rate),
                write_requires_reason: self
                    .write_requires_reason
                    .unwrap_or(before.write_requires_reason),
                write_deadband: self.write_deadband.unwrap_or(before.write_deadband),
            },
            None => WritePolicy {
                writable: self.writable?,
                write_min: self.write_min?,
                write_max: self.write_max?,
                write_max_rate: self.write_max_rate?,
                write_requires_reason: self.write_requires_reason?,
                write_deadband: self.write_deadband?,
            },
        })
    }
}

/// A choice for the patch: sent when made and unlike the known setting.
fn changed_choice(now: Option<bool>, was: Option<bool>) -> Option<bool> {
    now.filter(|now| Some(*now) != was)
}

/// A limit for the patch. With the setting known, the field's value when
/// it differs (empty clears); with it unknown, only a value typed, so an
/// empty field leaves the server's limit as it is.
fn changed_limit<T: PartialEq + Copy>(now: Option<T>, was: Option<Option<T>>) -> Option<Option<T>> {
    match was {
        Some(was) => (now != was).then_some(now),
        None => now.map(Some),
    }
}

/// What the policy editor's fields change against `known`, the policy when
/// the console knows it. The server cannot be asked for a policy, so with
/// none known the editor sends only the choices made (`None` is a choice
/// left alone) and the limits typed, never defaults over the stored ones.
pub fn policy_patch(
    known: Option<&WritePolicy>,
    writable: Option<bool>,
    requires_reason: Option<bool>,
    min: &str,
    max: &str,
    rate: &str,
    deadband: &str,
) -> Result<PolicyPatch, String> {
    let min = finite("Minimum", min)?;
    let max = finite("Maximum", max)?;
    let rate = optional::<i64>("Writes per minute", rate)?;
    let deadband = finite("Deadband", deadband)?;
    if let (Some(min), Some(max)) = (min, max)
        && min > max
    {
        return Err(format!("The minimum, {min}, is above the maximum, {max}"));
    }
    if rate.is_some_and(|rate| rate < 0) {
        return Err("Writes per minute cannot be negative".into());
    }
    if deadband.is_some_and(|deadband| deadband < 0.0) {
        return Err("The deadband cannot be negative".into());
    }
    Ok(PolicyPatch {
        writable: changed_choice(writable, known.map(|policy| policy.writable)),
        write_min: changed_limit(min, known.map(|policy| policy.write_min)),
        write_max: changed_limit(max, known.map(|policy| policy.write_max)),
        write_max_rate: changed_limit(rate, known.map(|policy| policy.write_max_rate)),
        write_requires_reason: changed_choice(
            requires_reason,
            known.map(|policy| policy.write_requires_reason),
        ),
        write_deadband: changed_limit(deadband, known.map(|policy| policy.write_deadband)),
    })
}

// ── Tag tree ──────────────────────────────────────────────────────────────

/// One node of `GET /api/v1/tags/tree`: a path segment, the tags whose
/// path ends here, and the segments below.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TreeNode {
    pub name: String,
    /// Tags at this node itself (not below).
    #[serde(rename = "tagCount")]
    pub tag_count: usize,
    #[serde(rename = "tagIds")]
    pub tag_ids: Vec<i64>,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TagTree {
    pub tree: Vec<TreeNode>,
}

impl TreeNode {
    /// Tags here and below.
    pub fn total(&self) -> usize {
        self.tag_ids.len() + self.children.iter().map(Self::total).sum::<usize>()
    }

    /// Every tag here and below, with its path relative to this node
    /// (empty for the node's own tags).
    pub fn tags_below(&self) -> Vec<(i64, String)> {
        let mut out = Vec::new();
        self.collect("", &mut out);
        out
    }

    fn collect(&self, relative: &str, out: &mut Vec<(i64, String)>) {
        out.extend(self.tag_ids.iter().map(|id| (*id, relative.to_owned())));
        for child in &self.children {
            child.collect(&join(relative, &child.name), out);
        }
    }
}

/// `parent/name`, or `name` at the root.
pub fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// The node at `path` (segments joined by `/`).
pub fn find<'a>(tree: &'a [TreeNode], path: &str) -> Option<&'a TreeNode> {
    let mut nodes = tree;
    let mut found = None;
    for segment in path.split('/') {
        let node = nodes.iter().find(|node| node.name == segment)?;
        nodes = &node.children;
        found = Some(node);
    }
    found
}

/// One line of the tree as the page lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    /// A path segment: its full path, how deep it sits, the tags under it
    /// and whether its rows show.
    Node {
        path: String,
        name: String,
        depth: usize,
        total: usize,
        open: bool,
    },
    /// A tag whose path ends at the node above it.
    Tag { id: i64, path: String, depth: usize },
}

/// The tree's rows, depth first: each open node's own tags, then its
/// children. Nodes are open unless their path is in `closed`.
pub fn tree_rows(tree: &[TreeNode], closed: &HashSet<String>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    for node in tree {
        push_rows(node, "", 0, closed, &mut rows);
    }
    rows
}

fn push_rows(
    node: &TreeNode,
    parent: &str,
    depth: usize,
    closed: &HashSet<String>,
    rows: &mut Vec<TreeRow>,
) {
    let path = join(parent, &node.name);
    let open = !closed.contains(&path);
    rows.push(TreeRow::Node {
        path: path.clone(),
        name: node.name.clone(),
        depth,
        total: node.total(),
        open,
    });
    if !open {
        return;
    }
    rows.extend(node.tag_ids.iter().map(|id| TreeRow::Tag {
        id: *id,
        path: path.clone(),
        depth: depth + 1,
    }));
    for child in &node.children {
        push_rows(child, &path, depth + 1, closed, rows);
    }
}

/// A path as the operator typed it, checked like the server checks it:
/// `None` (empty) clears the path; every segment between slashes needs a
/// name. Spaces around segments are dropped.
pub fn check_path(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let segments: Vec<&str> = text.split('/').map(str::trim).collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err("Each part between slashes needs a name, as in Site/Area/Unit".into());
    }
    Ok(Some(segments.join("/")))
}

/// Where each tag under `node` goes when the branch moves to `to`: every
/// tag keeps its place below the branch. `None` clears the paths of the
/// branch's own tags (the server then files them under their device) and
/// moves the ones below to the root.
pub fn moved_paths(node: &TreeNode, to: Option<&str>) -> Vec<(i64, Option<String>)> {
    node.tags_below()
        .into_iter()
        .map(|(id, relative)| {
            let path = match (to, relative.is_empty()) {
                (Some(to), true) => Some(to.to_owned()),
                (Some(to), false) => Some(join(to, &relative)),
                (None, true) => None,
                (None, false) => Some(relative),
            };
            (id, path)
        })
        .collect()
}

// ── Naming convention ─────────────────────────────────────────────────────

/// `GET /api/v1/settings/naming-convention`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct NamingConvention {
    pub template: String,
    /// The pattern in force: the template's, or the custom one.
    pub regex: String,
    pub custom_regex: Option<String>,
    pub templates: Vec<NamingTemplate>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct NamingTemplate {
    pub id: String,
    pub label: String,
}

/// The body of `PUT /api/v1/settings/naming-convention`. The server keeps
/// a custom pattern only for the `custom` template.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NamingUpdate {
    pub template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_regex: Option<String>,
}

impl NamingUpdate {
    pub fn new(template: &str, custom_regex: &str) -> Self {
        let custom_regex = custom_regex.trim();
        Self {
            template: template.to_owned(),
            custom_regex: (template == "custom" && !custom_regex.is_empty())
                .then(|| custom_regex.to_owned()),
        }
    }
}

// ── Annotations ───────────────────────────────────────────────────────────

/// Mirrors `voltrus::models::Annotation`; `tag_id` is the tag's key and
/// `data_timestamp` Unix seconds.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Annotation {
    pub id: i64,
    pub tag_id: String,
    pub annotation_text: String,
    pub data_timestamp: i64,
    pub author_id: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `GET /api/v1/annotations?tag_id=`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Annotations {
    pub annotations: Vec<Annotation>,
    pub count: usize,
}

/// The body of `POST /api/v1/annotations`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewAnnotation {
    pub tag_id: String,
    pub annotation_text: String,
    pub data_timestamp: i64,
}

/// The server's answer to a create: the new row's id.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Created {
    pub id: i64,
    pub ok: bool,
}

/// `GET` path of a tag's annotations, newest first as the server sorts.
pub fn annotations_path(key: &str) -> String {
    format!("/api/v1/annotations?tag_id={}", query_value(key))
}

// ── History for the tag detail ────────────────────────────────────────────

/// The detail chart's preset spans: id, label, length. Each id is a server
/// `range` except `5m`, which the server's `30m` covers.
pub const SPANS: [(&str, &str, f64); 6] = [
    ("5m", "5 min", 5.0 * 60_000.0),
    ("30m", "30 min", 30.0 * 60_000.0),
    ("1h", "1 h", 3_600_000.0),
    ("6h", "6 h", 6.0 * 3_600_000.0),
    ("24h", "24 h", 24.0 * 3_600_000.0),
    ("7d", "7 d", 7.0 * 24.0 * 3_600_000.0),
];

/// What the chart covers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Span {
    /// The last stretch up to now, one of [`SPANS`]; live values extend it.
    Last(&'static str),
    /// A fixed stretch in the past, Unix ms.
    Between { start_ms: i64, end_ms: i64 },
}

impl Default for Span {
    fn default() -> Self {
        Self::Last("1h")
    }
}

impl Span {
    /// The span's length in ms.
    #[allow(clippy::cast_precision_loss)]
    pub fn length_ms(self) -> f64 {
        match self {
            Self::Last(id) => SPANS
                .iter()
                .find(|(span, _, _)| *span == id)
                .map_or(3_600_000.0, |(_, _, length)| *length),
            Self::Between { start_ms, end_ms } => (end_ms - start_ms).max(1) as f64,
        }
    }

    /// The server's `range` parameter: a preset, or `startMs-endMs`.
    pub fn range(self) -> String {
        match self {
            Self::Last("5m") => "30m".to_owned(),
            Self::Last(id) => id.to_owned(),
            Self::Between { start_ms, end_ms } => format!("{start_ms}-{end_ms}"),
        }
    }

    pub fn is_live(self) -> bool {
        matches!(self, Self::Last(_))
    }
}

/// History resolution: the server's own (`auto`) or every second (`full`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detail {
    #[default]
    Auto,
    Full,
}

/// How the server folds samples into a bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Aggregation {
    /// One sample per bucket.
    #[default]
    Sampling,
    Average,
    /// The web's min/max: the server adds min and max arrays to the mean;
    /// the console draws the mean.
    MinMax,
}

impl Aggregation {
    pub const ALL: [Self; 3] = [Self::Sampling, Self::Average, Self::MinMax];

    pub fn label(self) -> &'static str {
        match self {
            Self::Sampling => "Sample",
            Self::Average => "Average",
            Self::MinMax => "Min/max",
        }
    }

    fn param(self) -> &'static str {
        match self {
            Self::Sampling => "sampling",
            Self::Average => "avg",
            Self::MinMax => "minmax",
        }
    }
}

/// The detail chart's `GET /api/v1/history` request.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HistoryQuery {
    pub span: Span,
    pub detail: Detail,
    pub agg: Aggregation,
}

impl HistoryQuery {
    pub fn path(&self) -> String {
        let mut path = format!("/api/v1/history?range={}", self.span.range());
        if self.detail == Detail::Full {
            path.push_str("&detail=full");
        }
        let _ = write!(path, "&agg={}", self.agg.param());
        path.push_str("&keyed_by=id");
        path
    }
}

/// `GET /api/v1/history` for `query`, limited to the tags with these keys.
pub fn history_path(query: &HistoryQuery, keys: &[String]) -> String {
    let mut path = query.path();
    if !keys.is_empty() {
        let _ = write!(path, "&sensor={}", query_value(&keys.join(",")));
    }
    path
}

/// The detail chart's history answer: the series of the tags asked for, by
/// data source id (`keyed_by=id`; tag keys are not unique).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct HistoryAnswer {
    pub data: HashMap<String, HistorySeries>,
    /// The bucket the server used, ms.
    pub interval: u64,
    pub range: String,
}

/// A series' samples as the chart draws them: `(Unix ms, value)`.
#[allow(clippy::cast_precision_loss)]
pub fn chart_samples(series: &HistorySeries) -> Vec<(f64, f64)> {
    series
        .samples()
        .map(|(at_ms, value)| (at_ms as f64, value))
        .collect()
}

/// Fetched history followed by the live samples newer than its end.
pub fn merge(fetched: &[(f64, f64)], live: impl Iterator<Item = (f64, f64)>) -> Vec<(f64, f64)> {
    let end = fetched.last().map_or(f64::NEG_INFINITY, |(at, _)| *at);
    let mut merged = fetched.to_vec();
    merged.extend(live.filter(|(at, _)| *at > end));
    merged
}

/// The figures under the detail chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    /// Points in the window, broken ones included.
    pub points: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}

/// Points, and min, max and mean of the finite values, in `[from, to]`;
/// `None` without a finite value there.
pub fn stats(samples: &[(f64, f64)], from_ms: f64, to_ms: f64) -> Option<Stats> {
    let start = samples.partition_point(|(at, _)| *at < from_ms);
    let mut points = 0;
    let mut finite = 0_u32;
    let (mut min, mut max, mut mean) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
    for (_, value) in samples[start..].iter().take_while(|(at, _)| *at <= to_ms) {
        points += 1;
        if value.is_finite() {
            finite += 1;
            min = min.min(*value);
            max = max.max(*value);
            mean += (value - mean) / f64::from(finite);
        }
    }
    (finite > 0).then_some(Stats {
        points,
        min,
        max,
        mean,
    })
}

// ── Times on the operator's clock ─────────────────────────────────────────

/// `2026-10-25 14:05:09` on the clock at `offset`.
pub fn format_local(at_ms: i64, offset: UtcOffset) -> String {
    let Ok(at) = OffsetDateTime::from_unix_timestamp(at_ms.div_euclid(1000)) else {
        return "—".to_owned();
    };
    let at = at.to_offset(offset);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

/// `2026-10-25 14:05` or `2026-10-25 14:05:09` (a `T` works as the
/// separator too) on the clock at `offset`, as Unix ms.
pub fn parse_local(text: &str, offset: UtcOffset) -> Option<i64> {
    let text = text.trim();
    let (date, clock) = text.split_once([' ', 'T'])?;
    let mut date_parts = date.trim().split('-');
    let year: i32 = date_parts.next()?.parse().ok()?;
    let month: u8 = date_parts.next()?.parse().ok()?;
    let day: u8 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() {
        return None;
    }
    let mut clock_parts = clock.trim().split(':');
    let hour: u8 = clock_parts.next()?.parse().ok()?;
    let minute: u8 = clock_parts.next()?.parse().ok()?;
    let second: u8 = match clock_parts.next() {
        Some(second) => second.parse().ok()?,
        None => 0,
    };
    if clock_parts.next().is_some() {
        return None;
    }
    let date = Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()?;
    let clock = Time::from_hms(hour, minute, second).ok()?;
    let at = PrimitiveDateTime::new(date, clock).assume_offset(offset);
    Some(at.unix_timestamp().saturating_mul(1000))
}

/// A custom chart span from two local times, `from` before `to`.
pub fn parse_between(from: &str, to: &str, offset: UtcOffset) -> Result<Span, String> {
    let start_ms = parse_local(from, offset)
        .ok_or_else(|| format!("From: “{}” is not a time", from.trim()))?;
    let end_ms =
        parse_local(to, offset).ok_or_else(|| format!("To: “{}” is not a time", to.trim()))?;
    if start_ms <= 0 {
        return Err("From: the server keeps no history that old".into());
    }
    if end_ms <= start_ms {
        return Err("To must come after From".into());
    }
    Ok(Span::Between { start_ms, end_ms })
}

// ── CSV ───────────────────────────────────────────────────────────────────

/// One column of an export: a tag's key and its samples, oldest first.
pub struct CsvColumn<'a> {
    pub key: &'a str,
    pub samples: &'a [(f64, f64)],
}

/// The columns' samples in `[from, to]` as CSV: one row per instant, the
/// time as Unix ms and on the operator's clock, then one column per tag
/// (named by key, like the server's export), empty where a tag has no
/// sample at that instant. Also returns the number of rows.
#[allow(clippy::cast_possible_truncation)]
pub fn csv(
    columns: &[CsvColumn<'_>],
    from_ms: f64,
    to_ms: f64,
    offset: UtcOffset,
) -> (String, usize) {
    let windows: Vec<&[(f64, f64)]> = columns
        .iter()
        .map(|column| {
            let start = column.samples.partition_point(|(at, _)| *at < from_ms);
            let end = column.samples.partition_point(|(at, _)| *at <= to_ms);
            &column.samples[start..end.max(start)]
        })
        .collect();
    let mut instants: Vec<f64> = windows
        .iter()
        .flat_map(|window| window.iter().map(|(at, _)| *at))
        .collect();
    instants.sort_by(f64::total_cmp);
    instants.dedup();

    let mut out = String::from("unix_ms,time");
    for column in columns {
        out.push(',');
        out.push_str(&csv_field(column.key));
    }
    out.push('\n');
    let mut cursors = vec![0_usize; windows.len()];
    for at in &instants {
        let _ = write!(out, "{},{}", *at as i64, format_local(*at as i64, offset));
        for (window, cursor) in windows.iter().zip(cursors.iter_mut()) {
            out.push(',');
            while *cursor < window.len() && window[*cursor].0 < *at {
                *cursor += 1;
            }
            if let Some((sample_at, value)) = window.get(*cursor)
                && sample_at.total_cmp(at).is_eq()
                && value.is_finite()
            {
                let _ = write!(out, "{value}");
            }
        }
        out.push('\n');
    }
    (out, instants.len())
}

/// A CSV field, quoted when it holds a comma, a quote or a line break.
fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// `text` for a URL query value: unreserved characters as they are, the
/// rest percent-encoded.
pub fn query_value(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_pick_the_write_modes() {
        assert_eq!(
            WriteMode::allowed(true, true),
            [WriteMode::Direct, WriteMode::Immediate, WriteMode::Queued]
        );
        // Demo and integration users write through REST only.
        assert_eq!(
            WriteMode::allowed(false, true),
            [WriteMode::Immediate, WriteMode::Queued]
        );
        assert!(WriteMode::allowed(false, false).is_empty());
        assert_eq!(WriteMode::Direct.policy(), None);
        assert_eq!(WriteMode::Queued.policy(), Some("queued"));
    }

    #[test]
    fn writes_are_checked_like_the_server_checks_them() {
        let immediate = WriteMode::Immediate;
        assert_eq!(check_write(" 12.5 ", "", immediate, None), Ok(12.5));
        assert!(check_write("", "", immediate, None).is_err());
        assert!(check_write("abc", "", immediate, None).is_err());
        assert!(check_write("inf", "", immediate, None).is_err());
        let policy = WritePolicy {
            writable: true,
            write_min: Some(0.0),
            write_max: Some(100.0),
            write_requires_reason: true,
            ..WritePolicy::default()
        };
        assert_eq!(
            check_write("-1", "fix", immediate, Some(&policy)),
            Err("Below the minimum, 0".into())
        );
        assert_eq!(
            check_write("101", "fix", immediate, Some(&policy)),
            Err("Above the maximum, 100".into())
        );
        assert!(check_write("50", "  ", immediate, Some(&policy)).is_err());
        assert_eq!(check_write("50", "fix", immediate, Some(&policy)), Ok(50.0));
        // A direct write cannot carry the reason the policy wants.
        assert!(check_write("50", "fix", WriteMode::Direct, Some(&policy)).is_err());
        let locked = WritePolicy::default();
        assert!(check_write("1", "", immediate, Some(&locked)).is_err());
    }

    #[test]
    fn a_rest_write_sends_value_policy_and_a_reason_only_when_given() {
        let body = write_request(42.0, "  ", WriteMode::Queued).unwrap();
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({"value": 42.0, "policy": "queued"})
        );
        let body = write_request(1.0, " setpoint change ", WriteMode::Immediate).unwrap();
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({"value": 1.0, "policy": "immediate", "reason": "setpoint change"})
        );
        assert_eq!(write_request(1.0, "", WriteMode::Direct), None);
    }

    #[test]
    fn the_write_answer_carries_the_command_id() {
        // `api/commands.rs::write_tag`'s 202 body.
        let body = r#"{"command_id": "k3v9x0q2m7c1b8n4z6w5r", "status": "pending",
            "data_source_id": 12, "value": 42.5, "created_at": 1761390000}"#;
        let answer: WriteAccepted = serde_json::from_str(body).unwrap();
        assert_eq!(answer.command_id, "k3v9x0q2m7c1b8n4z6w5r");
        assert_eq!(
            (answer.status.as_str(), answer.data_source_id),
            ("pending", 12)
        );
    }

    #[test]
    fn the_policy_editor_sends_what_changed_and_clears_with_null() {
        let known = WritePolicy {
            writable: true,
            write_min: Some(0.0),
            write_max: Some(100.0),
            write_max_rate: None,
            write_requires_reason: false,
            write_deadband: Some(0.5),
        };
        // PATCH semantics on the server: null clears, a value sets, an
        // absent field stays.
        let patch =
            policy_patch(Some(&known), Some(true), Some(false), "0", "", "6", "0.5").unwrap();
        assert_eq!(
            serde_json::to_value(&patch).unwrap(),
            serde_json::json!({"write_max": null, "write_max_rate": 6})
        );
        assert_eq!(
            patch.applied(Some(&known)),
            Some(WritePolicy {
                write_max: None,
                write_max_rate: Some(6),
                ..known
            })
        );
        assert!(
            policy_patch(Some(&known), Some(true), Some(false), "0", "100", "", "0.5")
                .unwrap()
                .is_empty()
        );
        assert!(policy_patch(None, None, None, "10", "5", "", "").is_err());
        assert!(policy_patch(None, None, None, "x", "", "", "").is_err());
        assert!(policy_patch(None, None, None, "", "", "1.5", "").is_err());
        assert!(policy_patch(None, None, None, "", "", "-1", "").is_err());
        assert!(policy_patch(None, None, None, "", "", "", "-0.5").is_err());
        // The server's own WritePolicy round-trips.
        let stored: WritePolicy = serde_json::from_str(
            r#"{"writable": true, "write_min": null, "write_max": 80.0,
                "write_max_rate": null, "write_requires_reason": true, "write_deadband": 0.5}"#,
        )
        .unwrap();
        assert_eq!(stored.write_max, Some(80.0));
        assert!(stored.write_requires_reason);
    }

    /// With no policy known, the editor's empty fields and untouched
    /// choices must not overwrite the stored policy with defaults.
    #[test]
    fn an_unknown_policy_is_patched_only_where_the_operator_typed() {
        let patch = policy_patch(None, None, None, "", "80", "", "").unwrap();
        assert_eq!(
            serde_json::to_value(&patch).unwrap(),
            serde_json::json!({"write_max": 80.0})
        );
        // The rest stays unknown, so the console still knows no policy.
        assert_eq!(patch.applied(None), None);
        let locked = policy_patch(None, Some(false), None, "", "", "", "").unwrap();
        assert_eq!(
            serde_json::to_value(&locked).unwrap(),
            serde_json::json!({"writable": false})
        );
        assert!(
            policy_patch(None, None, None, "", "", "", "")
                .unwrap()
                .is_empty()
        );
        let all = policy_patch(None, Some(true), Some(true), "1", "2", "3", "0").unwrap();
        assert_eq!(
            all.applied(None),
            Some(WritePolicy {
                writable: true,
                write_min: Some(1.0),
                write_max: Some(2.0),
                write_max_rate: Some(3),
                write_requires_reason: true,
                write_deadband: Some(0.0),
            })
        );
    }

    /// `nan` and `inf` parse as floats; typed as a limit they are an input
    /// error, not a silent clear.
    #[test]
    fn a_non_finite_limit_is_refused() {
        let known = WritePolicy {
            writable: true,
            write_min: Some(0.0),
            write_max: Some(100.0),
            ..WritePolicy::default()
        };
        for text in ["nan", "NaN", "inf", "-inf", "infinity"] {
            assert!(
                policy_patch(Some(&known), Some(true), Some(false), text, "100", "", "").is_err(),
                "{text} as the minimum"
            );
            assert!(policy_patch(None, None, None, "", text, "", "").is_err());
            assert!(policy_patch(None, None, None, "", "", "", text).is_err());
        }
    }

    fn sample_tree() -> Vec<TreeNode> {
        // `GET /api/v1/tags/tree` (storage/device_crud.rs::build_tag_tree).
        let body = r#"{"tree": [
            {"name": "Plant", "tagCount": 0, "tagIds": [], "children": [
                {"name": "Inverters", "tagCount": 2, "tagIds": [1, 2], "children": [
                    {"name": "INV1", "tagCount": 1, "tagIds": [3], "children": []}
                ]}
            ]},
            {"name": "grid-meter", "tagCount": 1, "tagIds": [9], "children": []}
        ]}"#;
        serde_json::from_str::<TagTree>(body).unwrap().tree
    }

    #[test]
    fn the_tree_decodes_and_lists_open_nodes_with_their_tags() {
        let tree = sample_tree();
        assert_eq!(tree[0].total(), 3);
        let rows = tree_rows(&tree, &HashSet::new());
        assert_eq!(rows.len(), 8);
        assert!(
            matches!(&rows[1], TreeRow::Node { path, depth: 1, total: 3, .. } if path == "Plant/Inverters")
        );
        assert!(matches!(
            &rows[2],
            TreeRow::Tag {
                id: 1,
                depth: 2,
                ..
            }
        ));
        assert!(matches!(&rows[4], TreeRow::Node { path, .. } if path == "Plant/Inverters/INV1"));
        let closed: HashSet<String> = ["Plant/Inverters".to_owned()].into();
        let rows = tree_rows(&tree, &closed);
        assert_eq!(rows.len(), 4, "a closed node hides its tags and children");
        assert!(matches!(&rows[1], TreeRow::Node { open: false, .. }));
        assert_eq!(
            find(&tree, "Plant/Inverters/INV1").map(|node| node.tag_ids.clone()),
            Some(vec![3])
        );
        assert_eq!(find(&tree, "Plant/Nowhere"), None);
    }

    #[test]
    fn moving_a_branch_keeps_each_tag_below_it() {
        let tree = sample_tree();
        let branch = find(&tree, "Plant/Inverters").unwrap();
        assert_eq!(
            moved_paths(branch, Some("Site/PV")),
            [
                (1, Some("Site/PV".to_owned())),
                (2, Some("Site/PV".to_owned())),
                (3, Some("Site/PV/INV1".to_owned()))
            ]
        );
        assert_eq!(
            moved_paths(branch, None),
            [(1, None), (2, None), (3, Some("INV1".to_owned()))]
        );
        assert_eq!(
            check_path(" Site / Area /Unit "),
            Ok(Some("Site/Area/Unit".into()))
        );
        assert_eq!(check_path("  "), Ok(None));
        assert!(check_path("Site//Unit").is_err());
        assert!(check_path("/Site").is_err());
    }

    #[test]
    fn the_naming_convention_decodes_and_sends_a_custom_pattern_only_for_custom() {
        // `api/naming_convention.rs::get_naming_convention`.
        let body = r#"{"template": "isa-5.1", "regex": "^[A-Za-z0-9_ .-]+$",
            "custom_regex": null, "templates": [
                {"id": "isa-5.1", "label": "ISA-5.1 (e.g., PT001_TEMP, TT102_ALARM)"},
                {"id": "custom", "label": "Custom (user-defined regex)"}]}"#;
        let naming: NamingConvention = serde_json::from_str(body).unwrap();
        assert_eq!(naming.template, "isa-5.1");
        assert_eq!(naming.templates.len(), 2);
        assert_eq!(
            serde_json::to_value(NamingUpdate::new("kks", "^x$")).unwrap(),
            serde_json::json!({"template": "kks"})
        );
        assert_eq!(
            serde_json::to_value(NamingUpdate::new("custom", " ^PT\\d+$ ")).unwrap(),
            serde_json::json!({"template": "custom", "custom_regex": "^PT\\d+$"})
        );
    }

    #[test]
    fn annotations_decode_and_are_asked_for_by_key() {
        // `api/annotations.rs::list_annotations`.
        let body = r#"{"annotations": [{"id": 5, "tag_id": "ac_current_l1",
            "annotation_text": "Breaker test @ops", "data_timestamp": 1761390000,
            "author_id": 1, "created_at": 1761390010, "updated_at": 1761390010}], "count": 1}"#;
        let list: Annotations = serde_json::from_str(body).unwrap();
        assert_eq!(list.annotations[0].data_timestamp, 1_761_390_000);
        assert_eq!(list.annotations[0].annotation_text, "Breaker test @ops");
        let created: Created = serde_json::from_str(r#"{"id": 6, "ok": true}"#).unwrap();
        assert_eq!(created.id, 6);
        assert_eq!(
            annotations_path("PLC 1/temp"),
            "/api/v1/annotations?tag_id=PLC%201%2Ftemp"
        );
    }

    #[test]
    fn history_asks_only_for_the_charted_tags() {
        let query = HistoryQuery::default();
        assert_eq!(history_path(&query, &[]), query.path());
        assert_eq!(
            history_path(&query, &["inv.p".to_owned(), "PLC 1/t".to_owned()]),
            "/api/v1/history?range=1h&agg=sampling&keyed_by=id&sensor=inv.p%2CPLC%201%2Ft"
        );
    }

    #[test]
    fn history_queries_name_range_detail_and_aggregation() {
        let query = HistoryQuery::default();
        assert_eq!(
            query.path(),
            "/api/v1/history?range=1h&agg=sampling&keyed_by=id"
        );
        let query = HistoryQuery {
            span: Span::Last("5m"),
            detail: Detail::Full,
            agg: Aggregation::MinMax,
        };
        assert_eq!(
            query.path(),
            "/api/v1/history?range=30m&detail=full&agg=minmax&keyed_by=id"
        );
        let custom = Span::Between {
            start_ms: 1_761_300_000_000,
            end_ms: 1_761_303_600_000,
        };
        assert_eq!(custom.range(), "1761300000000-1761303600000");
        assert!((custom.length_ms() - 3_600_000.0).abs() < f64::EPSILON);
        assert!(!custom.is_live() && Span::Last("6h").is_live());
        let answer: HistoryAnswer = serde_json::from_str(
            r#"{"sensors": [], "range": "1h", "interval": 10000,
                "data": {"42": {"values": [1.0, 2.0], "start": 1000, "interval": 10000, "count": 2}}}"#,
        )
        .unwrap();
        assert_eq!(
            chart_samples(&answer.data["42"]),
            [(1000.0, 1.0), (11000.0, 2.0)]
        );
    }

    #[test]
    fn local_times_round_trip_on_the_operator_clock() {
        let offset = UtcOffset::from_hms(7, 0, 0).unwrap();
        let at = parse_local("2026-10-25 14:05:09", offset).unwrap();
        assert_eq!(at, 1_792_911_909_000);
        assert_eq!(format_local(at, offset), "2026-10-25 14:05:09");
        assert_eq!(
            parse_local("2026-10-25T14:05", offset),
            Some(1_792_911_900_000)
        );
        assert_eq!(parse_local("2026-13-01 00:00", offset), None);
        assert_eq!(parse_local("yesterday", offset), None);
        assert!(parse_between("2026-10-25 14:00", "2026-10-25 13:00", offset).is_err());
        assert!(matches!(
            parse_between("2026-10-25 13:00", "2026-10-25 14:00", offset),
            Ok(Span::Between { .. })
        ));
    }

    #[test]
    fn live_samples_extend_the_fetched_history_and_stats_cover_the_window() {
        let fetched = [(1000.0, 1.0), (2000.0, 3.0)];
        let live = [
            (1500.0, 9.0),
            (2000.0, 9.0),
            (3000.0, f64::NAN),
            (4000.0, 5.0),
        ];
        let merged = merge(&fetched, live.into_iter());
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[3], (4000.0, 5.0));
        let stats = stats(&merged, 1500.0, 4000.0).unwrap();
        assert_eq!(stats.points, 3, "the broken sample counts as a point");
        assert_eq!((stats.min, stats.max), (3.0, 5.0));
        assert!((stats.mean - 4.0).abs() < 1e-9);
        assert_eq!(super::stats(&merged, 5000.0, 6000.0), None);
    }

    #[test]
    fn csv_lines_up_the_series_by_instant() {
        let power = [(1000.0, 1.5), (2000.0, 2.5), (9000.0, 0.0)];
        let current = [(2000.0, 10.0), (3000.0, f64::NAN)];
        let columns = [
            CsvColumn {
                key: "ac_power",
                samples: &power,
            },
            CsvColumn {
                key: "odd,key",
                samples: &current,
            },
        ];
        let (text, rows) = csv(&columns, 1000.0, 3000.0, UtcOffset::UTC);
        assert_eq!(rows, 3);
        assert_eq!(
            text,
            "unix_ms,time,ac_power,\"odd,key\"\n\
             1000,1970-01-01 00:00:01,1.5,\n\
             2000,1970-01-01 00:00:02,2.5,10\n\
             3000,1970-01-01 00:00:03,,\n"
        );
    }
}
