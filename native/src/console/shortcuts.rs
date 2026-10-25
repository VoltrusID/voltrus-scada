//! The user's server-kept keyboard shortcuts as live key bindings.
//!
//! The server keeps two maps per user, shared with the web client:
//! navigation targets (`GET`/`PUT /api/v1/nav-shortcuts`: `screen:all`,
//! `action:alarms`, `view:…`, `mode:…`, `screen:<id>`) and one shortcut per
//! tag (`GET`/`PUT /api/v1/shortcuts`, keyed by data source id). Combos are
//! the web's form: `Mod+Shift+ArrowUp`, `Alt+F2`, `Mod+K` (`Mod` is ⌘ on
//! macOS and Ctrl elsewhere; a printable key is written as typed, so Shift
//! shows only on named keys).
//!
//! GPUI's bindings are static once bound, so [`rebind`] rebuilds the whole
//! keymap: vt-ui's text field keys, every [`super::keys::key_bindings`] row
//! (less what the kiosk lock blocks) and one binding per shortcut that
//! works here. A shortcut that takes a built-in key is skipped and listed
//! as a conflict; a target without a native counterpart (the web's views
//! and modes) is kept on the server but not bound. Any new static binding
//! must come from `keys::key_bindings()` so a rebind keeps it.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{
    Action, App, Context, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, Keystroke,
};
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::shell::ShortcutMap;
use voltrus_scada_native::runtime::{Call, CallKey};

use super::Console;
use super::keys::{self, NOT_TYPING, Quit, ShowPage, SignOut};
use super::pages::Page;
use super::shell::Shell;

/// Open the tag with this data source id (a per-tag shortcut).
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = voltrus_console, no_json)]
pub struct ShellOpenTag(pub i64);

/// The web's navigation targets, in its order, with a label. The server
/// adds `screen:<id>` per device screen.
pub const NAV_TARGETS: [(&str, &str); 9] = [
    ("screen:all", "Overview"),
    ("screen:system", "System panel"),
    ("view:cards", "Grid view"),
    ("view:canvas", "Canvas view"),
    ("view:twin", "3D twin view"),
    ("view:scada", "2D SCADA view"),
    ("mode:live", "Live mode"),
    ("mode:history", "History mode"),
    ("action:alarms", "Alarms"),
];

/// What a shortcut does here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Page(Page),
    Tag(i64),
}

impl Target {
    fn action(self) -> Box<dyn Action> {
        match self {
            Self::Page(page) => Box::new(ShowPage(page)),
            Self::Tag(id) => Box::new(ShellOpenTag(id)),
        }
    }
}

/// The console page a navigation target opens, if it has one.
pub fn nav_page(target: &str) -> Option<Page> {
    match target {
        "screen:all" => Some(Page::Overview),
        "action:alarms" => Some(Page::Alarms),
        _ => None,
    }
}

/// A navigation target as the editor names it.
pub fn nav_label(target: &str) -> String {
    NAV_TARGETS
        .iter()
        .find(|(id, _)| *id == target)
        .map_or_else(
            || match target.strip_prefix("screen:") {
                Some(screen) => format!("Screen {screen}"),
                None => target.to_owned(),
            },
            |(_, label)| (*label).to_owned(),
        )
}

/// A web combo (`Mod+Shift+ArrowUp`) as a GPUI keystroke
/// (`secondary-shift-up`); `None` if it names a key GPUI does not know.
pub fn to_keystroke(combo: &str) -> Option<String> {
    // A trailing space is the Space key (`Mod+ `), not padding.
    let combo = combo.trim_start();
    if combo.trim().is_empty() {
        return None;
    }
    // The key itself may be `+` (`Mod++`).
    let (modifiers, key) = match combo.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None => match combo.rsplit_once('+') {
            Some((modifiers, key)) => (modifiers, key),
            None => ("", combo),
        },
    };
    let mut parts: Vec<&str> = Vec::new();
    for modifier in modifiers.split('+').filter(|part| !part.is_empty()) {
        let part = match modifier.to_ascii_lowercase().as_str() {
            "mod" | "ctrl" | "control" | "meta" | "cmd" | "command" => "secondary",
            "alt" | "option" => "alt",
            "shift" => "shift",
            _ => return None,
        };
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    let key = match key {
        " " | "Space" | "Spacebar" => "space".to_owned(),
        "ArrowUp" => "up".to_owned(),
        "ArrowDown" => "down".to_owned(),
        "ArrowLeft" => "left".to_owned(),
        "ArrowRight" => "right".to_owned(),
        "Enter" => "enter".to_owned(),
        "Escape" => "escape".to_owned(),
        "Tab" => "tab".to_owned(),
        "Backspace" => "backspace".to_owned(),
        "Delete" => "delete".to_owned(),
        "Insert" => "insert".to_owned(),
        "Home" => "home".to_owned(),
        "End" => "end".to_owned(),
        "PageUp" => "pageup".to_owned(),
        "PageDown" => "pagedown".to_owned(),
        key if key.chars().count() == 1 => key.to_lowercase(),
        key if key.len() <= 3
            && key
                .strip_prefix(['F', 'f'])
                .is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n))) =>
        {
            key.to_ascii_lowercase()
        }
        _ => return None,
    };
    parts.push(&key);
    Some(parts.join("-"))
}

