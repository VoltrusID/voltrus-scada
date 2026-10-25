//! The shell around the pages: preferences (theme, language, alarm
//! cooldown, history aggregation, kiosk lock, OS notifications), the
//! server-kept shortcuts, user notifications, the edition and sign-in
//! methods, and the hooks the console calls (sign-in, sign-out, the
//! ticker, replies). The views are `views::{settings, notifications,
//! edition}`; the state is [`Shell`] in `Console::state`.

use std::any::TypeId;
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AppContext as _, Bounds, Context, Entity, Keystroke, Pixels, Stateful, Subscription, Window, px,
};
use serde::de::IgnoredAny;
use time::OffsetDateTime;
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::shell::{
    AlarmCooldown, AuthProviders, LicenseStatus, MAX_COOLDOWN_SECS, ShortcutMap, ThemeMode, UiLang,
    UiTheme, UserNotification, UserNotifications,
};
use voltrus_scada_native::runtime::{Call, CallKey, Command, RaisedAlarm, Reply};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::{self, Theme};

use super::keys::{
    Quit, ShellEditShortcuts, ShellOpenSettings, ShellPickTheme, ShellToggleNotifications, SignOut,
    hint,
};
use super::overlay::Overlay;
use super::prefs::{self, Prefs};
use super::shortcuts::{self, Entry, Plan, ShellOpenTag};
use super::{Console, Phase, severity_rank, views};

/// How often, in seconds, the user's notifications are fetched again (the
/// web client polls every 30 s).
const NOTIFICATIONS_REFRESH_SECS: u64 = 30;

/// Whether this platform shows the console's system notifications.
pub const OS_NOTIFICATIONS: bool = cfg!(target_os = "macos");

/// One bundled theme as the picker shows it.
pub struct ThemeEntry {
    pub name: String,
    pub dark: bool,
    pub swatches: [gpui::Hsla; 5],
}

/// The theme picker while it is open: which brightness it chooses for.
pub struct Picker {
    pub dark: bool,
    pub filter: Entity<TextInput>,
    _subscription: Subscription,
}

/// The shortcut editor's search field.
pub struct Editor {
    pub filter: Entity<TextInput>,
    _subscription: Subscription,
}

#[derive(Default)]
pub struct Shell {
    pub prefs: Prefs,
    /// `--appearance`: the server's theme is not applied until the
    /// operator picks one.
    pub pinned: Option<bool>,
    /// The server's appearance for the user, once known.
    pub mode: Option<ThemeMode>,
    pub lang: Option<String>,
    pub cooldown: Option<i64>,
    pub cooldown_input: Option<Entity<TextInput>>,
    /// Unread first, then newest first.
    pub notifications: Vec<UserNotification>,
    pub notifications_error: Option<String>,
    pub license: Option<LicenseStatus>,
    pub providers: Option<AuthProviders>,
    pub providers_error: Option<String>,
    probe_seq: i64,
    probed: String,
    pub nav: ShortcutMap,
    pub tags: ShortcutMap,
    pub plan: Plan,
    /// The editor row waiting for keys.
    pub recording: Option<Entry>,
    recorder: Option<Subscription>,
    pub editor: Option<Editor>,
    pub picker: Option<Picker>,
    themes: Vec<ThemeEntry>,
    pub bell_anchor: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Alarms already announced to the system.
    notified: HashSet<i64>,
    subscriptions: Vec<Subscription>,
}

impl Console {
    /// The kiosk lock: quitting, signing out and closing the window are
    /// blocked.
    pub fn kiosk_locked(&self) -> bool {
        self.states
            .get(&TypeId::of::<Shell>())
            .and_then(|state| state.downcast_ref::<Shell>())
            .is_some_and(|shell| shell.prefs.kiosk)
    }

    /// Unread user notifications: the bell's badge.
    pub fn unread_notifications(&self) -> usize {
        self.states
            .get(&TypeId::of::<Shell>())
            .and_then(|state| state.downcast_ref::<Shell>())
            .map_or(0, |shell| {
                shell.notifications.iter().filter(|n| !n.read).count()
            })
    }
}

// ── Hooks the console calls ───────────────────────────────────────────────

