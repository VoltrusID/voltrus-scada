//! The Scripts section's rules, apart from drawing: the editor's draft of
//! a script and its validation, the bodies sent to the server, the test
//! input, the list's search and sort, and the labels of an execution.

use std::cmp::Ordering;

use voltrus_scada_native::api::eng_scripts::{
    CreateScript, MIB, Script, ScriptExecution, UpdateScript,
};

/// A new script's source (the web client's).
pub(crate) const NEW_SOURCE: &str =
    "// Write your script here\nfunction main(input) {\n  return input;\n}\n";
/// A new script's limits (the web client's form defaults).
pub(crate) const NEW_CPU_MS: i64 = 5000;
pub(crate) const NEW_MEMORY_MB: i64 = 32;
/// The test panel's first input (the web client's).
pub(crate) const SAMPLE_INPUT: &str = "{\n  \"value\": 42\n}";
/// The web client's floor for the CPU limit.
const MIN_CPU_MS: i64 = 100;

/// What the editor holds for a script: its fields as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Draft {
    pub name: String,
    pub source: String,
    pub cpu_ms: String,
    pub memory_mb: String,
    pub enabled: bool,
}

impl Draft {
    /// The form for a new script.
    pub(crate) fn new_script() -> Self {
        Self {
            name: String::new(),
            source: NEW_SOURCE.to_owned(),
            cpu_ms: NEW_CPU_MS.to_string(),
            memory_mb: NEW_MEMORY_MB.to_string(),
            enabled: true,
        }
    }

    /// `script` as the server has it.
    pub(crate) fn of(script: &Script) -> Self {
        Self {
            name: script.name.clone(),
            source: unix_lines(&script.source),
            cpu_ms: script.cpu_limit_ms.to_string(),
            memory_mb: memory_mb(script.memory_limit_bytes).to_string(),
            enabled: script.enabled,
        }
    }
}

/// `text` with `\n` line breaks, as the code editor holds it.
pub(crate) fn unix_lines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// A memory limit in whole MiB, rounded as the web client shows it.
pub(crate) fn memory_mb(bytes: i64) -> i64 {
    (bytes + MIB / 2).div_euclid(MIB)
}

/// The limits a draft asks for. The memory limit is kept to the byte when
/// its MiB figure was not changed (the server may hold a figure that is no
/// whole MiB).
fn limits(draft: &Draft, original: Option<&Script>) -> Result<(i64, i64), String> {
    let cpu = draft
        .cpu_ms
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|cpu| *cpu >= MIN_CPU_MS)
        .ok_or_else(|| {
            format!("CPU limit: a whole number of milliseconds, at least {MIN_CPU_MS}")
        })?;
    let mb = draft
        .memory_mb
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|mb| *mb >= 1)
        .ok_or_else(|| "Memory limit: a whole number of MB, at least 1".to_owned())?;
    let bytes = match original {
        Some(script) if memory_mb(script.memory_limit_bytes) == mb => script.memory_limit_bytes,
        _ => mb
            .checked_mul(MIB)
            .ok_or_else(|| "Memory limit: too large".to_owned())?,
    };
    Ok((cpu, bytes))
}

/// `POST /api/v1/scripts` for a new script.
pub(crate) fn create_body(draft: &Draft) -> Result<CreateScript, String> {
    let name = draft.name.trim();
    if name.is_empty() {
        return Err("Script name is required".into());
    }
    let (cpu_limit_ms, memory_limit_bytes) = limits(draft, None)?;
    Ok(CreateScript {
        name: name.to_owned(),
        source: draft.source.clone(),
        cpu_limit_ms,
        memory_limit_bytes,
    })
}

/// `PUT /api/v1/scripts/{id}` with what changed from `script`; `None` when
/// nothing did.
pub(crate) fn update_body(draft: &Draft, script: &Script) -> Result<Option<UpdateScript>, String> {
    let name = draft.name.trim();
    if name.is_empty() {
        return Err("Script name is required".into());
    }
    let (cpu, memory) = limits(draft, Some(script))?;
    let body = UpdateScript {
        name: (name != script.name).then(|| name.to_owned()),
        source: (draft.source != unix_lines(&script.source)).then(|| draft.source.clone()),
        cpu_limit_ms: (cpu != script.cpu_limit_ms).then_some(cpu),
        memory_limit_bytes: (memory != script.memory_limit_bytes).then_some(memory),
        enabled: (draft.enabled != script.enabled).then_some(draft.enabled),
    };
    Ok((body != UpdateScript::default()).then_some(body))
}

/// Whether the draft holds edits the server does not have.
pub(crate) fn is_dirty(draft: &Draft, script: Option<&Script>) -> bool {
    match script {
        Some(script) => *draft != Draft::of(script),
        None => *draft != Draft::new_script(),
    }
}