/// A typed keystroke in the web's combo form, as `normalizeKeyEvent` in
/// the web client writes it; `None` for a lone modifier.
pub fn from_keystroke(keystroke: &Keystroke) -> Option<String> {
    let modifiers = &keystroke.modifiers;
    let named = match keystroke.key.as_str() {
        "up" => Some("ArrowUp"),
        "down" => Some("ArrowDown"),
        "left" => Some("ArrowLeft"),
        "right" => Some("ArrowRight"),
        "enter" => Some("Enter"),
        "escape" => Some("Escape"),
        "tab" => Some("Tab"),
        "backspace" => Some("Backspace"),
        "delete" => Some("Delete"),
        "insert" => Some("Insert"),
        "home" => Some("Home"),
        "end" => Some("End"),
        "pageup" => Some("PageUp"),
        "pagedown" => Some("PageDown"),
        "space" => Some(" "),
        "shift" | "control" | "alt" | "platform" | "function" | "" => return None,
        _ => None,
    };
    let key = match named {
        Some(name) => name.to_owned(),
        None if keystroke
            .key
            .strip_prefix('f')
            .is_some_and(|n| n.parse::<u8>().is_ok()) =>
        {
            keystroke.key.to_uppercase()
        }
        None => {
            // What Shift typed (`!` for Shift-1), as the browser's key.
            let typed = keystroke
                .key_char
                .as_deref()
                .filter(|typed| modifiers.shift && typed.chars().count() == 1)
                .unwrap_or(&keystroke.key);
            if typed.chars().count() != 1 {
                return None;
            }
            typed.to_uppercase()
        }
    };
    let mut parts: Vec<&str> = Vec::new();
    if modifiers.control || modifiers.platform {
        parts.push("Mod");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift && key.chars().count() > 1 {
        parts.push("Shift");
    }
    parts.push(&key);
    Some(parts.join("+"))
}

/// A GPUI keystroke as the operator reads it. `keys::display` splits on
/// `-`, so the minus key goes by a name there.
fn display_keys(stroke: &str) -> String {
    match stroke.strip_suffix("--") {
        Some(modifiers) => keys::display(&format!("{modifiers}-minus")).replace("MINUS", "-"),
        None if stroke == "-" => "-".to_owned(),
        None => keys::display(stroke),
    }
}

/// A combo as the operator reads it (`⇧⌘↑`), or as stored when it names a
/// key this console does not know.
pub fn display(combo: &str) -> String {
    to_keystroke(combo).map_or_else(|| combo.to_owned(), |stroke| display_keys(&stroke))
}

/// Whether two GPUI keystrokes are the same keys (`cmd-k` and
/// `secondary-k` on macOS).
fn same_keys(a: &str, b: &str) -> bool {
    match (Keystroke::parse(a), Keystroke::parse(b)) {
        (Ok(a), Ok(b)) => a.modifiers == b.modifiers && a.key == b.key,
        _ => a == b,
    }
}

/// The built-in keys a shortcut may not take: every command and page key
/// that works outside a text field.
fn builtin_keys() -> Vec<String> {
    let mut taken: Vec<String> = keys::COMMANDS
        .iter()
        .filter(|command| command.context == keys::CONSOLE || command.context == NOT_TYPING)
        .flat_map(|command| command.keys.iter().map(|keys| (*keys).to_owned()))
        .collect();
    taken.extend(
        super::pages::PAGES
            .iter()
            .filter_map(|spec| spec.digit.map(keys::page_keys)),
    );
    taken
}

/// Why a stored shortcut is not bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// The web's views and modes, screens: nothing here to open.
    WebOnly,
    /// A key GPUI does not know.
    Unreadable,
    /// A built-in command has these keys.
    Taken(String),
    /// An earlier shortcut has these keys.
    Duplicate(String),
    /// A key without ⌘, ⌥ or ⇧ (Enter, Space, an arrow, a letter): it would
    /// take the key from lists and dialogs. Function keys may stand alone.
    Unmodified,
}

