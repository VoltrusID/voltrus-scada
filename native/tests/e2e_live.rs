//! Live end-to-end test against a running voltrus server.
//! Opt-in: only runs when VOLTRUS_E2E_URL / VOLTRUS_E2E_PASSWORD are set.
//!
//! It acknowledges one open alarm and then all of them, makes a generic
//! REST call through the runtime, walks the alarm management paths
//! (summary, history, analytics, a shelve with a reason and its journal
//! and audit entries, the unshelve), changes the user's password and changes
//! it back (the user ends with the password it started with), and checks
//! that signing out revokes the session's refresh token.
//!
//!   VOLTRUS_E2E_URL=http://localhost:3100 \
//!   VOLTRUS_E2E_USERNAME=admin \
//!   VOLTRUS_E2E_PASSWORD=... \
//!   cargo test -p voltrus-scada-native --test e2e_live -- --nocapture

use std::time::Duration;

use voltrus_scada_native::api::{Alarm, REFRESH_COOKIE, Version, VoltrusApi};
use voltrus_scada_native::api::{
    AlarmAnalyticsEntry, AlarmAuditEntry, AlarmJournalEntry, AlarmSummary,
};
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

    alarm_management(&handle, &events, &alarms).await;

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

    data_manager_sources_round_trip(&url, &user, &pass).await;
    operations_pages(&url, &user, &pass).await;
}

/// [`call`], which must succeed.
async fn answer<T: Send + 'static>(
    handle: &runtime::Handle,
    events: &async_channel::Receiver<Event>,
    name: &'static str,
    request: Call,
) -> T {
    call(handle, events, CallKey::new(name), request)
        .await
        .unwrap_or_else(|error| panic!("{name} failed: {error}"))
}

/// `Command::RefreshTags`: the tag list as the server has it now.
async fn refreshed_tags(
    handle: &runtime::Handle,
    events: &async_channel::Receiver<Event>,
) -> Vec<voltrus_scada_native::api::DataSource> {
    handle.send(Command::RefreshTags);
    wait_for(events, 20, |event| match event {
        Event::Tags(list) => Some(list),
        _ => None,
    })
    .await
    .expect("a refreshed tag list")
}

