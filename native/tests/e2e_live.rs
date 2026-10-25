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