/// Once, as the console starts: local preferences, fields, the saved
/// theme, the keymap, and the sign-in card's look at the server.
pub fn init(
    console: &mut Console,
    appearance: Option<bool>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let cooldown_input = cx.new(|cx| {
        TextInput::new(cx)
            .kind(InputKind::Number)
            .mono()
            .placeholder("300")
    });
    let mut subscriptions = vec![
        cx.subscribe_in(
            &cooldown_input,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => save_cooldown(console, cx),
                InputEvent::Cancelled => console.close_overlay(window, cx),
                _ => {}
            },
        ),
        cx.subscribe_in(
            &console.inputs.url.clone(),
            window,
            |console, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Blurred | InputEvent::Submitted) {
                    probe(console, cx);
                }
            },
        ),
        // A theme on screen for a look goes when its picker closes some
        // other way than choosing (a click outside); so does a shortcut
        // recording when its editor closes.
        cx.observe_self(|console, cx| {
            if console.state::<Shell>().picker.is_some() && !console.overlay_is("themes") {
                console.state::<Shell>().picker = None;
                apply_theme(console, cx);
            }
            if console.state::<Shell>().recording.is_some() && !console.overlay_is("shortcuts") {
                stop_recording(console, cx);
            }
        }),
        cx.observe_window_appearance(window, |console, _, cx| {
            let shell = console.state::<Shell>();
            if shell.pinned.is_none() && shell.mode == Some(ThemeMode::System) {
                apply_theme(console, cx);
            }
        }),
    ];
    let shell = console.state::<Shell>();
    shell.prefs = prefs::load();
    shell.pinned = appearance;
    shell.cooldown_input = Some(cooldown_input);
    shell.subscriptions.append(&mut subscriptions);
    // `main` started on the house theme of the right brightness; show the
    // one chosen for it.
    apply_theme(console, cx);
    shortcuts::rebind(console, cx);
    probe(console, cx);
}

/// Signed in: the user's preferences, shortcuts and notifications, and the
/// server's edition.
pub fn signed_in(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        return;
    }
    console.call(
        CallKey::new("shell.theme"),
        Call::get::<UiTheme>("/api/v1/settings/ui-theme"),
    );
    console.call(
        CallKey::new("shell.lang"),
        Call::get::<UiLang>("/api/v1/settings/ui-lang"),
    );
    fetch_cooldown(console);
    refresh_notifications(console);
    shortcuts::reload(console);
    let server = console.server.clone();
    probe_at(console, server, cx);
}

/// Signed out: the user's data goes, the keymap loses their shortcuts.
pub fn signed_out(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let shell = console.state::<Shell>();
    shell.notifications.clear();
    shell.notifications_error = None;
    shell.nav.clear();
    shell.tags.clear();
    shell.mode = None;
    shell.lang = None;
    shell.cooldown = None;
    shell.picker = None;
    shell.editor = None;
    stop_recording(console, cx);
    shortcuts::rebind(console, cx);
}

/// Every second while in the console.
pub fn tick(console: &mut Console, tick: u64, _: &mut Context<Console>) {
    if tick.is_multiple_of(NOTIFICATIONS_REFRESH_SECS) && console.stress.is_none() {
        refresh_notifications(console);
    }
}

/// A new alarm: a system notification when it is critical and the
/// operator asked for them.
pub fn alarm_raised(console: &mut Console, alarm: &RaisedAlarm, _: &mut Context<Console>) {
    let name = console
        .store
        .by_key(&alarm.tag_id)
        .map_or(alarm.tag_id.clone(), |index| {
            console.store.tags[index].info.name.clone()
        });
    let shell = console.state::<Shell>();
    let wanted =
        OS_NOTIFICATIONS && shell.prefs.os_notifications && severity_rank(&alarm.severity) == 0;
    // Each alarm once, even if the server raises it again.
    if !(wanted && shell.notified.insert(alarm.alarm_id)) {
        return;
    }
    let value = vt_ui::chart::format_value(alarm.value);
    os_notify(
        &format!("Critical alarm: {name}"),
        &if alarm.message.is_empty() {
            format!("Value {value}")
        } else {
            format!("{} (value {value})", alarm.message)
        },
    );
}

