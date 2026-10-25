//! The operator console: one model entity, views as functions over it (the
//! structure Voltrus Tools uses, after muxy).

mod config;
pub mod store;
mod views;

use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, KeyBinding, Menu, MenuItem, OsAction,
    SharedString, Subscription, Task, UniformListScrollHandle, Window, actions,
};
use time::UtcOffset;
use voltrus_scada_native::api::Alarm;
use voltrus_scada_native::runtime::{Command, Event, Handle, Link};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::{self, ThemeCatalog};

use store::{Rate, Store};

/// How often, in seconds, the open alarms are fetched again while signed in.
const ALARM_REFRESH_SECS: u64 = 10;

actions!(
    voltrus_console,
    [
        Quit,
        SignOut,
        ShowOverview,
        ShowTags,
        ShowAlarms,
        ShowTrends,
        AcknowledgeAll,
        NextTag,
        PreviousTag,
        ToggleAppearance,
    ]
);

pub const CONSOLE: &str = "Console";

pub fn key_bindings() -> Vec<KeyBinding> {
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let key = |keys: &str| format!("{primary}-{keys}");
    vec![
        KeyBinding::new(&key("q"), Quit, None),
        KeyBinding::new(&key("1"), ShowOverview, Some(CONSOLE)),
        KeyBinding::new(&key("2"), ShowTags, Some(CONSOLE)),
        KeyBinding::new(&key("3"), ShowAlarms, Some(CONSOLE)),
        KeyBinding::new(&key("4"), ShowTrends, Some(CONSOLE)),
        KeyBinding::new(&key("shift-a"), AcknowledgeAll, Some(CONSOLE)),
        KeyBinding::new(&key("shift-l"), ToggleAppearance, None),
        KeyBinding::new("down", NextTag, Some(CONSOLE)),
        KeyBinding::new("up", PreviousTag, Some(CONSOLE)),
    ]
}

pub fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Voltrus SCADA".into(),
            items: vec![
                MenuItem::action("Sign Out", SignOut),
                MenuItem::separator(),
                MenuItem::action("Quit Voltrus SCADA", Quit),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Cut", vt_ui::text_input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", vt_ui::text_input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", vt_ui::text_input::Paste, OsAction::Paste),
                MenuItem::os_action(
                    "Select All",
                    vt_ui::text_input::SelectAll,
                    OsAction::SelectAll,
                ),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Overview", ShowOverview),
                MenuItem::action("Tags", ShowTags),
                MenuItem::action("Alarms", ShowAlarms),
                MenuItem::action("Trends", ShowTrends),
                MenuItem::separator(),
                MenuItem::action("Light or Dark", ToggleAppearance),
            ],
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Tags,
    Alarms,
    Trends,
}

impl Page {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "overview" => Self::Overview,
            "tags" => Self::Tags,
            "alarms" => Self::Alarms,
            "trends" => Self::Trends,
            _ => return None,
        })
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Tags => "Tags",
            Self::Alarms => "Alarms",
            Self::Trends => "Trends",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    SignIn,
    ChangePassword,
    Console,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlarmFilter {
    Active,
    Unacknowledged,
    Shelved,
}

/// Trend ranges: an id and its label. Five minutes is drawn from the
/// server's shortest history (see [`history_range`]) plus live values.
pub const RANGES: [(&str, &str); 5] = [
    ("5m", "5 min"),
    ("30m", "30 min"),
    ("1h", "1 h"),
    ("6h", "6 h"),
    ("24h", "24 h"),
];

pub fn range_ms(range: &str) -> f64 {
    match range {
        "5m" => 5.0 * 60_000.0,
        "30m" => 30.0 * 60_000.0,
        "6h" => 6.0 * 3_600_000.0,
        "24h" => 24.0 * 3_600_000.0,
        _ => 3_600_000.0,
    }
}

