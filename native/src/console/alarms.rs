//! Alarm management, the model under the Alarms page (`views::alarms`):
//! which alarms a filter shows and in what order, the search, the summary
//! counts (the server's, or counted here for roles that cannot ask), the
//! shelve durations, the audit trail's details and alarm-flood detection.
//! Pure functions over the server's rows, so they are tested here.

use voltrus_scada_native::api::{Alarm, AlarmSummary};

use super::pages::Role;
use super::{is_shelved, severity_rank};

/// How long an alarm can be shelved for: the web console's choices.
pub const SHELVE_DURATIONS: [(u64, &str); 5] = [
    (900, "15 min"),
    (1800, "30 min"),
    (3600, "1 h"),
    (14_400, "4 h"),
    (28_800, "8 h"),
];

/// The duration the shelve dialog starts on (30 min, as on the web).
pub const DEFAULT_SHELVE: usize = 1;

/// How many alarms the History filter asks for (`status=all`).
pub const HISTORY_LIMIT: usize = 500;

/// How far back the Analytics view counts.
pub const ANALYTICS_SECS: i64 = 24 * 3600;

/// How many sources the Analytics view ranks.
pub const TOP: usize = 10;

/// The page's views. Analytics and audit read operator routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    List,
    Analytics,
    Audit,
}

impl View {
    pub const ALL: [Self; 3] = [Self::List, Self::Analytics, Self::Audit];

    pub fn title(self) -> &'static str {
        match self {
            Self::List => "Alarms",
            Self::Analytics => "Analytics",
            Self::Audit => "Audit trail",
        }
    }

    /// The least role that may open the view: the server serves analytics
    /// and the audit trail on its operator routes only.
    pub fn role(self) -> Role {
        match self {
            Self::List => Role::Any,
            Self::Analytics | Self::Audit => Role::Operator,
        }
    }

    /// The views `role` may open, in order.
    pub fn allowed(role: Option<Role>) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|view| role.is_some_and(|role| role >= view.role()))
            .collect()
    }
}

/// Which alarms the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    /// Unresolved and not shelved.
    #[default]
    Open,
    /// Open and nobody acknowledged them yet.
    Unacknowledged,
    /// Open and acknowledged.
    Acknowledged,
    Shelved,
    /// Everything the server kept, resolved too (`status=all`).
    History,
}

impl Filter {
    pub const ALL: [Self; 5] = [
        Self::Open,
        Self::Unacknowledged,
        Self::Acknowledged,
        Self::Shelved,
        Self::History,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Unacknowledged => "To acknowledge",
            Self::Acknowledged => "Acknowledged",
            Self::Shelved => "Shelved",
            Self::History => "History",
        }
    }

    /// The next filter (← and →), wrapping around.
    #[must_use]
    pub fn step(self, forward: bool) -> Self {
        let at = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        let len = Self::ALL.len();
        Self::ALL[if forward {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        }]
    }

    /// Whether the filter shows `alarm`. History shows every row of its
    /// own list; the others read the open alarms.
    pub fn admits(self, alarm: &Alarm) -> bool {
        let open = !is_resolved(alarm) && !is_shelved(alarm);
        match self {
            Self::Open => open,
            Self::Unacknowledged => open && alarm.acknowledged_at.is_none(),
            Self::Acknowledged => open && alarm.acknowledged_at.is_some(),
            Self::Shelved => !is_resolved(alarm) && is_shelved(alarm),
            Self::History => true,
        }
    }
}

/// The list's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// Worst priority first, then newest.
    #[default]
    Priority,
    Newest,
    /// By the source's name, then newest.
    Source,
}

impl Sort {
    pub const ALL: [Self; 3] = [Self::Priority, Self::Newest, Self::Source];

    pub fn label(self) -> &'static str {
        match self {
            Self::Priority => "Priority",
            Self::Newest => "Newest",
            Self::Source => "Source",
        }
    }

    #[must_use]
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }
}

