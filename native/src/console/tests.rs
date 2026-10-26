//! Tests of the console as a whole: the session's end, the kiosk lock,
//! the keymap and the routing of REST answers.

use std::path::Path;

use gpui::{AppContext as _, TestAppContext};
use voltrus_scada_native::api::Alarm;
use voltrus_scada_native::runtime::{Command, Event, Link};

use super::audio::Annunciator;
use super::keys::{StartStress, StopStress};
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
