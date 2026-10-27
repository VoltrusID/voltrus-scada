//! Data Manager › MQTT broker: the web client's `MqttBrokerPanel.tsx` —
//! the embedded broker's status and connected clients (every 5 s), start
//! and stop, its configuration (port, max connections, authentication and
//! users), the topic tree with each topic's last value and recent messages
//! (every 3 s), the live message log (every 2 s) and a publish form.
//!
//! Everyone sees the status and the clients; the rest are the server's
//! operator routes, so a viewer sees them disabled with the reason. As on
//! the web, Topics, Messages and Publish need a running broker.
//!
//! Keys: `[` `]` tabs, S start/stop, C configure, P publish, R refresh,
//! `/` filter topics or messages; in the topic tree ↵ opens a folder or
//! picks a topic; in the log the cursor's message shows in full.

use std::collections::BTreeSet;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, App, ClipboardItem, Context, Div, Entity, FontWeight, Hsla,
    Subscription, Window, div, px, uniform_list,
};
use voltrus_scada_native::api::eng_mqtt::{
    self as model, ActionStatus, BrokerStatus, ConfigDraft, ConfigUpdated, Messages, Published,
    TopicMessages, TopicRow, Topics, UserDraft,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, kbd, pill, segmented, status_dot};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use crate::console::keys::{
    EngMqttConfigure, EngMqttNextTab, EngMqttPrevTab, EngMqttPublish, EngMqttRefresh,
    EngMqttStartStop,
};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::text_area::TextArea;
use crate::console::views::{
    ListKeys, age, card, card_title, focus_ring, keyboard_list, local_time, ops_kit, page_header,
    rows_on_screen, tail_row, tail_rows,
};
use crate::console::{Console, Page};

const ID: &str = "mqtt";
const CLIENTS: &str = "data.mqtt.clients";
const TOPICS: &str = "data.mqtt.topics";
const MESSAGES: &str = "data.mqtt.messages";
const CONFIG: &str = "data-mqtt-config";
const ROW: f32 = 34.0;
const DETAIL: f32 = 360.0;
/// The log shows the newest this many (the server keeps 500).
const LOG_LIMIT: usize = 200;
/// A payload shows at most this many lines; the rest are counted.
const PAYLOAD_LINES: usize = 40;
const OPERATORS: &str = "Operators only (admin, demo, integration)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Overview,
    Topics,
    Messages,
    Publish,
}

impl Tab {
    const ALL: [Self; 4] = [Self::Overview, Self::Topics, Self::Messages, Self::Publish];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Topics => "Topics",
            Self::Messages => "Messages",
            Self::Publish => "Publish",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Overview => Icon::Gauge,
            Self::Topics => Icon::Layers,
            Self::Messages => Icon::ScrollText,
            Self::Publish => Icon::Send,
        }
    }
}

/// Why a tab cannot open now, if it cannot: the operator routes, and a
/// broker that is not running (as the web panel disables them).
fn tab_blocked(tab: Tab, operator: bool, running: bool) -> Option<&'static str> {
    match tab {
        Tab::Overview => None,
        _ if !operator => Some(OPERATORS),
        _ if !running => Some("Start the broker first"),
        _ => None,
    }
}

struct ConfigForm {
    port: Entity<TextInput>,
    max: Entity<TextInput>,
    auth: bool,
    users: Vec<(Entity<TextInput>, Entity<TextInput>)>,
    error: Option<String>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
struct State {
    tab: Tab,
    status: Option<Result<BrokerStatus, String>>,
    /// Shared with each render's list closure without copying the topics.
    topics: Option<Result<Rc<Topics>, String>>,
    messages: Option<Result<Messages, String>>,
    selected: Option<String>,
    topic_messages: Option<Result<TopicMessages, String>>,
    expanded: BTreeSet<String>,
    /// Request numbers per poll; older answers are dropped.
    status_seq: i64,
    topics_seq: i64,
    messages_seq: i64,
    topic_seq: i64,
    /// The newest start/stop, config save and publish.
    busy_seq: i64,
    config_seq: i64,
    publish_seq: i64,
    asked: bool,
    /// Start or stop on its way.
    busy: bool,
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    form: Option<ConfigForm>,
    window: Option<AnyWindowHandle>,
    // Publish.
    topic: Option<Entity<TextInput>>,
    payload: Option<Entity<TextArea>>,
    _publish: Vec<Subscription>,
    /// The current payload field's; replaced (not added to) with the field.
    _payload: Option<Subscription>,
    publish_error: Option<String>,
    publishing: bool,
}

impl State {
    fn running(&self) -> bool {
        matches!(&self.status, Some(Ok(status)) if status.running())
    }
}

pub fn on_open(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        return;
    }
    if !console.state::<State>().asked {
        console.state::<State>().asked = true;
        fetch_status(console);
    }
    ops_kit::poll_while_shown(
        console,
        Page::DataManager,
        Duration::from_secs(5),
        tick_status,
        cx,
    );
    ops_kit::poll_while_shown(
        console,
        Page::DataManager,
        Duration::from_secs(3),
        tick_topics,
        cx,
    );
    ops_kit::poll_while_shown(
        console,
        Page::DataManager,
        Duration::from_secs(2),
        tick_messages,
        cx,
    );
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    match console.state::<State>().tab {
        Tab::Overview => console.focus_list(CLIENTS, window, cx),
        Tab::Topics => console.focus_list(TOPICS, window, cx),
        Tab::Messages => console.focus_list(MESSAGES, window, cx),
        Tab::Publish => {
            if let Some(field) = console.state::<State>().topic.clone() {
                field.read(cx).focus(window);
            }
        }
    }
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console.state_ref::<State>()?.search.clone()
}

fn shown(console: &Console) -> bool {
    super::current_id(console) == Some(ID)
}

// ── Fetching ─────────────────────────────────────────────────────────────

fn fetch_status(console: &mut Console) {
    let state = console.state::<State>();
    state.status_seq += 1;
    let seq = state.status_seq;
    console.call(
        CallKey::new("data.mqtt.status").with(seq),
        Call::new(|api| async move { api.mqtt_status().await }),
    );
}

fn fetch_topics(console: &mut Console) {
    let state = console.state::<State>();
    state.topics_seq += 1;
    let seq = state.topics_seq;
    let selected = state.selected.clone();
    console.call(
        CallKey::new("data.mqtt.topics").with(seq),
        Call::new(|api| async move { api.mqtt_topics().await }),
    );
    if let Some(topic) = selected {
        fetch_topic(console, topic);
    }
}

fn fetch_topic(console: &mut Console, topic: String) {
    let state = console.state::<State>();
    state.topic_seq += 1;
    let seq = state.topic_seq;
    console.call(
        CallKey::new("data.mqtt.topic").with(seq),
        Call::new(move |api| async move { api.mqtt_topic_messages(&topic).await }),
    );
}

