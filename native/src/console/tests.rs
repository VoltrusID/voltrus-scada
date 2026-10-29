//! Tests of the console as a whole: the session's end, the kiosk lock,
//! the keymap and the routing of REST answers.

use std::path::Path;
use std::time::Duration;

use gpui::{Action, AppContext as _, Bounds, Pixels, TestAppContext, px, size};
use voltrus_scada_native::api::Alarm;
use voltrus_scada_native::runtime::{CallKey, Command, Event, Link, Reply};

use super::audio::Annunciator;
use super::keys::{
    ShellOpenSettings, ShellToggleNotifications, ShowMore, ShowNavMenu, StartStress, StopStress,
    ToggleHelp, TogglePalette,
};
use super::navfit::Fit;
use super::shell::Shell;
use super::station::Status;
use super::testing::Harness;
use super::text_area::TextArea;
use super::{Phase, pages, views};

/// Page state a session leaves behind.
#[derive(Default)]
struct Leftover(u32);

fn critical_alarm() -> Alarm {
    Alarm {
        id: 1,
        source_key: "plts_1.p".into(),
        severity: "critical".into(),
        state: "active".into(),
        ..Alarm::default()
    }
}

fn sounding(h: &mut Harness) -> bool {
    h.update(|console, _, _| {
        console
            .state_ref::<Annunciator>()
            .and_then(Annunciator::wanted)
            .is_some()
    })
}

#[gpui::test]
fn signing_out_drops_the_sessions_page_state(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    h.sign_in("admin");
    let epoch = h.update(|console, _, _| {
        console.state::<Leftover>().0 = 7;
        console.state::<Shell>().prefs.os_notifications = true;
        console.session_epoch()
    });
    h.update(|console, window, cx| console.sign_out(window, cx));
    h.update(|console, _, _| {
        assert!(
            console.state_ref::<Leftover>().is_none(),
            "a page's state does not reach the next user"
        );
        assert!(
            console
                .state_ref::<Shell>()
                .is_some_and(|shell| shell.prefs.os_notifications),
            "the operator's own preferences stay"
        );
        assert!(console.session_epoch() > epoch);
    });
}

#[gpui::test]
fn a_later_sign_in_starts_without_the_earlier_sessions_state(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    h.sign_in("admin");
    // What a page made after the link failed, before signing in again.
    h.event(Event::Link(Link::Failed("Not authorised: revoked".into())));
    h.update(|console, _, _| console.state::<Leftover>().0 = 3);
    h.sign_in("demo");
    assert!(h.update(|console, _, _| console.state_ref::<Leftover>().is_none()));
}

#[gpui::test]
fn a_failed_link_silences_the_alarm_sound(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    h.sign_in("admin");
    h.event(Event::Alarms {
        alarms: vec![critical_alarm()],
        capped: false,
    });
    assert!(sounding(&mut h), "an unacknowledged critical alarm sounds");
    h.event(Event::Link(Link::Failed("Not authorised: revoked".into())));
    assert!(!sounding(&mut h), "the session ended: silence");
    h.update(|console, _, _| {
        assert!(console.alarms.is_empty());
        assert_eq!(console.phase, Phase::Console, "the reason stays on show");
    });
}

fn stress_started(h: &mut Harness) -> bool {
    h.cx.dispatch_action(StartStress);
    h.commands()
        .iter()
        .any(|command| matches!(command, Command::Stress(_)))
}

#[gpui::test]
fn only_an_admin_without_the_kiosk_lock_starts_a_stress_run(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    h.sign_in("demo");
    assert!(!stress_started(&mut h), "an operator keeps the session");

    let mut h = Harness::new(cx);
    h.sign_in("admin");
    h.update(|console, _, _| console.state::<Shell>().prefs.kiosk = true);
    assert!(!stress_started(&mut h), "the kiosk lock keeps the session");
    h.update(|console, _, _| console.state::<Shell>().prefs.kiosk = false);
    assert!(stress_started(&mut h));

    // Locked during the run: stopping it would sign out.
    h.update(|console, _, _| console.state::<Shell>().prefs.kiosk = true);
    h.cx.dispatch_action(StopStress);
    assert!(
        !h.commands()
            .iter()
            .any(|command| matches!(command, Command::SignOut))
    );
    assert_eq!(h.update(|console, _, _| console.phase), Phase::Console);
}