/// The test panel's input, as the JSON the server passes the script.
pub(crate) fn parse_input(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|error| format!("Test input must be valid JSON: {error}"))
}

/// The list's columns that sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SortKey {
    /// The server's own order.
    #[default]
    Name,
    /// Enabled first, then by name.
    State,
    /// Most recently saved first.
    Updated,
}

impl SortKey {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::State => "state",
            Self::Updated => "updated",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::Name => Self::State,
            Self::State => Self::Updated,
            Self::Updated => Self::Name,
        }
    }
}

/// Whether `script` matches the search: its name, or its id.
pub(crate) fn matches(script: &Script, query: &str) -> bool {
    let query = query.trim();
    query.is_empty()
        || script.name.to_lowercase().contains(&query.to_lowercase())
        || script.id.to_string() == query.trim_start_matches('#')
}

/// The rows the list shows: positions in `scripts`, searched and sorted.
pub(crate) fn rows(scripts: &[Script], query: &str, sort: SortKey, descending: bool) -> Vec<usize> {
    let mut rows: Vec<usize> = (0..scripts.len())
        .filter(|index| matches(&scripts[*index], query))
        .collect();
    let by_name = |a: &Script, b: &Script| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    };
    rows.sort_by(|a, b| {
        let (a, b) = (&scripts[*a], &scripts[*b]);
        let order = match sort {
            SortKey::Name => by_name(a, b),
            SortKey::State => b.enabled.cmp(&a.enabled).then_with(|| by_name(a, b)),
            SortKey::Updated => b.updated_at.cmp(&a.updated_at).then_with(|| by_name(a, b)),
        };
        if descending { order.reverse() } else { order }
    });
    rows
}

/// How an execution's status reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Success,
    Timeout,
    Failed,
}

pub(crate) fn outcome(status: &str) -> Outcome {
    match status {
        "success" => Outcome::Success,
        "timeout" => Outcome::Timeout,
        _ => Outcome::Failed,
    }
}

/// `4 ms`, `1.25 s`.
pub(crate) fn duration_label(ms: i64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else {
        #[allow(clippy::cast_precision_loss)]
        let seconds = ms as f64 / 1000.0;
        format!("{seconds:.2} s")
    }
}

/// The execution to open after a fresh list: the newest, when one arrived
/// since `previous` and did not succeed (the web history expands it).
pub(crate) fn newest_failure(previous: &[ScriptExecution], now: &[ScriptExecution]) -> Option<i64> {
    let newest = now.first()?;
    let arrived = !previous.is_empty()
        && previous.first().is_none_or(|before| before.id != newest.id)
        && now.len() >= previous.len();
    (arrived && !newest.succeeded()).then_some(newest.id)
}