/// Data Manager › Sources against the live plant, through the runtime as
/// the console does it: create a source (its settings sent as an object),
/// edit it, move it to the top, duplicate it, set and clear a per-tag
/// shortcut, list the Modbus devices and one device's registers, then
/// delete both sources and check the rest of the list kept its order.
async fn data_manager_sources_round_trip(url: &str, user: &str, pass: &str) {
    use serde::de::IgnoredAny;
    use serde_json::json;
    use voltrus_scada_native::api::{
        Created, Method, ModbusDevice, ModbusDeviceDetail, TagShortcuts,
    };

    let (handle, events) = runtime::start().expect("runtime");
    handle.send(Command::SignIn {
        url: url.to_owned(),
        username: user.to_owned(),
        password: pass.to_owned(),
    });
    let signed_in = wait_for(&events, 30, |event| match event {
        Event::SignInFailed(error) => panic!("sign-in failed: {error}"),
        Event::Tags(list) => Some(list),
        _ => None,
    })
    .await
    .expect("the tag list after signing in");
    assert!(!signed_in.is_empty(), "the plant has data sources");

    // Create: POST /api/v1/data-sources.
    let key = format!("e2e_native_source_{}", std::process::id());
    let created: Created = answer(
        &handle,
        &events,
        "data.sources.save",
        Call::send::<Created>(
            Method::POST,
            "/api/v1/data-sources",
            json!({
                "key": key, "name": "E2E native source", "unit": "V", "color": "#22c55e",
                "min_val": 0.0, "max_val": 10.0, "warn_val": 7.0, "crit_val": 9.0,
                "base_val": 5.0, "decimals": 2, "source_type": "sine",
                "source_config": { "amplitude": 3, "low_warn": 1 }, "chart_type": "gauge",
            }),
        ),
    )
    .await;
    let list = refreshed_tags(&handle, &events).await;
    let row = list
        .iter()
        .find(|source| source.id == created.id)
        .expect("the new source is listed");
    assert_eq!(row.key, key);
    let config: serde_json::Value =
        serde_json::from_str(&row.source_config).expect("the settings are JSON");
    assert_eq!(
        config["amplitude"], 3,
        "settings sent as an object are stored as one"
    );
    println!("created data source {} ({key})", created.id);

    // Edit: PUT /api/v1/data-sources/{id}, with the alarm delays.
    answer::<IgnoredAny>(
        &handle,
        &events,
        "data.sources.save",
        Call::send::<IgnoredAny>(
            Method::PUT,
            format!("/api/v1/data-sources/{}", created.id),
            json!({ "name": "E2E native source edited", "on_delay_secs": 4, "off_delay_secs": 2 }),
        ),
    )
    .await;
    let list = refreshed_tags(&handle, &events).await;
    let row = list
        .iter()
        .find(|source| source.id == created.id)
        .expect("the edited source is listed");
    assert_eq!(row.name, "E2E native source edited");
    assert_eq!((row.on_delay_secs, row.off_delay_secs), (4, 2));

    // Reorder: PATCH /api/v1/data-sources/reorder with every id.
    let mut order: Vec<i64> = list.iter().map(|source| source.id).collect();
    order.retain(|id| *id != created.id);
    order.insert(0, created.id);
    // The plant's sources, in the order the reorder gives them.
    let plant: Vec<i64> = order[1..].to_vec();
    answer::<IgnoredAny>(
        &handle,
        &events,
        "data.sources.reorder",
        Call::send::<IgnoredAny>(
            Method::PATCH,
            "/api/v1/data-sources/reorder",
            json!({ "order": order }),
        ),
    )
    .await;
    let list = refreshed_tags(&handle, &events).await;
    assert_eq!(list[0].id, created.id, "the moved source comes first");

    // Duplicate: the copy under <key>_copy, its delays in a follow-up PUT.
    let copy: Created = answer(
        &handle,
        &events,
        "data.sources.duplicate",
        Call::send::<Created>(
            Method::POST,
            "/api/v1/data-sources",
            json!({
                "key": format!("{key}_copy"), "name": "E2E native source edited copy",
                "unit": "V", "color": "#22c55e", "min_val": 0.0, "max_val": 10.0,
                "warn_val": 7.0, "crit_val": 9.0, "base_val": 5.0, "decimals": 2,
                "source_type": "sine", "source_config": config, "chart_type": "gauge",
            }),
        ),
    )
    .await;
    answer::<IgnoredAny>(
        &handle,
        &events,
        "data.sources.duplicate",
        Call::send::<IgnoredAny>(
            Method::PUT,
            format!("/api/v1/data-sources/{}", copy.id),
            json!({ "on_delay_secs": 4, "off_delay_secs": 2 }),
        ),
    )
    .await;
    let list = refreshed_tags(&handle, &events).await;
    let row = list
        .iter()
        .find(|source| source.id == copy.id)
        .expect("the copy is listed");
    assert_eq!(row.key, format!("{key}_copy"));
    assert_eq!((row.on_delay_secs, row.off_delay_secs), (4, 2));

    // Per-tag shortcut: PUT /api/v1/shortcuts replaces the user's map.
    let before: TagShortcuts = answer(
        &handle,
        &events,
        "data.sources.shortcuts",
        Call::get::<TagShortcuts>("/api/v1/shortcuts"),
    )
    .await;
    let kept: serde_json::Map<String, serde_json::Value> = before
        .by_id()
        .into_iter()
        .map(|(id, combo)| (id.to_string(), json!(combo)))
        .collect();
    let mut with = kept.clone();
    with.insert(created.id.to_string(), json!("Mod+Shift+9"));
    for (map, expected) in [(with, Some("Mod+Shift+9")), (kept, None)] {
        answer::<IgnoredAny>(
            &handle,
            &events,
            "data.sources.save",
            Call::send::<IgnoredAny>(
                Method::PUT,
                "/api/v1/shortcuts",
                serde_json::Value::Object(map),
            ),
        )
        .await;
        let now: TagShortcuts = answer(
            &handle,
            &events,
            "data.sources.shortcuts",
            Call::get::<TagShortcuts>("/api/v1/shortcuts"),
        )
        .await;
        assert_eq!(
            now.by_id().get(&created.id).map(String::as_str),
            expected,
            "the shortcut is set, then cleared"
        );
    }

    // The Modbus devices a source can read, and one device's registers.
    let devices: Vec<ModbusDevice> = answer(
        &handle,
        &events,
        "data.sources.devices",
        Call::get::<Vec<ModbusDevice>>("/api/v1/modbus-devices"),
    )
    .await;
    let device = devices.first().expect("the solar farm has Modbus devices");
    let detail: ModbusDeviceDetail = answer(
        &handle,
        &events,
        "data.sources.registers",
        Call::get::<ModbusDeviceDetail>(format!("/api/v1/modbus-devices/{}", device.id)),
    )
    .await;
    assert_eq!(detail.device.id, device.id);
    assert!(!detail.registers.is_empty(), "the device has registers");
    println!(
        "modbus devices: {}, {} has {} registers",
        devices.len(),
        device.id,
        detail.registers.len()
    );

    // Delete: DELETE /api/v1/data-sources/{id}, both of them.
    for id in [created.id, copy.id] {
        answer::<IgnoredAny>(
            &handle,
            &events,
            "data.sources.delete",
            Call::send_empty::<IgnoredAny>(Method::DELETE, format!("/api/v1/data-sources/{id}")),
        )
        .await;
    }
    let list = refreshed_tags(&handle, &events).await;
    let ids: Vec<i64> = list.iter().map(|source| source.id).collect();
    assert_eq!(ids, plant, "the plant's sources keep their order");
    println!("data sources created, edited, moved, duplicated and deleted");
    handle.send(Command::Shutdown);
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

/// The Tags slice's REST paths against the live plant: the tag tree and a
/// path move, the naming convention, a write policy and the REST write it
/// governs (refused without the reason, then accepted with a command id),
/// annotations, and the detail chart's history with a custom span, full
/// detail and an aggregation. Everything changed is put back.
#[tokio::test]
async fn live_tag_tree_policy_writes_notes_and_detail_history() {
    use reqwest::Method;
    use serde::de::IgnoredAny;
    use voltrus_scada_native::api::DataSource;
    use voltrus_scada_native::tag_api::{
        self, Aggregation, Annotations, Created, Detail, HistoryAnswer, HistoryQuery,
        NamingConvention, NamingUpdate, NewAnnotation, Span, TagTree, TreeNode, WriteAccepted,
        WriteMode, WritePolicy,
    };

    let Ok(url) = std::env::var("VOLTRUS_E2E_URL") else {
        eprintln!("skipping: VOLTRUS_E2E_URL not set");
        return;
    };
    let user = std::env::var("VOLTRUS_E2E_USERNAME").unwrap_or_else(|_| "admin".into());
    let pass = std::env::var("VOLTRUS_E2E_PASSWORD").expect("VOLTRUS_E2E_PASSWORD");
    let mut api = VoltrusApi::new(&url);
    api.login(&user, &pass).await.expect("sign in");

    // GET /tags/tree: every tag filed somewhere.
    let tags: Vec<DataSource> = api.data_sources().await.expect("the tag list");
    let tree: TagTree = api
        .get_json("/api/v1/tags/tree")
        .await
        .expect("the tag tree");
    let filed: usize = tree.tree.iter().map(TreeNode::total).sum();
    assert_eq!(filed, tags.len(), "the tree files every tag once");
    let tag = tags
        .iter()
        .find(|tag| tag.source_type == "modbus" && !tag.device_id.is_empty())
        .expect("a polled tag")
        .clone();

    // PUT /tags/{id}/path, found again by a tree search, then put back.
    let path = format!("/api/v1/tags/{}/path", tag.id);
    api.send_json::<_, IgnoredAny>(
        Method::PUT,
        &path,
        &serde_json::json!({"path": "E2E/Native console"}),
    )
    .await
    .expect("move the tag");
    let found: TagTree = api
        .get_json(&format!(
            "/api/v1/tags/tree?search={}",
            tag_api::query_value("Native console")
        ))
        .await
        .expect("search the tree");
    let node = tag_api::find(&found.tree, "E2E/Native console").expect("the moved tag's node");
    assert_eq!(node.tag_ids, [tag.id]);
    assert!(
        tag_api::check_path("E2E//x").is_err(),
        "the console refuses an empty segment"
    );
    let refused = api
        .send_json::<_, IgnoredAny>(Method::PUT, &path, &serde_json::json!({"path": "E2E//x"}))
        .await;
    assert!(refused.is_err(), "so does the server");
    api.send_json::<_, IgnoredAny>(Method::PUT, &path, &serde_json::json!({"path": tag.path}))
        .await
        .expect("put the path back");
    println!("moved tag {} and back", tag.id);

    // GET and PUT /settings/naming-convention: the same convention back.
    let naming: NamingConvention = api
        .get_json("/api/v1/settings/naming-convention")
        .await
        .expect("the naming convention");
    assert!(
        naming
            .templates
            .iter()
            .any(|template| template.id == naming.template)
    );
    let update = NamingUpdate::new(
        &naming.template,
        naming.custom_regex.as_deref().unwrap_or(""),
    );
    api.send_json::<_, IgnoredAny>(Method::PUT, "/api/v1/settings/naming-convention", &update)
        .await
        .expect("save the naming convention");
    let again: NamingConvention = api
        .get_json("/api/v1/settings/naming-convention")
        .await
        .expect("the naming convention again");
    assert_eq!(
        (again.template, again.regex),
        (naming.template, naming.regex)
    );

    // PATCH the write policy; the REST write it governs.
    let policy = WritePolicy {
        writable: true,
        write_min: Some(-1.0e9),
        write_max: Some(1.0e9),
        write_requires_reason: true,
        ..WritePolicy::default()
    };
    let policy_path = format!("/api/v1/data-sources/{}/write-policy", tag.id);
    api.send_json::<_, IgnoredAny>(Method::PATCH, &policy_path, &policy)
        .await
        .expect("save the write policy");
    let write_path = format!("/api/v1/tags/{}/write", tag.id);
    let value = tag.base_val;
    assert!(
        tag_api::check_write(&value.to_string(), "", WriteMode::Queued, Some(&policy)).is_err(),
        "the console asks for the reason"
    );
    let without = tag_api::write_request(value, "", WriteMode::Queued).expect("a REST write");
    let refused = api
        .send_json::<_, WriteAccepted>(Method::POST, &write_path, &without)
        .await;
    assert!(refused.is_err(), "the server wants the reason too");
    let out_of_range =
        tag_api::write_request(2.0e9, "native e2e", WriteMode::Immediate).expect("a REST write");
    assert!(
        api.send_json::<_, WriteAccepted>(Method::POST, &write_path, &out_of_range)
            .await
            .is_err(),
        "the server checks the maximum"
    );
    let with =
        tag_api::write_request(value, "native e2e", WriteMode::Queued).expect("a REST write");
    let accepted = api
        .send_json::<_, WriteAccepted>(Method::POST, &write_path, &with)
        .await
        .expect("the REST write");
    assert!(
        !accepted.command_id.is_empty(),
        "the write names its command"
    );
    assert_eq!(accepted.data_source_id, tag.id);
    println!(
        "queued command {} ({})",
        accepted.command_id, accepted.status
    );
    api.send_json::<_, IgnoredAny>(Method::PATCH, &policy_path, &WritePolicy::default())
        .await
        .expect("lock the tag again");

    // Annotations: add one, find it on the tag, delete it.
    #[allow(clippy::cast_possible_truncation)]
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs() as i64;
    let created: Created = api
        .send_json(
            Method::POST,
            "/api/v1/annotations",
            &NewAnnotation {
                tag_id: tag.key.clone(),
                annotation_text: "native e2e note".into(),
                data_timestamp: now_secs - 60,
            },
        )
        .await
        .expect("add a note");
    let notes: Annotations = api
        .get_json(&tag_api::annotations_path(&tag.key))
        .await
        .expect("the tag's notes");
    let note = notes
        .annotations
        .iter()
        .find(|note| note.id == created.id)
        .expect("the new note");
    assert_eq!(
        (note.annotation_text.as_str(), note.data_timestamp),
        ("native e2e note", now_secs - 60)
    );
    api.send_empty::<IgnoredAny>(
        Method::DELETE,
        &format!("/api/v1/annotations/{}", created.id),
    )
    .await
    .expect("delete the note");
    let notes: Annotations = api
        .get_json(&tag_api::annotations_path(&tag.key))
        .await
        .expect("the tag's notes again");
    assert!(notes.annotations.iter().all(|note| note.id != created.id));

    // The detail chart's history: the last 10 minutes as a custom span,
    // every second, averaged.
    let end_ms = now_secs * 1000;
    let query = HistoryQuery {
        span: Span::Between {
            start_ms: end_ms - 600_000,
            end_ms,
        },
        detail: Detail::Full,
        agg: Aggregation::Average,
    };
    let history: HistoryAnswer = api.get_json(&query.path()).await.expect("history");
    let series = history.data.get(&tag.key).expect("the tag's series");
    let samples = tag_api::chart_samples(series);
    assert!(!samples.is_empty(), "the plant recorded the tag");
    #[allow(clippy::cast_precision_loss)]
    let (start, end) = ((end_ms - 600_000) as f64, end_ms as f64);
    assert!(
        samples
            .iter()
            .all(|(at, _)| *at >= start - 60_000.0 && *at <= end + 60_000.0),
        "the samples lie in the span"
    );
    println!(
        "history: {} samples of {} in the custom span, bucket {} ms",
        samples.len(),
        tag.key,
        history.interval
    );
}

/// A generic REST call through the runtime, answered under its key.
async fn call<T: Send + 'static>(
    handle: &runtime::Handle,
    events: &async_channel::Receiver<Event>,
    key: CallKey,
    request: Call,
) -> Result<T, String> {
    handle.send(Command::Call { key, call: request });
    wait_for(events, 20, |event| match event {
        Event::Reply {
            key: answered,
            reply,
        } if answered == key => Some(reply.take::<T>()),
        _ => None,
    })
    .await
    .unwrap_or_else(|| panic!("no reply to {}", key.name))
}