/// The server's `range` parameter that covers a trend range.
pub fn history_range(range: &str) -> &str {
    if range == "5m" { "30m" } else { range }
}

/// A range id as `--range` gives it, if the console offers it.
pub fn parse_range(text: &str) -> Option<&'static str> {
    RANGES.iter().map(|(id, _)| *id).find(|id| *id == text)
}

pub struct Inputs {
    pub url: Entity<TextInput>,
    pub username: Entity<TextInput>,
    pub password: Entity<TextInput>,
    pub new_password: Entity<TextInput>,
    pub confirm_password: Entity<TextInput>,
    pub filter: Entity<TextInput>,
    pub write: Entity<TextInput>,
}

pub struct Toast {
    pub message: SharedString,
    pub error: bool,
    id: u64,
}

/// What `--sign-in` and `--page` set up, for screenshots and kiosks.
pub struct Launch {
    pub sign_in: Option<(String, String, String)>,
    pub page: Option<Page>,
    /// A tag key to open on the Tags page once the tag list arrives.
    pub tag: Option<String>,
    /// The trend range to start with, one of [`RANGES`].
    pub range: Option<&'static str>,
    /// Read before any thread started: `time` cannot ask the system later.
    pub utc_offset: UtcOffset,
}

pub struct Console {
    runtime: Handle,
    pub catalog: Rc<ThemeCatalog>,
    pub phase: Phase,
    pub link: Link,
    pub user: Option<(String, String)>,
    pub server: String,
    /// For showing alarm times on the operator's clock.
    pub utc_offset: UtcOffset,
    pub store: Store,
    pub alarms: Vec<Alarm>,
    /// Why the alarm list could not be fetched last time: it is stale.
    pub alarms_error: Option<String>,
    pub page: Page,
    pub selected: Option<usize>,
    pub trend: BTreeSet<usize>,
    pub range: &'static str,
    pub alarm_filter: AlarmFilter,
    pub inputs: Inputs,
    pub busy: bool,
    pub error: Option<String>,
    pub write_result: Option<(usize, Result<String, String>)>,
    /// The tag whose write awaits the server's answer.
    pub writing: Option<usize>,
    pub rate: Rate,
    pub toast: Option<Toast>,
    toast_seq: u64,
    pub focus: FocusHandle,
    pub tags_scroll: UniformListScrollHandle,
    pub alarms_scroll: UniformListScrollHandle,
    pending_page: Option<Page>,
    pending_tag: Option<String>,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

fn input(
    text: String,
    placeholder: &'static str,
    cx: &mut Context<Console>,
    configure: impl FnOnce(TextInput) -> TextInput,
) -> Entity<TextInput> {
    cx.new(|cx| configure(TextInput::new(cx).text(text).placeholder(placeholder)))
}

impl Console {
    pub fn new(
        runtime: Handle,
        events: async_channel::Receiver<Event>,
        catalog: Rc<ThemeCatalog>,
        launch: Launch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let saved = config::load();
        let inputs = Inputs {
            url: input(
                saved.url.unwrap_or_else(|| "http://localhost:3000".into()),
                "http://scada.plant.local:3000",
                cx,
                TextInput::mono,
            ),
            username: input(
                saved.username.unwrap_or_default(),
                "Username",
                cx,
                |input| input,
            ),
            password: input(String::new(), "Password", cx, TextInput::secure),
            new_password: input(
                String::new(),
                "New password (8 or more characters)",
                cx,
                TextInput::secure,
            ),
            confirm_password: input(
                String::new(),
                "Repeat the new password",
                cx,
                TextInput::secure,
            ),
            filter: input(String::new(), "Filter tags", cx, |input| input),
            write: input(String::new(), "New value", cx, |input| {
                input.kind(InputKind::Number).mono()
            }),
        };

        let pump = cx.spawn(async move |this, cx| {
            while let Ok(first) = events.recv().await {
                let mut batch = vec![first];
                while batch.len() < 2048
                    && let Ok(event) = events.try_recv()
                {
                    batch.push(event);
                }
                let applied = this.update(cx, |console, cx| {
                    for event in batch {
                        console.apply(event, cx);
                    }
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        });

        let mut subscriptions = Vec::new();
        for field in [
            inputs.url.clone(),
            inputs.username.clone(),
            inputs.password.clone(),
        ] {
            subscriptions.push(cx.subscribe_in(
                &field,
                window,
                |console, _, event: &InputEvent, window, cx| {
                    if *event == InputEvent::Submitted {
                        console.sign_in(window, cx);
                    }
                },
            ));
        }
        for field in [inputs.new_password.clone(), inputs.confirm_password.clone()] {
            subscriptions.push(cx.subscribe_in(
                &field,
                window,
                |console, _, event: &InputEvent, _, cx| {
                    if *event == InputEvent::Submitted {
                        console.change_password(cx);
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &inputs.write,
            window,
            |console, _, event: &InputEvent, window, cx| {
                if *event == InputEvent::Submitted {
                    console.write_selected(window, cx);
                }
            },
        ));
        subscriptions.push(
            cx.subscribe(&inputs.filter, |_, _, event: &InputEvent, cx| {
                if *event == InputEvent::Changed {
                    cx.notify();
                }
            }),
        );

        // Once a second: ages and the update rate move on even when no data
        // arrives, and every few seconds the alarms are fetched again, as
        // they resolve on the server without a WebSocket message.
        let ticker = cx.spawn(async move |this, cx| {
            let mut tick: u64 = 0;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                tick += 1;
                let alive = this.update(cx, |console, cx| {
                    if console.phase == Phase::Console {
                        if tick.is_multiple_of(ALARM_REFRESH_SECS) {
                            console.send(Command::RefreshAlarms);
                        }
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let focus = cx.focus_handle();
        let mut console = Self {
            runtime,
            catalog,
            phase: Phase::SignIn,
            link: Link::SignedOut,
            user: None,
            server: String::new(),
            utc_offset: launch.utc_offset,
            store: Store::default(),
            alarms: Vec::new(),
            alarms_error: None,
            page: Page::Overview,
            selected: None,
            trend: BTreeSet::new(),
            range: launch.range.unwrap_or("1h"),
            alarm_filter: AlarmFilter::Active,
            inputs,
            busy: false,
            error: None,
            write_result: None,
            writing: None,
            rate: Rate::default(),
            toast: None,
            toast_seq: 0,
            focus,
            tags_scroll: UniformListScrollHandle::new(),
            alarms_scroll: UniformListScrollHandle::new(),
            pending_page: launch.page,
            pending_tag: launch.tag,
            _tasks: vec![pump, ticker],
            _subscriptions: subscriptions,
        };
        let focus_field = if console.inputs.username.read(cx).value().is_empty() {
            console.inputs.username.clone()
        } else {
            console.inputs.password.clone()
        };
        focus_field.update(cx, |field, _| field.focus(window));
        if let Some((url, username, password)) = launch.sign_in {
            console
                .inputs
                .url
                .update(cx, |field, cx| field.set_text(url, cx));
            console
                .inputs
                .username
                .update(cx, |field, cx| field.set_text(username, cx));
            console
                .inputs
                .password
                .update(cx, |field, cx| field.set_text(password, cx));
            console.sign_in(window, cx);
        }
        console
    }

    fn send(&self, command: Command) {
        self.runtime.send(command);
    }

    // ── Runtime events ────────────────────────────────────────────────────

    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        // What a signed-out session's tasks reported before the runtime
        // heard of the sign-out arrives here before the next sign-in.
        let session_data = !matches!(
            event,
            Event::Link(_)
                | Event::SignedIn { .. }
                | Event::SignInFailed(_)
                | Event::PasswordChanged(_)
        );
        if session_data && self.user.is_none() {
            return;
        }
        match event {
            Event::Link(link) => {
                if let Link::Failed(reason) = &link {
                    // The session ended; the sign-in card says why too.
                    self.busy = false;
                    self.error = Some(reason.clone());
                    if self.phase == Phase::Console {
                        self.show_toast(reason.clone(), true, cx);
                    }
                }
                self.link = link;
            }
            Event::SignedIn {
                username,
                role,
                server,
                must_change_password,
            } => {
                self.busy = false;
                self.error = None;
                config::save(&server, &username);
                self.user = Some((username, role));
                self.server = server;
                self.inputs
                    .password
                    .update(cx, |field, cx| field.set_text("", cx));
                if must_change_password {
                    self.phase = Phase::ChangePassword;
                } else {
                    self.enter_console();
                }
            }
            Event::SignInFailed(error) => {
                self.busy = false;
                self.error = Some(error);
            }
            Event::PasswordChanged(result) => {
                self.busy = false;
                match result {
                    Ok(()) => {
                        self.error = None;
                        for field in [&self.inputs.new_password, &self.inputs.confirm_password] {
                            field.update(cx, |field, cx| field.set_text("", cx));
                        }
                        self.enter_console();
                        self.show_toast("Password changed", false, cx);
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Event::Tags(list) => {
                self.store.replace(list);
                let count = self.store.tags.len();
                self.trend.retain(|index| *index < count);
                if self.trend.is_empty() {
                    self.trend = (0..count.min(4)).collect();
                }
                if self.selected.is_some_and(|index| index >= count) {
                    self.selected = None;
                }
                self.send(Command::Subscribe(self.store.keys()));
                self.send(Command::History(history_range(self.range).to_owned()));
                if let Some(index) = self
                    .pending_tag
                    .take()
                    .and_then(|key| self.store.position(&key))
                {
                    self.open_tag(index, cx);
                }
            }
            Event::Updates(updates) => {
                let applied = self.store.apply(&updates);
                self.rate.record(applied, Instant::now());
            }
            Event::Alarms(alarms) => {
                self.alarms = alarms;
                self.alarms_error = None;
            }
            Event::AlarmsFailed(error) => self.alarms_error = Some(error),
            Event::AlarmRaised(alarm) => {
                let name = self
                    .store
                    .position(&alarm.tag_id)
                    .map_or(alarm.tag_id.clone(), |index| {
                        self.store.tags[index].info.name.clone()
                    });
                let severity = views::capitalized(&alarm.severity);
                let value = vt_ui::chart::format_value(alarm.value);
                self.show_toast(format!("{severity} alarm: {name} at {value}"), true, cx);
            }
            Event::History { range, series } => {
                if range == history_range(self.range) {
                    self.store.backfill(&series);
                }
            }
            Event::Written { tag, result, late } => {
                if let Some(index) = self.store.position(&tag) {
                    // A late answer to an earlier write must not settle the
                    // one now waiting.
                    if late && self.writing == Some(index) {
                        return;
                    }
                    if self.writing == Some(index) {
                        self.writing = None;
                    }
                    self.write_result = Some((index, result));
                }
            }
            Event::Done { action, result } => {
                if let Err(error) = result {
                    self.show_toast(format!("{action} failed: {error}"), true, cx);
                }
            }
        }
    }

    fn enter_console(&mut self) {
        self.phase = Phase::Console;
        if let Some(page) = self.pending_page.take() {
            self.page = page;
        }
    }

    // ── Sign-in ───────────────────────────────────────────────────────────

    pub fn sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self
            .inputs
            .url
            .read(cx)
            .value()
            .trim()
            .trim_end_matches('/')
            .to_owned();
        let username = self.inputs.username.read(cx).value().trim().to_owned();
        let password = self.inputs.password.read(cx).value().to_owned();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            self.error = Some("The server address starts with http:// or https://".into());
        } else if username.is_empty() || password.is_empty() {
            self.error = Some("Enter your username and password".into());
        } else {
            self.error = None;
            self.busy = true;
            self.send(Command::SignIn {
                url,
                username,
                password,
            });
            window.focus(&self.focus);
        }
        cx.notify();
    }

    pub fn change_password(&mut self, cx: &mut Context<Self>) {
        let new = self.inputs.new_password.read(cx).value().to_owned();
        let confirm = self.inputs.confirm_password.read(cx).value().to_owned();
        if new.chars().count() < 8 {
            self.error = Some("Use at least 8 characters".into());
        } else if new != confirm {
            self.error = Some("The two passwords differ".into());
        } else {
            // The runtime knows the current password from signing in.
            self.error = None;
            self.busy = true;
            self.send(Command::ChangePassword {
                current: String::new(),
                new,
            });
        }
        cx.notify();
    }

    pub fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.send(Command::SignOut);
        self.phase = Phase::SignIn;
        self.user = None;
        // A password change still in flight is never answered once the
        // session ends, so nothing else would clear this.
        self.busy = false;
        self.writing = None;
        self.write_result = None;
        self.store = Store::default();
        self.alarms.clear();
        self.alarms_error = None;
        self.selected = None;
        self.trend.clear();
        for field in [&self.inputs.new_password, &self.inputs.confirm_password] {
            field.update(cx, |field, cx| field.set_text("", cx));
        }
        self.inputs
            .password
            .update(cx, |field, _| field.focus(window));
        cx.notify();
    }

    // ── Navigation and selection ──────────────────────────────────────────

    pub fn show(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        cx.notify();
    }

    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.selected != Some(index) {
            self.write_result = None;
            self.inputs
                .write
                .update(cx, |field, cx| field.set_text("", cx));
        }
        self.selected = Some(index);
        cx.notify();
    }

    /// Show one tag on the Tags page, with the filter cleared so that the
    /// list row is the tag's index.
    pub fn open_tag(&mut self, index: usize, cx: &mut Context<Self>) {
        self.inputs
            .filter
            .update(cx, |field, cx| field.set_text("", cx));
        self.page = Page::Tags;
        self.select(index, cx);
        self.tags_scroll
            .scroll_to_item(index, gpui::ScrollStrategy::Center);
    }

    pub fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        let visible = self.visible_tags(cx);
        if visible.is_empty() {
            return;
        }
        let current = self
            .selected
            .and_then(|selected| visible.iter().position(|index| *index == selected));
        let next = match current {
            None => 0,
            Some(position) => {
                let last = isize::try_from(visible.len() - 1).unwrap_or(0);
                let position = isize::try_from(position).unwrap_or(0);
                usize::try_from((position + step).clamp(0, last)).unwrap_or(0)
            }
        };
        self.select(visible[next], cx);
        self.tags_scroll
            .scroll_to_item(next, gpui::ScrollStrategy::Center);
    }

    /// Tags matching the filter, in list order.
    pub fn visible_tags(&self, cx: &gpui::App) -> Vec<usize> {
        let filter = self.inputs.filter.read(cx).value().trim().to_lowercase();
        self.store
            .tags
            .iter()
            .enumerate()
            .filter(|(_, tag)| {
                filter.is_empty()
                    || tag.info.name.to_lowercase().contains(&filter)
                    || tag.info.key.to_lowercase().contains(&filter)
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn toggle_trend(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.trend.remove(&index) {
            self.trend.insert(index);
        }
        cx.notify();
    }

    pub fn set_range(&mut self, range: &'static str, cx: &mut Context<Self>) {
        if self.range != range {
            self.range = range;
            self.send(Command::History(history_range(range).to_owned()));
        }
        cx.notify();
    }

    // ── Commands to the server ────────────────────────────────────────────

    pub fn write_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Enter in the field is not disabled like the button: one write at
        // a time.
        let Some(index) = self.selected.filter(|_| self.writing.is_none()) else {
            return;
        };
        let text = self.inputs.write.read(cx).value().trim().to_owned();
        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => {
                self.write_result = None;
                self.writing = Some(index);
                self.send(Command::Write {
                    key: self.store.tags[index].info.key.clone(),
                    id: self.store.tags[index].info.id,
                    value,
                });
                window.focus(&self.focus);
            }
            _ => self.write_result = Some((index, Err(format!("“{text}” is not a number")))),
        }
        cx.notify();
    }

    pub fn acknowledge(&mut self, alarm: i64) {
        self.send(Command::Acknowledge(alarm));
    }

    pub fn acknowledge_all(&mut self) {
        self.send(Command::AcknowledgeAll);
    }

    pub fn shelve(&mut self, alarm: i64, seconds: u64) {
        self.send(Command::Shelve { alarm, seconds });
    }

    pub fn unshelve(&mut self, alarm: i64) {
        self.send(Command::Unshelve(alarm));
    }

    pub fn shutdown(&self) {
        self.send(Command::Shutdown);
    }

    // ── Appearance and toasts ─────────────────────────────────────────────

    pub fn toggle_appearance(&mut self, cx: &mut Context<Self>) {
        let dark = theme::theme(cx).dark;
        let next = self.catalog.resolve(
            if dark {
                theme::DEFAULT_LIGHT
            } else {
                theme::DEFAULT_DARK
            },
            !dark,
        );
        theme::set_theme(next, cx);
    }

    pub fn show_toast(
        &mut self,
        message: impl Into<SharedString>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        self.toast_seq += 1;
        let id = self.toast_seq;
        self.toast = Some(Toast {
            message: message.into(),
            error,
            id,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            let _ = this.update(cx, |console, cx| {
                if console.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    console.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Positions in `alarms` of the chosen filter's alarms, most severe
    /// first.
    pub fn filtered_alarms(&self) -> Vec<usize> {
        let mut alarms: Vec<usize> = self
            .alarms
            .iter()
            .enumerate()
            .filter(|(_, alarm)| {
                let shelved = is_shelved(alarm);
                match self.alarm_filter {
                    AlarmFilter::Active => !shelved,
                    AlarmFilter::Unacknowledged => !shelved && alarm.acknowledged_at.is_none(),
                    AlarmFilter::Shelved => shelved,
                }
            })
            .map(|(index, _)| index)
            .collect();
        alarms.sort_by_key(|index| {
            let alarm = &self.alarms[*index];
            (
                severity_rank(&alarm.severity),
                std::cmp::Reverse(alarm.triggered_at),
            )
        });
        alarms
    }

    fn has_role(&self, roles: &[&str]) -> bool {
        self.user.as_ref().is_some_and(|(_, role)| {
            roles
                .iter()
                .any(|allowed| role.eq_ignore_ascii_case(allowed))
        })
    }

    /// The server lets only admins write tags.
    pub fn can_write(&self) -> bool {
        self.has_role(&["admin"])
    }

    /// The server's shelf routes take admin, demo and integration users.
    pub fn can_shelve(&self) -> bool {
        self.has_role(&["admin", "demo", "integration"])
    }

    /// Open alarms nobody has acknowledged yet: the sidebar badge.
    pub fn unacknowledged(&self) -> usize {
        self.alarms
            .iter()
            .filter(|alarm| !is_shelved(alarm) && alarm.acknowledged_at.is_none())
            .count()
    }
}

pub fn is_shelved(alarm: &Alarm) -> bool {
    alarm.state == "shelved"
        || alarm
            .shelved_until
            .is_some_and(|until| until > time_now_secs())
}

pub fn severity_rank(severity: &str) -> u8 {
    match severity.to_ascii_lowercase().as_str() {
        "critical" => 0,
        "high" | "major" => 1,
        "warning" | "medium" => 2,
        _ => 3,
    }
}

pub fn time_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}