fn fetch_messages(console: &mut Console) {
    let state = console.state::<State>();
    state.messages_seq += 1;
    let seq = state.messages_seq;
    console.call(
        CallKey::new("data.mqtt.messages").with(seq),
        Call::new(|api| async move { api.mqtt_messages(LOG_LIMIT).await }),
    );
}

/// Whether the operator routes may be asked now.
fn live(console: &mut Console) -> bool {
    console.is_operator() && console.state::<State>().running()
}

fn tick_status(console: &mut Console, _: &mut Context<Console>) {
    if shown(console) {
        fetch_status(console);
    }
}

fn tick_topics(console: &mut Console, _: &mut Context<Console>) {
    if shown(console) && live(console) && console.state::<State>().tab == Tab::Topics {
        fetch_topics(console);
    }
}

fn tick_messages(console: &mut Console, _: &mut Context<Console>) {
    if shown(console) && live(console) && console.state::<State>().tab == Tab::Messages {
        fetch_messages(console);
    }
}

fn refresh(console: &mut Console, cx: &mut Context<Console>) {
    fetch_status(console);
    if live(console) {
        match console.state::<State>().tab {
            Tab::Topics | Tab::Publish => fetch_topics(console),
            Tab::Messages => fetch_messages(console),
            Tab::Overview => {}
        }
    }
    cx.notify();
}

// ── Tabs ─────────────────────────────────────────────────────────────────

fn set_tab(console: &mut Console, tab: Tab, window: &mut Window, cx: &mut Context<Console>) {
    let operator = console.is_operator();
    let running = console.state::<State>().running();
    if let Some(reason) = tab_blocked(tab, operator, running) {
        console.show_toast(format!("{}: {reason}", tab.label()), false, cx);
        return;
    }
    console.state::<State>().tab = tab;
    match tab {
        Tab::Topics | Tab::Publish => fetch_topics(console),
        Tab::Messages => fetch_messages(console),
        Tab::Overview => {}
    }
    if tab == Tab::Publish {
        ensure_publish(console, window, cx);
    }
    focus(console, window, cx);
    cx.notify();
}

fn step_tab(console: &mut Console, forward: bool, window: &mut Window, cx: &mut Context<Console>) {
    let operator = console.is_operator();
    let running = console.state::<State>().running();
    let current = console.state::<State>().tab;
    let at = Tab::ALL.iter().position(|tab| *tab == current).unwrap_or(0);
    let count = Tab::ALL.len();
    // The next tab that can open; the overview always can.
    let next = (1..count)
        .map(|step| {
            if forward {
                Tab::ALL[(at + step) % count]
            } else {
                Tab::ALL[(at + count - step) % count]
            }
        })
        .find(|tab| tab_blocked(*tab, operator, running).is_none())
        .unwrap_or(Tab::Overview);
    set_tab(console, next, window, cx);
}

// ── Start, stop ──────────────────────────────────────────────────────────

fn start_stop(console: &mut Console, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.show_toast(
            format!("Starting and stopping the broker: {OPERATORS}"),
            true,
            cx,
        );
        return;
    }
    let state = console.state::<State>();
    if state.busy {
        return;
    }
    let running = state.running();
    state.busy = true;
    state.busy_seq += 1;
    let seq = state.busy_seq;
    if running {
        console.call(
            CallKey::new("data.mqtt.stop").with(seq),
            Call::new(|api| async move { api.mqtt_stop().await }),
        );
    } else {
        console.call(
            CallKey::new("data.mqtt.start").with(seq),
            Call::new(|api| async move { api.mqtt_start().await }),
        );
    }
    cx.notify();
}

// ── Config ───────────────────────────────────────────────────────────────

fn field_events(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Submitted => save_config(console, cx),
        InputEvent::Cancelled => console.close_overlay(window, cx),
        InputEvent::Changed | InputEvent::Blurred => {}
    }
}

fn number_field(text: String, cx: &mut Context<Console>) -> Entity<TextInput> {
    cx.new(|cx| {
        TextInput::new(cx)
            .text(text)
            .kind(InputKind::Unsigned)
            .mono()
    })
}

fn open_config(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.show_toast(format!("The broker's configuration: {OPERATORS}"), true, cx);
        return;
    }
    let config = match &console.state::<State>().status {
        Some(Ok(status)) => status.config,
        _ => None,
    };
    let draft = ConfigDraft::from_status(config);
    let port = number_field(draft.port, cx);
    let max = number_field(draft.max_connections, cx);
    let subscriptions = vec![
        ops_kit::on_field(&port, field_events, window, cx),
        ops_kit::on_field(&max, field_events, window, cx),
    ];
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.form = Some(ConfigForm {
        port: port.clone(),
        max,
        auth: draft.auth,
        users: Vec::new(),
        error: None,
        saving: false,
        _subscriptions: subscriptions,
    });
    let overlay = Overlay::modal(CONFIG, px(480.0), config_dialog, cx);
    console.open_overlay(overlay, window, cx);
    port.read(cx).focus(window);
}

fn add_user(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let username = ops_kit::text_field("username", true, cx);
    let password = cx.new(|cx| TextInput::new(cx).placeholder("password").secure());
    let subscriptions = [
        ops_kit::on_field(&username, field_events, window, cx),
        ops_kit::on_field(&password, field_events, window, cx),
    ];
    if let Some(form) = console.state::<State>().form.as_mut() {
        form.users.push((username.clone(), password));
        form._subscriptions.extend(subscriptions);
    }
    username.read(cx).focus(window);
    cx.notify();
}

fn draft_of(form: &ConfigForm, cx: &App) -> ConfigDraft {
    ConfigDraft {
        port: ops_kit::value(&form.port, cx),
        max_connections: ops_kit::value(&form.max, cx),
        auth: form.auth,
        users: form
            .users
            .iter()
            .map(|(username, password)| UserDraft {
                username: ops_kit::value(username, cx),
                password: password.read(cx).value().to_owned(),
            })
            .collect(),
    }
}

fn save_config(console: &mut Console, cx: &mut Context<Console>) {
    let running = console.state::<State>().running();
    let Some(form) = console.state::<State>().form.as_mut() else {
        return;
    };
    if form.saving {
        return;
    }
    if running {
        form.error = Some("Stop the broker before changing its configuration".to_owned());
        cx.notify();
        return;
    }
    let body = match draft_of(form, cx).body() {
        Ok(body) => body,
        Err(error) => {
            form.error = Some(error);
            cx.notify();
            return;
        }
    };
    form.saving = true;
    form.error = None;
    let state = console.state::<State>();
    state.config_seq += 1;
    let seq = state.config_seq;
    console.call(
        CallKey::new("data.mqtt.config").with(seq),
        Call::new(move |api| async move { api.mqtt_update_config(&body).await }),
    );
    cx.notify();
}