/// Wait for the outcome of the runtime action named `action`; it must
/// have succeeded.
async fn done(events: &async_channel::Receiver<Event>, action: &str) {
    let result = wait_for(events, 30, |event| match event {
        Event::Done {
            action: finished,
            result,
        } if finished == action => Some(result),
        _ => None,
    })
    .await
    .unwrap_or_else(|| panic!("no answer to “{action}”"));
    if let Err(error) = result {
        panic!("“{action}” failed: {error}");
    }
}

/// The shelf as the server lists it (`status=shelved`), resolved alarms
/// too, unlike the console's open alarms.
async fn shelved(handle: &runtime::Handle, events: &async_channel::Receiver<Event>) -> Vec<Alarm> {
    call::<Vec<Alarm>>(
        handle,
        events,
        CallKey::new("alarms.shelf"),
        Call::get::<Vec<Alarm>>("/api/v1/alarms?status=shelved&limit=500"),
    )
    .await
    .unwrap_or_else(|error| panic!("the shelved alarms failed: {error}"))
}

/// The Alarms page's operator paths: the summary, the history, the
/// analytics, a shelve with a reason (seen on the shelf, in the journal and
/// in the audit trail) and the unshelve.
async fn alarm_management(
    handle: &runtime::Handle,
    events: &async_channel::Receiver<Event>,
    open: &[Alarm],
) {
    let summary = call::<AlarmSummary>(
        handle,
        events,
        CallKey::new("alarms.summary"),
        Call::get::<AlarmSummary>("/api/v1/alarms/summary"),
    )
    .await
    .unwrap_or_else(|error| panic!("the alarm summary failed: {error}"));
    println!(
        "alarm summary: {} critical, {} warning, {} info, {} shelved, {} in 24 h",
        summary.active_critical,
        summary.active_warning,
        summary.active_info,
        summary.shelved,
        summary.recent_24h
    );

    let history = call::<Vec<Alarm>>(
        handle,
        events,
        CallKey::new("alarms.history"),
        Call::get::<Vec<Alarm>>("/api/v1/alarms?status=all&limit=500"),
    )
    .await
    .unwrap_or_else(|error| panic!("the alarm history failed: {error}"));
    for alarm in open {
        assert!(
            history.len() == 500 || history.iter().any(|kept| kept.id == alarm.id),
            "the history holds open alarm {}",
            alarm.id
        );
    }

    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock after 1970")
            .as_secs(),
    )
    .expect("seconds fit an i64");
    let analytics = call::<Vec<AlarmAnalyticsEntry>>(
        handle,
        events,
        CallKey::new("alarms.analytics"),
        Call::get::<Vec<AlarmAnalyticsEntry>>(format!(
            "/api/v1/alarms/analytics?from={}&to={now}",
            now - 86_400
        )),
    )
    .await
    .unwrap_or_else(|error| panic!("the alarm analytics failed: {error}"));
    let raised_today = history
        .iter()
        .filter(|alarm| alarm.triggered_at >= now - 86_400)
        .count();
    if raised_today > 0 {
        assert!(
            !analytics.is_empty(),
            "the last day had alarms, so analytics rank their sources"
        );
    }
    assert!(
        analytics
            .windows(2)
            .all(|pair| pair[0].trigger_count >= pair[1].trigger_count),
        "most frequent first"
    );
    assert!(analytics.iter().all(|entry| entry.trigger_count > 0));
    println!(
        "alarm history: {} alarms, {raised_today} in 24 h over {} sources",
        history.len(),
        analytics.len()
    );

    let Some(id) = history
        .iter()
        .chain(open)
        .find(|alarm| alarm.resolved_at.is_none() && alarm.state == "active")
        .map(|alarm| alarm.id)
    else {
        eprintln!("note: no open alarm to shelve");
        return;
    };
    let reason = "e2e: native shelve with a reason".to_owned();
    handle.send(Command::Shelve {
        alarm: id,
        seconds: 900,
        reason: reason.clone(),
    });
    done(events, &format!("Shelve alarm {id}")).await;
    let shelf = shelved(handle, events).await;
    let on_shelf = shelf
        .iter()
        .find(|alarm| alarm.id == id)
        .unwrap_or_else(|| panic!("alarm {id} is not on the shelf"));
    assert_eq!(on_shelf.shelved_reason.as_deref(), Some(reason.as_str()));
    assert!(
        on_shelf
            .shelved_until
            .is_some_and(|until| until > now && until <= now + 960),
        "shelved for 15 minutes: {:?}",
        on_shelf.shelved_until
    );

    let journal = call::<Vec<AlarmJournalEntry>>(
        handle,
        events,
        CallKey::new("alarms.journal").with(id),
        Call::get::<Vec<AlarmJournalEntry>>(format!("/api/v1/alarms/{id}/journal")),
    )
    .await
    .unwrap_or_else(|error| panic!("the journal of alarm {id} failed: {error}"));
    assert!(
        journal
            .iter()
            .any(|entry| entry.alarm_id == id && entry.event == "shelved"),
        "the journal records the shelve: {journal:?}"
    );
    assert!(
        journal.windows(2).all(|pair| pair[0].at <= pair[1].at),
        "oldest first"
    );

    let audit = call::<Vec<AlarmAuditEntry>>(
        handle,
        events,
        CallKey::new("alarms.audit"),
        Call::get::<Vec<AlarmAuditEntry>>(format!("/api/v1/alarms/audit?alarm_id={id}&limit=20")),
    )
    .await
    .unwrap_or_else(|error| panic!("the alarm audit failed: {error}"));
    assert!(
        audit.iter().all(|entry| entry.alarm_id == id),
        "filtered to alarm {id}"
    );
    assert!(
        audit
            .iter()
            .any(|entry| entry.action == "shelved" && entry.details.contains(&reason)),
        "the audit trail carries the reason: {audit:?}"
    );

    handle.send(Command::Unshelve(id));
    done(events, &format!("Unshelve alarm {id}")).await;
    assert!(
        !shelved(handle, events)
            .await
            .iter()
            .any(|alarm| alarm.id == id),
        "alarm {id} stayed on the shelf"
    );
    println!("shelved alarm {id} with a reason, found it in the journal and audit, unshelved it");
}
