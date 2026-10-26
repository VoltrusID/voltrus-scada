//! Tests of the console as a whole: the session's end, the kiosk lock,
//! the keymap and the routing of REST answers.

use std::path::Path;

use gpui::{Action, AppContext as _, Bounds, Pixels, TestAppContext, px, size};
use voltrus_scada_native::api::Alarm;
use voltrus_scada_native::runtime::{Command, Event, Link};

use super::audio::Annunciator;
use super::keys::{
    ShellOpenSettings, ShellToggleNotifications, ShowMore, ShowNavMenu, StartStress, StopStress,
    ToggleHelp, TogglePalette,
};
use super::navfit::Fit;
use super::shell::Shell;
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
        // The avatar and sign-out whole, clear of the window's edge.
        for (name, at) in &parts[parts.len() - 2..] {
            assert!(
                at.right() <= px(width - 12.0 + 0.5),
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