/// Whether `a` sorts before `b` in the server's order (by name), for
/// placing a script the server just created.
pub(crate) fn by_server_order(a: &Script, b: &Script) -> Ordering {
    a.name.cmp(&b.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(id: i64, name: &str) -> Script {
        Script {
            id,
            name: name.into(),
            source: "function main(input) { return input; }".into(),
            syntax_error: None,
            cpu_limit_ms: 1000,
            memory_limit_bytes: 1_048_576,
            enabled: true,
            created_at: 1_761_390_000,
            updated_at: 1_761_390_000 + id,
        }
    }

    fn execution(id: i64, status: &str) -> ScriptExecution {
        ScriptExecution {
            id,
            script_id: 1,
            flow_id: None,
            trigger_type: Some("test".into()),
            duration_ms: 3,
            status: status.into(),
            error_message: None,
            output: None,
            created_at: 1_761_390_000 + id,
        }
    }

    #[test]
    fn a_new_script_is_the_web_clients_and_needs_a_name() {
        let mut draft = Draft::new_script();
        assert!(!is_dirty(&draft, None));
        assert_eq!(create_body(&draft).unwrap_err(), "Script name is required");
        draft.name = "  Scale power ".into();
        assert!(is_dirty(&draft, None));
        let body = create_body(&draft).unwrap();
        assert_eq!(body.name, "Scale power");
        assert_eq!(body.source, NEW_SOURCE);
        assert_eq!(
            (body.cpu_limit_ms, body.memory_limit_bytes),
            (5000, 32 * MIB)
        );
    }

    #[test]
    fn limits_are_whole_numbers_within_the_floors() {
        let mut draft = Draft::new_script();
        draft.name = "x".into();
        draft.cpu_ms = "50".into();
        assert!(create_body(&draft).unwrap_err().starts_with("CPU limit"));
        draft.cpu_ms = "1.5".into();
        assert!(create_body(&draft).is_err());
        draft.cpu_ms = " 250 ".into();
        draft.memory_mb = "0".into();
        assert!(create_body(&draft).unwrap_err().starts_with("Memory limit"));
        draft.memory_mb = "4".into();
        let body = create_body(&draft).unwrap();
        assert_eq!((body.cpu_limit_ms, body.memory_limit_bytes), (250, 4 * MIB));
        draft.memory_mb = i64::MAX.to_string();
        assert_eq!(create_body(&draft).unwrap_err(), "Memory limit: too large");
    }

    #[test]
    fn an_update_sends_only_what_changed() {
        let saved = script(3, "Scale");
        let mut draft = Draft::of(&saved);
        assert!(!is_dirty(&draft, Some(&saved)));
        assert_eq!(update_body(&draft, &saved), Ok(None));

        draft.enabled = false;
        draft.source.push_str("\n// note");
        assert!(is_dirty(&draft, Some(&saved)));
        let body = update_body(&draft, &saved).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({
                "source": "function main(input) { return input; }\n// note",
                "enabled": false
            })
        );

        draft.name = " ".into();
        assert_eq!(
            update_body(&draft, &saved).unwrap_err(),
            "Script name is required"
        );
    }

    #[test]
    fn crlf_source_opens_clean() {
        let mut saved = script(3, "Scale");
        saved.source = "function main(input) {\r\n  return input;\r\n}\r".into();
        // What the code editor holds once it is opened.
        let mut draft = Draft::of(&saved);
        draft.source = "function main(input) {\n  return input;\n}\n".into();
        assert!(!is_dirty(&draft, Some(&saved)));
        assert_eq!(update_body(&draft, &saved), Ok(None));
    }

    #[test]
    fn a_memory_limit_off_the_mib_is_kept_unless_edited() {
        let mut saved = script(3, "Scale");
        saved.memory_limit_bytes = 1_500_000;
        let mut draft = Draft::of(&saved);
        assert_eq!(draft.memory_mb, "1", "shown rounded");
        draft.cpu_ms = "2000".into();
        let body = update_body(&draft, &saved).unwrap().unwrap();
        assert_eq!(body.memory_limit_bytes, None, "untouched: not sent");
        assert_eq!(body.cpu_limit_ms, Some(2000));
        draft.memory_mb = "2".into();
        let body = update_body(&draft, &saved).unwrap().unwrap();
        assert_eq!(body.memory_limit_bytes, Some(2 * MIB));
        assert_eq!(memory_mb(33_554_432), 32);
        assert_eq!(memory_mb(MIB / 2), 1, "half rounds up like Math.round");
    }

    #[test]
    fn the_test_input_must_be_json() {
        assert_eq!(
            parse_input(SAMPLE_INPUT).unwrap(),
            serde_json::json!({ "value": 42 })
        );
        assert_eq!(parse_input("7").unwrap(), serde_json::json!(7));
        assert!(
            parse_input("{ value: 1 }")
                .unwrap_err()
                .starts_with("Test input must be valid JSON")
        );
    }

    #[test]
    fn the_list_searches_and_sorts() {
        let mut scripts = vec![script(1, "beta"), script(2, "Alpha"), script(3, "gamma")];
        scripts[0].enabled = false;
        assert_eq!(rows(&scripts, "", SortKey::Name, false), [1, 0, 2]);
        assert_eq!(rows(&scripts, "", SortKey::Name, true), [2, 0, 1]);
        assert_eq!(rows(&scripts, "", SortKey::State, false), [1, 2, 0]);
        assert_eq!(rows(&scripts, "", SortKey::Updated, false), [2, 1, 0]);
        assert_eq!(rows(&scripts, "AL", SortKey::Name, false), [1]);
        assert_eq!(rows(&scripts, "#3", SortKey::Name, false), [2]);
        assert!(rows(&scripts, "zeta", SortKey::Name, false).is_empty());
        assert_eq!(SortKey::Updated.next(), SortKey::Name);
        assert_eq!(SortKey::default().label(), "name");
        assert_eq!(
            by_server_order(&scripts[1], &scripts[0]),
            Ordering::Less,
            "the server sorts by name as stored"
        );
    }

    #[test]
    fn executions_read_as_the_web_history_shows_them() {
        assert_eq!(outcome("success"), Outcome::Success);
        assert_eq!(outcome("timeout"), Outcome::Timeout);
        assert_eq!(outcome("error"), Outcome::Failed);
        assert_eq!(duration_label(4), "4 ms");
        assert_eq!(duration_label(1250), "1.25 s");

        let before = vec![execution(1, "success")];
        let failed = vec![execution(2, "error"), execution(1, "success")];
        assert_eq!(newest_failure(&before, &failed), Some(2));
        let fine = vec![execution(2, "success"), execution(1, "success")];
        assert_eq!(newest_failure(&before, &fine), None);
        assert_eq!(
            newest_failure(&[], &failed),
            None,
            "the first load opens nothing"
        );
        assert_eq!(newest_failure(&failed, &failed), None, "nothing new");
    }
}