/// Resolved: the value went back inside its limit.
pub fn is_resolved(alarm: &Alarm) -> bool {
    alarm.resolved_at.is_some() || alarm.state == "resolved"
}

/// The alarm's priority, its severity when the server sent none (the
/// server fills it with the severity by default).
pub fn priority(alarm: &Alarm) -> &str {
    if alarm.priority.is_empty() {
        &alarm.severity
    } else {
        &alarm.priority
    }
}

/// Whether `alarm` (whose source the console names `name`) matches the
/// search `query`, already lowercased: `#12` finds alarm 12; anything else
/// is looked for in the name, key, message, severity and priority.
pub fn matches(alarm: &Alarm, name: &str, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    if let Some(id) = query.strip_prefix('#') {
        return id.parse::<i64>().is_ok_and(|id| id == alarm.id);
    }
    [
        name,
        &alarm.source_key,
        &alarm.message,
        &alarm.severity,
        &alarm.priority,
    ]
    .iter()
    .any(|text| text.to_lowercase().contains(query))
}

/// Positions in `alarms` of the rows `filter` and `query` keep, in `sort`
/// order. `name` gives the name the console shows for an alarm's source.
pub fn arrange(
    alarms: &[Alarm],
    filter: Filter,
    sort: Sort,
    query: &str,
    name: impl Fn(&Alarm) -> String,
) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut rows: Vec<(usize, String)> = alarms
        .iter()
        .enumerate()
        .filter(|(_, alarm)| filter.admits(alarm))
        .map(|(index, alarm)| (index, name(alarm)))
        .filter(|(index, name)| matches(&alarms[*index], name, &query))
        .collect();
    rows.sort_by(|(a, a_name), (b, b_name)| {
        let (a, b) = (&alarms[*a], &alarms[*b]);
        let newest = b.triggered_at.cmp(&a.triggered_at).then(b.id.cmp(&a.id));
        match sort {
            Sort::Priority => severity_rank(priority(a))
                .cmp(&severity_rank(priority(b)))
                .then(newest),
            Sort::Newest => newest,
            Sort::Source => a_name
                .to_lowercase()
                .cmp(&b_name.to_lowercase())
                .then(newest),
        }
    });
    rows.into_iter().map(|(index, _)| index).collect()
}

/// The summary strip's figures.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Counts {
    /// Open, unacknowledged, by priority.
    pub critical: i64,
    pub warning: i64,
    pub info: i64,
    pub shelved: i64,
    /// Only the server knows these.
    pub recent_24h: Option<i64>,
    pub avg_ack_secs: Option<f64>,
    /// From `GET /alarms/summary` rather than counted here.
    pub from_server: bool,
}

impl Counts {
    pub fn from_server(summary: &AlarmSummary) -> Self {
        Self {
            critical: summary.active_critical,
            warning: summary.active_warning,
            info: summary.active_info,
            shelved: summary.shelved,
            recent_24h: Some(summary.recent_24h),
            avg_ack_secs: (summary.avg_ack_time_secs > 0.0).then_some(summary.avg_ack_time_secs),
            from_server: true,
        }
    }

    /// Counted from the open alarms, for roles the summary route refuses.
    pub fn local(alarms: &[Alarm]) -> Self {
        let mut counts = Self::default();
        for alarm in alarms.iter().filter(|alarm| !is_resolved(alarm)) {
            if is_shelved(alarm) {
                counts.shelved += 1;
            } else if alarm.acknowledged_at.is_none() {
                match severity_rank(priority(alarm)) {
                    0 => counts.critical += 1,
                    1 | 2 => counts.warning += 1,
                    _ => counts.info += 1,
                }
            }
        }
        counts
    }
}

/// What a shelve's audit entry carries: `{"reason":…,"until":…}`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShelveDetails {
    pub reason: Option<String>,
    pub until: Option<i64>,
}