impl Skipped {
    pub fn reason(&self) -> String {
        match self {
            Self::WebOnly => "Web client only".into(),
            Self::Unreadable => "Not a key this console knows".into(),
            Self::Taken(stroke) => format!("{} is a built-in key", display_keys(stroke)),
            Self::Duplicate(stroke) => format!("{} is used above", display_keys(stroke)),
            Self::Unmodified => "Needs ⌘, ⌥ or ⇧, or a function key".into(),
        }
    }
}

/// What to bind: each working shortcut's keystroke and target, and why the
/// rest are not.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub bound: Vec<(String, Target)>,
    /// By map entry: `nav:<target>` or `tag:<id>`.
    pub skipped: BTreeMap<String, Skipped>,
}

/// Plan the bindings for the two maps. Navigation first, in the web's
/// order, then tags by id; the first of two equal shortcuts wins.
pub fn plan(nav: &ShortcutMap, tags: &ShortcutMap) -> Plan {
    let taken = builtin_keys();
    let mut plan = Plan::default();
    let mut nav_entries: Vec<(&String, &String)> = nav.iter().collect();
    nav_entries.sort_by_key(|(target, _)| {
        NAV_TARGETS
            .iter()
            .position(|(id, _)| id == target)
            .unwrap_or(NAV_TARGETS.len())
    });
    let tag_entries = tags.iter().filter_map(|(id, combo)| {
        id.parse::<i64>()
            .ok()
            .map(|id| (format!("tag:{id}"), Some(Target::Tag(id)), combo))
    });
    let entries = nav_entries
        .into_iter()
        .map(|(target, combo)| {
            (
                format!("nav:{target}"),
                nav_page(target).map(Target::Page),
                combo,
            )
        })
        .chain(tag_entries);
    for (entry, target, combo) in entries {
        if combo.trim().is_empty() {
            continue;
        }
        let Some(target) = target else {
            plan.skipped.insert(entry, Skipped::WebOnly);
            continue;
        };
        let Some(stroke) = to_keystroke(combo).filter(|stroke| Keystroke::parse(stroke).is_ok())
        else {
            plan.skipped.insert(entry, Skipped::Unreadable);
            continue;
        };
        if !modified_or_function(&stroke) {
            plan.skipped.insert(entry, Skipped::Unmodified);
            continue;
        }
        if let Some(builtin) = taken.iter().find(|builtin| same_keys(builtin, &stroke)) {
            // The web's default `Mod+1` for the overview is ⌘1 here too.
            let same = matches!(target, Target::Page(page)
                if super::pages::spec(page).digit.map(keys::page_keys).as_deref() == Some(builtin.as_str()));
            if !same {
                plan.skipped.insert(entry, Skipped::Taken(builtin.clone()));
            }
            continue;
        }
        if plan
            .bound
            .iter()
            .any(|(bound, _)| same_keys(bound, &stroke))
        {
            plan.skipped.insert(entry, Skipped::Duplicate(stroke));
            continue;
        }
        plan.bound.push((stroke, target));
    }
    plan
}

/// Whether a GPUI keystroke has a modifier, or is a function key (F1–F24).
fn modified_or_function(stroke: &str) -> bool {
    Keystroke::parse(stroke).is_ok_and(|keystroke| {
        keystroke.modifiers.modified()
            || keystroke
                .key
                .strip_prefix('f')
                .is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)))
    })
}

/// A binding from server data: `None` if GPUI refuses it.
fn binding(keys: &str, action: Box<dyn Action>, context: &str) -> Option<KeyBinding> {
    let predicate = KeyBindingContextPredicate::parse(context).ok()?;
    KeyBinding::load(
        keys,
        action,
        Some(Rc::new(predicate)),
        false,
        None,
        &DummyKeyboardMapper,
    )
    .ok()
}