/// The answer to a call under `shell.*`.
pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "shell.theme" => {
            if let Ok(answer) = reply.take::<UiTheme>() {
                let shell = console.state::<Shell>();
                shell.mode = Some(ThemeMode::parse(&answer.theme));
                apply_theme(console, cx);
            }
        }
        "shell.theme.save" => {
            if let Err(error) = reply.take::<UiTheme>() {
                console.show_toast(format!("Saving the appearance failed: {error}"), true, cx);
            }
        }
        "shell.lang" | "shell.lang.save" => match reply.take::<UiLang>() {
            Ok(answer) => console.state::<Shell>().lang = Some(answer.lang),
            Err(error) if key.name == "shell.lang.save" => {
                console.show_toast(format!("Saving the language failed: {error}"), true, cx);
            }
            Err(_) => {}
        },
        "shell.cooldown" | "shell.cooldown.save" => match reply.take::<AlarmCooldown>() {
            Ok(answer) => {
                let shell = console.state::<Shell>();
                shell.cooldown = Some(answer.secs);
                if let Some(input) = shell.cooldown_input.clone() {
                    input.update(cx, |field, cx| field.set_text(answer.secs.to_string(), cx));
                }
                if key.name == "shell.cooldown.save" {
                    console.show_toast(
                        format!("Alarm cooldown set to {} s", answer.secs),
                        false,
                        cx,
                    );
                }
            }
            Err(error) if key.name == "shell.cooldown.save" => {
                console.show_toast(
                    format!("Saving the alarm cooldown failed: {error}"),
                    true,
                    cx,
                );
            }
            Err(_) => {}
        },
        "shell.notifications" => match reply.take::<UserNotifications>() {
            Ok(mut answer) => {
                sort_notifications(&mut answer.notifications);
                let shell = console.state::<Shell>();
                shell.notifications = answer.notifications;
                shell.notifications_error = None;
            }
            Err(error) => console.state::<Shell>().notifications_error = Some(error),
        },
        "shell.notifications.read" => match reply.take::<IgnoredAny>() {
            Ok(_) => {
                let shell = console.state::<Shell>();
                if let Some(notification) = shell
                    .notifications
                    .iter_mut()
                    .find(|notification| notification.id == key.arg)
                {
                    notification.read = true;
                }
                sort_notifications(&mut shell.notifications);
            }
            Err(error) => console.show_toast(format!("Marking read failed: {error}"), true, cx),
        },
        "shell.notifications.dismiss" => match reply.take::<IgnoredAny>() {
            Ok(_) => console
                .state::<Shell>()
                .notifications
                .retain(|notification| notification.id != key.arg),
            Err(error) => console.show_toast(format!("Dismissing failed: {error}"), true, cx),
        },
        "shell.nav_shortcuts" | "shell.tag_shortcuts" => match reply.take::<ShortcutMap>() {
            Ok(map) => {
                let shell = console.state::<Shell>();
                if key.name == "shell.nav_shortcuts" {
                    shell.nav = map;
                } else {
                    shell.tags = map;
                }
                shortcuts::rebind(console, cx);
            }
            Err(error) => {
                console.show_toast(format!("Loading shortcuts failed: {error}"), true, cx)
            }
        },
        "shell.nav_shortcuts.save" | "shell.tag_shortcuts.save" => {
            match reply.take::<IgnoredAny>() {
                Ok(_) => shortcuts::reload(console),
                Err(error) => {
                    console.show_toast(format!("Saving the shortcut failed: {error}"), true, cx);
                }
            }
        }
        _ => {}
    }
    cx.notify();
}

/// The answer to a public call (the sign-in card's look at the server).
pub fn on_public_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    let shell = console.state::<Shell>();
    if key.arg != shell.probe_seq {
        return;
    }
    match key.name {
        "shell.providers" => match reply.take::<AuthProviders>() {
            Ok(providers) => {
                shell.providers = Some(providers);
                shell.providers_error = None;
            }
            Err(error) => {
                shell.providers = None;
                shell.providers_error = Some(error);
            }
        },
        "shell.license" => shell.license = reply.take::<LicenseStatus>().ok(),
        _ => {}
    }
    cx.notify();
}

