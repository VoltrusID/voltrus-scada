//! Live end-to-end test against a running voltrus server.
//! Opt-in: only runs when VOLTRUS_E2E_URL / VOLTRUS_E2E_PASSWORD are set.
//!
//! It acknowledges one open alarm and then all of them, makes a generic
//! REST call through the runtime, changes the user's password and changes
//! it back (the user ends with the password it started with), and checks
//! that signing out revokes the session's refresh token.
//!
//!   VOLTRUS_E2E_URL=http://localhost:3100 \
//!   VOLTRUS_E2E_USERNAME=admin \
//!   VOLTRUS_E2E_PASSWORD=... \
//!   cargo test -p voltrus-scada-native --test e2e_live -- --nocapture

use std::time::Duration;

use voltrus_scada_native::api::{Alarm, REFRESH_COOKIE, Version, VoltrusApi};
use voltrus_scada_native::runtime::{self, Call, CallKey, Command, Event, Link};

/// `POST /api/v1/auth/refresh` with a refresh token: its status.
async fn refresh_status(base: &str, refresh: &str) -> reqwest::StatusCode {
    reqwest::Client::new()
        .post(format!("{base}/api/v1/auth/refresh"))
        .header("Cookie", format!("{REFRESH_COOKIE}={refresh}"))
        .send()
        .await
        .expect("the server answers refresh")
        .status()
}

/// Sign-out revokes the refresh token. A fresh session's token refreshes
/// (the control); a fresh session that signs out cannot. Each sign-in
/// revokes the user's older refresh tokens, so the two run one after the
/// other and nothing signs in between the sign-out and its check.
async fn logout_revokes_the_refresh_token(url: &str, user: &str, pass: &str) {
    let mut kept = VoltrusApi::new(url);
    kept.login(user, pass).await.expect("sign in");
    let kept_refresh = kept.refresh.clone().expect("a refresh cookie");
    assert!(
        refresh_status(url, &kept_refresh).await.is_success(),
        "a live session's refresh token works"
    );
    let mut left = VoltrusApi::new(url);
    left.login(user, pass).await.expect("sign in");
    let left_refresh = left.refresh.clone().expect("a refresh cookie");
    left.logout().await.expect("sign out");
    assert_eq!(
        refresh_status(url, &left_refresh).await,
        reqwest::StatusCode::UNAUTHORIZED,
        "a signed-out session's refresh token is revoked"
    );
    println!("sign-out revoked the refresh token");
}

async fn next(events: &async_channel::Receiver<Event>) -> Option<Event> {
    tokio::time::timeout(Duration::from_millis(500), events.recv())
        .await
        .ok()
        .and_then(Result::ok)
}

/// The first event `pick` takes within `secs` seconds.
async fn wait_for<T>(
    events: &async_channel::Receiver<Event>,
    secs: u64,
    mut pick: impl FnMut(Event) -> Option<T>,
) -> Option<T> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while tokio::time::Instant::now() < deadline {
        if let Some(found) = next(events).await.and_then(&mut pick) {
            return Some(found);
        }
    }
    None
}