// ── Publish ──────────────────────────────────────────────────────────────

fn publish_events(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Submitted => publish(console, cx),
        InputEvent::Cancelled => {
            let nav = console.list(CLIENTS, cx).focus.clone();
            window.focus(&nav);
        }
        InputEvent::Changed => cx.notify(),
        InputEvent::Blurred => {}
    }
}

fn new_payload(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let payload =
        cx.new(|cx| TextArea::new(6, cx).placeholder("{\"temperature\": 25.5}   ⌘↵ publish"));
    let subscription = cx.subscribe_in(
        &payload,
        window,
        |console, _, event: &InputEvent, window, cx| {
            publish_events(console, event, window, cx);
        },
    );
    let state = console.state::<State>();
    state.payload = Some(payload);
    state._payload = Some(subscription);
}

fn ensure_publish(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().topic.is_some() {
        return;
    }
    let topic = ops_kit::text_field("e.g. devices/sensor1/temperature", true, cx);
    let subscription = ops_kit::on_field(&topic, publish_events, window, cx);
    let state = console.state::<State>();
    state.topic = Some(topic);
    state.window = Some(window.window_handle());
    state._publish.push(subscription);
    new_payload(console, window, cx);
}

fn publish(console: &mut Console, cx: &mut Context<Console>) {
    if !console.is_operator() {
        return;
    }
    let state = console.state::<State>();
    if state.publishing {
        return;
    }
    let topic = state
        .topic
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    let payload = state
        .payload
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    let body = match model::publish_body(&topic, &payload) {
        Ok(body) => body,
        Err(error) => {
            state.publish_error = Some(error);
            cx.notify();
            return;
        }
    };
    state.publishing = true;
    state.publish_error = None;
    state.publish_seq += 1;
    let seq = state.publish_seq;
    console.call(
        CallKey::new("data.mqtt.publish").with(seq),
        Call::new(move |api| async move { api.mqtt_publish(&body).await }),
    );
    cx.notify();
}

/// After a publish: an empty payload field (the web form clears it); the
/// topic stays for the next message.
fn clear_payload_later(console: &mut Console, cx: &mut Context<Console>) {
    let Some(handle) = console.state::<State>().window else {
        return;
    };
    let entity = cx.entity();
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            entity.update(cx, |console, cx| {
                new_payload(console, window, cx);
                cx.notify();
            });
        });
    });
}

fn set_topic(console: &mut Console, topic: String, cx: &mut Context<Console>) {
    if let Some(field) = console.state::<State>().topic.clone() {
        field.update(cx, |field, cx| field.set_text(topic, cx));
    }
    cx.notify();
}

// ── Topics ───────────────────────────────────────────────────────────────

fn filter_text(console: &Console, cx: &App) -> String {
    console
        .state_ref::<State>()
        .and_then(|state| state.search.as_ref())
        .map(|field| ops_kit::value(field, cx))
        .unwrap_or_default()
}

fn topic_rows(console: &Console, cx: &App) -> Vec<TopicRow> {
    let Some(state) = console.state_ref::<State>() else {
        return Vec::new();
    };
    match &state.topics {
        Some(Ok(topics)) => {
            model::topic_rows(&topics.topics, &filter_text(console, cx), &state.expanded)
        }
        _ => Vec::new(),
    }
}

fn select_topic(console: &mut Console, topic: String, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.selected.as_deref() != Some(topic.as_str()) {
        state.topic_messages = None;
    }
    state.selected = Some(topic.clone());
    fetch_topic(console, topic);
    cx.notify();
}

/// Enter or Space on a tree line: a folder opens or closes; a topic is
/// picked (a topic that is also a folder opens too).
fn activate_topic(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let rows = topic_rows(console, cx);
    let Some(line) = rows.get(row) else {
        return;
    };
    if line.has_children {
        let state = console.state::<State>();
        if !state.expanded.remove(&line.path) {
            state.expanded.insert(line.path.clone());
        }
    }
    if line.topic.is_some() {
        select_topic(console, line.path.clone(), cx);
    }
    cx.notify();
}

// ── Messages ─────────────────────────────────────────────────────────────

/// The log, newest first, filtered by topic.
fn log(console: &Console, cx: &App) -> Vec<model::RecentMessage> {
    let Some(Some(Ok(messages))) = console
        .state_ref::<State>()
        .map(|state| state.messages.as_ref())
    else {
        return Vec::new();
    };
    let filter = filter_text(console, cx);
    let mut found: Vec<model::RecentMessage> = model::filter_messages(&messages.messages, &filter)
        .into_iter()
        .cloned()
        .collect();
    found.reverse();
    found
}

fn copy_payload(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(message) = log(console, cx).get(row) {
        cx.write_to_clipboard(ClipboardItem::new_string(message.payload.clone()));
        console.show_toast(
            format!("Copied the payload of {}", message.topic),
            false,
            cx,
        );
    }
}

fn copy_client(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let id = match &console.state::<State>().status {
        Some(Ok(status)) => status
            .clients
            .get(row)
            .map(|client| client.client_id.clone()),
        _ => None,
    };
    if let Some(id) = id {
        cx.write_to_clipboard(ClipboardItem::new_string(id.clone()));
        console.show_toast(format!("Copied {id}"), false, cx);
    }
}

// ── Replies ──────────────────────────────────────────────────────────────

