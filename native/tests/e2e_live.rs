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
}

/// Screens and card dashboards against the live plant: the screen list,
/// the overview's layouts, and a screen of its own (created, tags added,
/// laid out, renamed, a tag removed, deleted), so the plant ends as it
/// started.
#[tokio::test]
async fn live_screens_and_layouts() {
    use serde::de::IgnoredAny;
    use voltrus_scada_native::api::{DataSource, Method};
    use voltrus_scada_native::screens::{
        self, Card, CardKind, OVERVIEW, Screen, ScreenLayouts, layouts_path, tags_path,
    };

    let Ok(url) = std::env::var("VOLTRUS_E2E_URL") else {
        eprintln!("skipping: VOLTRUS_E2E_URL not set");
        return;
    };
    let user = std::env::var("VOLTRUS_E2E_USERNAME").unwrap_or_else(|_| "admin".into());
    let pass = std::env::var("VOLTRUS_E2E_PASSWORD").expect("VOLTRUS_E2E_PASSWORD");
    let mut api = VoltrusApi::new(&url);
    api.login(&user, &pass).await.expect("sign in");

    let listed: Vec<Screen> = api.get_json("/api/v1/screens").await.expect("GET /screens");
    println!(
        "screens: {}",
        listed
            .iter()
            .map(|screen| screen.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(first) = listed.first() {
        let rows: Vec<DataSource> = api
            .get_json(&tags_path(&first.id))
            .await
            .expect("a screen's tags");
        println!("{} has {} tags", first.name, rows.len());
    }
    let overview: ScreenLayouts = api
        .get_json(&layouts_path(OVERVIEW))
        .await
        .expect("the overview's layouts");
    println!("overview layout: {} cards on lg", overview.lg.len());

    // A screen of its own, with two of the plant's tags.
    let tags: Vec<DataSource> = api.get_json(&tags_path(OVERVIEW)).await.expect("every tag");
    assert!(tags.len() >= 2, "the plant has tags to put on a screen");
    let name = format!("Native e2e {}", std::process::id());
    let created: Screen = api
        .send_json(
            Method::POST,
            "/api/v1/screens",
            &serde_json::json!({ "name": name }),
        )
        .await
        .expect("POST /screens");
    assert_eq!(created.name, name);
    assert!(!created.id.is_empty());
    let path = format!("/api/v1/screens/{}", created.id);
    for tag in &tags[..2] {
        api.send_json::<_, IgnoredAny>(
            Method::POST,
            &format!("{path}/data-sources"),
            &serde_json::json!({ "data_source_id": tag.id }),
        )
        .await
        .expect("POST /screens/{id}/data-sources");
    }
    let on_screen: Vec<DataSource> = api
        .get_json(&tags_path(&created.id))
        .await
        .expect("the new screen's tags");
    assert_eq!(
        on_screen.iter().map(|tag| tag.id).collect::<Vec<_>>(),
        [tags[0].id, tags[1].id],
        "tags in the order they were added"
    );

    // Arrange them and save every breakpoint; the server keeps lg, md, sm.
    let cards: Vec<Card> = on_screen
        .iter()
        .map(|tag| Card {
            id: tag.id,
            device: &tag.device_id,
            kind: CardKind::of(&tag.chart_type),
        })
        .collect();
    let layouts = screens::auto_arrange_all(&cards);
    api.send_json::<_, IgnoredAny>(Method::PUT, &layouts_path(&created.id), &layouts)
        .await
        .expect("PUT /screens/{id}/layouts");
    let saved: ScreenLayouts = api
        .get_json(&layouts_path(&created.id))
        .await
        .expect("the saved layouts");
    assert_eq!(saved.lg, layouts.lg);
    assert_eq!(saved.md, layouts.md);
    assert_eq!(saved.sm, layouts.sm);

    // Rename it, take a tag off, delete it.
    let renamed = format!("{name} renamed");
    api.send_json::<_, IgnoredAny>(Method::PUT, &path, &serde_json::json!({ "name": renamed }))
        .await
        .expect("PUT /screens/{id}");
    let listed: Vec<Screen> = api.get_json("/api/v1/screens").await.expect("GET /screens");
    assert!(
        listed
            .iter()
            .any(|screen| screen.id == created.id && screen.name == renamed),
        "the screen shows its new name"
    );
    api.send_empty::<IgnoredAny>(
        Method::DELETE,
        &format!("{path}/data-sources/{}", tags[0].id),
    )
    .await
    .expect("DELETE /screens/{id}/data-sources/{ds}");
    let left: Vec<DataSource> = api
        .get_json(&tags_path(&created.id))
        .await
        .expect("the screen's tags");
    assert_eq!(
        left.iter().map(|tag| tag.id).collect::<Vec<_>>(),
        [tags[1].id]
    );
    api.send_empty::<IgnoredAny>(Method::DELETE, &path)
        .await
        .expect("DELETE /screens/{id}");
    let listed: Vec<Screen> = api.get_json("/api/v1/screens").await.expect("GET /screens");
    assert!(
        !listed.iter().any(|screen| screen.id == created.id),
        "the screen is gone"
    );
    println!(
        "screen {} created, laid out, renamed and deleted",
        created.id
    );
}