#[gpui::test]
fn the_kiosk_lock_holds_in_the_console_only(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    h.update(|console, _, _| console.state::<Shell>().prefs.kiosk = true);
    assert!(
        !h.update(|console, _, _| console.kiosk_locked()),
        "the sign-in card can be quit"
    );
    h.sign_in("admin");
    assert!(h.update(|console, _, _| console.kiosk_locked()));
}

#[gpui::test]
fn a_text_area_adds_no_key_bindings(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    let count = |h: &mut Harness| {
        h.cx.update(|_, cx| cx.key_bindings().borrow().bindings().len())
    };
    let before = count(&mut h);
    let _area = h.cx.update(|_, cx| cx.new(|cx| TextArea::new(3, cx)));
    assert_eq!(count(&mut h), before, "the keymap already has its keys");
}

#[gpui::test]
fn a_code_editor_adds_no_key_bindings(cx: &mut TestAppContext) {
    use super::views::data_manager::scripts::code_editor::CodeEditor;
    let mut h = Harness::new(cx);
    let count = |h: &mut Harness| {
        h.cx.update(|_, cx| cx.key_bindings().borrow().bindings().len())
    };
    let before = count(&mut h);
    let _editor = h.cx.update(|_, cx| cx.new(CodeEditor::new));
    assert_eq!(count(&mut h), before, "the keymap already has its keys");
}

/// Every call key name written out in the console's source.
fn call_key_names(dir: &Path, names: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("the console's source") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            call_key_names(&path, names);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = std::fs::read_to_string(&path).expect("a source file");
            for (at, found) in text.match_indices("CallKey::new(\"") {
                let rest = &text[at + found.len()..];
                if let Some(end) = rest.find('"') {
                    names.push((rest[..end].to_owned(), path.display().to_string()));
                }
            }
        }
    }
}

#[test]
fn every_call_key_scope_has_an_owner() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/console");
    let mut names = Vec::new();
    call_key_names(&dir, &mut names);
    assert!(names.len() > 20, "found the calls: {names:?}");
    for (name, file) in names {
        let mut parts = name.split('.');
        let scope = parts.next().unwrap_or_default();
        let owned = match scope {
            "shell" => true,
            // The local station's health checks (`console::station`).
            "station" => true,
            "data" => parts.next().is_some_and(|section| {
                views::data_manager::SECTIONS
                    .iter()
                    .any(|spec| spec.id == section && spec.on_reply.is_some())
            }),
            _ => pages::by_id(scope).is_some_and(|spec| spec.on_reply.is_some()),
        };
        assert!(
            owned,
            "{name} ({file}): no page or section answers it, so its reply is lost"
        );
    }
}

/// The navbar's parts left to right as drawn at `fit`.
fn navbar_parts(h: &mut Harness, fit: Fit) -> Vec<(&'static str, Bounds<Pixels>)> {
    let mut names = vec!["nav-brand"];
    if fit.host() {
        names.push("nav-host");
    }
    names.extend(["nav-tabs", "nav-search"]);
    if fit.folds_controls() {
        names.push("nav-menu");
    } else {
        names.extend([
            "nav-control-0",
            "nav-control-1",
            "nav-control-2",
            "nav-control-3",
            "nav-control-4",
        ]);
    }
    names.extend(["nav-avatar", "nav-sign-out"]);
    names
        .into_iter()
        .map(|name| {
            let bounds =
                h.cx.debug_bounds(name)
                    .unwrap_or_else(|| panic!("{name} is drawn at {fit:?}"));
            (name, bounds)
        })
        .collect()
}