/// The shell's commands, on the console's root.
pub fn actions(root: Stateful<gpui::Div>, cx: &mut Context<Console>) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|console, _: &ShellOpenSettings, window, cx| {
        if console.overlay_is("settings") {
            console.close_overlay(window, cx);
        } else if console.phase == Phase::Console {
            open_settings(console, window, cx);
        }
    }))
    .on_action(
        cx.listener(|console, _: &ShellToggleNotifications, window, cx| {
            if console.overlay_is("notifications") {
                console.close_overlay(window, cx);
            } else if console.phase == Phase::Console {
                open_notifications(console, window, cx);
            }
        }),
    )
    .on_action(cx.listener(|console, _: &ShellEditShortcuts, window, cx| {
        if console.phase == Phase::Console {
            open_shortcuts(console, window, cx);
        }
    }))
    .on_action(cx.listener(|console, _: &ShellPickTheme, window, cx| {
        if console.phase == Phase::Console {
            let dark = theme::theme(cx).dark;
            open_picker(console, dark, window, cx);
        }
    }))
    .on_action(cx.listener(|console, action: &ShellOpenTag, window, cx| {
        if console.phase != Phase::Console {
            return;
        }
        if console.store.position(action.0).is_some() {
            console.open_tag(action.0, window, cx);
        } else {
            console.show_toast(format!("No tag {} on this server", action.0), true, cx);
        }
    }))
    // The kiosk lock: ⌘Q, the menus and the palette all end here first.
    .capture_action(cx.listener(|console, _: &Quit, _, cx| {
        if console.kiosk_locked() {
            cx.stop_propagation();
            kiosk_refused(console, cx);
        }
    }))
    .capture_action(cx.listener(|console, _: &SignOut, _, cx| {
        if console.kiosk_locked() && console.phase == Phase::Console {
            cx.stop_propagation();
            kiosk_refused(console, cx);
        }
    }))
}

fn kiosk_refused(console: &mut Console, cx: &mut Context<Console>) {
    console.show_toast(
        format!(
            "The kiosk lock is on: turn it off in Settings ({})",
            hint(&ShellOpenSettings).unwrap_or_default()
        ),
        true,
        cx,
    );
}

// ── Sign-in card: sign-in methods and edition ─────────────────────────────

/// Ask the server in the sign-in card's address field for its sign-in
/// methods and edition, once per address.
pub fn probe(console: &mut Console, cx: &mut Context<Console>) {
    let url = console
        .inputs
        .url
        .read(cx)
        .value()
        .trim()
        .trim_end_matches('/')
        .to_owned();
    if console.state::<Shell>().probed != url {
        probe_at(console, url, cx);
    }
}

fn probe_at(console: &mut Console, base: String, cx: &mut Context<Console>) {
    if !(base.starts_with("http://") || base.starts_with("https://")) || console.stress.is_some() {
        return;
    }
    let shell = console.state::<Shell>();
    shell.probe_seq += 1;
    shell.probed.clone_from(&base);
    let seq = shell.probe_seq;
    console.send(Command::Public {
        base: base.clone(),
        key: CallKey::new("shell.providers").with(seq),
        call: Call::new(|api| async move {
            api.get_public::<AuthProviders>("/api/v1/auth/providers")
                .await
        }),
    });
    console.send(Command::Public {
        base,
        key: CallKey::new("shell.license").with(seq),
        call: Call::new(|api| async move {
            api.get_public::<LicenseStatus>("/api/license/status").await
        }),
    });
    cx.notify();
}

// ── Appearance ────────────────────────────────────────────────────────────

/// Whether the console should be dark: the server's appearance unless
/// `--appearance` pinned one, else what is on screen.
fn wanted_dark(shell: &Shell, system_dark: bool, now_dark: bool) -> bool {
    match (shell.pinned, shell.mode) {
        (Some(pinned), _) => pinned,
        (None, Some(mode)) => mode.is_dark(system_dark),
        (None, None) => now_dark,
    }
}

/// Show the theme chosen for the wanted brightness.
pub fn apply_theme(console: &mut Console, cx: &mut Context<Console>) {
    let system_dark = theme::is_dark(cx.window_appearance());
    let now_dark = theme::theme(cx).dark;
    let shell = console.state::<Shell>();
    let dark = wanted_dark(shell, system_dark, now_dark);
    let name = shell.prefs.theme_for(dark).to_owned();
    show_theme(console.catalog.resolve(&name, dark), cx);
}

