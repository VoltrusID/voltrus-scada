//! Live end-to-end test against a running voltrus server.
//! Opt-in: only runs when VOLTRUS_E2E_URL / VOLTRUS_E2E_PASSWORD are set.
//!
//!   VOLTRUS_E2E_URL=http://localhost:3100 \
//!   VOLTRUS_E2E_USERNAME=admin \
//!   VOLTRUS_E2E_PASSWORD=... \
//!   cargo test -p voltrus-scada-native --test e2e_live -- --nocapture

use std::time::Duration;

use voltrus_scada_native::runtime::{self, Command, Event, Link};

async fn next(events: &async_channel::Receiver<Event>) -> Option<Event> {
    tokio::time::timeout(Duration::from_millis(500), events.recv())
        .await
        .ok()
        .and_then(Result::ok)
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
        url,
        username: user,
        password: pass,
    });
    let mut signed_in = false;
    let mut tags: Option<Vec<String>> = None;
    let mut live = false;
    let mut updates = 0usize;
    let mut history = false;
    let mut alarms: Option<usize> = None;
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
            Some(Event::Alarms(list)) => alarms = Some(list.len()),
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
        "tags: {}, updates: {updates}, open alarms: {alarms}",
        tags.len()
    );
    if std::env::var_os("VOLTRUS_E2E_EXPECT_UPDATES").is_some() {
        assert!(updates > 0, "expected live tag updates from the simulators");
    } else if updates == 0 {
        eprintln!("note: no live tag updates; are protocol simulators running?");
    }
    handle.send(Command::Shutdown);
}