/// The reason and end of a shelve in an audit entry's details, if they are
/// the JSON the server writes for one; other details are plain text.
pub fn shelve_details(details: &str) -> Option<ShelveDetails> {
    let value: serde_json::Value = serde_json::from_str(details.trim()).ok()?;
    let object = value.as_object()?;
    let reason = object
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .filter(|reason| !reason.is_empty())
        .map(str::to_owned);
    let until = object.get("until").and_then(serde_json::Value::as_i64);
    (reason.is_some() || until.is_some()).then_some(ShelveDetails { reason, until })
}

/// Other JSON details made readable: a raise's severity, value and limit
/// (`{"severity":…,"value":…,"threshold":…,"alarm_type":…}`), else each key
/// and value. `None` for plain text.
pub fn readable_details(details: &str) -> Option<String> {
    use serde_json::Value;
    let parsed: Value = serde_json::from_str(details.trim()).ok()?;
    let object = parsed.as_object()?;
    let number = |key: &str| object.get(key).and_then(Value::as_f64);
    if let (Some(value), Some(threshold)) = (number("value"), number("threshold")) {
        let (value, threshold) = (
            vt_ui::chart::format_value(value),
            vt_ui::chart::format_value(threshold),
        );
        // A raise names its severity and side; a resolution only the value.
        let Some(severity) = object.get("severity").and_then(Value::as_str) else {
            return Some(format!("Value {value}, limit {threshold}"));
        };
        let mut words = severity.chars();
        let severity: String = words
            .next()
            .map(|first| first.to_uppercase().chain(words).collect())
            .unwrap_or_default();
        let side = if object.get("alarm_type").and_then(Value::as_str) == Some("low") {
            "at or below"
        } else {
            "at or above"
        };
        return Some(format!("{severity} at {value}, limit {threshold} ({side})"));
    }
    let parts: Vec<String> = object
        .iter()
        .map(|(key, value)| match value {
            Value::String(text) => format!("{key} {text}"),
            other => format!("{key} {other}"),
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Alarms raised in the last five minutes when they make a flood (more
/// than ten, the web console's rule), counted over the history's rows
/// (`status=all`, [`HISTORY_LIMIT`] newest). The web client, the parity
/// source, instead sums the 24-hour trigger counts of the ranked sources
/// last raised in that window, which overstates the figure; this counts the
/// raises themselves.
pub fn flood(history: &[Alarm], now: i64) -> Option<usize> {
    let since = now - 300;
    let raised = history
        .iter()
        .filter(|alarm| alarm.triggered_at > since)
        .count();
    (raised > 10).then_some(raised)
}

/// `45 s`, `12 min`, `1 h 5 min`: how long acknowledging takes.
pub fn duration(seconds: f64) -> String {
    #[allow(clippy::cast_possible_truncation)]
    let seconds = seconds.round().max(0.0) as i64;
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min", seconds / 60),
        _ if seconds % 3600 / 60 == 0 => format!("{} h", seconds / 3600),
        _ => format!("{} h {} min", seconds / 3600, seconds % 3600 / 60),
    }
}

/// What `--alarms` opens once signed in (screenshots, kiosks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    History,
    Analytics,
    Audit,
    /// The worst alarm's detail sheet with its journal.
    Detail,
    /// The shelve dialog for the worst alarm.
    Shelve,
    /// The acknowledge-all confirmation.
    Confirm,
}

impl Launch {
    pub const NAMES: &'static str = "history|analytics|audit|detail|shelve|confirm";

    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "history" => Self::History,
            "analytics" => Self::Analytics,
            "audit" => Self::Audit,
            "detail" => Self::Detail,
            "shelve" => Self::Shelve,
            "confirm" => Self::Confirm,
            _ => return None,
        })
    }

    /// Opens on an alarm, so it waits for the alarm list.
    pub fn needs_alarm(self) -> bool {
        matches!(self, Self::Detail | Self::Shelve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alarm(id: i64, priority: &str, triggered_at: i64) -> Alarm {
        Alarm {
            id,
            source_key: format!("tag_{id}"),
            severity: priority.into(),
            priority: priority.into(),
            triggered_at,
            state: "active".into(),
            ..Alarm::default()
        }
    }

    fn plant() -> Vec<Alarm> {
        let far = super::super::time_now_secs() + 3600;
        vec![
            alarm(1, "warning", 100),
            Alarm {
                acknowledged_at: Some(150),
                ..alarm(2, "critical", 110)
            },
            alarm(3, "critical", 120),
            Alarm {
                shelved_until: Some(far),
                state: "shelved".into(),
                ..alarm(4, "critical", 130)
            },
            Alarm {
                resolved_at: Some(200),
                state: "resolved".into(),
                ..alarm(5, "info", 140)
            },
            alarm(6, "info", 90),
        ]
    }

    fn ids(alarms: &[Alarm], rows: &[usize]) -> Vec<i64> {
        rows.iter().map(|row| alarms[*row].id).collect()
    }

    #[test]
    fn filters_split_open_acknowledged_shelved_and_history() {
        let alarms = plant();
        let shown = |filter| {
            ids(
                &alarms,
                &arrange(&alarms, filter, Sort::Newest, "", |a| a.source_key.clone()),
            )
        };
        assert_eq!(shown(Filter::Open), [3, 2, 1, 6]);
        assert_eq!(shown(Filter::Unacknowledged), [3, 1, 6]);
        assert_eq!(shown(Filter::Acknowledged), [2]);
        assert_eq!(shown(Filter::Shelved), [4]);
        assert_eq!(shown(Filter::History), [5, 4, 3, 2, 1, 6]);
    }

    #[test]
    fn sorting_puts_the_worst_first_then_the_newest() {
        // An old critical alarm and a newer info one: priority and recency
        // disagree, so only a priority sort puts the critical first.
        let mut alarms = plant();
        alarms.push(alarm(7, "critical", 50));
        alarms.push(alarm(8, "info", 500));
        let order = |sort| {
            ids(
                &alarms,
                &arrange(&alarms, Filter::Open, sort, "", |a| a.source_key.clone()),
            )
        };
        assert_eq!(order(Sort::Priority), [3, 2, 7, 1, 8, 6]);
        assert_eq!(order(Sort::Newest), [8, 3, 2, 1, 6, 7]);
        // By name: the names here reverse the ids.
        let by_name = arrange(&alarms, Filter::Open, Sort::Source, "", |a| {
            format!("{}", 10 - a.id)
        });
        assert_eq!(ids(&alarms, &by_name), [8, 7, 6, 3, 2, 1]);
        assert_eq!(Sort::Source.next(), Sort::Priority);
    }

    #[test]
    fn search_finds_names_keys_messages_and_ids() {
        let alarms = vec![Alarm {
            message: "Inverter 3 over temperature".into(),
            ..alarm(12, "critical", 1)
        }];
        let found = |query: &str| {
            arrange(&alarms, Filter::History, Sort::Newest, query, |_| {
                "Inverter temp".into()
            })
            .len()
        };
        assert_eq!(found("INVERTER"), 1, "case-insensitive");
        assert_eq!(found("tag_12"), 1, "the key");
        assert_eq!(found("over temp"), 1, "the message");
        assert_eq!(found("crit"), 1, "the priority");
        assert_eq!(found("#12"), 1);
        assert_eq!(found("#1"), 0, "an id matches whole");
        assert_eq!(found("pump"), 0);
        assert_eq!(found("   "), 1, "blank finds all");
    }

    #[test]
    fn filters_cycle_both_ways() {
        assert_eq!(Filter::Open.step(true), Filter::Unacknowledged);
        assert_eq!(Filter::Open.step(false), Filter::History);
        assert_eq!(Filter::History.step(true), Filter::Open);
        let mut filter = Filter::Shelved;
        for _ in 0..Filter::ALL.len() {
            filter = filter.step(true);
        }
        assert_eq!(filter, Filter::Shelved);
    }

    #[test]
    fn the_local_summary_counts_like_the_server() {
        let counts = Counts::local(&plant());
        assert_eq!(
            (counts.critical, counts.warning, counts.info, counts.shelved),
            (1, 1, 1, 1),
            "acknowledged, shelved and resolved alarms are not counted by priority"
        );
        assert!(!counts.from_server && counts.recent_24h.is_none());
        let server = Counts::from_server(&AlarmSummary {
            active_critical: 4,
            active_warning: 2,
            active_info: 0,
            shelved: 1,
            recent_24h: 37,
            avg_ack_time_secs: 0.0,
        });
        assert_eq!(server.recent_24h, Some(37));
        assert_eq!(server.avg_ack_secs, None, "no acknowledgements yet");
        assert!(server.from_server);
    }

    #[test]
    fn analytics_and_audit_need_an_operator() {
        assert!(View::allowed(None).is_empty());
        assert_eq!(View::allowed(Some(Role::Any)), [View::List]);
        assert_eq!(View::allowed(Some(Role::Operator)), View::ALL);
        assert_eq!(View::allowed(Some(Role::Admin)), View::ALL);
    }

    #[test]
    fn shelve_details_read_the_servers_json() {
        assert_eq!(
            shelve_details(r#"{"reason":"Sensor swap","until":1761400000}"#),
            Some(ShelveDetails {
                reason: Some("Sensor swap".into()),
                until: Some(1_761_400_000),
            })
        );
        assert_eq!(
            shelve_details(r#"{"reason":"","until":5}"#),
            Some(ShelveDetails {
                reason: None,
                until: Some(5),
            })
        );
        assert_eq!(shelve_details("acknowledged by admin"), None);
        assert_eq!(shelve_details("{}"), None);
    }

    #[test]
    fn raise_details_read_as_a_sentence() {
        // What the server audits when it raises an alarm.
        assert_eq!(
            readable_details(
                r#"{"severity":"warning","value":411.5263366699219,"threshold":400,"alarm_type":"high"}"#
            )
            .as_deref(),
            Some("Warning at 411.53, limit 400 (at or above)")
        );
        assert_eq!(
            readable_details(
                r#"{"severity":"critical","value":2,"threshold":5,"alarm_type":"low"}"#
            )
            .as_deref(),
            Some("Critical at 2, limit 5 (at or below)")
        );
        assert_eq!(
            readable_details(r#"{"value":4.843,"threshold":5}"#).as_deref(),
            Some("Value 4.843, limit 5"),
            "a resolution has no side"
        );
        assert_eq!(
            readable_details(r#"{"by":"admin","count":3}"#).as_deref(),
            Some("by admin · count 3")
        );
        assert_eq!(readable_details("acknowledged from the web"), None);
        assert_eq!(readable_details("{}"), None);
    }

    #[test]
    fn a_flood_counts_the_alarms_raised_in_the_last_five_minutes() {
        let now = 10_000;
        // A source that alarmed all day: 40 raises earlier in the 24 hours
        // (its trigger count) and `recent` of them in the window.
        let day = |recent: i64| -> Vec<Alarm> {
            (0..recent)
                .map(|i| alarm(i, "warning", now - 10 - i))
                .chain((0..40).map(|i| alarm(100 + i, "warning", now - 300 - i * 600)))
                .collect()
        };
        assert_eq!(flood(&day(12), now), Some(12), "only the window counts");
        assert_eq!(flood(&day(10), now), None, "ten is not a flood");
        assert_eq!(flood(&day(0), now), None, "busy all day, quiet now");
    }

    #[test]
    fn durations_and_launch_names_read_well() {
        assert_eq!(duration(42.4), "42 s");
        assert_eq!(duration(125.0), "2 min");
        assert_eq!(duration(7200.0), "2 h");
        assert_eq!(duration(3900.0), "1 h 5 min");
        assert_eq!(SHELVE_DURATIONS[DEFAULT_SHELVE].0, 1800);
        for name in Launch::NAMES.split('|') {
            assert!(Launch::parse(name).is_some(), "{name}");
        }
        assert_eq!(Launch::parse("everything"), None);
        assert!(Launch::Shelve.needs_alarm() && !Launch::Audit.needs_alarm());
    }
}