/// What the kiosk lock takes away: quitting and signing out.
pub fn kiosk_blocks(action: &dyn Action) -> bool {
    action.partial_eq(&Quit) || action.partial_eq(&SignOut)
}

/// Every key binding: the text field keys, the built-in commands (less the
/// kiosk-blocked ones) and the planned shortcuts, outside text fields.
pub fn key_bindings(plan: &Plan, kiosk: bool) -> Vec<KeyBinding> {
    let mut bindings = vt_ui::text_input::key_bindings();
    bindings.extend(super::text_area::key_bindings());
    bindings.extend(
        keys::key_bindings()
            .into_iter()
            .filter(|binding| !(kiosk && kiosk_blocks(binding.action()))),
    );
    bindings.extend(
        plan.bound
            .iter()
            .filter_map(|(keys, target)| binding(keys, target.action(), NOT_TYPING)),
    );
    bindings
}

/// The menus, without Quit and Sign Out under the kiosk lock.
pub fn menus(kiosk: bool) -> Vec<gpui::Menu> {
    let mut menus = keys::menus();
    if kiosk {
        for menu in &mut menus {
            menu.items.retain(|item| match item {
                gpui::MenuItem::Action { action, .. } => !kiosk_blocks(action.as_ref()),
                _ => true,
            });
        }
    }
    menus
}

/// Rebuild the keymap and the menus from the loaded shortcuts and the
/// kiosk lock (which holds in the console only).
pub fn rebind(console: &mut Console, cx: &mut App) {
    let kiosk = console.kiosk_locked();
    let shell = console.state::<Shell>();
    let plan = plan(&shell.nav, &shell.tags);
    let bindings = key_bindings(&plan, kiosk);
    shell.plan = plan;
    cx.clear_key_bindings();
    cx.bind_keys(bindings);
    cx.set_menus(menus(kiosk));
}

/// Fetch both maps again; the keymap follows when they arrive. Call after
/// changing them on the server.
pub fn reload(console: &Console) {
    console.call(
        CallKey::new("shell.nav_shortcuts"),
        Call::get::<ShortcutMap>("/api/v1/nav-shortcuts"),
    );
    console.call(
        CallKey::new("shell.tag_shortcuts"),
        Call::get::<ShortcutMap>("/api/v1/shortcuts"),
    );
}

/// Which map entry the editor's row changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Nav(String),
    Tag(i64),
}

/// The body of `PUT /api/v1/nav-shortcuts` or `/api/v1/shortcuts` that
/// sets `entry` to `combo` (`None` removes it). Both routes replace the
/// user's whole map, so the body carries every entry. Navigation keeps a
/// removed target as `null` (the server stores it as "none", overriding
/// its default); tags drop it.
pub fn save_body(
    map: &ShortcutMap,
    key: &str,
    combo: Option<&str>,
    nav: bool,
) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    for (entry, value) in map {
        if !value.is_empty() {
            body.insert(entry.clone(), serde_json::Value::String(value.clone()));
        } else if nav {
            body.insert(entry.clone(), serde_json::Value::Null);
        }
    }
    match combo.filter(|combo| !combo.is_empty()) {
        Some(combo) => {
            body.insert(key.to_owned(), serde_json::Value::String(combo.to_owned()));
        }
        None if nav => {
            body.insert(key.to_owned(), serde_json::Value::Null);
        }
        None => {
            body.remove(key);
        }
    }
    serde_json::Value::Object(body)
}