#[tokio::test]
async fn live_login_sensors_ws_stream() {
    let Ok(url) = std::env::var("VOLTRUS_E2E_URL") else {
        eprintln!("skipping: VOLTRUS_E2E_URL not set");
        return;
    };
    let user = std::env::var("VOLTRUS_E2E_USERNAME").unwrap_or_else(|_| "admin".into());
    let pass = std::env::var("VOLTRUS_E2E_PASSWORD").expect("VOLTRUS_E2E_PASSWORD");

    let (handle, events) = runtime::start().expect("runtime");

    // A wrong password must come back as a failed sign-in, not silence.
    handle.send(Command::SignIn {
        url: url.clone(),
        username: "no-such-user-probe".into(),
        password: "wrong".into(),
    });
    let mut refused = false;
    for _ in 0..30 {
        if let Some(Event::SignInFailed(_)) = next(&events).await {
            refused = true;
            break;
        }
    }
    assert!(refused, "a bad sign-in produced no event");

    handle.send(Command::SignIn {
        url: url.clone(),
        username: user.clone(),
        password: pass.clone(),
    });
    let mut signed_in = false;
    let mut tags: Option<Vec<String>> = None;
    let mut live = false;
    let mut updates = 0usize;
    let mut history = false;
    let mut alarms: Option<Vec<Alarm>> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline
        && !(signed_in && live && tags.is_some() && history && alarms.is_some() && updates > 0)
    {
        match next(&events).await {
            Some(Event::SignedIn { .. }) => signed_in = true,
            Some(Event::SignInFailed(error)) => panic!("sign-in failed: {error}"),
            Some(Event::Tags(list)) => {
                let keys: Vec<String> = list.iter().map(|tag| tag.key.clone()).collect();
                handle.send(Command::Subscribe(keys.clone()));
                handle.send(Command::History("30m".into()));
                tags = Some(keys);
            }
            Some(Event::Link(Link::Live)) => live = true,
            Some(Event::Updates(batch)) => updates += batch.len(),
            Some(Event::History { .. }) => history = true,
            Some(Event::Alarms(list)) => alarms = Some(list),
            _ => {}
        }
    }

    assert!(signed_in, "expected a successful sign-in");
    let tags = tags.expect("expected the tag list");
    assert!(!tags.is_empty(), "expected at least one tag");
    assert!(live, "expected the WebSocket to go live");
    assert!(history, "expected history for the tags");
    let alarms = alarms.expect("expected the open alarms");
    println!(
        "tags: {}, updates: {updates}, open alarms: {}",
        tags.len(),
        alarms.len()
    );
    if std::env::var_os("VOLTRUS_E2E_EXPECT_UPDATES").is_some() {
        assert!(updates > 0, "expected live tag updates from the simulators");
    } else if updates == 0 {
        eprintln!("note: no live tag updates; are protocol simulators running?");
    }

    // Acknowledge one open alarm; a later alarm list shows it acknowledged
    // (or resolved and gone).
    if let Some(id) = alarms
        .iter()
        .find(|alarm| alarm.acknowledged_at.is_none())
        .map(|alarm| alarm.id)
    {
        handle.send(Command::Acknowledge(id));
        let action = format!("Acknowledge alarm {id}");
        let mut done = false;
        let acknowledged = wait_for(&events, 30, |event| match event {
            Event::Done {
                action: finished,
                result,
            } if finished == action => {
                if let Err(error) = result {
                    panic!("acknowledging alarm {id} failed: {error}");
                }
                done = true;
                None
            }
            Event::Alarms(list) if done => list
                .iter()
                .find(|alarm| alarm.id == id)
                .is_none_or(|alarm| alarm.acknowledged_at.is_some())
                .then_some(()),
            _ => None,
        })
        .await;
        assert!(
            acknowledged.is_some(),
            "alarm {id} never showed as acknowledged"
        );
        println!("acknowledged alarm {id}");
    } else {
        eprintln!("note: no unacknowledged alarm to acknowledge");
    }

    // Acknowledge all: `POST /api/v1/alarms` (the old native path got 405).
    handle.send(Command::AcknowledgeAll);
    let acknowledged_all = wait_for(&events, 20, |event| match event {
        Event::Done { action, result } if action == "Acknowledge all alarms" => Some(result),
        _ => None,
    })
    .await
    .expect("no answer to acknowledge all");
    if let Err(error) = acknowledged_all {
        panic!("acknowledging all alarms failed: {error}");
    }
    println!("acknowledged all alarms");

    // A generic REST call answers under its key, decoded off the UI thread.
    let key = CallKey::new("shell.version");
    handle.send(Command::Call {
        key,
        call: Call::get::<Version>("/api/v1/version"),
    });
    let version = wait_for(&events, 20, |event| match event {
        Event::Reply {
            key: answered,
            reply,
        } if answered == key => Some(reply.take::<Version>()),
        _ => None,
    })
    .await
    .expect("no reply to the version call");
    let version = version.unwrap_or_else(|error| panic!("the version call failed: {error}"));
    assert!(!version.version.is_empty(), "the server names its version");
    println!("server version {}", version.version);

    // Change the password (the runtime supplies the current one) and back.
    let temporary = format!("{pass}-e2e-temporary");
    for new in [temporary, pass.clone()] {
        handle.send(Command::ChangePassword {
            current: String::new(),
            new,
        });
        let changed = wait_for(&events, 20, |event| match event {
            Event::PasswordChanged(result) => Some(result),
            _ => None,
        })
        .await
        .expect("no answer to the password change");
        if let Err(error) = changed {
            panic!("changing the password failed: {error}");
        }
    }

    // The original password still signs in.
    handle.send(Command::SignOut);
    handle.send(Command::SignIn {
        url: url.clone(),
        username: user.clone(),
        password: pass.clone(),
    });
    let signed_in = wait_for(&events, 20, |event| match event {
        Event::SignedIn { .. } => Some(Ok(())),
        Event::SignInFailed(error) => Some(Err(error)),
        _ => None,
    })
    .await
    .expect("no answer to signing in again");
    if let Err(error) = signed_in {
        panic!("the original password no longer signs in: {error}");
    }
    println!("password changed and restored");
    handle.send(Command::Shutdown);

    logout_revokes_the_refresh_token(&url, &user, &pass).await;
    operations_pages(&url, &user, &pass).await;
}