fn show_theme(next: Theme, cx: &mut Context<Console>) {
    if theme::theme(cx).name != next.name {
        theme::set_theme(next, cx);
    }
}

/// The operator picks an appearance: shown at once, kept on the server.
pub fn set_mode(console: &mut Console, mode: ThemeMode, cx: &mut Context<Console>) {
    let shell = console.state::<Shell>();
    shell.mode = Some(mode);
    shell.pinned = None;
    apply_theme(console, cx);
    if console.is_signed_in() && console.stress.is_none() {
        console.call(
            CallKey::new("shell.theme.save"),
            Call::send::<UiTheme>(
                Method::PUT,
                "/api/v1/settings/ui-theme",
                serde_json::json!({ "theme": mode.as_str() }),
            ),
        );
    }
    cx.notify();
}

/// ⇧⌘L and the navbar's sun and moon: the other brightness, kept.
pub fn toggle_appearance(console: &mut Console, cx: &mut Context<Console>) {
    let mode = if theme::theme(cx).dark {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    set_mode(console, mode, cx);
}

/// Every bundled theme with its brightness and a few colours, built once.
pub fn themes(console: &mut Console) -> &[ThemeEntry] {
    let catalog = console.catalog.clone();
    let shell = console.state::<Shell>();
    if shell.themes.is_empty() {
        shell.themes = catalog
            .names()
            .filter_map(|name| catalog.get(name))
            .map(|theme| ThemeEntry {
                swatches: [
                    theme.bg,
                    theme.surface,
                    theme.accent,
                    theme.warning,
                    theme.danger,
                ],
                dark: theme.dark,
                name: theme.name,
            })
            .collect();
    }
    &console.state::<Shell>().themes
}

/// The picker's themes: those of its brightness matching its search, as
/// positions in [`themes`].
pub fn picker_rows(console: &mut Console, cx: &gpui::App) -> Vec<usize> {
    let Some((dark, query)) = console.state::<Shell>().picker.as_ref().map(|picker| {
        (
            picker.dark,
            picker.filter.read(cx).value().trim().to_lowercase(),
        )
    }) else {
        return Vec::new();
    };
    themes(console)
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.dark == dark)
        .filter(|(_, entry)| query.is_empty() || entry.name.to_lowercase().contains(&query))
        .map(|(index, _)| index)
        .collect()
}

/// Open the theme picker for the dark (or light) theme.
pub fn open_picker(
    console: &mut Console,
    dark: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let filter = cx.new(|cx| TextInput::new(cx).placeholder("Find a theme  /"));
    let subscription = cx.subscribe_in(
        &filter,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                console.list(views::settings::THEMES_LIST, cx).cursor = Some(0);
                cx.notify();
            }
            InputEvent::Submitted => {
                let row = console.list(views::settings::THEMES_LIST, cx).cursor;
                if let Some(row) = row {
                    choose_theme(console, row, window, cx);
                }
            }
            InputEvent::Cancelled => cancel_picker(console, window, cx),
            InputEvent::Blurred => {}
        },
    );
    console.state::<Shell>().picker = Some(Picker {
        dark,
        filter,
        _subscription: subscription,
    });
    let overlay = Overlay::modal("themes", px(560.0), views::settings::themes, cx);
    console.open_overlay(overlay, window, cx);
    // The cursor starts on the theme chosen now.
    let chosen = console.state::<Shell>().prefs.theme_for(dark).to_owned();
    let rows = picker_rows(console, cx);
    let all = themes(console);
    let row = rows
        .iter()
        .position(|index| all[*index].name == chosen)
        .unwrap_or(0);
    let nav = console.list(views::settings::THEMES_LIST, cx);
    nav.set_len(rows.len());
    nav.select(row);
    window.focus(&nav.focus);
}

/// The cursor is on picker row `row`: show that theme for a look.
pub fn preview_theme(console: &mut Console, row: usize, cx: &mut Context<Console>) {
    let rows = picker_rows(console, cx);
    let Some(index) = rows.get(row).copied() else {
        return;
    };
    let name = themes(console)[index].name.clone();
    if let Some(theme) = console.catalog.get(&name) {
        show_theme(theme, cx);
    }
    cx.notify();
}