/// Set (or with `None` clear) one shortcut on the server.
pub fn save(console: &mut Console, entry: &Entry, combo: Option<&str>, cx: &mut Context<Console>) {
    let shell = console.state::<Shell>();
    let (key, path, body) = match entry {
        Entry::Nav(target) => (
            CallKey::new("shell.nav_shortcuts.save"),
            "/api/v1/nav-shortcuts",
            save_body(&shell.nav, target, combo, true),
        ),
        Entry::Tag(id) => (
            CallKey::new("shell.tag_shortcuts.save"),
            "/api/v1/shortcuts",
            save_body(&shell.tags, &id.to_string(), combo, false),
        ),
    };
    console.call(
        key,
        Call::send::<serde::de::IgnoredAny>(Method::PUT, path, body),
    );
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: &[(&str, &str)]) -> ShortcutMap {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn web_combos_become_gpui_keystrokes() {
        assert_eq!(to_keystroke("Mod+1").as_deref(), Some("secondary-1"));
        assert_eq!(
            to_keystroke("Mod+Shift+ArrowUp").as_deref(),
            Some("secondary-shift-up")
        );
        assert_eq!(to_keystroke("Alt+F2").as_deref(), Some("alt-f2"));
        assert_eq!(to_keystroke("Mod+K").as_deref(), Some("secondary-k"));
        assert_eq!(to_keystroke("Ctrl+Meta+K").as_deref(), Some("secondary-k"));
        assert_eq!(to_keystroke("Mod++").as_deref(), Some("secondary-+"));
        assert_eq!(to_keystroke("Mod+-").as_deref(), Some("secondary--"));
        assert!(Keystroke::parse("secondary--").is_ok_and(|stroke| stroke.key == "-"));
        assert!(display("Mod+-").ends_with('-'));
        assert_eq!(to_keystroke("Mod+ ").as_deref(), Some("secondary-space"));
        assert_eq!(to_keystroke("F13").as_deref(), Some("f13"));
        assert_eq!(to_keystroke(""), None);
        assert_eq!(to_keystroke("Mod+MediaPlay"), None, "unknown key");
        assert_eq!(to_keystroke("Hyper+K"), None, "unknown modifier");
        assert!(Keystroke::parse(&to_keystroke("Mod+Shift+PageDown").unwrap()).is_ok());
    }

    fn typed(key: &str, key_char: Option<&str>, set: impl Fn(&mut gpui::Modifiers)) -> Keystroke {
        let mut modifiers = gpui::Modifiers::default();
        set(&mut modifiers);
        Keystroke {
            modifiers,
            key: key.into(),
            key_char: key_char.map(str::to_owned),
        }
    }

    #[test]
    fn typed_keys_are_written_like_the_web_client() {
        let cmd_k = typed("k", Some("k"), |m| m.platform = true);
        assert_eq!(from_keystroke(&cmd_k).as_deref(), Some("Mod+K"));
        let ctrl_alt_f2 = typed("f2", None, |m| {
            m.control = true;
            m.alt = true;
        });
        assert_eq!(from_keystroke(&ctrl_alt_f2).as_deref(), Some("Mod+Alt+F2"));
        let shift_up = typed("up", None, |m| {
            m.platform = true;
            m.shift = true;
        });
        assert_eq!(
            from_keystroke(&shift_up).as_deref(),
            Some("Mod+Shift+ArrowUp")
        );
        // Shift on a printable key is the character it types, as in a
        // browser.
        let shift_1 = typed("1", Some("!"), |m| {
            m.platform = true;
            m.shift = true;
        });
        assert_eq!(from_keystroke(&shift_1).as_deref(), Some("Mod+!"));
        assert_eq!(from_keystroke(&typed("shift", None, |_| {})), None);
        // What the console writes, it reads back as the same keys.
        for keystroke in [cmd_k, ctrl_alt_f2, shift_up] {
            let combo = from_keystroke(&keystroke).unwrap();
            let back = Keystroke::parse(&to_keystroke(&combo).unwrap()).unwrap();
            assert_eq!(back.key, keystroke.key, "{combo}");
            assert_eq!(
                back.modifiers.platform || back.modifiers.control,
                keystroke.modifiers.platform || keystroke.modifiers.control
            );
        }
    }

    #[test]
    fn the_plan_binds_what_works_here_and_says_why_not_otherwise() {
        let nav = map(&[
            ("screen:all", "Mod+1"),
            ("screen:system", "Mod+0"),
            ("view:cards", ""),
            ("action:alarms", "Mod+Shift+A"),
            ("screen:7", "Mod+2"),
        ]);
        let tags = map(&[
            ("12", "Alt+F2"),
            ("13", "Alt+F2"),
            ("14", "Mod+K"),
            ("15", "Mod+Bogus"),
        ]);
        let plan = plan(&nav, &tags);
        assert_eq!(
            plan.bound,
            [("alt-f2".to_owned(), Target::Tag(12))],
            "⌘1 is already the Overview's own key"
        );
        assert_eq!(plan.skipped["nav:screen:system"], Skipped::WebOnly);
        assert_eq!(plan.skipped["nav:screen:7"], Skipped::WebOnly);
        assert!(
            !plan.skipped.contains_key("nav:view:cards"),
            "empty is none"
        );
        assert!(!plan.skipped.contains_key("nav:screen:all"));
        assert_eq!(
            plan.skipped["nav:action:alarms"],
            Skipped::Taken("secondary-shift-a".into()),
            "⇧⌘A acknowledges all"
        );
        assert_eq!(plan.skipped["tag:13"], Skipped::Duplicate("alt-f2".into()));
        assert_eq!(
            plan.skipped["tag:14"],
            Skipped::Taken("secondary-k".into()),
            "⌘K is the palette"
        );
        assert_eq!(plan.skipped["tag:15"], Skipped::Unreadable);
        assert_eq!(nav_label("screen:7"), "Screen 7");
        assert_eq!(nav_label("action:alarms"), "Alarms");
    }

    #[test]
    fn a_key_without_a_modifier_is_no_shortcut_but_a_function_key_is() {
        let tags = map(&[
            ("1", "Enter"),
            ("2", " "),
            ("3", "ArrowDown"),
            ("4", "J"),
            ("5", "F7"),
            ("6", "Shift+Enter"),
        ]);
        let plan = plan(&ShortcutMap::new(), &tags);
        for id in 1..=4 {
            assert_eq!(
                plan.skipped[&format!("tag:{id}")],
                Skipped::Unmodified,
                "tag {id}"
            );
        }
        assert_eq!(
            plan.bound,
            [
                ("f7".to_owned(), Target::Tag(5)),
                ("shift-enter".to_owned(), Target::Tag(6)),
            ]
        );
    }

    #[test]
    fn the_keymap_carries_every_command_plus_shortcuts_and_the_kiosk_drops_quit() {
        let plan = plan(&map(&[("action:alarms", "F9")]), &map(&[("12", "Alt+F2")]));
        assert_eq!(plan.bound.len(), 2);
        let base = vt_ui::text_input::key_bindings().len()
            + super::super::text_area::key_bindings().len()
            + keys::key_bindings().len();
        let open = key_bindings(&plan, false);
        assert_eq!(open.len(), base + 2);
        assert!(
            open.iter()
                .any(|binding| binding.action().partial_eq(&ShellOpenTag(12)))
        );
        // ⌘3 from the page registry, plus F9 from the server.
        let to_alarms = |bindings: &[KeyBinding]| {
            bindings
                .iter()
                .filter(|binding| binding.action().partial_eq(&ShowPage(Page::Alarms)))
                .count()
        };
        assert_eq!(to_alarms(&open), to_alarms(&keys::key_bindings()) + 1);
        let locked = key_bindings(&plan, true);
        assert!(locked.len() < open.len());
        assert!(!locked.iter().any(|binding| kiosk_blocks(binding.action())));
        assert!(
            open.iter()
                .any(|binding| binding.action().partial_eq(&Quit))
        );
        let menu_actions = |menus: Vec<gpui::Menu>| -> usize {
            menus
                .iter()
                .flat_map(|menu| menu.items.iter())
                .filter(|item| {
                    matches!(item, gpui::MenuItem::Action { action, .. } if kiosk_blocks(action.as_ref()))
                })
                .count()
        };
        assert!(menu_actions(menus(false)) >= 2);
        assert_eq!(menu_actions(menus(true)), 0);
    }

    #[test]
    fn saving_sends_the_whole_map() {
        let nav = map(&[("screen:all", "Mod+1"), ("view:cards", "")]);
        assert_eq!(
            save_body(&nav, "action:alarms", Some("F9"), true),
            serde_json::json!({"screen:all": "Mod+1", "view:cards": null, "action:alarms": "F9"})
        );
        assert_eq!(
            save_body(&nav, "screen:all", None, true),
            serde_json::json!({"screen:all": null, "view:cards": null}),
            "a cleared target overrides the server's default"
        );
        let tags = map(&[("12", "Alt+F2"), ("13", "")]);
        assert_eq!(
            save_body(&tags, "12", None, false),
            serde_json::json!({}),
            "a cleared tag is dropped"
        );
        assert_eq!(
            save_body(&tags, "14", Some("Mod+Alt+T"), false),
            serde_json::json!({"12": "Alt+F2", "14": "Mod+Alt+T"})
        );
    }
}
