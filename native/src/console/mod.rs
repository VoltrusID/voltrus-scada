//! The operator console: one model entity, [`Console`]; views are functions
//! over it, and each part of the window (navbar, page, status bar, toasts,
//! overlay, HUD) is drawn by its own cached view (`views::cached`), so an
//! animation redraws only its part. `native/docs/architecture.md` explains
//! how to add pages, REST calls, keys, overlays and animations.

mod anim;
mod config;
pub mod keys;
mod meter;
mod nav;
pub mod overlay;
pub mod pages;
pub mod store;
mod views;

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, Pixels, ScrollHandle, SharedString,
    Subscription, Task, Window, px,
};
use time::UtcOffset;
use voltrus_scada_native::api::Alarm;
use voltrus_scada_native::perf::{self, Report};
use voltrus_scada_native::runtime::{Call, CallKey, Command, Event, Handle, Link, Reply};
use voltrus_scada_native::stress::StressOptions;
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::{self, ThemeCatalog};

pub use keys::{Quit, key_bindings, menus};
pub use pages::Page;

use meter::Meter;
use nav::{ListNav, Move};
use overlay::{Overlay, Palette};
use pages::Role;
use store::{Rate, Store};
use views::cached::CachedView;

/// How often, in seconds, the open alarms are fetched again while signed in.
const ALARM_REFRESH_SECS: u64 = 10;

/// How often the HUD redraws its figures while shown.
const HUD_REFRESH: Duration = Duration::from_millis(250);

/// Pages remembered for ⌘[ and ⌘].
const HISTORY_DEPTH: usize = 32;

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
    pub trend_filter: Entity<TextInput>,
    pub write: Entity<TextInput>,
}

pub struct Toast {
    pub message: SharedString,
    pub error: bool,
    pub id: u64,
    pub shown_at: Instant,
}

/// Toasts shown at once; a newer one pushes the oldest out.
const MAX_TOASTS: usize = 3;

/// A device's link as the server last reported it (WebSocket
/// `DeviceStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceLink {
    pub online: bool,
    pub latency_ms: Option<u64>,
}

/// An overlay to open once signed in (`--overlay`), for screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOverlay {
    Palette,
    Help,
}

/// How a stress run goes (`--stress …` or the palette).
#[derive(Debug, Clone, PartialEq)]
pub struct StressPlan {
    pub options: StressOptions,
    /// Switch to the next page this often; `None` stays put.
    pub switch_every: Option<Duration>,
    /// End the run (and quit, when launched with it) after this long.
    pub stop_after: Option<Duration>,
    /// Where to write the report too.
    pub report: Option<PathBuf>,
}

impl Default for StressPlan {
    fn default() -> Self {
        Self {
            options: StressOptions::default(),
            switch_every: Some(Duration::from_secs(2)),
            stop_after: None,
            report: None,
        }
    }
}

/// A stress run in progress.
pub struct StressRun {
    pub plan: StressPlan,
    pub started: Instant,
    /// Updates the store applied since the start.
    pub applied: u64,
    last_switch: Instant,
    /// Quit when the run ends on its own (launched with `--stress-seconds`).
    quit_when_done: bool,
    _task: Task<()>,
}

/// What the command line sets up, for screenshots, kiosks and stress runs.
pub struct Launch {
    pub sign_in: Option<(String, String, String)>,
    pub page: Option<Page>,
    /// A tag key to open on the Tags page once the tag list arrives.
    pub tag: Option<String>,
    /// The trend range to start with, one of [`RANGES`].
    pub range: Option<&'static str>,
    pub hud: bool,
    pub overlay: Option<LaunchOverlay>,
    pub stress: Option<StressPlan>,
    /// Read before any thread started: `time` cannot ask the system later.
    pub utc_offset: UtcOffset,
}

/// The cached views that draw the window's parts.
struct Views {
    navbar: Entity<CachedView>,
    status_bar: Entity<CachedView>,
    toasts: Entity<CachedView>,
    overlay: Entity<CachedView>,
    hud: Entity<CachedView>,
    /// One per page, made on its first show and kept: a page keeps its
    /// state (scroll, cursor, filters) and switching back draws at once.
    pages: HashMap<Page, Entity<CachedView>>,
}