/// A status answer: a stopped broker leaves only the overview, but a failed
/// poll keeps the tab the user is on.
fn apply_status(state: &mut State, status: Result<BrokerStatus, String>) {
    let stopped = matches!(&status, Ok(status) if !status.running());
    state.status = Some(status);
    if stopped && tab_blocked(state.tab, true, false).is_some() {
        state.tab = Tab::Overview;
    }
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    // A dialog closed (Esc) before its answer: the error goes to a toast.
    let config_open = console.overlay_is(CONFIG);
    let state = console.state::<State>();
    match key.name {
        "data.mqtt.status" => {
            if key.arg == state.status_seq {
                apply_status(state, reply.take::<BrokerStatus>());
            }
        }
        "data.mqtt.topics" => {
            if key.arg == state.topics_seq {
                state.topics = Some(reply.take::<Topics>().map(Rc::new));
            }
        }
        "data.mqtt.topic" => {
            if key.arg == state.topic_seq {
                state.topic_messages = Some(reply.take::<TopicMessages>());
            }
        }
        "data.mqtt.messages" => {
            if key.arg == state.messages_seq {
                state.messages = Some(reply.take::<Messages>());
            }
        }
        "data.mqtt.start" | "data.mqtt.stop" => {
            // The broker acted whichever request it was; only the newest
            // frees the button.
            if key.arg == state.busy_seq {
                state.busy = false;
            }
            match reply.take::<ActionStatus>() {
                Ok(done) => console.show_toast(done.message, false, cx),
                Err(error) => console.show_toast(format!("Broker: {error}"), true, cx),
            }
            fetch_status(console);
        }
        "data.mqtt.config" if key.arg == state.config_seq => match reply.take::<ConfigUpdated>() {
            Ok(updated) => {
                state.form = None;
                if let Some(window) = state.window {
                    ops_kit::close_overlay_in(window, CONFIG, cx);
                }
                console.show_toast(updated.message, false, cx);
                fetch_status(console);
            }
            Err(error) => match state.form.as_mut().filter(|_| config_open) {
                Some(form) => {
                    form.saving = false;
                    form.error = Some(error);
                }
                None => console.show_toast(format!("Broker config: {error}"), true, cx),
            },
        },
        "data.mqtt.publish" if key.arg == state.publish_seq => {
            state.publishing = false;
            match reply.take::<Published>() {
                Ok(published) => {
                    console.show_toast(format!("Published to {}", published.topic), false, cx);
                    clear_payload_later(console, cx);
                    fetch_messages(console);
                    fetch_topics(console);
                }
                Err(error) => state.publish_error = Some(error),
            }
        }
        _ => {}
    }
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn state_color(status: &BrokerStatus, theme: &Theme) -> Hsla {
    match status.state() {
        model::State::Running => theme.success,
        model::State::Starting => theme.warning,
        model::State::Stopped => theme.fg_dim,
        model::State::Failed => theme.danger,
    }
}

fn stat(label: &'static str, value: String, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_1)
        .min_w(px(110.0))
        .child(
            div()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child(label.to_uppercase()),
        )
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_EMPHASIS)
                .child(value),
        )
}