#[gpui::test]
fn the_navbar_steps_down_to_fit_every_window_width(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    h.sign_in("admin");
    let mut fits = Vec::new();
    for width in [
        2400.0_f32, 1728.0, 1440.0, 1280.0, 1180.0, 1100.0, 1024.0, 960.0, 900.0, 860.0, 800.0,
        760.0, 720.0,
    ] {
        h.cx.simulate_resize(size(px(width), px(760.0)));
        h.draw();
        let fit = h.update(|console, _, _| console.nav_fit);
        if let Some(wider) = fits.last() {
            assert!(fit >= *wider, "{width}: {fit:?} after {wider:?}");
        }
        fits.push(fit);
        let parts = navbar_parts(&mut h, fit);
        for pair in parts.windows(2) {
            let ((a, a_at), (b, b_at)) = (pair[0], pair[1]);
            assert!(
                a_at.right() <= b_at.left() + px(0.5),
                "{width} ({fit:?}): {a} {a_at:?} runs into {b} {b_at:?}"
            );
        }
        // The avatar and sign-out whole, clear of the window's edge (and
        // of its controls, where the console draws them).
        let controls = if views::window_controls::SHOWN {
            f32::from(views::window_controls::WIDTH)
        } else {
            0.0
        };
        for (name, at) in &parts[parts.len() - 2..] {
            assert!(
                at.right() <= px(width - 12.0 - controls + 0.5),
                "{width} ({fit:?}): {name} {at:?} is clipped"
            );
            assert!(at.size.width >= px(22.0), "{width}: {name} {at:?} squeezed");
        }
        // The tabs are not clipped either: the More button ends inside.
        let tabs = h.cx.debug_bounds("nav-tabs").expect("the tabs");
        let more = h.cx.debug_bounds("nav-more").expect("the More button");
        assert!(
            more.right() <= tabs.right() + px(0.5),
            "{width} ({fit:?}): the tabs {tabs:?} clip More {more:?}"
        );
    }
    assert_eq!(fits[0], Fit::Full, "a wide window shows everything");
    assert!(
        fits.last().is_some_and(|fit| fit.folds_controls()),
        "a narrow window folds the controls: {fits:?}"
    );
}

#[gpui::test]
fn menus_and_popovers_open_inside_a_small_window(cx: &mut TestAppContext) {
    let mut h = Harness::new(cx);
    h.sign_in("admin");
    let window = size(px(760.0), px(480.0));
    h.cx.simulate_resize(window);
    h.draw();
    assert!(h.update(|console, _, _| console.nav_fit.folds_controls()));
    let opens: [(&str, Box<dyn Action>); 6] = [
        (views::navbar::MENU, Box::new(ShowNavMenu)),
        ("more", Box::new(ShowMore)),
        ("palette", Box::new(TogglePalette)),
        ("help", Box::new(ToggleHelp)),
        ("settings", Box::new(ShellOpenSettings)),
        ("notifications", Box::new(ShellToggleNotifications)),
    ];
    for (id, action) in opens {
        h.cx.update(|window, cx| window.dispatch_action(action, cx));
        h.draw();
        assert!(
            h.update(|console, _, _| console.overlay_is(id)),
            "{id} opens"
        );
        let card =
            h.cx.debug_bounds("overlay-card")
                .expect("the overlay's card");
        assert!(
            card.left() >= px(0.0)
                && card.top() >= px(0.0)
                && card.right() <= window.width
                && card.bottom() <= window.height,
            "{id}: {card:?} is outside the window"
        );
        h.update(|console, window, cx| console.close_overlay(window, cx));
    }
    // The notifications popover opens under the menu button standing in for
    // the bell, lined up with its right edge.
    let button = h.cx.debug_bounds("nav-menu").expect("the menu button");
    h.cx.update(|window, cx| window.dispatch_action(Box::new(ShellToggleNotifications), cx));
    h.draw();
    let card = h.cx.debug_bounds("overlay-card").expect("the popover");
    assert!(
        (card.right() - button.right()).abs() < px(1.0),
        "{card:?} {button:?}"
    );
    h.update(|console, window, cx| console.close_overlay(window, cx));
    // The menu's rows run the controls: the second is Settings.
    h.cx.update(|window, cx| window.dispatch_action(Box::new(ShowNavMenu), cx));
    h.draw();
    h.keys("down enter");
    assert!(h.update(|console, _, _| console.overlay_is("settings")));
}