/// Choose picker row `row` for its brightness, and go back to the
/// settings.
pub fn choose_theme(
    console: &mut Console,
    row: usize,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let rows = picker_rows(console, cx);
    let Some(index) = rows.get(row).copied() else {
        return;
    };
    let name = themes(console)[index].name.clone();
    let shell = console.state::<Shell>();
    let Some(dark) = shell.picker.take().map(|picker| picker.dark) else {
        return;
    };
    if dark {
        shell.prefs.dark_theme = name;
    } else {
        shell.prefs.light_theme = name;
    }
    prefs::save(&shell.prefs);
    apply_theme(console, cx);
    open_settings(console, window, cx);
}

/// Leave the picker without choosing: the theme before it comes back.
pub fn cancel_picker(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.state::<Shell>().picker = None;
    apply_theme(console, cx);
    open_settings(console, window, cx);
}

// ── Settings ──────────────────────────────────────────────────────────────

/// ⌘,: the settings sheet.
pub fn open_settings(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let overlay = Overlay::sheet("settings", px(480.0), views::settings::render, cx);
    console.open_overlay(overlay, window, cx);
    if console.is_signed_in() && console.stress.is_none() {
        // Another operator may have changed it.
        fetch_cooldown(console);
    }
}

fn fetch_cooldown(console: &Console) {
    console.call(
        CallKey::new("shell.cooldown"),
        Call::get::<AlarmCooldown>("/api/v1/settings/alarm-cooldown"),
    );
}

pub fn set_lang(console: &mut Console, lang: &'static str, cx: &mut Context<Console>) {
    console.call(
        CallKey::new("shell.lang.save"),
        Call::send::<UiLang>(
            Method::PUT,
            "/api/v1/settings/ui-lang",
            serde_json::json!({ "lang": lang }),
        ),
    );
    // Shown at once; a refusal puts the toast up.
    console.state::<Shell>().lang = Some(lang.to_owned());
    cx.notify();
}

/// A cooldown as typed, if the server would keep it as typed.
pub fn parse_cooldown(text: &str) -> Result<i64, String> {
    match text.trim().parse::<i64>() {
        Ok(secs) if (0..=MAX_COOLDOWN_SECS).contains(&secs) => Ok(secs),
        Ok(_) => Err(format!("Between 0 and {MAX_COOLDOWN_SECS} seconds")),
        Err(_) => Err("A whole number of seconds".into()),
    }
}

pub fn save_cooldown(console: &mut Console, cx: &mut Context<Console>) {
    if !console.is_operator() {
        return;
    }
    let Some(input) = console.state::<Shell>().cooldown_input.clone() else {
        return;
    };
    match parse_cooldown(input.read(cx).value()) {
        Ok(secs) => console.call(
            CallKey::new("shell.cooldown.save"),
            Call::send::<AlarmCooldown>(
                Method::PUT,
                "/api/v1/settings/alarm-cooldown",
                serde_json::json!({ "secs": secs }),
            ),
        ),
        Err(error) => console.show_toast(format!("Alarm cooldown: {error}"), true, cx),
    }
}

pub fn set_history_agg(console: &mut Console, agg: &'static str, cx: &mut Context<Console>) {
    let shell = console.state::<Shell>();
    agg.clone_into(&mut shell.prefs.history_agg);
    prefs::save(&shell.prefs);
    cx.notify();
}

pub fn set_kiosk(console: &mut Console, on: bool, cx: &mut Context<Console>) {
    let shell = console.state::<Shell>();
    shell.prefs.kiosk = on;
    prefs::save(&shell.prefs);
    shortcuts::rebind(console, cx);
    console.show_toast(
        if on {
            "Kiosk lock on: quitting, signing out and closing the window are blocked"
        } else {
            "Kiosk lock off"
        },
        false,
        cx,
    );
}

pub fn set_os_notifications(console: &mut Console, on: bool, cx: &mut Context<Console>) {
    let shell = console.state::<Shell>();
    shell.prefs.os_notifications = on;
    prefs::save(&shell.prefs);
    if on {
        // One now, so the system asks for permission while the operator
        // looks.
        os_notify(
            "Voltrus SCADA",
            "Critical alarms will show here while the console runs.",
        );
    }
    cx.notify();
}