fn ensure_search(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().search.is_some() {
        return;
    }
    let field = ops_kit::text_field("Filter topics  /", true, cx);
    let subscription = cx.subscribe_in(
        &field,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                if console.state::<State>().tab == Tab::Overview && live(console) {
                    console.state::<State>().tab = Tab::Topics;
                    fetch_topics(console);
                }
                console.list(TOPICS, cx).cursor = None;
                console.list(MESSAGES, cx).cursor = None;
                cx.notify();
            }
            InputEvent::Submitted | InputEvent::Cancelled => {
                let list = if console.state::<State>().tab == Tab::Messages {
                    MESSAGES
                } else {
                    TOPICS
                };
                let nav = console.list(list, cx);
                nav.step(Move::First);
                window.focus(&nav.focus);
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    );
    let state = console.state::<State>();
    state.search = Some(field);
    state._search = Some(subscription);
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    ensure_search(console, window, cx);
    // A form closed with Esc keeps no password.
    if !console.overlay_is(CONFIG) {
        console.state::<State>().form = None;
    }
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let state = console.state::<State>();
    let tab = state.tab;
    let running = state.running();
    let busy = state.busy;
    let search_field = state.search.clone();
    let status = state.status.clone();

    let subtitle = match &status {
        Some(Ok(status)) => format!(
            "{} · {} clients · [ ] tabs · S start/stop · C configure · P publish · R refresh",
            status.headline(),
            status.connected_clients
        ),
        Some(Err(error)) => format!("The broker's status failed: {error}"),
        None => "Asking the broker…".to_owned(),
    };

    let segments = Tab::ALL
        .iter()
        .map(|each| {
            let each = *each;
            Segment::new(
                each.label(),
                tab == each,
                cx.listener(move |console, _, window, cx| set_tab(console, each, window, cx)),
            )
            .icon(each.icon())
        })
        .collect();
    let blocked_note = Tab::ALL
        .iter()
        .find_map(|each| tab_blocked(*each, operator, running))
        .map(|reason| format!("Topics, Messages and Publish: {reason}"));

    let controls = vec![
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .w(px(200.0))
            .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
            .children(search_field.map(|field| div().flex_1().child(field)))
            .into_any_element(),
        IconButton::new("mqtt-refresh", Icon::RefreshCw)
            .tooltip("Refresh  R")
            .on_click(cx.listener(|console, _, _, cx| refresh(console, cx)))
            .into_any_element(),
        Button::new("mqtt-config", "Configure")
            .icon(Icon::Settings)
            .tooltip(if operator { "C" } else { OPERATORS })
            .disabled(!operator)
            .on_click(cx.listener(|console, _, window, cx| open_config(console, window, cx)))
            .into_any_element(),
        Button::new(
            "mqtt-start-stop",
            match (busy, running) {
                (true, true) => "Stopping…",
                (true, false) => "Starting…",
                (false, true) => "Stop",
                (false, false) => "Start",
            },
        )
        .map(|button| {
            if running {
                button.danger()
            } else {
                button.primary()
            }
        })
        .icon(if running { Icon::Square } else { Icon::Play })
        .tooltip(if operator { "S" } else { OPERATORS })
        .disabled(!operator || busy || status.is_none())
        .on_click(cx.listener(|console, _, _, cx| start_stop(console, cx)))
        .into_any_element(),
    ];

    let body = match tab {
        Tab::Overview => overview(console, status.as_ref(), window, cx),
        Tab::Topics => topics_tab(console, window, cx),
        Tab::Messages => messages_tab(console, window, cx),
        Tab::Publish => publish_tab(console, cx),
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("MQTT broker", subtitle, controls, cx))
        .child(
            div()
                .key_context("EngMqtt")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &EngMqttNextTab, window, cx| {
                    step_tab(console, true, window, cx);
                }))
                .on_action(cx.listener(|console, _: &EngMqttPrevTab, window, cx| {
                    step_tab(console, false, window, cx);
                }))
                .on_action(
                    cx.listener(|console, _: &EngMqttStartStop, _, cx| start_stop(console, cx)),
                )
                .on_action(cx.listener(|console, _: &EngMqttConfigure, window, cx| {
                    open_config(console, window, cx);
                }))
                .on_action(cx.listener(|console, _: &EngMqttPublish, window, cx| {
                    set_tab(console, Tab::Publish, window, cx);
                }))
                .on_action(cx.listener(|console, _: &EngMqttRefresh, _, cx| refresh(console, cx)))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(SPACE_5)
                        .px(SPACE_9)
                        .py(SPACE_4)
                        .border_b_1()
                        .border_color(theme.border)
                        .child(segmented("mqtt-tabs", segments, cx))
                        .when_some(blocked_note, |this, note| {
                            this.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(SPACE_2)
                                    .min_w_0()
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child(vt_ui::icon(Icon::Lock, ICON_SM, theme.fg_dim))
                                    .child(div().truncate().child(note)),
                            )
                        }),
                )
                .child(body),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn overview(
    console: &mut Console,
    status: Option<&Result<BrokerStatus, String>>,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let status = match status {
        None => return ops_kit::notice("Loading the broker's status…", false, &theme),
        Some(Err(error)) => return ops_kit::notice(format!("Broker: {error}"), true, &theme),
        Some(Ok(status)) => status.clone(),
    };
    let color = state_color(&status, &theme);
    let config = status.config;
    let clients: Rc<[model::ConnectedClient]> = status.clients.clone().into();
    let count = clients.len();
    let zone = console.zone.clone();
    let list = keyboard_list(
        ListKeys {
            id: CLIENTS,
            on_move: None,
            on_activate: copy_client,
        },
        count,
        rows_on_screen(window, ROW),
        console,
        cx,
    );
    let nav = console.list(CLIENTS, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let now = time::OffsetDateTime::now_utc().unix_timestamp();

    let summary = card(&theme)
        .flex_none()
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .px(SPACE_7)
                .py(SPACE_6)
                .child(status_dot(color))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .child(
                            div()
                                .text_size(FONT_TITLE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Embedded MQTT broker"),
                        )
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(color)
                                .child(status.headline()),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(SPACE_8)
                .px(SPACE_7)
                .pb(SPACE_6)
                .child(stat("Status", status.status.clone(), &theme))
                .child(stat(
                    "Port",
                    config.map_or("—".to_owned(), |config| config.port.to_string()),
                    &theme,
                ))
                .child(stat(
                    "Max connections",
                    config.map_or("—".to_owned(), |config| config.max_connections.to_string()),
                    &theme,
                ))
                .child(stat(
                    "Authentication",
                    config.map_or("—".to_owned(), |config| {
                        if config.auth_enabled { "password" } else { "none" }.to_owned()
                    }),
                    &theme,
                ))
                .child(stat("Clients", status.connected_clients.to_string(), &theme)),
        )
        .when(config.is_none(), |this| {
            this.child(
                div()
                    .px(SPACE_7)
                    .pb(SPACE_6)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child("No configuration yet: Configure sets one (the server's file has none), then Start."),
            )
        });

    let rows = uniform_list(
        "mqtt-clients",
        count + tail_rows(count, ROW),
        cx.processor(
            move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = vt_ui::theme::theme(cx).clone();
                range
                    .map(|index| match clients.get(index) {
                        Some(client) => div()
                            .id(("mqtt-client", index))
                            .relative()
                            .flex()
                            .items_center()
                            .gap(SPACE_5)
                            .h(px(ROW))
                            .px(SPACE_7)
                            .border_b_1()
                            .border_color(theme.border.alpha(0.6))
                            .when(cursor == Some(index), |this| this.bg(theme.selected))
                            .when(focused && cursor == Some(index), |this| {
                                this.child(focus_ring(&theme))
                            })
                            .child(
                                div()
                                    .w(px(220.0))
                                    .flex_none()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .child(client.client_id.clone()),
                            )
                            .child(
                                div()
                                    .w(px(150.0))
                                    .flex_none()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child(format!(
                                        "{} · {} ago",
                                        local_time(client.connected_at, &zone),
                                        age(now - client.connected_at)
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.accent)
                                    .child(if client.subscriptions.is_empty() {
                                        "—".to_owned()
                                    } else {
                                        client.subscriptions.join("  ")
                                    }),
                            )
                            .into_any_element(),
                        None => tail_row(ROW),
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .size_full();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .gap(SPACE_6)
        .p(SPACE_9)
        .child(summary)
        .child(
            card(&theme)
                .flex_1()
                .min_h_0()
                .child(card_title("Connected clients", Some(count), None, &theme))
                .child(if count == 0 {
                    ops_kit::notice(
                        if status.running() {
                            "No clients connected"
                        } else {
                            "The broker is not running"
                        },
                        false,
                        &theme,
                    )
                } else {
                    list.flex_1().min_h_0().child(rows).into_any_element()
                }),
        )
        .into_any_element()
}

/// The first `max` lines of `text` and how many more there are.
fn capped_lines(text: &str, max: usize) -> (impl Iterator<Item = &str>, usize) {
    let hidden = text.lines().count().saturating_sub(max);
    (text.lines().take(max), hidden)
}

fn payload_block(payload: &str, theme: &Theme) -> Div {
    let (text, json) = model::pretty_payload(payload);
    let (lines, hidden) = capped_lines(&text, PAYLOAD_LINES);
    div()
        .p(SPACE_4)
        .rounded(RADIUS_MD)
        .bg(theme.bg)
        .font_family(MONO)
        .text_size(FONT_SMALL)
        .text_color(if json { theme.accent } else { theme.fg })
        .children(lines.map(|line| div().child(line.to_owned())))
        .when(hidden > 0, |this| {
            this.child(
                div()
                    .text_color(theme.fg_dim)
                    .child(format!("… {hidden} more lines")),
            )
        })
}

#[allow(clippy::too_many_lines)]
fn topics_tab(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let rows: Rc<[TopicRow]> = topic_rows(console, cx).into();
    let count = rows.len();
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let state = console.state::<State>();
    let selected = state.selected.clone();
    let (infos, error) = match &state.topics {
        Some(Ok(topics)) => (Some(topics.clone()), None),
        Some(Err(error)) => (None, Some(error.clone())),
        None => (None, None),
    };
    let loaded = state.topics.is_some();
    let topic_messages = state.topic_messages.clone();
    let list = keyboard_list(
        ListKeys {
            id: TOPICS,
            on_move: None,
            on_activate: activate_topic,
        },
        count,
        rows_on_screen(window, ROW),
        console,
        cx,
    );
    let nav = console.list(TOPICS, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let total = infos.as_ref().map_or(0, |topics| topics.topics.len());

    let left = if let Some(error) = error {
        ops_kit::notice(format!("Topics: {error}"), true, &theme)
    } else if !loaded {
        ops_kit::notice("Loading topics…", false, &theme)
    } else if count == 0 {
        ops_kit::notice(
            if total == 0 {
                "No topics yet. Connect a client and publish to see topics here."
            } else {
                "No topic matches the filter."
            },
            false,
            &theme,
        )
    } else {
        let infos = infos.clone();
        let selected_row = selected.clone();
        list.flex_1()
            .min_h_0()
            .child(
                uniform_list(
                    "mqtt-topics",
                    count + tail_rows(count, ROW),
                    cx.processor(
                        move |_: &mut Console,
                              range: Range<usize>,
                              _,
                              cx: &mut Context<Console>| {
                            let theme = vt_ui::theme::theme(cx).clone();
                            range
                                .map(|index| match rows.get(index) {
                                    Some(line) => {
                                        let info = line.topic.and_then(|at| {
                                            infos.as_ref().and_then(|topics| topics.topics.get(at))
                                        });
                                        let picked =
                                            selected_row.as_deref() == Some(line.path.as_str());
                                        let path = line.path.clone();
                                        let has_children = line.has_children;
                                        let is_topic = line.topic.is_some();
                                        div()
                                            .id(("mqtt-topic", index))
                                            .relative()
                                            .flex()
                                            .items_center()
                                            .gap(SPACE_3)
                                            .h(px(ROW))
                                            .pl(px(16.0 + 16.0 * line.depth as f32))
                                            .pr(SPACE_6)
                                            .cursor_pointer()
                                            .when(picked, |this| this.bg(theme.accent_soft))
                                            .when(!picked && cursor == Some(index), |this| {
                                                this.bg(theme.selected)
                                            })
                                            .when(focused && cursor == Some(index), |this| {
                                                this.child(focus_ring(&theme))
                                            })
                                            .on_click(cx.listener(move |console, _, window, cx| {
                                                console.list(TOPICS, cx).select(index);
                                                let focus = console.list(TOPICS, cx).focus.clone();
                                                window.focus(&focus);
                                                if has_children {
                                                    let state = console.state::<State>();
                                                    if !state.expanded.remove(&path) {
                                                        state.expanded.insert(path.clone());
                                                    }
                                                }
                                                if is_topic {
                                                    select_topic(console, path.clone(), cx);
                                                }
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .w(px(14.0))
                                                    .flex_none()
                                                    .text_color(theme.fg_dim)
                                                    .child(if has_children {
                                                        if line.expanded { "▾" } else { "▸" }
                                                    } else {
                                                        "•"
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .font_family(MONO)
                                                    .text_size(FONT_SMALL)
                                                    .text_color(if is_topic {
                                                        theme.fg
                                                    } else {
                                                        theme.fg_muted
                                                    })
                                                    .child(line.name.clone()),
                                            )
                                            .when_some(info, |this, info| {
                                                this.child(
                                                    div()
                                                        .flex_none()
                                                        .font_family(MONO)
                                                        .text_size(FONT_CAPTION)
                                                        .text_color(theme.fg_dim)
                                                        .child(format!(
                                                            "{} ago",
                                                            age(now - info.last_update)
                                                        )),
                                                )
                                                .child(pill(
                                                    info.message_count.to_string(),
                                                    theme.accent,
                                                ))
                                            })
                                            .into_any_element()
                                    }
                                    None => tail_row(ROW),
                                })
                                .collect::<Vec<_>>()
                        },
                    ),
                )
                .track_scroll(scroll)
                .size_full(),
            )
            .into_any_element()
    };

    let info = selected.as_ref().and_then(|topic| {
        infos.as_ref().and_then(|topics| {
            topics
                .topics
                .iter()
                .find(|info| &info.topic == topic)
                .cloned()
        })
    });
    let detail = match &selected {
        None => ops_kit::notice(
            "Pick a topic (↵) to see its last value and messages",
            false,
            &theme,
        ),
        Some(topic) => div()
            .id("mqtt-topic-detail")
            .flex()
            .flex_col()
            .gap(SPACE_5)
            .p(SPACE_7)
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .font_family(MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(topic.clone()),
            )
            .when_some(info.clone(), |this, info| {
                this.child(
                    div()
                        .flex()
                        .gap(SPACE_6)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(format!("{} messages", info.message_count))
                        .child(format!("updated {} ago", age(now - info.last_update)))
                        .when(info.retain, |this| {
                            this.child(pill("RETAIN", theme.warning))
                        }),
                )
                .child(ops_kit::labelled(
                    "Last value",
                    payload_block(&info.last_payload, &theme),
                    &theme,
                ))
            })
            .child(ops_kit::labelled(
                "Recent messages",
                match &topic_messages {
                    None => div().text_color(theme.fg_dim).child("Loading…"),
                    Some(Err(error)) => div().text_color(theme.danger).child(error.clone()),
                    Some(Ok(answer)) if answer.messages.is_empty() => {
                        div().text_color(theme.fg_dim).child("No messages yet")
                    }
                    Some(Ok(answer)) => div().flex().flex_col().gap(SPACE_4).children(
                        answer.messages.iter().rev().map(|message| {
                            div()
                                .flex()
                                .flex_col()
                                .gap(SPACE_1)
                                .child(
                                    div()
                                        .flex()
                                        .gap(SPACE_3)
                                        .font_family(MONO)
                                        .text_size(FONT_CAPTION)
                                        .text_color(theme.fg_dim)
                                        .child(local_time(message.timestamp, &console.zone))
                                        .when(message.retain, |this| {
                                            this.child(pill("RETAIN", theme.warning))
                                        }),
                                )
                                .child(payload_block(&message.payload, &theme))
                        }),
                    ),
                },
                &theme,
            ))
            .pb(LIST_TAIL_PX)
            .into_any_element(),
    };

    div()
        .flex()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex_none()
                        .px(SPACE_7)
                        .py(SPACE_3)
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.fg_dim)
                        .child(format!("TOPIC TREE · {total} TOPICS · ↵ open/pick")),
                )
                .child(left),
        )
        .child(
            div()
                .w(px(DETAIL))
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .child(detail),
        )
        .into_any_element()
}

/// Room under the scrolling detail, clear of the toasts.
const LIST_TAIL_PX: gpui::Pixels = crate::console::views::LIST_TAIL;

#[allow(clippy::too_many_lines)]
fn messages_tab(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let messages: Rc<[model::RecentMessage]> = log(console, cx).into();
    let count = messages.len();
    let zone = console.zone.clone();
    // The list's rows own a copy; the detail below reads its own.
    let rows_zone = zone.clone();
    let loaded = console
        .state_ref::<State>()
        .and_then(|state| state.messages.as_ref())
        .map(|answer| answer.as_ref().map(|_| ()).map_err(Clone::clone));
    let list = keyboard_list(
        ListKeys {
            id: MESSAGES,
            on_move: None,
            on_activate: copy_payload,
        },
        count,
        rows_on_screen(window, ROW),
        console,
        cx,
    );
    let nav = console.list(MESSAGES, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let picked = cursor.and_then(|row| messages.get(row).cloned());

    let left = match loaded {
        None => ops_kit::notice("Loading messages…", false, &theme),
        Some(Err(error)) => ops_kit::notice(format!("Messages: {error}"), true, &theme),
        Some(Ok(())) if count == 0 => ops_kit::notice(
            "No messages yet. Publish one (P) to see it here.",
            false,
            &theme,
        ),
        Some(Ok(())) => list
            .flex_1()
            .min_h_0()
            .child(
                uniform_list(
                    "mqtt-messages",
                    count + tail_rows(count, ROW),
                    cx.processor(
                        move |_: &mut Console,
                              range: Range<usize>,
                              _,
                              cx: &mut Context<Console>| {
                            let theme = vt_ui::theme::theme(cx).clone();
                            range
                                .map(|index| match messages.get(index) {
                                    Some(message) => div()
                                        .id(("mqtt-message", index))
                                        .relative()
                                        .flex()
                                        .items_center()
                                        .gap(SPACE_5)
                                        .h(px(ROW))
                                        .px(SPACE_7)
                                        .border_b_1()
                                        .border_color(theme.border.alpha(0.6))
                                        .cursor_pointer()
                                        .when(cursor == Some(index), |this| this.bg(theme.selected))
                                        .when(focused && cursor == Some(index), |this| {
                                            this.child(focus_ring(&theme))
                                        })
                                        .on_click(cx.listener(move |console, _, window, cx| {
                                            console.list(MESSAGES, cx).select(index);
                                            let focus = console.list(MESSAGES, cx).focus.clone();
                                            window.focus(&focus);
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .w(px(76.0))
                                                .flex_none()
                                                .font_family(MONO)
                                                .text_size(FONT_SMALL)
                                                .text_color(theme.fg_dim)
                                                .child(local_time(message.timestamp, &rows_zone)),
                                        )
                                        .child(
                                            div()
                                                .w(px(220.0))
                                                .flex_none()
                                                .truncate()
                                                .font_family(MONO)
                                                .text_size(FONT_SMALL)
                                                .text_color(theme.accent)
                                                .child(message.topic.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .font_family(MONO)
                                                .text_size(FONT_SMALL)
                                                .child(message.payload.replace('\n', " ")),
                                        )
                                        .when(message.retain, |this| {
                                            this.child(pill("RETAIN", theme.warning))
                                        })
                                        .into_any_element(),
                                    None => tail_row(ROW),
                                })
                                .collect::<Vec<_>>()
                        },
                    ),
                )
                .track_scroll(scroll)
                .size_full(),
            )
            .into_any_element(),
    };

    let detail = match picked {
        None => ops_kit::notice(
            "Move to a message to read it; ↵ copies its payload",
            false,
            &theme,
        ),
        Some(message) => div()
            .id("mqtt-message-detail")
            .flex()
            .flex_col()
            .gap(SPACE_4)
            .p(SPACE_7)
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .font_family(MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(message.topic.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(local_time(message.timestamp, &zone))
                    .when(message.retain, |this| {
                        this.child(pill("RETAIN", theme.warning))
                    }),
            )
            .child(payload_block(&message.payload, &theme))
            .pb(LIST_TAIL_PX)
            .into_any_element(),
    };

    div()
        .flex()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(SPACE_3)
                        .px(SPACE_7)
                        .py(SPACE_3)
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.fg_dim)
                        .child(format!("MESSAGE STREAM · {count} · NEWEST FIRST"))
                        .child(pill("LIVE · 2 s", theme.success)),
                )
                .child(left),
        )
        .child(
            div()
                .w(px(DETAIL))
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .child(detail),
        )
        .into_any_element()
}

fn publish_tab(console: &mut Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let (topic, payload) = (state.topic.clone(), state.payload.clone());
    let (error, publishing) = (state.publish_error.clone(), state.publishing);
    let typed = topic
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    let suggestions: Vec<String> = match &state.topics {
        Some(Ok(topics)) => model::suggestions(&topics.topics, &typed)
            .into_iter()
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };
    div()
        .id("mqtt-publish")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .p(SPACE_9)
        .child(
            card(&theme)
                .max_w(px(640.0))
                .p(SPACE_7)
                .gap(SPACE_6)
                .child(
                    div()
                        .text_size(FONT_EMPHASIS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Publish a message"),
                )
                .children(topic.map(|field| ops_kit::labelled("Topic", field, &theme)))
                .when(!suggestions.is_empty(), |this| {
                    this.child(div().flex().flex_wrap().gap(SPACE_2).children(
                        suggestions.into_iter().enumerate().map(|(at, suggestion)| {
                            Button::new(("mqtt-suggestion", at), suggestion.clone())
                                .compact()
                                .ghost()
                                .on_click(cx.listener(move |console, _, _, cx| {
                                    set_topic(console, suggestion.clone(), cx);
                                }))
                        }),
                    ))
                })
                .children(payload.map(|field| ops_kit::labelled("Payload", field, &theme)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(SPACE_2)
                                .map(|this| match &error {
                                    Some(error) => this
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.danger)
                                        .child(vt_ui::icon(
                                            Icon::CircleAlert,
                                            ICON_SM,
                                            theme.danger,
                                        ))
                                        .child(div().min_w_0().truncate().child(error.clone())),
                                    None => this
                                        .text_size(FONT_CAPTION)
                                        .text_color(theme.fg_dim)
                                        .child(kbd("↵", cx))
                                        .child("in the topic or")
                                        .child(kbd("⌘↵", cx))
                                        .child("in the payload publishes"),
                                }),
                        )
                        .child(
                            Button::new(
                                "mqtt-publish-send",
                                if publishing {
                                    "Publishing…"
                                } else {
                                    "Publish"
                                },
                            )
                            .primary()
                            .icon(Icon::Send)
                            .disabled(publishing || typed.trim().is_empty())
                            .on_click(cx.listener(|console, _, _, cx| publish(console, cx))),
                        ),
                ),
        )
        .into_any_element()
}

fn config_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let running = console.state::<State>().running();
    let Some(form) = console.state::<State>().form.as_ref() else {
        return div().into_any_element();
    };
    let (port, max, auth) = (form.port.clone(), form.max.clone(), form.auth);
    let users = form.users.clone();
    let (error, saving) = (form.error.clone(), form.saving);
    let set_auth = |on: bool| {
        move |console: &mut Console,
              _: &gpui::ClickEvent,
              _: &mut Window,
              cx: &mut Context<Console>| {
            if let Some(form) = console.state::<State>().form.as_mut() {
                form.auth = on;
            }
            cx.notify();
        }
    };
    div()
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(vt_ui::icon(Icon::Settings, ICON_LG, theme.accent))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Broker configuration"),
                ),
        )
        .when(running, |this| {
            this.child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.warning)
                    .child("The broker is running: stop it before saving (the server refuses changes while it runs)."),
            )
        })
        .child(
            div()
                .flex()
                .gap(SPACE_6)
                .child(div().flex_1().child(ops_kit::labelled("Port", port, &theme)))
                .child(div().flex_1().child(ops_kit::labelled("Max connections", max, &theme))),
        )
        .child(ops_kit::labelled(
            "Authentication required",
            segmented(
                "mqtt-auth",
                vec![
                    Segment::new("Off", !auth, cx.listener(set_auth(false))),
                    Segment::new("On", auth, cx.listener(set_auth(true))).icon(Icon::Lock),
                ],
                cx,
            ),
            &theme,
        ))
        .when(auth, |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_3)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(FONT_CAPTION)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.fg_dim)
                                    .child(format!("USERS · {}", users.len())),
                            )
                            .child(
                                Button::new("mqtt-add-user", "Add user")
                                    .compact()
                                    .icon(Icon::Plus)
                                    .on_click(cx.listener(|console, _, window, cx| add_user(console, window, cx))),
                            ),
                    )
                    .when(users.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child("No users: with none the server treats authentication as off."),
                        )
                    })
                    .children(users.into_iter().enumerate().map(|(at, (username, password))| {
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_3)
                            .child(div().flex_1().child(username))
                            .child(div().flex_1().child(password))
                            .child(
                                IconButton::new(("mqtt-remove-user", at), Icon::X)
                                    .tooltip("Remove")
                                    .color(theme.danger)
                                    .on_click(cx.listener(move |console, _, _, cx| {
                                        if let Some(form) = console.state::<State>().form.as_mut()
                                            && at < form.users.len()
                                        {
                                            form.users.remove(at);
                                        }
                                        cx.notify();
                                    })),
                            )
                    })),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .map(|this| match &error {
                            Some(error) => this
                                .text_size(FONT_SMALL)
                                .text_color(theme.danger)
                                .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.danger))
                                .child(div().min_w_0().truncate().child(error.clone())),
                            None => this
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .child(kbd("↵", cx))
                                .child("save")
                                .child(kbd("Esc", cx))
                                .child("close"),
                        }),
                )
                .child(
                    Button::new("mqtt-config-cancel", "Cancel")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx))),
                )
                .child(
                    Button::new("mqtt-config-save", if saving { "Saving…" } else { "Save" })
                        .primary()
                        .icon(Icon::Check)
                        .disabled(saving || running)
                        .on_click(cx.listener(|console, _, _, cx| save_config(console, cx))),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_follow_the_role_and_the_broker() {
        assert_eq!(tab_blocked(Tab::Overview, false, false), None);
        assert_eq!(tab_blocked(Tab::Topics, false, true), Some(OPERATORS));
        assert_eq!(
            tab_blocked(Tab::Messages, true, false),
            Some("Start the broker first")
        );
        assert_eq!(tab_blocked(Tab::Publish, true, true), None);
    }

    #[test]
    fn the_state_knows_a_running_broker() {
        let mut state = State::default();
        assert!(!state.running());
        state.status = Some(Ok(BrokerStatus {
            status: "running".into(),
            ..BrokerStatus::default()
        }));
        assert!(state.running());
        state.status = Some(Err("down".into()));
        assert!(!state.running());
    }

    #[test]
    fn a_failed_status_poll_keeps_the_tab() {
        let mut state = State {
            tab: Tab::Topics,
            ..State::default()
        };
        apply_status(&mut state, Err("timeout".into()));
        assert_eq!(state.tab, Tab::Topics, "a failed poll keeps the tab");
        apply_status(
            &mut state,
            Ok(BrokerStatus {
                status: "stopped".into(),
                ..BrokerStatus::default()
            }),
        );
        assert_eq!(
            state.tab,
            Tab::Overview,
            "a stopped broker leaves the overview"
        );
    }

    #[test]
    fn a_payload_shows_capped_lines() {
        let text = (0..100)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (lines, hidden) = capped_lines(&text, PAYLOAD_LINES);
        assert_eq!(lines.count(), PAYLOAD_LINES);
        assert_eq!(hidden, 100 - PAYLOAD_LINES);
        let (lines, hidden) = capped_lines("a\nb", PAYLOAD_LINES);
        assert_eq!((lines.count(), hidden), (2, 0));
    }

    mod ui {
        use super::super::*;
        use crate::console::pages::Role;
        use crate::console::testing::Harness;
        use crate::console::views::data_manager::admin_kit;
        use gpui::TestAppContext;

        fn open(h: &mut Harness) {
            h.sign_in("admin");
            h.update(|console, window, cx| {
                assert!(admin_kit::open_section(
                    console,
                    ID,
                    Role::Any,
                    "MQTT",
                    window,
                    cx
                ));
            });
            h.draw();
            h.calls();
        }

        #[gpui::test]
        fn publishing_again_keeps_one_payload_subscription(cx: &mut TestAppContext) {
            let mut h = Harness::new(cx);
            open(&mut h);
            h.update(ensure_publish);
            let before = h.update(|console, _, _| console.state::<State>()._publish.len());
            for _ in 0..3 {
                h.update(new_payload);
            }
            h.update(|console, _, _| {
                let state = console.state::<State>();
                assert_eq!(state._publish.len(), before, "no subscription per publish");
                assert!(state._payload.is_some());
            });
        }

        #[gpui::test]
        fn a_closed_config_form_keeps_no_password(cx: &mut TestAppContext) {
            let mut h = Harness::new(cx);
            open(&mut h);
            h.update(open_config);
            h.update(add_user);
            h.update(|console, _, cx| {
                let form = console
                    .state::<State>()
                    .form
                    .as_ref()
                    .expect("the form is open");
                let password = form.users[0].1.clone();
                password.update(cx, |field, cx| field.set_text("hunter2", cx));
            });
            h.update(|console, window, cx| console.close_overlay(window, cx));
            h.draw();
            assert!(
                h.update(|console, _, _| console.state::<State>().form.is_none()),
                "closing the dialog drops its passwords"
            );
        }

        #[gpui::test]
        fn a_late_save_error_is_shown(cx: &mut TestAppContext) {
            let mut h = Harness::new(cx);
            open(&mut h);
            h.update(open_config);
            h.update(|console, _, cx| save_config(console, cx));
            let seq = h.update(|console, _, _| console.state::<State>().config_seq);
            assert!(
                h.calls()
                    .contains(&CallKey::new("data.mqtt.config").with(seq))
            );
            h.update(|console, window, cx| console.close_overlay(window, cx));
            h.reply(
                CallKey::new("data.mqtt.config").with(seq),
                Reply::failed("port 1883 in use"),
            );
            assert!(
                h.update(|console, _, _| console
                    .toasts
                    .iter()
                    .any(|toast| toast.error && toast.message.contains("port 1883 in use"))),
                "the error reaches the user after the dialog closed"
            );
        }
    }
}