/// A station directory of its own, with a `station.toml` if `credential`.
fn station_dir(name: &str, credential: bool) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "voltrus-native-console-station-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if credential {
        std::fs::write(
            dir.join("station.toml"),
            "url = \"http://127.0.0.1:3999\"\nusername = \"station\"\npassword = \"s3cret\"\n",
        )
        .unwrap();
    }
    dir
}

/// A console launched in local station mode on `dir`.
fn at_station(cx: &mut TestAppContext, dir: &Path) -> Harness {
    let dir = dir.to_owned();
    let mut h = Harness::with(cx, move |launch| {
        launch.station_dir = Some(dir);
        launch.local_station = true;
    });
    h.draw();
    h
}

/// The health checks sent since the last look, by their sequence.
fn health_checks(h: &mut Harness) -> Vec<i64> {
    h.commands()
        .into_iter()
        .filter_map(|command| match command {
            Command::Public { key, .. } if key.name == "station.health" => Some(key.arg),
            _ => None,
        })
        .collect()
}

fn station_status(h: &mut Harness) -> Option<Status> {
    h.update(|console, _, _| {
        console
            .station
            .local
            .as_ref()
            .map(|local| local.status.clone())
    })
}

#[gpui::test]
fn a_local_station_shows_its_state_and_signs_in_by_itself(cx: &mut TestAppContext) {
    let dir = station_dir("connect", true);
    let mut h = at_station(cx, &dir);
    assert_eq!(station_status(&mut h), Some(Status::Connecting));
    assert!(h.cx.debug_bounds("station-dir").is_some());
    assert!(
        h.cx.debug_bounds("sign-in-card").is_none(),
        "the station's state stands in for the sign-in card"
    );
    // The server is up: the console signs in with the station's account.
    h.event(Event::PublicReply {
        key: CallKey::new("station.health").with(1),
        reply: Reply::ok(()),
    });
    let sign_in = h.commands().into_iter().find_map(|command| match command {
        Command::SignIn {
            url,
            username,
            password,
        } => Some((url, username, password)),
        _ => None,
    });
    assert_eq!(
        sign_in,
        Some((
            "http://127.0.0.1:3999".to_owned(),
            "station".to_owned(),
            "s3cret".to_owned()
        ))
    );
    assert_eq!(station_status(&mut h), Some(Status::SigningIn));
    h.sign_in("admin");
    assert_eq!(station_status(&mut h), Some(Status::Live));
    // The session ends: it signs in again, without the card.
    h.event(Event::Link(Link::Failed("Not authorised".into())));
    assert_eq!(health_checks(&mut h), [2]);
    assert_eq!(station_status(&mut h), Some(Status::Connecting));
    // Once: refused again, it waits for the operator.
    h.event(Event::PublicReply {
        key: CallKey::new("station.health").with(2),
        reply: Reply::ok(()),
    });
    h.commands();
    h.event(Event::SignInFailed {
        message: "Invalid credentials".into(),
        refused: true,
    });
    assert!(matches!(station_status(&mut h), Some(Status::Refused(_))));
    h.cx.executor().advance_clock(Duration::from_secs(60));
    h.cx.run_until_parked();
    assert!(health_checks(&mut h).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[gpui::test]
fn an_unreachable_station_waits_and_asks_again(cx: &mut TestAppContext) {
    let dir = station_dir("unreachable", true);
    let mut h = at_station(cx, &dir);
    h.event(Event::PublicReply {
        key: CallKey::new("station.health").with(1),
        reply: Reply::failed("connection refused"),
    });
    h.draw();
    assert!(matches!(
        station_status(&mut h),
        Some(Status::Unreachable(reason)) if reason.contains("connection refused")
    ));
    assert!(h.cx.debug_bounds("station-reason").is_some());
    assert!(h.cx.debug_bounds("sign-in-card").is_none());
    assert!(health_checks(&mut h).is_empty(), "not before the back-off");
    h.cx.executor().advance_clock(Duration::from_secs(1));
    h.cx.run_until_parked();
    assert_eq!(health_checks(&mut h), [2], "after a second");
    // A sign-in that did not reach the server is tried again too, never
    // shown on the card.
    h.event(Event::PublicReply {
        key: CallKey::new("station.health").with(2),
        reply: Reply::ok(()),
    });
    h.commands();
    h.event(Event::SignInFailed {
        message: "error sending request".into(),
        refused: false,
    });
    h.draw();
    h.update(|console, _, _| {
        assert_eq!(console.phase, Phase::SignIn);
        assert_eq!(console.error, None);
        assert!(!console.busy);
        assert!(console.station.local.as_ref().is_some_and(|l| l.idle()));
    });
    assert!(h.cx.debug_bounds("sign-in-card").is_none());
    h.cx.executor().advance_clock(Duration::from_secs(2));
    h.cx.run_until_parked();
    assert_eq!(health_checks(&mut h), [3], "after the back-off");
    // A refused one is not: it counts towards locking the account. The
    // station screen says why and waits for Retry.
    h.event(Event::PublicReply {
        key: CallKey::new("station.health").with(3),
        reply: Reply::ok(()),
    });
    h.commands();
    h.event(Event::SignInFailed {
        message: "Invalid credentials".into(),
        refused: true,
    });
    h.draw();
    assert!(matches!(
        station_status(&mut h),
        Some(Status::Refused(reason)) if reason.contains("Invalid credentials")
    ));
    assert!(h.update(|console, _, _| !console.busy && console.error.is_none()));
    h.cx.executor().advance_clock(Duration::from_secs(60));
    h.cx.run_until_parked();
    assert!(health_checks(&mut h).is_empty(), "no retry by itself");
    // Retry asks once, at once.
    h.update(|console, _, cx| super::station::retry(console, cx));
    assert_eq!(health_checks(&mut h), [4]);
    assert_eq!(station_status(&mut h), Some(Status::Connecting));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[gpui::test]
fn signing_out_of_the_station_offers_it_again(cx: &mut TestAppContext) {
    let dir = station_dir("sign-out", true);
    let mut h = at_station(cx, &dir);
    h.sign_in("admin");
    h.update(|console, window, cx| console.sign_out(window, cx));
    h.draw();
    assert!(h.update(|console, _, _| console.station.local.is_none()));
    assert!(h.cx.debug_bounds("sign-in-card").is_some());
    assert!(h.cx.debug_bounds("station-return").is_some());
    assert!(h.cx.debug_bounds("station-setup").is_none());
    h.update(|console, _, cx| super::station::open(console, cx));
    assert_eq!(health_checks(&mut h), [1]);
    // (Drawn bounds outlive their element, so the card's cannot tell.)
    assert_eq!(station_status(&mut h), Some(Status::Connecting));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[gpui::test]
fn the_card_offers_a_station_only_where_there_is_one_or_a_server(cx: &mut TestAppContext) {
    // No station dir: nothing.
    let mut h = Harness::new(cx);
    h.draw();
    assert!(h.cx.debug_bounds("station-return").is_none());
    assert!(h.cx.debug_bounds("station-setup").is_none());
    // A station with a credential, launched with `--sign-in` or a saved
    // server: the card, with the way back to the station.
    let dir = station_dir("card", true);
    let mut h = Harness::with(cx, |launch| launch.station_dir = Some(dir.clone()));
    h.draw();
    assert!(h.cx.debug_bounds("sign-in-card").is_some());
    assert!(h.cx.debug_bounds("station-return").is_some());
    std::fs::remove_dir_all(&dir).unwrap();
    // No station yet, but the server beside the console: set one up.
    let dir = station_dir("setup", false);
    let mut h = Harness::with(cx, |launch| {
        launch.station_dir = Some(dir.clone());
        launch.bundled_server = Some(dir.join("voltrus"));
    });
    h.draw();
    assert!(h.cx.debug_bounds("station-setup").is_some());
    assert!(h.cx.debug_bounds("station-return").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}