pub struct Console {
    runtime: Handle,
    pub catalog: Rc<ThemeCatalog>,
    pub phase: Phase,
    pub link: Link,
    pub user: Option<(String, String)>,
    pub server: String,
    /// The server's version (`GET /api/v1/version`), once it answered.
    pub server_version: Option<String>,
    /// For showing alarm times on the operator's clock.
    pub utc_offset: UtcOffset,
    pub store: Store,
    pub alarms: Vec<Alarm>,
    /// Why the alarm list could not be fetched last time: it is stale.
    pub alarms_error: Option<String>,
    pub devices: HashMap<String, DeviceLink>,
    pub page: Page,
    back: Vec<Page>,
    forward: Vec<Page>,
    /// Counts page switches: each one starts the page's entrance anew.
    pub switch_seq: u64,
    pub switched_at: Instant,
    /// The tag (data source id) open in the Tags page's detail panel.
    pub selected: Option<i64>,
    /// Tags (data source ids) on the Trends chart.
    pub trend: BTreeSet<i64>,
    pub range: &'static str,
    pub alarm_filter: AlarmFilter,
    pub inputs: Inputs,
    pub busy: bool,
    pub error: Option<String>,
    pub write_result: Option<(i64, Result<String, String>)>,
    /// The tag whose write awaits the server's answer.
    pub writing: Option<i64>,
    pub rate: Rate,
    /// Oldest first; each dismisses itself after a while.
    pub toasts: Vec<Toast>,
    toast_seq: u64,
    pub focus: FocusHandle,
    lists: HashMap<&'static str, ListNav>,
    scrolls: HashMap<&'static str, ScrollHandle>,
    pub overlay: Option<Overlay>,
    /// Where focus goes back to when the overlay closes.
    return_focus: Option<FocusHandle>,
    pub palette: Option<Palette>,
    /// Where the navbar's More button is, for its menu.
    pub more_anchor: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub meter: Meter,
    pub hud: bool,
    hud_task: Option<Task<()>>,
    pub stress: Option<StressRun>,
    pub last_report: Option<Report>,
    views: Views,
    /// Per-page state owned by the page's module (see [`Console::state`]).
    states: HashMap<TypeId, Box<dyn Any>>,
    pending_page: Option<Page>,
    pending_tag: Option<String>,
    pending_overlay: Option<LaunchOverlay>,
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
            filter: input(String::new(), "Filter tags  /", cx, |input| input),
            trend_filter: input(String::new(), "Find a tag  /", cx, |input| input),
            write: input(String::new(), "New value", cx, |input| {
                input.kind(InputKind::Number).mono()
            }),
        };

        // Events come in batches: everything waiting is applied in one
        // update, which notifies once, so a busy stream costs one redraw
        // per frame at most.
        let pump = cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = events.recv().await {
                let mut batch = vec![first];
                while batch.len() < 2048
                    && let Ok(event) = events.try_recv()
                {
                    batch.push(event);
                }
                let applied = this.update_in(cx, |console, window, cx| {
                    for event in batch {
                        console.apply(event, window, cx);
                    }
                    console.meter.borrow_mut().expect_frame(Instant::now());
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
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Submitted => console.write_selected(window, cx),
                InputEvent::Cancelled => console.focus_page(window, cx),
                _ => {}
            },
        ));
        // A page's search field: typing filters, Enter or Esc goes back to
        // the list with the cursor on the first match.
        for (field, list) in [
            (inputs.filter.clone(), views::tags::LIST),
            (inputs.trend_filter.clone(), views::trends::LIST),
        ] {
            subscriptions.push(cx.subscribe_in(
                &field,
                window,
                move |console, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Changed => {
                        if let Some(nav) = console.lists.get_mut(list) {
                            nav.cursor = None;
                        }
                        cx.notify();
                    }
                    InputEvent::Submitted | InputEvent::Cancelled => {
                        let nav = console.list(list, cx);
                        nav.step(Move::First);
                        window.focus(&nav.focus);
                        cx.notify();
                    }
                    InputEvent::Blurred => {}
                },
            ));
        }

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

        let views = Views {
            navbar: CachedView::new(views::navbar::render, cx),
            status_bar: CachedView::new(views::status_bar::render, cx),
            toasts: CachedView::new(views::toasts::render, cx),
            overlay: CachedView::new(views::overlays::render, cx),
            hud: CachedView::new(views::hud::render, cx),
            pages: HashMap::new(),
        };

        let focus = cx.focus_handle();
        let mut console = Self {
            runtime,
            catalog,
            phase: Phase::SignIn,
            link: Link::SignedOut,
            user: None,
            server: String::new(),
            server_version: None,
            utc_offset: launch.utc_offset,
            store: Store::default(),
            alarms: Vec::new(),
            alarms_error: None,
            devices: HashMap::new(),
            page: Page::Overview,
            back: Vec::new(),
            forward: Vec::new(),
            switch_seq: 0,
            switched_at: Instant::now(),
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
            toasts: Vec::new(),
            toast_seq: 0,
            focus,
            lists: HashMap::new(),
            scrolls: HashMap::new(),
            overlay: None,
            return_focus: None,
            palette: None,
            more_anchor: Rc::default(),
            meter: Rc::new(RefCell::new(meter::FrameMeter::default())),
            hud: false,
            hud_task: None,
            stress: None,
            last_report: None,
            views,
            states: HashMap::new(),
            pending_page: launch.page,
            pending_tag: launch.tag,
            pending_overlay: launch.overlay,
            _tasks: vec![pump, ticker],
            _subscriptions: subscriptions,
        };
        let focus_field = if console.inputs.username.read(cx).value().is_empty() {
            console.inputs.username.clone()
        } else {
            console.inputs.password.clone()
        };
        focus_field.update(cx, |field, _| field.focus(window));
        if launch.hud {
            console.set_hud(true, cx);
        }
        if let Some(plan) = launch.stress {
            console.start_stress(plan, true, window, cx);
        } else if let Some((url, username, password)) = launch.sign_in {
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

    /// Run a generic REST call for the session; its answer comes back to
    /// the `on_reply` of the page whose id is the key's scope.
    pub fn call(&self, key: CallKey, call: Call) {
        self.send(Command::Call { key, call });
    }

    /// A page module's own state, made on first use. Keyed by type, so a
    /// page keeps its state in its own module without a field here.
    pub fn state<S: Default + 'static>(&mut self) -> &mut S {
        self.states
            .entry(TypeId::of::<S>())
            .or_insert_with(|| Box::new(S::default()))
            .downcast_mut::<S>()
            .expect("states are keyed by their type")
    }

    /// The keyboard list `id`, made on first use.
    pub fn list(&mut self, id: &'static str, cx: &mut gpui::App) -> &mut ListNav {
        self.lists.entry(id).or_insert_with(|| ListNav::new(cx))
    }

    /// A scroll handle that outlives the page's view, so a page scrolled
    /// away from comes back where it was.
    pub fn scroll(&mut self, id: &'static str) -> ScrollHandle {
        self.scrolls.entry(id).or_default().clone()
    }

    // ── Runtime events ────────────────────────────────────────────────────

    fn apply(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
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
                // A stress run's synthetic server is not worth remembering.
                if self.stress.is_none() {
                    config::save(&server, &username);
                }
                self.user = Some((username, role));
                self.server = server;
                self.server_version = None;
                if self.stress.is_none() {
                    self.call(
                        CallKey::new("shell.version"),
                        Call::get::<voltrus_scada_native::api::Version>("/api/v1/version"),
                    );
                }
                self.inputs
                    .password
                    .update(cx, |field, cx| field.set_text("", cx));
                if must_change_password {
                    self.phase = Phase::ChangePassword;
                } else {
                    self.enter_console(window, cx);
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
                        self.enter_console(window, cx);
                        self.show_toast("Password changed", false, cx);
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Event::Tags(list) => {
                self.store.replace(list);
                self.trend.retain(|id| self.store.position(*id).is_some());
                if self.trend.is_empty() {
                    self.trend = self
                        .store
                        .tags
                        .iter()
                        .take(4)
                        .map(|tag| tag.info.id)
                        .collect();
                }
                if self
                    .selected
                    .is_some_and(|id| self.store.position(id).is_none())
                {
                    self.selected = None;
                }
                self.send(Command::Subscribe(self.store.keys()));
                if self.stress.is_none() {
                    self.send(Command::History(history_range(self.range).to_owned()));
                }
                if let Some(index) = self
                    .pending_tag
                    .take()
                    .and_then(|key| self.store.by_key(&key))
                {
                    let id = self.store.tags[index].info.id;
                    self.open_tag(id, window, cx);
                }
            }
            Event::Updates(updates) => {
                let applied = self.store.apply(&updates);
                self.rate.record(applied, Instant::now());
                if let Some(run) = &mut self.stress {
                    run.applied += applied as u64;
                }
            }
            Event::Alarms(alarms) => {
                self.alarms = alarms;
                self.alarms_error = None;
            }
            Event::AlarmsFailed(error) => self.alarms_error = Some(error),
            Event::AlarmRaised(alarm) => {
                let name = self
                    .store
                    .by_key(&alarm.tag_id)
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
                // A late answer to an earlier write must not settle the one
                // now waiting.
                if late && self.writing == Some(tag) {
                    return;
                }
                if self.writing == Some(tag) {
                    self.writing = None;
                }
                self.write_result = Some((tag, result));
            }
            Event::Done { action, result } => {
                if let Err(error) = result {
                    self.show_toast(format!("{action} failed: {error}"), true, cx);
                }
            }
            Event::Reply { key, reply } => self.reply(key, reply, cx),
            Event::DeviceStatus {
                device_id,
                online,
                latency_ms,
            } => {
                self.devices
                    .insert(device_id, DeviceLink { online, latency_ms });
            }
        }
    }

    /// Route a generic REST reply to the page whose id is the key's scope;
    /// a failure nobody handles becomes a toast.
    fn reply(&mut self, key: CallKey, reply: Reply, cx: &mut Context<Self>) {
        if key == CallKey::new("shell.version") {
            self.server_version = reply
                .take::<voltrus_scada_native::api::Version>()
                .ok()
                .map(|answer| answer.version);
            return;
        }
        match pages::by_id(key.scope()).and_then(|spec| spec.on_reply) {
            Some(on_reply) => on_reply(self, key, reply, cx),
            None => {
                if let Some(error) = reply.error() {
                    self.show_toast(format!("{} failed: {error}", key.name), true, cx);
                }
            }
        }
    }

    fn enter_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.phase = Phase::Console;
        let page = self.pending_page.take().unwrap_or(self.page);
        let page = if self.allows(pages::spec(page).role) {
            page
        } else {
            Page::Overview
        };
        self.back.clear();
        self.forward.clear();
        self.switch_to(page, window, cx);
        match self.pending_overlay.take() {
            Some(LaunchOverlay::Palette) => self.open_palette(window, cx),
            Some(LaunchOverlay::Help) => self.open_help(window, cx),
            None => {}
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
        if self.stress.is_some() {
            self.finish_stress(cx);
        }
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
        self.devices.clear();
        self.selected = None;
        self.trend.clear();
        self.overlay = None;
        self.palette = None;
        for nav in self.lists.values_mut() {
            nav.cursor = None;
        }
        for field in [&self.inputs.new_password, &self.inputs.confirm_password] {
            field.update(cx, |field, cx| field.set_text("", cx));
        }
        self.inputs
            .password
            .update(cx, |field, _| field.focus(window));
        cx.notify();
    }

    // ── Roles ─────────────────────────────────────────────────────────────

    /// The signed-in user's role group, `None` when signed out.
    pub fn role(&self) -> Option<Role> {
        let (_, role) = self.user.as_ref()?;
        Some(match role.to_ascii_lowercase().as_str() {
            "admin" => Role::Admin,
            "demo" | "integration" => Role::Operator,
            _ => Role::Any,
        })
    }

    /// Whether the user may see what needs `needed`.
    pub fn allows(&self, needed: Role) -> bool {
        self.role().is_some_and(|role| role >= needed)
    }

    /// Any signed-in user.
    pub fn is_signed_in(&self) -> bool {
        self.allows(Role::Any)
    }

    /// Admin, demo or integration: the server's operator routes (shelve,
    /// alarm journal and summary, handover, annotations, …).
    pub fn is_operator(&self) -> bool {
        self.allows(Role::Operator)
    }

    /// Strictly admin: tag writes over the WebSocket, users, roles, keys.
    pub fn is_admin(&self) -> bool {
        self.allows(Role::Admin)
    }

    // ── Navigation ────────────────────────────────────────────────────────

    /// Show `page`, remembering the current one for ⌘[.
    pub fn show(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if page == self.page && self.phase == Phase::Console {
            self.focus_page(window, cx);
            return;
        }
        if !self.allows(pages::spec(page).role) {
            self.show_toast(format!("{} needs another role", page.title()), true, cx);
            return;
        }
        self.back.push(self.page);
        if self.back.len() > HISTORY_DEPTH {
            self.back.remove(0);
        }
        self.forward.clear();
        self.switch_to(page, window, cx);
    }

    /// ⌘[ and ⌘]: back or forward through the pages shown.
    pub fn history_step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (from, to) = if forward {
            (&mut self.forward, &mut self.back)
        } else {
            (&mut self.back, &mut self.forward)
        };
        let Some(page) = from.pop() else {
            return;
        };
        to.push(self.page);
        self.switch_to(page, window, cx);
    }

    fn switch_to(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        self.switch_seq += 1;
        self.switched_at = Instant::now();
        self.meter.borrow_mut().page_switch(self.switched_at);
        if self.overlay.is_some() {
            self.close_overlay(window, cx);
        }
        self.page_view(page, cx);
        if let Some(on_show) = pages::spec(page).on_show {
            on_show(self, window, cx);
        } else {
            window.focus(&self.focus);
        }
        cx.notify();
    }

    /// The cached view drawing `page`, made on its first show.
    fn page_view(&mut self, page: Page, cx: &mut Context<Self>) -> Entity<CachedView> {
        self.views
            .pages
            .entry(page)
            .or_insert_with(|| {
                CachedView::new(
                    move |console, window, cx| views::page(console, page, window, cx),
                    cx,
                )
            })
            .clone()
    }

    /// Put focus where the page's keys work: its first list, or the root.
    pub fn focus_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match pages::spec(self.page).on_show {
            Some(on_show) => on_show(self, window, cx),
            None => window.focus(&self.focus),
        }
        cx.notify();
    }

    /// Focus the keyboard list `id`, its cursor on the first row if it
    /// has none.
    pub fn focus_list(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let nav = self.list(id, cx);
        if nav.cursor.is_none() && nav.len > 0 {
            nav.cursor = Some(0);
        }
        window.focus(&nav.focus);
    }

    /// `/`: the page's search field.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = pages::spec(self.page)
            .search
            .and_then(|search| search(self))
        {
            field.update(cx, |field, _| field.focus(window));
        }
    }

    /// Move the cursor of keyboard list `id`; the new row.
    pub fn list_move(
        &mut self,
        id: &'static str,
        step: Move,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let row = self.list(id, cx).step(step);
        cx.notify();
        row
    }

    /// Esc: close the overlay, else the page's detail, else back to the
    /// page's list.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_some() {
            self.close_overlay(window, cx);
        } else if self.page == Page::Tags && self.selected.is_some() {
            self.selected = None;
            self.focus_page(window, cx);
        } else {
            self.focus_page(window, cx);
        }
        cx.notify();
    }

    /// Tab and Shift-Tab; inside an overlay, focus stays in it.
    pub fn focus_step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let step = |window: &mut Window| {
            if forward {
                window.focus_next();
            } else {
                window.focus_prev();
            }
        };
        let Some(trap) = self.overlay.as_ref().map(|overlay| overlay.focus.clone()) else {
            step(window);
            return;
        };
        // Walk the tab stops until one inside the overlay; a handful of
        // controls behind it are skipped at most once around.
        for _ in 0..256 {
            step(window);
            if trap.contains_focused(window, cx) {
                return;
            }
        }
        window.focus(&trap);
    }

    // ── Overlays ──────────────────────────────────────────────────────────

    /// Open `overlay` in the slot (closing any other), focusing it.
    pub fn open_overlay(&mut self, overlay: Overlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_none() {
            self.return_focus = window.focused(cx);
        }
        self.palette = None;
        window.focus(&overlay.focus);
        self.overlay = Some(overlay);
        cx.notify();
    }

    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = None;
        self.palette = None;
        match self.return_focus.take() {
            Some(focus) => window.focus(&focus),
            None => window.focus(&self.focus),
        }
        cx.notify();
    }

    pub fn overlay_is(&self, id: &str) -> bool {
        self.overlay
            .as_ref()
            .is_some_and(|overlay| overlay.id == id)
    }

    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let overlay = Overlay::modal("palette", px(600.0), views::overlays::palette, cx);
        self.open_overlay(overlay, window, cx);
        let field = cx.new(|cx| TextInput::new(cx).placeholder("Go to a page or run a command…"));
        let subscription = cx.subscribe_in(
            &field,
            window,
            |console, _, event: &InputEvent, window, cx| match event {
                InputEvent::Changed => {
                    console.search_palette(cx);
                    console.list(views::overlays::PALETTE_LIST, cx).cursor = Some(0);
                    cx.notify();
                }
                InputEvent::Submitted => console.run_palette(None, window, cx),
                InputEvent::Cancelled => console.close_overlay(window, cx),
                InputEvent::Blurred => {}
            },
        );
        field.update(cx, |field, _| field.focus(window));
        self.palette = Some(Palette {
            input: field,
            results: Vec::new(),
            _subscription: subscription,
        });
        self.search_palette(cx);
        self.list(views::overlays::PALETTE_LIST, cx).cursor = Some(0);
    }

    fn search_palette(&mut self, cx: &mut Context<Self>) {
        let role = self.role();
        if let Some(palette) = &mut self.palette {
            let query = palette.input.read(cx).value().to_owned();
            palette.results = overlay::search(&overlay::entries(role), &query);
        }
    }

    /// Run the palette's entry at result `row` (the cursor's by default).
    pub fn run_palette(&mut self, row: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let row = row.or_else(|| self.list(views::overlays::PALETTE_LIST, cx).cursor);
        let action = self.palette.as_ref().and_then(|palette| {
            let index = *palette.results.get(row?)?;
            overlay::entries(self.role())
                .into_iter()
                .nth(index)
                .map(|entry| entry.action)
        });
        self.close_overlay(window, cx);
        if let Some(action) = action {
            window.dispatch_action(action, cx);
        }
    }

    pub fn open_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let overlay = Overlay::sheet("help", px(440.0), views::overlays::help, cx);
        self.open_overlay(overlay, window, cx);
    }

    pub fn open_more(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.more_anchor.get().map_or_else(
            || gpui::point(px(240.0), theme::TITLE_BAR_HEIGHT),
            |bounds| gpui::point(bounds.left(), bounds.bottom() + px(4.0)),
        );
        let overlay = Overlay::popover("more", px(240.0), at, views::overlays::more, cx);
        self.open_overlay(overlay, window, cx);
        // The menu's list, inside the overlay, takes the keys.
        let nav = self.list(views::overlays::MORE_LIST, cx);
        nav.cursor = Some(0);
        window.focus(&nav.focus);
    }

    // ── Selection ─────────────────────────────────────────────────────────

    /// Open tag `id` in the Tags page's detail panel.
    pub fn select(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.selected != Some(id) {
            self.write_result = None;
            self.inputs
                .write
                .update(cx, |field, cx| field.set_text("", cx));
        }
        self.selected = Some(id);
        cx.notify();
    }

    /// Show one tag on the Tags page, with the filter cleared so that the
    /// list row is the tag's position.
    pub fn open_tag(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.inputs
            .filter
            .update(cx, |field, cx| field.set_text("", cx));
        self.show(Page::Tags, window, cx);
        self.select(id, cx);
        if let Some(row) = self.store.position(id) {
            let nav = self.list(views::tags::LIST, cx);
            nav.select(row);
            window.focus(&nav.focus);
        }
    }

    /// Tags matching the Tags page's filter, as positions in the store.
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

    pub fn toggle_trend(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.trend.remove(&id) {
            self.trend.insert(id);
        }
        cx.notify();
    }

    pub fn set_range(&mut self, range: &'static str, cx: &mut Context<Self>) {
        if self.range != range {
            self.range = range;
            if self.stress.is_none() {
                self.send(Command::History(history_range(range).to_owned()));
            }
        }
        cx.notify();
    }

    // ── Commands to the server ────────────────────────────────────────────

    pub fn write_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Enter in the field is not disabled like the button: one write at
        // a time.
        let Some(id) = self.selected.filter(|_| self.writing.is_none()) else {
            return;
        };
        let text = self.inputs.write.read(cx).value().trim().to_owned();
        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => {
                self.write_result = None;
                self.writing = Some(id);
                self.send(Command::Write { id, value });
                self.focus_page(window, cx);
            }
            _ => self.write_result = Some((id, Err(format!("“{text}” is not a number")))),
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

    /// The window closes or the console quits: report a stress run, stop
    /// the runtime.
    pub fn shutdown(&mut self) {
        if let Some(report) = self.stress_report() {
            println!("{report}");
            self.write_report(&report);
        }
        self.send(Command::Shutdown);
    }

    // ── Stress test ───────────────────────────────────────────────────────

    /// Feed a synthetic plant through the runtime (no server) and measure.
    /// `launched` runs come from the command line and quit when they end.
    pub fn start_stress(
        &mut self,
        plan: StressPlan,
        launched: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.meter.borrow_mut().reset();
        self.send(Command::Stress(plan.options));
        // The ticker's alarm refresh would only report "not signed in".
        self.alarms_error = None;
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let alive =
                    this.update_in(cx, |console, window, cx| console.stress_tick(window, cx));
                if alive.is_err() {
                    break;
                }
            }
        });
        let now = Instant::now();
        self.stress = Some(StressRun {
            plan,
            started: now,
            applied: 0,
            last_switch: now,
            quit_when_done: launched,
            _task: task,
        });
        self.set_hud(true, cx);
        cx.notify();
    }

    fn stress_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = &mut self.stress else {
            return;
        };
        let now = Instant::now();
        if run
            .plan
            .stop_after
            .is_some_and(|after| now.duration_since(run.started) >= after)
        {
            let quit = run.quit_when_done;
            self.finish_stress(cx);
            if quit {
                self.send(Command::Shutdown);
                cx.quit();
            } else {
                self.sign_out(window, cx);
            }
            return;
        }
        if self.phase == Phase::Console
            && self.overlay.is_none()
            && run
                .plan
                .switch_every
                .is_some_and(|every| now.duration_since(run.last_switch) >= every)
        {
            run.last_switch = now;
            let position = pages::PAGES
                .iter()
                .position(|spec| spec.page == self.page)
                .unwrap_or(0);
            let next = pages::PAGES[(position + 1) % pages::PAGES.len()].page;
            self.show(next, window, cx);
        }
    }

    /// The running stress test's figures so far.
    pub fn stress_report(&self) -> Option<Report> {
        let run = self.stress.as_ref()?;
        let meter = self.meter.borrow();
        let (interval, cpu) = meter.stats.whole_run();
        Some(Report {
            duration: run.started.elapsed(),
            tags: run.plan.options.tags,
            rate: run.plan.options.rate,
            applied: run.applied,
            frames: meter.stats.total,
            expected: meter.stats.expected,
            dropped: meter.stats.dropped,
            refresh_ms: meter.stats.refresh_ms(),
            interval,
            cpu,
            switches: meter.switches.len(),
            switch: meter.switch_spread(),
            peak_rss_bytes: perf::peak_rss_bytes(),
        })
    }

    fn write_report(&self, report: &Report) {
        let Some(path) = self
            .stress
            .as_ref()
            .and_then(|run| run.plan.report.as_ref())
        else {
            return;
        };
        if let Err(error) = std::fs::write(path, report.to_string()) {
            eprintln!(
                "voltrus-native: could not write the stress report to {}: {error}",
                path.display()
            );
        }
    }

    /// End the stress run: print and keep its report, stop the feed.
    pub fn finish_stress(&mut self, cx: &mut Context<Self>) {
        let Some(report) = self.stress_report() else {
            return;
        };
        println!("{report}");
        self.write_report(&report);
        self.last_report = Some(report);
        self.stress = None;
        cx.notify();
    }

    // ── Appearance, HUD and toasts ────────────────────────────────────────

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

    /// Show or hide the performance HUD. While shown it redraws its own
    /// view a few times a second; hidden, it costs nothing.
    pub fn set_hud(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.hud = shown;
        self.hud_task = shown.then(|| {
            let hud = self.views.hud.clone();
            cx.spawn(async move |_, cx| {
                loop {
                    cx.background_executor().timer(HUD_REFRESH).await;
                    if hud.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })
        });
        cx.notify();
    }

    pub fn show_toast(
        &mut self,
        message: impl Into<SharedString>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        self.toast_seq += 1;
        let id = self.toast_seq;
        if self.toasts.len() == MAX_TOASTS {
            self.toasts.remove(0);
        }
        self.toasts.push(Toast {
            message: message.into(),
            error,
            id,
            shown_at: Instant::now(),
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            let _ = this.update(cx, |console, cx| {
                let before = console.toasts.len();
                console.toasts.retain(|toast| toast.id != id);
                if console.toasts.len() != before {
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

    /// Open alarms nobody has acknowledged yet: the Alarms tab's badge.
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