// ── Shortcut editor ───────────────────────────────────────────────────────

/// The shortcut editor: navigation targets and every tag.
pub fn open_shortcuts(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let filter = cx.new(|cx| TextInput::new(cx).placeholder("Find a target or tag  /"));
    let subscription = cx.subscribe_in(
        &filter,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                console.list(views::settings::SHORTCUTS_LIST, cx).cursor = Some(0);
                cx.notify();
            }
            InputEvent::Submitted | InputEvent::Cancelled => {
                console.focus_list(views::settings::SHORTCUTS_LIST, window, cx);
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    );
    console.state::<Shell>().editor = Some(Editor {
        filter,
        _subscription: subscription,
    });
    let overlay = Overlay::modal("shortcuts", px(680.0), views::settings::shortcuts, cx);
    console.open_overlay(overlay, window, cx);
    console.focus_list(views::settings::SHORTCUTS_LIST, window, cx);
    if console.is_signed_in() && console.stress.is_none() {
        shortcuts::reload(console);
    }
}

/// Wait for the next keys typed and make them `entry`'s shortcut: Esc
/// cancels, Backspace or Delete clears.
pub fn record(console: &mut Console, entry: Entry, cx: &mut Context<Console>) {
    stop_recording(console, cx);
    let weak = cx.weak_entity();
    let recorder = cx.intercept_keystrokes(move |event, _, cx| {
        cx.stop_propagation();
        let keystroke = event.keystroke.clone();
        let _ = weak.update(cx, |console, cx| recorded(console, &keystroke, cx));
    });
    let shell = console.state::<Shell>();
    shell.recording = Some(entry);
    shell.recorder = Some(recorder);
    cx.notify();
}

fn recorded(console: &mut Console, keystroke: &Keystroke, cx: &mut Context<Console>) {
    let Some(entry) = console.state::<Shell>().recording.clone() else {
        return;
    };
    let plain = !(keystroke.modifiers.control
        || keystroke.modifiers.platform
        || keystroke.modifiers.alt
        || keystroke.modifiers.shift);
    match keystroke.key.as_str() {
        "escape" if plain => {}
        "backspace" | "delete" if plain => shortcuts::save(console, &entry, None, cx),
        _ => match shortcuts::from_keystroke(keystroke) {
            Some(combo) => shortcuts::save(console, &entry, Some(&combo), cx),
            // A lone modifier: keep waiting.
            None => return,
        },
    }
    stop_recording(console, cx);
}

pub fn stop_recording(console: &mut Console, cx: &mut Context<Console>) {
    let shell = console.state::<Shell>();
    shell.recording = None;
    // Not dropped inside its own callback.
    if let Some(recorder) = shell.recorder.take() {
        cx.defer(move |_| drop(recorder));
    }
    cx.notify();
}

// ── Notifications ─────────────────────────────────────────────────────────

/// Unread first, then newest first.
pub fn sort_notifications(list: &mut [UserNotification]) {
    list.sort_by_key(|notification| {
        (
            notification.read,
            std::cmp::Reverse(notification.created_at),
            std::cmp::Reverse(notification.id),
        )
    });
}

fn refresh_notifications(console: &Console) {
    if console.is_signed_in() {
        console.call(
            CallKey::new("shell.notifications"),
            Call::get::<UserNotifications>("/api/v1/user-notifications"),
        );
    }
}

/// Where the bell's popover opens: right-aligned under the bell.
pub fn popover_at(bell: Bounds<Pixels>) -> gpui::Point<Pixels> {
    gpui::point(
        bell.right() - views::notifications::WIDTH,
        bell.bottom() + px(4.0),
    )
}

/// The bell's popover, below the bell (where the navbar drew it last; the
/// bell moves it there if it opens before the navbar was drawn).
pub fn open_notifications(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let width = views::notifications::WIDTH;
    let at = console.state::<Shell>().bell_anchor.get().map_or_else(
        || {
            gpui::point(
                window.viewport_size().width - width - px(12.0),
                theme::TITLE_BAR_HEIGHT,
            )
        },
        popover_at,
    );
    let overlay = Overlay::popover(
        "notifications",
        width,
        at,
        views::notifications::popover,
        cx,
    );
    console.open_overlay(overlay, window, cx);
    refresh_notifications(console);
    console.focus_list(views::notifications::LIST, window, cx);
}

