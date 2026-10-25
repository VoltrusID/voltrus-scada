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

    shell_preferences_notifications_and_edition(&url, &user, &pass).await;
}

/// The shell's paths (settings sheet, shortcuts, bell, edition, sign-in
/// methods): each read, each write and back, the role gate on the alarm
/// cooldown, and user notifications end to end: an operator mentions the
/// admin and a viewer in a shift handover; the viewer reads, marks read and
/// dismisses theirs; the admin's stays unread (the bell's screenshot shows
/// it). Leaves the admin's preferences as it found them, and removes the
/// two users it makes.
async fn shell_preferences_notifications_and_edition(url: &str, user: &str, pass: &str) {
    use serde::de::IgnoredAny;
    use voltrus_scada_native::api::Method;
    use voltrus_scada_native::api::shell::{
        AlarmCooldown, AuthProviders, LicenseStatus, ShortcutMap, SignInMethod, Tier, UiLang,
        UiTheme, UserNotifications,
    };

    // Public, before signing in, through the runtime as the sign-in card
    // asks.
    let (handle, events) = runtime::start().expect("runtime");
    let key = CallKey::new("shell.providers").with(1);
    handle.send(Command::Public {
        base: url.to_owned(),
        key,
        call: Call::new(|api| async move {
            api.get_public::<AuthProviders>("/api/v1/auth/providers")
                .await
        }),
    });
    let providers = wait_for(&events, 20, |event| match event {
        Event::PublicReply { key: got, reply } if got == key => Some(reply.take::<AuthProviders>()),
        _ => None,
    })
    .await
    .expect("no answer about sign-in methods")
    .unwrap_or_else(|error| panic!("sign-in methods: {error}"));
    assert!(
        providers
            .providers
            .iter()
            .any(|provider| provider.method() == SignInMethod::Password),
        "the password is a sign-in method"
    );
    handle.send(Command::Shutdown);
    let license: LicenseStatus = VoltrusApi::new(url)
        .get_public("/api/license/status")
        .await
        .expect("licence status");
    assert!(!license.status.is_empty(), "the licence names its status");
    println!(
        "sign-in methods: {}, edition: {} ({})",
        providers.providers.len(),
        license.tier().label(),
        license.status
    );
    if license.status == "Community" {
        assert_eq!(license.tier(), Tier::Community);
    }

    let mut admin = VoltrusApi::new(url);
    admin.login(user, pass).await.expect("sign in");

    // Appearance and language: set, read back, restore.
    let theme: UiTheme = admin
        .get_json("/api/v1/settings/ui-theme")
        .await
        .expect("ui-theme");
    for value in ["light", theme.theme.as_str()] {
        let saved: UiTheme = admin
            .send_json(
                Method::PUT,
                "/api/v1/settings/ui-theme",
                &serde_json::json!({ "theme": value }),
            )
            .await
            .expect("save ui-theme");
        assert_eq!(saved.theme, value);
        let read: UiTheme = admin
            .get_json("/api/v1/settings/ui-theme")
            .await
            .expect("ui-theme");
        assert_eq!(read.theme, value);
    }
    let lang: UiLang = admin
        .get_json("/api/v1/settings/ui-lang")
        .await
        .expect("ui-lang");
    for value in ["id", lang.lang.as_str()] {
        let saved: UiLang = admin
            .send_json(
                Method::PUT,
                "/api/v1/settings/ui-lang",
                &serde_json::json!({ "lang": value }),
            )
            .await
            .expect("save ui-lang");
        assert_eq!(saved.lang, value);
    }
    let cooldown: AlarmCooldown = admin
        .get_json("/api/v1/settings/alarm-cooldown")
        .await
        .expect("alarm cooldown");
    for secs in [cooldown.secs + 30, cooldown.secs] {
        let saved: AlarmCooldown = admin
            .send_json(
                Method::PUT,
                "/api/v1/settings/alarm-cooldown",
                &serde_json::json!({ "secs": secs }),
            )
            .await
            .expect("save alarm cooldown");
        assert_eq!(saved.secs, secs);
    }
    println!(
        "preferences: theme {}, lang {}, cooldown {} s (each set and restored)",
        theme.theme, lang.lang, cooldown.secs
    );

    // Shortcuts: a tag's and a navigation target's, set and cleared. The
    // plant's admin has no shortcuts of its own to keep.
    let tags: Vec<voltrus_scada_native::api::DataSource> =
        admin.data_sources().await.expect("tags");
    let tag = tags.first().expect("a tag").id.to_string();
    let before: ShortcutMap = admin
        .get_json("/api/v1/shortcuts")
        .await
        .expect("shortcuts");
    let mut body = serde_json::Map::new();
    body.insert(tag.clone(), serde_json::Value::from("Alt+F9"));
    let _: IgnoredAny = admin
        .send_json(Method::PUT, "/api/v1/shortcuts", &body)
        .await
        .expect("save a tag shortcut");
    let saved: ShortcutMap = admin
        .get_json("/api/v1/shortcuts")
        .await
        .expect("shortcuts");
    assert_eq!(saved.get(&tag).map(String::as_str), Some("Alt+F9"));
    let restore: serde_json::Map<String, serde_json::Value> = before
        .into_iter()
        .filter(|(_, combo)| !combo.is_empty())
        .map(|(id, combo)| (id, serde_json::Value::String(combo)))
        .collect();
    let _: IgnoredAny = admin
        .send_json(Method::PUT, "/api/v1/shortcuts", &restore)
        .await
        .expect("restore tag shortcuts");
    let nav: ShortcutMap = admin
        .get_json("/api/v1/nav-shortcuts")
        .await
        .expect("nav shortcuts");
    assert_eq!(
        nav.get("screen:all").map(String::as_str),
        Some("Mod+1"),
        "the default"
    );
    let _: IgnoredAny = admin
        .send_json(
            Method::PUT,
            "/api/v1/nav-shortcuts",
            &serde_json::json!({ "action:alarms": "F9" }),
        )
        .await
        .expect("save a nav shortcut");
    let nav: ShortcutMap = admin
        .get_json("/api/v1/nav-shortcuts")
        .await
        .expect("nav shortcuts");
    assert_eq!(nav.get("action:alarms").map(String::as_str), Some("F9"));
    let _: IgnoredAny = admin
        .send_json(Method::PUT, "/api/v1/nav-shortcuts", &serde_json::json!({}))
        .await
        .expect("clear nav shortcuts");
    println!("shortcuts: a tag's and a target's set and cleared");

    // Two users: an operator who writes the handover, a viewer who reads.
    let suffix = format!("{:x}", std::process::id());
    let operator = format!("e2e_ops_{suffix}");
    let viewer = format!("e2e_view_{suffix}");
    let secret = format!("E2e-{suffix}-notify-pass");
    let mut made = Vec::new();
    for (name, role) in [(&operator, "demo"), (&viewer, "viewer")] {
        let answer: serde_json::Value = admin
            .send_json(
                Method::POST,
                "/api/v1/users",
                &serde_json::json!({ "username": name, "password": secret, "role": role }),
            )
            .await
            .expect("create a user");
        made.push(answer["id"].as_i64().expect("the new user's id"));
    }
    let mut ops = VoltrusApi::new(url);
    ops.login(&operator, &secret)
        .await
        .expect("the operator signs in");
    let today = time::OffsetDateTime::now_utc().date();
    let handover: serde_json::Value = ops
        .send_json(
            Method::POST,
            "/api/v1/shift-handover",
            &serde_json::json!({
                "shift_date": format!("{:04}-{:02}-{:02}", today.year(), u8::from(today.month()), today.day()),
                "shift_period": "morning",
                "what_happened": format!("Inverter fan noisy, @{user} and @{viewer} please check"),
                "whats_pending": "Spare fan on order",
                "what_to_watch": "Inverter temperature",
            }),
        )
        .await
        .expect("the operator writes a handover");
    let handover = handover["id"].as_i64().expect("the handover's id");

    let mut view = VoltrusApi::new(url);
    view.login(&viewer, &secret)
        .await
        .expect("the viewer signs in");
    let unread: UserNotifications = view
        .get_json("/api/v1/user-notifications?unread_only=true")
        .await
        .expect("the viewer's notifications");
    let mention = unread
        .notifications
        .iter()
        .find(|notification| notification.reference_id == Some(handover))
        .expect("the viewer was told of the mention")
        .id;
    let _: IgnoredAny = view
        .send_empty(
            Method::PUT,
            &format!("/api/v1/user-notifications/{mention}/read"),
        )
        .await
        .expect("mark read");
    let unread: UserNotifications = view
        .get_json("/api/v1/user-notifications?unread_only=true")
        .await
        .expect("the viewer's notifications");
    assert!(
        unread.notifications.iter().all(|n| n.id != mention),
        "read is not unread"
    );
    let all: UserNotifications = view
        .get_json("/api/v1/user-notifications")
        .await
        .expect("the viewer's notifications");
    assert!(all.notifications.iter().any(|n| n.id == mention && n.read));
    let _: IgnoredAny = view
        .send_empty(
            Method::DELETE,
            &format!("/api/v1/user-notifications/{mention}"),
        )
        .await
        .expect("dismiss");
    let all: UserNotifications = view
        .get_json("/api/v1/user-notifications")
        .await
        .expect("the viewer's notifications");
    assert!(
        all.notifications.iter().all(|n| n.id != mention),
        "dismissed is gone"
    );
    // A viewer is not an operator: the server refuses the cooldown.
    let refused = view
        .send_json::<_, IgnoredAny>(
            Method::PUT,
            "/api/v1/settings/alarm-cooldown",
            &serde_json::json!({ "secs": 60 }),
        )
        .await;
    assert!(
        refused.is_err(),
        "a viewer may not change the alarm cooldown"
    );
    println!("notifications: read, marked read and dismissed; a viewer cannot set the cooldown");

    let admins: UserNotifications = admin
        .get_json("/api/v1/user-notifications")
        .await
        .expect("the admin's notifications");
    assert!(
        admins
            .notifications
            .iter()
            .any(|n| n.reference_id == Some(handover) && !n.read),
        "the admin was told of the mention too"
    );
    for id in made {
        let _: IgnoredAny = admin
            .send_empty(Method::DELETE, &format!("/api/v1/users/{id}"))
            .await
            .expect("remove the test user");
    }
}