/// The operations pages' REST paths as the console builds them: commands
/// (history, the active lists, the audit trail, cancel), the shift handover
/// (create, the filtered log, the automatic report), the audit log (pages,
/// filters, sort, CSV export) and the telemetry export (JSON to a file,
/// aggregated CSV). Runs after the session tests: each sign-in revokes the
/// user's older refresh tokens.
async fn operations_pages(url: &str, user: &str, pass: &str) {
    use serde::de::IgnoredAny;
    use serde_json::json;
    use voltrus_scada_native::api::Method;
    use voltrus_scada_native::api::ops::{
        self, AuditPage, AuditQuery, AuditSort, CommandAudit, Created, ExportAnswer, ExportQuery,
        Format, HandoverList, NewHandover, QueuedCommand, ShiftReport,
    };

    let mut api = VoltrusApi::new(url);
    api.login(user, pass)
        .await
        .expect("sign in for the operations pages");

    // Commands. A write scheduled an hour ahead stays pending, so it can
    // be cancelled. Writes need a writable tag on a known device: try the
    // tags in turn, putting each back as it was found (not writable).
    let tags = api.data_sources().await.expect("the tag list");
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let mut command = None;
    for tag in tags.iter().rev() {
        let policy = format!("/api/v1/data-sources/{}/write-policy", tag.id);
        api.send_json::<_, IgnoredAny>(Method::PATCH, &policy, &json!({"writable": true}))
            .await
            .expect("an operator sets a write policy");
        let written = api
            .send_json::<_, serde_json::Value>(
                Method::POST,
                &format!("/api/v1/tags/{}/write", tag.id),
                &json!({
                    "value": 1.0,
                    "policy": "scheduled",
                    "scheduled_at": now + 3600,
                    "reason": "native console e2e"
                }),
            )
            .await;
        api.send_json::<_, IgnoredAny>(Method::PATCH, &policy, &json!({"writable": false}))
            .await
            .expect("the write policy goes back");
        if let Ok(written) = written
            && let Some(id) = written["command_id"].as_str()
        {
            command = Some(id.to_owned());
            break;
        }
    }
    let history: Vec<QueuedCommand> = api
        .get_json(&ops::commands_path(None, 20, 0))
        .await
        .expect("the command history");
    assert!(history.len() <= 20, "a page holds at most its limit");
    assert!(
        history
            .windows(2)
            .all(|pair| pair[0].created_at >= pair[1].created_at),
        "newest first"
    );
    let active = ops::active_commands(api.clone())
        .await
        .expect("the active commands");
    assert!(active.iter().all(QueuedCommand::is_active));
    match command {
        Some(id) => {
            assert!(
                history
                    .iter()
                    .any(|command| command.id == id && command.status == "pending"),
                "the scheduled write is in the history, pending"
            );
            assert!(
                active.iter().any(|command| command.id == id),
                "and among the active"
            );
            api.send_empty::<IgnoredAny>(Method::POST, &ops::cancel_path(&id))
                .await
                .expect("an operator cancels a pending command");
            let cancelled: Vec<QueuedCommand> = api
                .get_json(&ops::commands_path(Some("cancelled"), 20, 0))
                .await
                .expect("the cancelled commands");
            assert!(
                cancelled.iter().any(|command| command.id == id),
                "the status filter finds the cancelled command"
            );
            assert!(
                cancelled
                    .iter()
                    .all(|command| command.status == "cancelled")
            );
            let audit: Vec<CommandAudit> = api
                .get_json(&ops::command_audit_path(&id))
                .await
                .expect("the command's audit trail");
            assert!(
                audit
                    .iter()
                    .any(|entry| entry.new_status == "cancelled" && entry.changed_by == user),
                "the trail records the cancel: {audit:?}"
            );
            let again = api
                .send_empty::<IgnoredAny>(Method::POST, &ops::cancel_path(&id))
                .await;
            assert!(again.is_err(), "a cancelled command does not cancel twice");
            println!("command {id}: written, listed, cancelled, audited");
        }
        None => {
            // No tag took a write; the routes still answer as the console
            // expects: an empty trail and a refused cancel.
            eprintln!("note: no tag accepted a scheduled write; checking the routes only");
            let audit: Vec<CommandAudit> = api
                .get_json(&ops::command_audit_path("no-such-command"))
                .await
                .expect("the audit trail of an unknown command");
            assert!(audit.is_empty());
            let error = api
                .send_empty::<IgnoredAny>(Method::POST, &ops::cancel_path("no-such-command"))
                .await
                .expect_err("an unknown command does not cancel");
            assert!(
                error.to_string().to_lowercase().contains("not found"),
                "the cancel route answered: {error}"
            );
        }
    }

    // Shift handover: an entry for tonight's shift, found by date and
    // shift and not under another shift; the automatic report.
    let date = ops::format_date(time::OffsetDateTime::now_utc().date());
    let happened = format!("Native console e2e handover, @{user} to check DC string B");
    let created: Created = api
        .send_json(
            Method::POST,
            ops::HANDOVER_PATH,
            &NewHandover {
                shift_date: date.clone(),
                shift_period: "night".into(),
                what_happened: happened.clone(),
                whats_pending: "Inverter 2 fan noise".into(),
                what_to_watch: String::new(),
            },
        )
        .await
        .expect("an operator writes a handover entry");
    assert!(created.ok && created.id > 0);
    let night: HandoverList = api
        .get_json(&ops::handovers_path(Some(&date), Some("night")))
        .await
        .expect("the handover log for a date and shift");
    assert_eq!(night.count, night.handovers.len());
    let entry = night
        .handovers
        .iter()
        .find(|entry| entry.id == created.id)
        .expect("the new entry is in its shift's log");
    assert_eq!(entry.what_happened, happened);
    assert_eq!(ops::mention_spans(&entry.what_happened).len(), 1);
    let morning: HandoverList = api
        .get_json(&ops::handovers_path(Some(&date), Some("morning")))
        .await
        .expect("another shift's log");
    assert!(
        morning.handovers.iter().all(|entry| entry.id != created.id),
        "the shift filter holds"
    );
    let report: ShiftReport = api
        .get_json(ops::SHIFT_REPORT_PATH)
        .await
        .expect("the automatic shift report");
    assert!(ops::SHIFT_PERIODS.contains(&report.shift_period.as_str()));
    assert!(ops::parse_date(&report.shift_date).is_some());
    println!(
        "shift report {} {}: {} alarms, {} security events",
        report.shift_date,
        report.shift_period,
        report.alarms.len(),
        report.security_events.len()
    );

    // Audit log: the first page newest first, an action filter, a sort by
    // user, and the CSV export.
    let first: AuditPage = api
        .get_json(&AuditQuery::default().path())
        .await
        .expect("the audit log");
    assert_eq!((first.page, first.limit), (1, ops::AUDIT_PAGE));
    assert!(first.total >= i64::try_from(first.entries.len()).unwrap());
    assert!(
        first
            .entries
            .windows(2)
            .all(|pair| pair[0].created_at >= pair[1].created_at),
        "newest first"
    );
    if let Some(newest) = first.entries.first() {
        let filtered: AuditPage = api
            .get_json(
                &AuditQuery {
                    action: newest.action.clone(),
                    ..AuditQuery::default()
                }
                .path(),
            )
            .await
            .expect("the audit log by action");
        assert!(filtered.total >= 1);
        assert!(
            filtered
                .entries
                .iter()
                .all(|entry| entry.action == newest.action)
        );
    }
    let by_user: AuditPage = api
        .get_json(
            &AuditQuery {
                sort: AuditSort::User,
                descending: false,
                ..AuditQuery::default()
            }
            .path(),
        )
        .await
        .expect("the audit log by user");
    assert!(
        by_user
            .entries
            .windows(2)
            .all(|pair| pair[0].username <= pair[1].username),
        "sorted by user"
    );
    let csv = api
        .get_text(&AuditQuery::default().export_path())
        .await
        .expect("the audit CSV");
    assert!(csv.starts_with("id,action,username,resource,details,created_at"));
    println!(
        "audit log: {} entries, {} in the CSV",
        first.total,
        Format::Csv.count(&csv).0
    );

    // Telemetry export: JSON saved to a file, then aggregated CSV.
    let file = std::env::temp_dir().join(format!(
        "voltrus-native-e2e-export-{}.json",
        std::process::id()
    ));
    let saved = api
        .download(
            &ExportQuery {
                limit: 5,
                ..ExportQuery::default()
            }
            .path(),
            &file,
            Format::Json,
        )
        .await
        .expect("the JSON export, saved");
    let body: ExportAnswer =
        serde_json::from_str(&std::fs::read_to_string(&file).expect("the saved file"))
            .expect("the saved JSON reads back");
    assert_eq!(body.data.len(), saved.rows);
    assert!(saved.rows <= 5);
    assert_eq!((body.meta.limit, saved.total), (5, Some(body.meta.total)));
    let _ = std::fs::remove_file(&file);
    let csv = api
        .get_text(
            &ExportQuery {
                aggregation: Some("avg"),
                interval: "1h",
                format: Format::Csv,
                limit: 5,
                ..ExportQuery::default()
            }
            .path(),
        )
        .await
        .expect("the aggregated CSV export");
    assert!(csv.starts_with("timestamp,device,metric,value"));
    println!(
        "data export: {} of {} rows as JSON, {} hourly averages as CSV",
        saved.rows,
        body.meta.total,
        Format::Csv.count(&csv).0
    );
}