pub fn mark_read(console: &Console, id: i64) {
    console.call(
        CallKey::new("shell.notifications.read").with(id),
        Call::send_empty::<IgnoredAny>(
            Method::PUT,
            format!("/api/v1/user-notifications/{id}/read"),
        ),
    );
}

pub fn mark_all_read(console: &mut Console) {
    let unread: Vec<i64> = console
        .state::<Shell>()
        .notifications
        .iter()
        .filter(|notification| !notification.read)
        .map(|notification| notification.id)
        .collect();
    for id in unread {
        mark_read(console, id);
    }
}

pub fn dismiss(console: &Console, id: i64) {
    console.call(
        CallKey::new("shell.notifications.dismiss").with(id),
        Call::send_empty::<IgnoredAny>(Method::DELETE, format!("/api/v1/user-notifications/{id}")),
    );
}

// ── System notifications and the clock ────────────────────────────────────

/// An AppleScript string literal of `text`.
fn applescript_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A notification in the system's notification centre (macOS, through
/// `osascript`: no extra dependency and no entitlement).
fn os_notify(title: &str, body: &str) {
    if !OS_NOTIFICATIONS {
        return;
    }
    let script = format!(
        "display notification {} with title {}",
        applescript_string(body),
        applescript_string(title)
    );
    let spawned = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        // Reaped off the UI thread.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => eprintln!("voltrus-native: no system notification: {error}"),
    }
}

/// The status bar's clock on the operator's time: `Sat 25 Oct 14:05:09`.
pub fn clock(console: &Console) -> String {
    let now = OffsetDateTime::now_utc().to_offset(console.utc_offset);
    clock_text(now)
}

fn clock_text(now: OffsetDateTime) -> String {
    let weekday = now.weekday().to_string();
    let month = now.month().to_string();
    format!(
        "{} {} {} {:02}:{:02}:{:02}",
        weekday.get(..3).unwrap_or(&weekday),
        now.day(),
        month.get(..3).unwrap_or(&month),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_appearance_applies_unless_pinned() {
        let mut shell = Shell::default();
        assert!(
            wanted_dark(&shell, false, true),
            "unknown: what is on screen"
        );
        shell.mode = Some(ThemeMode::Light);
        assert!(!wanted_dark(&shell, true, true));
        shell.mode = Some(ThemeMode::System);
        assert!(wanted_dark(&shell, true, false) && !wanted_dark(&shell, false, true));
        shell.pinned = Some(true);
        assert!(wanted_dark(&shell, false, false), "--appearance dark wins");
    }

    #[test]
    fn cooldowns_stay_within_what_the_server_keeps() {
        assert_eq!(parse_cooldown(" 300 "), Ok(300));
        assert_eq!(parse_cooldown("0"), Ok(0));
        assert!(parse_cooldown("86401").is_err());
        assert!(parse_cooldown("-1").is_err());
        assert!(parse_cooldown("5 min").is_err());
    }

    #[test]
    fn notifications_list_unread_first_then_newest() {
        let note = |id, read, created_at| UserNotification {
            id,
            read,
            created_at,
            ..UserNotification::default()
        };
        let mut list = vec![
            note(1, true, 50),
            note(2, false, 10),
            note(3, false, 40),
            note(4, true, 60),
        ];
        sort_notifications(&mut list);
        let ids: Vec<i64> = list.iter().map(|n| n.id).collect();
        assert_eq!(ids, [3, 2, 4, 1]);
    }

    #[test]
    fn system_notification_text_is_quoted_for_applescript() {
        assert_eq!(
            applescript_string(r#"Tank "A" \ high"#),
            r#""Tank \"A\" \\ high""#
        );
    }

    #[test]
    fn the_clock_reads_day_date_and_time() {
        let at = OffsetDateTime::from_unix_timestamp(1_761_379_509).unwrap();
        assert_eq!(clock_text(at), "Sat 25 Oct 08:05:09");
    }
}
