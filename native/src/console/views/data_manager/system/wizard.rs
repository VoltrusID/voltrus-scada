//! System › Wizard, after the web client's `ProjectWizard.tsx` and
//! `QuickStartGuide.tsx`, both sending `POST /api/v1/wizard/create`:
//! - Quick start: a Modbus PLC's address, then the registers it found and
//!   the tags made from them, then done.
//! - Full wizard: industry → template (a vertical template from
//!   `GET /api/v1/templates`, or a FEED reference design from
//!   `GET /api/v1/wizard/feed-templates[/{id}]`, or none) → device →
//!   auto-scan (Modbus) → review → result.
//!
//! Keys: ↵ goes on (in a list it picks the row and goes on; Space only
//! picks), Esc goes back a step, [ and ] switch quick start and the full
//! wizard (on the template step: vertical and FEED templates).

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, SharedString,
    Subscription, Window, div, px,
};
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::eng_system::{
    FEED_TEMPLATES_PATH, FeedTemplate, FeedTemplateList, FeedTemplateSummary, INDUSTRIES,
    PROTOCOLS, REGISTER_AREAS, TemplateList, TemplateSource, TemplateSummary, WIZARD_CREATE_PATH,
    WizardConfig, WizardDraft, WizardResult, WizardStep, feed_template_path, quick_start_config,
    templates_path,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, field, kbd, pill, section_label, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::{Tab, form_note, set_tab};
use crate::console::keys::Dismiss;
use crate::console::views::ops_kit::on_field;
use crate::console::views::{LIST_TAIL, ListKeys, OnActivate, card, focus_ring, keyboard_list};
use crate::console::{Console, Page};

const INDUSTRY_LIST: &str = "data.system.wizard.industry";
const TEMPLATE_LIST: &str = "data.system.wizard.template";
const PROTOCOL_LIST: &str = "data.system.wizard.protocol";
const OPTIONS_LIST: &str = "data.system.wizard.options";
const AREA_LIST: &str = "data.system.wizard.area";
const SCROLL: &str = "data.system.wizard";
const PICK_ROW: f32 = 30.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Quick,
    Full,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum QuickStep {
    #[default]
    Connect,
    Results,
    Done,
}

struct Fields {
    address: Entity<TextInput>,
    port: Entity<TextInput>,
    unit_id: Entity<TextInput>,
    device_id: Entity<TextInput>,
    instances: Entity<TextInput>,
    start: Entity<TextInput>,
    end: Entity<TextInput>,
    ip: Entity<TextInput>,
    quick_port: Entity<TextInput>,
    quick_unit: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
struct State {
    mode: Mode,
    step: WizardStep,
    draft: WizardDraft,
    quick: QuickStep,
    fields: Option<Fields>,
    error: Option<String>,
    creating: bool,
    create_seq: i64,
    /// The full wizard's answer, and the quick start's.
    result: Option<WizardResult>,
    quick_result: Option<WizardResult>,
    asked: bool,
    /// The last ask for the template lists; older answers are dropped.
    lists_seq: i64,
    verticals: Option<Result<Vec<TemplateSummary>, String>>,
    feeds: Option<Result<Vec<FeedTemplateSummary>, String>>,
    feed: Option<Result<FeedTemplate, String>>,
    feed_seq: i64,
    /// The wizard's own focus, for the steps without a field or list (↵
    /// goes on there).
    body: Option<FocusHandle>,
}

pub(super) fn subtitle(console: &Console) -> String {
    let mode = console
        .state_ref::<State>()
        .map_or(Mode::Quick, |state| state.mode);
    match mode {
        Mode::Quick => {
            "Connect a Modbus PLC and discover its tags · ↵ go on · Esc back · [ ] full wizard"
                .to_owned()
        }
        Mode::Full => {
            "Set up a project step by step · ↵ next · Esc back · [ ] quick start".to_owned()
        }
    }
}

pub(super) fn open(console: &mut Console) {
    if !console.state::<State>().asked {
        refresh(console);
    }
}

/// The templates the full wizard offers, again.
pub(super) fn refresh(console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    state.asked = true;
    state.lists_seq += 1;
    let seq = state.lists_seq;
    console.call(
        CallKey::new("data.system.wizard.verticals").with(seq),
        Call::new(|api| async move {
            let list: TemplateList = api.get_json(&templates_path(None)).await?;
            anyhow::Ok(list.templates)
        }),
    );
    console.call(
        CallKey::new("data.system.wizard.feeds").with(seq),
        Call::get::<FeedTemplateList>(FEED_TEMPLATES_PATH),
    );
    let state = console.state::<State>();
    if state.draft.source == TemplateSource::Feed
        && let Some(id) = state.draft.template.clone()
    {
        fetch_feed(console, &id);
    }
}

fn fetch_feed(console: &mut Console, id: &str) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    state.feed_seq += 1;
    state.feed = None;
    let seq = state.feed_seq;
    console.call(
        CallKey::new("data.system.wizard.feed").with(seq),
        Call::get::<FeedTemplate>(feed_template_path(id)),
    );
}

pub(super) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    let state = console.state::<State>();
    match key.name {
        "data.system.wizard.verticals" => {
            if key.arg == state.lists_seq {
                state.verticals = Some(reply.take::<Vec<TemplateSummary>>());
            }
        }
        "data.system.wizard.feeds" => {
            if key.arg == state.lists_seq {
                state.feeds = Some(reply.take::<FeedTemplateList>().map(|list| list.templates));
            }
        }
        "data.system.wizard.feed" => {
            if key.arg == state.feed_seq {
                state.feed = Some(reply.take::<FeedTemplate>());
            }
        }
        "data.system.wizard.create" => {
            if key.arg != state.create_seq {
                return;
            }
            state.creating = false;
            match reply.take::<WizardResult>() {
                Ok(result) => {
                    state.error = None;
                    let device = result.device_id.clone();
                    match state.mode {
                        Mode::Full => {
                            state.result = Some(result);
                            state.step = WizardStep::Result;
                        }
                        Mode::Quick => {
                            state.quick_result = Some(result);
                            state.quick = QuickStep::Results;
                        }
                    }
                    console.refresh_tags();
                    console.show_toast(format!("Project set up on device {device}"), false, cx);
                }
                Err(error) => state.error = Some(error),
            }
        }
        _ => {}
    }
}

fn feeds(state: &State) -> Vec<FeedTemplateSummary> {
    match &state.feeds {
        Some(Ok(templates)) => templates.clone(),
        _ => Vec::new(),
    }
}

// ── Fields ───────────────────────────────────────────────────────────────

fn ensure_fields(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().fields.is_some() {
        return;
    }
    let draft = console.state::<State>().draft.clone();
    let make =
        |text: String, placeholder: &'static str, kind: InputKind, cx: &mut Context<Console>| {
            cx.new(|cx| {
                TextInput::new(cx)
                    .text(text)
                    .placeholder(placeholder)
                    .kind(kind)
                    .mono()
            })
        };
    let address = make(draft.address.clone(), "192.168.1.100", InputKind::Text, cx);
    let port = make(draft.port.clone(), "502", InputKind::Unsigned, cx);
    let unit_id = make(draft.unit_id.clone(), "1", InputKind::Unsigned, cx);
    let device_id = make(
        String::new(),
        "made from the protocol and industry",
        InputKind::Text,
        cx,
    );
    let instances = make(draft.instances.clone(), "4", InputKind::Unsigned, cx);
    let start = make(draft.start.clone(), "0", InputKind::Unsigned, cx);
    let end = make(draft.end.clone(), "499", InputKind::Unsigned, cx);
    let ip = make(
        "192.168.1.100".to_owned(),
        "192.168.1.100",
        InputKind::Text,
        cx,
    );
    let quick_port = make("502".to_owned(), "502", InputKind::Unsigned, cx);
    let quick_unit = make("1".to_owned(), "1", InputKind::Unsigned, cx);
    let subscriptions = [
        &address,
        &port,
        &unit_id,
        &device_id,
        &instances,
        &start,
        &end,
        &ip,
        &quick_port,
        &quick_unit,
    ]
    .into_iter()
    .map(|field| on_field(field, on_input, window, cx))
    .collect();
    console.state::<State>().fields = Some(Fields {
        address,
        port,
        unit_id,
        device_id,
        instances,
        start,
        end,
        ip,
        quick_port,
        quick_unit,
        _subscriptions: subscriptions,
    });
}

fn on_input(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Submitted => next(console, window, cx),
        InputEvent::Cancelled => {
            if !back(console, window, cx) {
                focus_body(console, window, cx);
            }
        }
        InputEvent::Changed => cx.notify(),
        InputEvent::Blurred => {}
    }
}

/// Copy the text fields into the draft.
fn collect(console: &mut Console, cx: &gpui::App) {
    let state = console.state::<State>();
    let Some(fields) = &state.fields else {
        return;
    };
    let text = |field: &Entity<TextInput>| field.read(cx).value().to_owned();
    state.draft.address = text(&fields.address);
    state.draft.port = text(&fields.port);
    state.draft.unit_id = text(&fields.unit_id);
    state.draft.device_id = text(&fields.device_id);
    state.draft.instances = text(&fields.instances);
    state.draft.start = text(&fields.start);
    state.draft.end = text(&fields.end);
}

fn focus_field(
    console: &mut Console,
    pick: fn(&Fields) -> &Entity<TextInput>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let field = console
        .state_ref::<State>()
        .and_then(|state| state.fields.as_ref())
        .map(|fields| pick(fields).clone());
    if let Some(field) = field {
        field.read(cx).focus(window);
    }
}

fn focus_body(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let handle = console
        .state::<State>()
        .body
        .get_or_insert_with(|| cx.focus_handle())
        .clone();
    window.focus(&handle);
}

pub(super) fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    let state = console.state::<State>();
    match (state.mode, state.step, state.quick) {
        (Mode::Quick, _, QuickStep::Connect) => {
            focus_field(console, |fields| &fields.ip, window, cx)
        }
        (Mode::Quick, _, _) => focus_body(console, window, cx),
        (Mode::Full, WizardStep::Industry, _) => console.focus_list(INDUSTRY_LIST, window, cx),
        (Mode::Full, WizardStep::Template, _) => console.focus_list(TEMPLATE_LIST, window, cx),
        (Mode::Full, WizardStep::Device, _) => console.focus_list(PROTOCOL_LIST, window, cx),
        (Mode::Full, WizardStep::Scan, _) => console.focus_list(AREA_LIST, window, cx),
        (Mode::Full, WizardStep::Review | WizardStep::Result, _) => {
            focus_body(console, window, cx);
        }
    }
    true
}

// ── Steps ────────────────────────────────────────────────────────────────

/// `[` and `]`: on the template step the other kind of template, else the
/// other mode.
pub(super) fn cycle(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.mode == Mode::Full && state.step == WizardStep::Template {
        let source = match state.draft.source {
            TemplateSource::Vertical => TemplateSource::Feed,
            TemplateSource::Feed => TemplateSource::Vertical,
        };
        set_source(console, source, cx);
    } else {
        let mode = match state.mode {
            Mode::Quick => Mode::Full,
            Mode::Full => Mode::Quick,
        };
        set_mode(console, mode, window, cx);
    }
}

fn set_mode(console: &mut Console, mode: Mode, window: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.mode = mode;
    state.error = None;
    focus(console, window, cx);
    cx.notify();
}

fn set_source(console: &mut Console, source: TemplateSource, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.draft.source != source {
        state.draft.source = source;
        state.draft.template = None;
        state.feed = None;
    }
    console.list(TEMPLATE_LIST, cx).cursor = None;
    cx.notify();
}

/// ↵: the next step, or the server.
fn next(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    collect(console, cx);
    let state = console.state::<State>();
    match state.mode {
        Mode::Quick => match state.quick {
            QuickStep::Connect => quick_connect(console, cx),
            QuickStep::Results => {
                state.quick = QuickStep::Done;
                focus_body(console, window, cx);
            }
            QuickStep::Done => {
                state.quick = QuickStep::Connect;
                state.quick_result = None;
                set_tab(console, Tab::Overview, window, cx);
            }
        },
        Mode::Full => match state.step {
            WizardStep::Review => create(console, cx),
            WizardStep::Result => {
                // Create another, as the web wizard's button.
                state.step = WizardStep::Industry;
                state.result = None;
                state.draft.template = None;
                state.error = None;
                focus(console, window, cx);
            }
            step => match state.draft.next(step) {
                Ok(to) => {
                    state.step = to;
                    state.error = None;
                    focus(console, window, cx);
                }
                Err(error) => state.error = Some(error),
            },
        },
    }
    cx.notify();
}

/// Esc: the step before; `false` on the first.
fn back(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    collect(console, cx);
    let state = console.state::<State>();
    let moved = match state.mode {
        Mode::Quick => match state.quick {
            QuickStep::Connect => false,
            QuickStep::Results => {
                state.quick = QuickStep::Connect;
                true
            }
            QuickStep::Done => {
                state.quick = QuickStep::Results;
                true
            }
        },
        Mode::Full => match state.draft.back(state.step) {
            Some(to) => {
                state.step = to;
                true
            }
            None => false,
        },
    };
    if moved {
        console.state::<State>().error = None;
        focus(console, window, cx);
        cx.notify();
    }
    moved
}

fn send_create(console: &mut Console, config: WizardConfig) {
    let state = console.state::<State>();
    state.creating = true;
    state.error = None;
    state.create_seq += 1;
    let seq = state.create_seq;
    console.call(
        CallKey::new("data.system.wizard.create").with(seq),
        Call::send::<WizardResult>(
            Method::POST,
            WIZARD_CREATE_PATH,
            serde_json::to_value(config).unwrap_or_default(),
        ),
    );
}

fn create(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        console.state::<State>().error = Some("A stress test has no server".to_owned());
        return;
    }
    let state = console.state::<State>();
    if state.creating {
        return;
    }
    match state.draft.config() {
        Ok(config) => send_create(console, config),
        Err(error) => state.error = Some(error),
    }
    cx.notify();
}

fn quick_connect(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        console.state::<State>().error = Some("A stress test has no server".to_owned());
        return;
    }
    let state = console.state::<State>();
    if state.creating {
        return;
    }
    let Some(fields) = &state.fields else {
        return;
    };
    let app: &gpui::App = cx;
    let text = |field: &Entity<TextInput>| field.read(app).value().to_owned();
    let (ip, port, unit) = (
        text(&fields.ip),
        text(&fields.quick_port),
        text(&fields.quick_unit),
    );
    match quick_start_config(&ip, &port, &unit) {
        Ok(config) => send_create(console, config),
        Err(error) => state.error = Some(error),
    }
}

// ── Lists ────────────────────────────────────────────────────────────────

fn pick_industry(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some((id, _)) = INDUSTRIES.get(row) {
        console.state::<State>().draft.industry = (*id).to_owned();
        if !toggle {
            next(console, window, cx);
        }
        cx.notify();
    }
}

/// The template step's rows: none, then the kind chosen.
fn template_rows(state: &State) -> Vec<(Option<String>, String, String)> {
    let mut rows = vec![(
        None,
        "None — manual setup".to_owned(),
        "skip the template".to_owned(),
    )];
    match state.draft.source {
        TemplateSource::Vertical => {
            if let Some(Ok(templates)) = &state.verticals {
                rows.extend(templates.iter().map(|template| {
                    (
                        Some(template.id.clone()),
                        template.name.clone(),
                        format!("{} tags · {}", template.tag_count, template.id),
                    )
                }));
            }
        }
        TemplateSource::Feed => {
            rows.extend(feeds(state).into_iter().map(|template| {
                (
                    Some(template.id.clone()),
                    template.name.clone(),
                    format!(
                        "~{} tags · {}",
                        template.estimated_tag_count, template.industry
                    ),
                )
            }));
        }
    }
    rows
}

fn pick_template(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let state = console.state::<State>();
    let Some((id, _, _)) = template_rows(state).into_iter().nth(row) else {
        return;
    };
    let feed = state.draft.source == TemplateSource::Feed;
    let changed = state.draft.template != id;
    state.draft.template = id.clone();
    if feed && changed {
        match &id {
            Some(id) => fetch_feed(console, id),
            None => console.state::<State>().feed = None,
        }
    }
    if !toggle {
        next(console, window, cx);
    }
    cx.notify();
}

fn pick_protocol(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(choice) = PROTOCOLS.get(row).copied() else {
        return;
    };
    collect(console, cx);
    let state = console.state::<State>();
    state.draft.set_protocol(choice.id);
    let port = state.draft.port.clone();
    if let Some(fields) = &state.fields {
        let field = fields.port.clone();
        field.update(cx, |field, cx| field.set_text(port, cx));
    }
    if !toggle {
        focus_field(console, |fields| &fields.address, window, cx);
    }
    cx.notify();
}

fn toggle_option(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let draft = &mut console.state::<State>().draft;
    match row {
        0 => draft.auto_scan = !draft.auto_scan,
        1 => draft.dashboard = !draft.dashboard,
        2 => draft.alarms = !draft.alarms,
        _ => return,
    }
    cx.notify();
}

fn pick_area(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some((id, _)) = REGISTER_AREAS.get(row) {
        console.state::<State>().draft.register_area = *id;
        if !toggle {
            focus_field(console, |fields| &fields.start, window, cx);
        }
        cx.notify();
    }
}

/// One row of a picker.
struct Pick {
    label: SharedString,
    detail: SharedString,
    chosen: bool,
}

/// A short keyboard list: j/k or ↑↓ move, ↵ picks and goes on, Space
/// picks, a click picks.
fn picker(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
    id: &'static str,
    items: Vec<Pick>,
    check: bool,
    on_activate: OnActivate,
) -> AnyElement {
    let theme = theme(cx).clone();
    let len = items.len();
    let list = keyboard_list(
        ListKeys {
            id,
            on_move: None,
            on_activate,
        },
        len,
        len,
        console,
        cx,
    );
    let nav = console.list(id, cx);
    if nav.cursor.is_none() {
        nav.cursor = items.iter().position(|item| item.chosen).or(Some(0));
    }
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));
    list.flex()
        .flex_col()
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(if focused {
            theme.accent
        } else {
            theme.border_strong
        })
        .bg(theme.bg)
        .overflow_hidden()
        .children(items.into_iter().enumerate().map(|(row, item)| {
            let hover = theme.hover;
            let current = cursor == Some(row);
            let icon = match (check, item.chosen) {
                (true, true) => Icon::Check,
                (true, false) => Icon::Square,
                (false, true) => Icon::CircleCheck,
                (false, false) => Icon::Circle,
            };
            div()
                .id(ElementId::NamedInteger(
                    SharedString::from(id),
                    u64::try_from(row).unwrap_or(0),
                ))
                .relative()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(PICK_ROW))
                .px(SPACE_5)
                .cursor_pointer()
                .when(current, |this| this.bg(theme.selected))
                .when(!current, |this| this.hover(move |style| style.bg(hover)))
                .when(focused && current, |this| this.child(focus_ring(&theme)))
                .on_click(cx.listener(move |console, _, window, cx| {
                    let nav = console.list(id, cx);
                    nav.select(row);
                    let focus = nav.focus.clone();
                    window.focus(&focus);
                    // A click picks; the keyboard's ↵ goes on.
                    on_activate(console, row, true, window, cx);
                }))
                .child(vt_ui::icon(
                    icon,
                    ICON_SM,
                    if item.chosen {
                        theme.accent
                    } else {
                        theme.fg_dim
                    },
                ))
                .child(div().flex_1().min_w_0().truncate().child(item.label))
                .child(
                    div()
                        .flex_none()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(item.detail),
                )
        }))
        .into_any_element()
}

// ── Drawing ──────────────────────────────────────────────────────────────

pub(super) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    ensure_fields(console, window, cx);
    let state = console.state::<State>();
    let mode = state.mode;
    let body_focus = state.body.get_or_insert_with(|| cx.focus_handle()).clone();
    let modes = [(Mode::Quick, "Quick start"), (Mode::Full, "Full wizard")]
        .into_iter()
        .map(|(each, label)| {
            Segment::new(
                label,
                each == mode,
                cx.listener(move |console, _, window, cx| set_mode(console, each, window, cx)),
            )
        })
        .collect();
    let bar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .child(segmented("system-wizard-mode", modes, cx))
        .child(kbd("[  ]", cx));
    let content = match mode {
        Mode::Quick => quick(console, window, cx),
        Mode::Full => full(console, window, cx),
    };
    let scroll = console.scroll(SCROLL);
    div()
        .id(SCROLL)
        .track_focus(&body_focus)
        .size_full()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .on_action(cx.listener(|console, _: &Dismiss, window, cx| {
            if !back(console, window, cx) {
                cx.propagate();
            }
        }))
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
            let own = console
                .state_ref::<State>()
                .and_then(|state| state.body.as_ref())
                .is_some_and(|focus| focus.is_focused(window));
            if own && event.keystroke.key == "enter" {
                next(console, window, cx);
                cx.stop_propagation();
            }
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .max_w(px(900.0))
                .p(SPACE_9)
                .pb(LIST_TAIL)
                .child(bar)
                .child(content),
        )
        .into_any_element()
}

fn error_line(error: Option<String>, theme: &Theme) -> Option<AnyElement> {
    error.map(|error| {
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .text_size(FONT_SMALL)
            .text_color(theme.danger)
            .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.danger))
            .child(error)
            .into_any_element()
    })
}

/// A number with its caption, for the results.
fn figure(label: &'static str, value: u32, color: gpui::Hsla, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(SPACE_1)
        .p(SPACE_6)
        .rounded(RADIUS_LG)
        .border_1()
        .border_color(theme.border)
        .bg(theme.bg)
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_DISPLAY)
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(value.to_string()),
        )
        .child(
            div()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(label),
        )
        .into_any_element()
}

fn lines(
    title: &'static str,
    items: &[String],
    color: gpui::Hsla,
    mark: &'static str,
    theme: &Theme,
) -> Option<AnyElement> {
    (!items.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap(SPACE_2)
            .child(
                div()
                    .text_size(FONT_EMPHASIS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(title),
            )
            .children(items.iter().map(|item| {
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(format!("{mark} {item}"))
            }))
            .into_any_element()
    })
}

#[allow(clippy::too_many_lines)]
fn quick(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let (step, creating, error, result) = (
        state.quick,
        state.creating,
        state.error.clone(),
        state.quick_result.clone(),
    );
    let Some(fields) = &state.fields else {
        return div().into_any_element();
    };
    let (ip, port, unit) = (
        fields.ip.clone(),
        fields.quick_port.clone(),
        fields.quick_unit.clone(),
    );
    let at = match step {
        QuickStep::Connect => 1,
        QuickStep::Results => 2,
        QuickStep::Done => 3,
    };
    let progress = div().flex().gap(SPACE_2).children((1..=3).map(|segment| {
        div()
            .flex_1()
            .h(px(6.0))
            .rounded(px(3.0))
            .bg(if segment <= at {
                theme.accent
            } else {
                theme.border
            })
    }));
    let content = match (step, result) {
        (QuickStep::Connect, _) | (QuickStep::Results, None) => card(&theme)
            .p(SPACE_8)
            .gap(SPACE_6)
            .child(
                div()
                    .text_size(FONT_EMPHASIS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Connect to your PLC"),
            )
            .child(field("IP address", ip, cx))
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(SPACE_6)
                    .child(field("Port", port, cx))
                    .child(field("Unit ID", unit, cx)),
            )
            .children(error_line(error, &theme))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_5)
                    .child(
                        Button::new(
                            "system-quick-connect",
                            if creating {
                                "Connecting…"
                            } else {
                                "Connect & auto-discover"
                            },
                        )
                        .primary()
                        .icon(Icon::Plug)
                        .disabled(creating)
                        .on_click(cx.listener(|console, _, window, cx| next(console, window, cx))),
                    )
                    .child(form_note(
                        None,
                        &[("↵", "connect"), ("Tab", "next field")],
                        cx,
                    )),
            )
            .into_any_element(),
        (QuickStep::Results, Some(result)) => card(&theme)
            .p(SPACE_8)
            .gap(SPACE_6)
            .border_color(if result.success {
                theme.success
            } else {
                theme.warning
            })
            .child(
                div()
                    .text_size(FONT_EMPHASIS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if result.success {
                        "Discovery complete"
                    } else {
                        "Discovery completed with warnings"
                    }),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(SPACE_6)
                    .child(figure(
                        "Tags discovered",
                        result.tags_discovered,
                        theme.success,
                        &theme,
                    ))
                    .child(figure(
                        "Tags created",
                        result.tags_created,
                        theme.accent,
                        &theme,
                    )),
            )
            .when(result.alarms_created > 0, |this| {
                this.child(format!("{} alarms generated", result.alarms_created))
            })
            .when(result.screens_created > 0, |this| {
                this.child(format!(
                    "{} dashboard screen{} created",
                    result.screens_created,
                    if result.screens_created == 1 { "" } else { "s" }
                ))
            })
            .when(result.flows_created > 0, |this| {
                this.child(format!("{} flows configured", result.flows_created))
            })
            .children(lines(
                "Warnings",
                &result.warnings,
                theme.warning,
                "•",
                &theme,
            ))
            .child(
                div().flex().child(
                    Button::new("system-quick-dashboard", "Go on")
                        .primary()
                        .icon(Icon::ChevronRight)
                        .tooltip("↵")
                        .on_click(cx.listener(|console, _, window, cx| next(console, window, cx))),
                ),
            )
            .into_any_element(),
        (QuickStep::Done, _) => card(&theme)
            .p(SPACE_9)
            .gap(SPACE_5)
            .items_center()
            .child(vt_ui::icon(Icon::CircleCheck, px(32.0), theme.success))
            .child(
                div()
                    .text_size(FONT_TITLE)
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.success)
                    .child("You're up and running"),
            )
            .child(div().text_color(theme.fg_muted).child(
                "Your PLC is connected, its tags are discovered and your dashboard is ready.",
            ))
            .child(
                div()
                    .flex()
                    .gap(SPACE_4)
                    .child(
                        Button::new("system-quick-open-dashboard", "Open the dashboard")
                            .icon(Icon::Gauge)
                            .on_click(cx.listener(|console, _, window, cx| {
                                console.state::<State>().quick = QuickStep::Connect;
                                console.show(Page::Dashboard, window, cx);
                            })),
                    )
                    .child(
                        Button::new("system-quick-done", "Done")
                            .primary()
                            .icon(Icon::Check)
                            .tooltip("↵")
                            .on_click(
                                cx.listener(|console, _, window, cx| next(console, window, cx)),
                            ),
                    ),
            )
            .into_any_element(),
    };
    div()
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .max_w(px(520.0))
        .child(
            div()
                .text_size(FONT_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child("Quick start"),
        )
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child("Connect to your Modbus PLC in under a minute: Voltrus discovers its registers, makes tags of them and builds a dashboard."),
        )
        .child(progress)
        .child(content)
        .into_any_element()
}

fn stepper(step: WizardStep, theme: &Theme) -> AnyElement {
    let at = step.index();
    div()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .children(WizardStep::LABELS.iter().enumerate().map(|(index, label)| {
            let (fill, text) = if index < at {
                (theme.success, theme.bg)
            } else if index == at {
                (theme.accent, theme.accent_fg)
            } else {
                (theme.border, theme.fg_muted)
            };
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.0))
                        .rounded(px(12.0))
                        .bg(fill)
                        .text_color(text)
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .font_weight(FontWeight::BOLD)
                        .child(if index < at {
                            "✓".to_owned()
                        } else {
                            (index + 1).to_string()
                        }),
                )
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(if index == at {
                            theme.fg
                        } else {
                            theme.fg_muted
                        })
                        .child(*label),
                )
                .when(index + 1 < WizardStep::LABELS.len(), |this| {
                    this.child(div().w(px(24.0)).h(px(1.0)).bg(theme.border))
                })
        }))
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn full(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let (step, draft, creating, error, result) = (
        state.step,
        state.draft.clone(),
        state.creating,
        state.error.clone(),
        state.result.clone(),
    );
    let template_picks = template_rows(state);
    let (verticals_error, feeds_error) = (
        match &state.verticals {
            Some(Err(error)) => Some(error.clone()),
            _ => None,
        },
        match &state.feeds {
            Some(Err(error)) => Some(error.clone()),
            _ => None,
        },
    );
    let feed = state.feed.clone();
    let Some(fields) = &state.fields else {
        return div().into_any_element();
    };
    let (address, port, unit_id, device_id, instances, start, end) = (
        fields.address.clone(),
        fields.port.clone(),
        fields.unit_id.clone(),
        fields.device_id.clone(),
        fields.instances.clone(),
        fields.start.clone(),
        fields.end.clone(),
    );

    let body: AnyElement = match step {
        WizardStep::Industry => card(&theme)
            .p(SPACE_8)
            .gap(SPACE_5)
            .child(
                div()
                    .text_size(FONT_EMPHASIS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("What industry are you in?"),
            )
            .child(picker(
                console,
                window,
                cx,
                INDUSTRY_LIST,
                INDUSTRIES
                    .iter()
                    .map(|(id, label)| Pick {
                        label: (*label).into(),
                        detail: SharedString::default(),
                        chosen: draft.industry == *id,
                    })
                    .collect(),
                false,
                pick_industry,
            ))
            .into_any_element(),
        WizardStep::Template => {
            let sources = [
                (TemplateSource::Vertical, "Vertical templates"),
                (TemplateSource::Feed, "FEED templates"),
            ]
            .into_iter()
            .map(|(each, label)| {
                Segment::new(
                    label,
                    each == draft.source,
                    cx.listener(move |console, _, _, cx| set_source(console, each, cx)),
                )
            })
            .collect();
            let picks = template_picks
                .iter()
                .map(|(id, label, detail)| Pick {
                    label: label.clone().into(),
                    detail: detail.clone().into(),
                    chosen: draft.template == *id,
                })
                .collect();
            let aside: AnyElement = match draft.source {
                TemplateSource::Vertical => card(&theme)
                    .p(SPACE_7)
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child("Vertical templates come with ISA-5.1 tags, alarms and dashboards; the wizard instantiates the one picked with your device.")
                    .children(verticals_error.map(|error| {
                        div().text_color(theme.danger).child(format!("Templates: {error}"))
                    }))
                    .into_any_element(),
                TemplateSource::Feed => match feed {
                    Some(Ok(feed)) => card(&theme)
                        .p(SPACE_7)
                        .gap(SPACE_3)
                        .child(
                            div()
                                .text_size(FONT_EMPHASIS)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(feed.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(feed.description.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(SPACE_6)
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .child(format!("{} I/O signals", feed.io_list.len()))
                                .child(format!("{} alarm strategies", feed.alarm_strategy.len()))
                                .child(format!("{} equipment units", feed.equipment_units.len())),
                        )
                        .child(div().flex().flex_wrap().gap(SPACE_2).children(
                            feed.equipment_units.iter().map(|unit| {
                                pill(format!("{} · {}", unit.name, unit.quantity), theme.info)
                            }),
                        ))
                        .child(
                            div()
                                .text_size(FONT_SMALL)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.warning)
                                .child("FEED templates are reference designs: the wizard creates the device without them. Use the I/O list to guide the configuration, or pick a vertical template for a one-step setup."),
                        )
                        .into_any_element(),
                    Some(Err(error)) => card(&theme)
                        .p(SPACE_7)
                        .text_color(theme.danger)
                        .child(format!("FEED template: {error}"))
                        .into_any_element(),
                    None => card(&theme)
                        .p(SPACE_7)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(match feeds_error {
                            Some(error) => format!("FEED templates: {error}"),
                            None if draft.template.is_some() => "Loading the FEED template…".to_owned(),
                            None => "Space on a FEED template shows its I/O list here.".to_owned(),
                        })
                        .into_any_element(),
                },
            };
            div()
                .flex()
                .flex_col()
                .gap(SPACE_5)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .child(segmented("system-wizard-source", sources, cx))
                        .child(kbd("[  ]", cx)),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_6)
                        .items_start()
                        .child(div().flex_1().min_w_0().child(picker(
                            console,
                            window,
                            cx,
                            TEMPLATE_LIST,
                            picks,
                            false,
                            pick_template,
                        )))
                        .child(div().w(px(340.0)).flex_none().child(aside)),
                )
                .into_any_element()
        }
        WizardStep::Device => {
            let protocols = PROTOCOLS
                .iter()
                .map(|choice| Pick {
                    label: choice.name.into(),
                    detail: choice.port.to_string().into(),
                    chosen: draft.protocol == choice.id,
                })
                .collect();
            let options = [
                ("Auto-scan registers (Modbus only)", draft.auto_scan),
                ("Generate dashboard", draft.dashboard),
                ("Generate alarms", draft.alarms),
            ]
            .into_iter()
            .map(|(label, on)| Pick {
                label: label.into(),
                detail: if on { "on" } else { "off" }.into(),
                chosen: on,
            })
            .collect();
            let modbus = draft.protocol == "modbus";
            card(&theme)
                .p(SPACE_8)
                .gap(SPACE_6)
                .child(
                    div()
                        .text_size(FONT_EMPHASIS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Device configuration"),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_8)
                        .items_start()
                        .child(
                            div()
                                .w(px(240.0))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap(SPACE_2)
                                .child(section_label("Protocol", cx))
                                .child(picker(
                                    console,
                                    window,
                                    cx,
                                    PROTOCOL_LIST,
                                    protocols,
                                    false,
                                    pick_protocol,
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap(SPACE_5)
                                .child(field("Address", address, cx))
                                .child(
                                    div()
                                        .grid()
                                        .grid_cols(3)
                                        .gap(SPACE_5)
                                        .child(field("Port", port, cx))
                                        .child(if modbus {
                                            field("Unit ID", unit_id, cx).into_any_element()
                                        } else {
                                            div().into_any_element()
                                        })
                                        .child(field("Instances", instances, cx)),
                                )
                                .child(field("Device ID (optional)", device_id, cx))
                                .child(section_label("Options", cx))
                                .child(picker(
                                    console,
                                    window,
                                    cx,
                                    OPTIONS_LIST,
                                    options,
                                    true,
                                    toggle_option,
                                )),
                        ),
                )
                .into_any_element()
        }
        WizardStep::Scan => {
            let areas = REGISTER_AREAS
                .iter()
                .map(|(id, label)| Pick {
                    label: (*label).into(),
                    detail: (*id).into(),
                    chosen: draft.register_area == *id,
                })
                .collect();
            let (from, to) = (
                start.read(cx).value().trim().to_owned(),
                end.read(cx).value().trim().to_owned(),
            );
            let count = match (from.parse::<u32>(), to.parse::<u32>()) {
                (Ok(from), Ok(to)) if to >= from => (to - from + 1).to_string(),
                _ => "?".to_owned(),
            };
            card(&theme)
                .p(SPACE_8)
                .gap(SPACE_6)
                .child(
                    div()
                        .text_size(FONT_EMPHASIS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Auto-scan configuration"),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_8)
                        .items_start()
                        .child(
                            div()
                                .w(px(300.0))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap(SPACE_2)
                                .child(section_label("Register area", cx))
                                .child(picker(
                                    console, window, cx, AREA_LIST, areas, false, pick_area,
                                )),
                        )
                        .child(
                            div()
                                .grid()
                                .grid_cols(2)
                                .gap(SPACE_5)
                                .flex_1()
                                .child(field("Start address", start, cx))
                                .child(field("End address", end, cx)),
                        ),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(format!(
                            "Will scan {count} registers on {}:{} (unit {})",
                            draft.address.trim(),
                            draft.port.trim(),
                            draft.unit_id.trim()
                        )),
                )
                .into_any_element()
        }
        WizardStep::Review => card(&theme)
            .p(SPACE_8)
            .gap(SPACE_2)
            .child(
                div()
                    .pb(SPACE_4)
                    .text_size(FONT_EMPHASIS)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Review configuration"),
            )
            .children(draft.review().into_iter().map(|(label, value)| {
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_6)
                    .py(SPACE_3)
                    .border_b_1()
                    .border_color(theme.border.alpha(0.6))
                    .child(
                        div()
                            .w(px(180.0))
                            .flex_none()
                            .text_color(theme.fg_muted)
                            .child(label),
                    )
                    .child(div().flex_1().min_w_0().font_family(MONO).child(value))
            }))
            .into_any_element(),
        WizardStep::Result => match result {
            Some(result) => card(&theme)
                .p(SPACE_8)
                .gap(SPACE_6)
                .border_color(if result.success {
                    theme.success
                } else {
                    theme.danger
                })
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if result.success {
                            theme.success
                        } else {
                            theme.danger
                        })
                        .child(if result.success {
                            "Project created"
                        } else {
                            "Wizard completed with warnings"
                        }),
                )
                .child(
                    div()
                        .grid()
                        .grid_cols(5)
                        .gap(SPACE_5)
                        .child(figure(
                            "Tags created",
                            result.tags_created,
                            theme.accent,
                            &theme,
                        ))
                        .child(figure(
                            "Alarms created",
                            result.alarms_created,
                            theme.info,
                            &theme,
                        ))
                        .child(figure(
                            "Flows created",
                            result.flows_created,
                            theme.series[2],
                            &theme,
                        ))
                        .child(figure(
                            "Dashboards",
                            result.screens_created,
                            theme.series[3],
                            &theme,
                        ))
                        .child(figure(
                            "Tags discovered",
                            result.tags_discovered,
                            theme.success,
                            &theme,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_3)
                        .child("Device")
                        .child(div().font_family(MONO).child(result.device_id.clone())),
                )
                .children(lines(
                    "Warnings",
                    &result.warnings,
                    theme.warning,
                    "•",
                    &theme,
                ))
                .children(lines(
                    "Next steps",
                    &result.next_steps,
                    theme.info,
                    "→",
                    &theme,
                ))
                .into_any_element(),
            None => div().into_any_element(),
        },
    };

    let nav = div()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .map(|this| match step {
            WizardStep::Result => this
                .child(
                    Button::new("system-wizard-another", "Create another")
                        .icon(Icon::Plus)
                        .tooltip("↵")
                        .on_click(cx.listener(|console, _, window, cx| next(console, window, cx))),
                )
                .child(
                    Button::new("system-wizard-dashboard", "Go to the dashboard")
                        .primary()
                        .icon(Icon::Gauge)
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.show(Page::Dashboard, window, cx);
                        })),
                ),
            _ => this
                .when(step != WizardStep::Industry, |this| {
                    this.child(
                        Button::new("system-wizard-back", "Back")
                            .ghost()
                            .icon(Icon::ChevronLeft)
                            .tooltip("Esc")
                            .on_click(cx.listener(|console, _, window, cx| {
                                back(console, window, cx);
                            })),
                    )
                })
                .when(step == WizardStep::Template, |this| {
                    this.child(
                        Button::new("system-wizard-skip", "Skip — manual setup")
                            .ghost()
                            .on_click(cx.listener(|console, _, window, cx| {
                                console.state::<State>().draft.template = None;
                                next(console, window, cx);
                            })),
                    )
                })
                .child(
                    Button::new(
                        "system-wizard-next",
                        match (step, creating) {
                            (WizardStep::Review, true) => "Creating…",
                            (WizardStep::Review, false) => "Create project",
                            _ => "Next",
                        },
                    )
                    .primary()
                    .icon(if step == WizardStep::Review {
                        Icon::Check
                    } else {
                        Icon::ChevronRight
                    })
                    .tooltip("↵")
                    .disabled(creating)
                    .on_click(cx.listener(|console, _, window, cx| next(console, window, cx))),
                )
                .child(form_note(None, hints(step), cx)),
        });

    div()
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .child(
            div()
                .text_size(FONT_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child("Project creation wizard"),
        )
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child("Set up a project in minutes: pick your industry and equipment, configure the device, and Voltrus generates the rest."),
        )
        .when(step != WizardStep::Result, |this| this.child(stepper(step, &theme)))
        .children(error_line(error, &theme))
        .child(body)
        .child(nav)
        .into_any_element()
}

/// The keys under a step.
fn hints(step: WizardStep) -> &'static [(&'static str, &'static str)] {
    match step {
        WizardStep::Industry | WizardStep::Template => {
            &[("↵", "pick and go on"), ("Space", "pick"), ("Esc", "back")]
        }
        WizardStep::Review => &[("↵", "create"), ("Esc", "back")],
        _ => &[("↵", "next"), ("Tab", "next field"), ("Esc", "back")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_rows_start_with_manual_setup_and_follow_the_kind() {
        let mut state = State {
            verticals: Some(Ok(vec![TemplateSummary {
                id: "car_wash".to_owned(),
                name: "Car Wash System".to_owned(),
                tag_count: 14,
                ..TemplateSummary::default()
            }])),
            ..State::default()
        };
        let rows = template_rows(&state);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, None);
        assert_eq!(rows[1].0.as_deref(), Some("car_wash"));
        state.draft.source = TemplateSource::Feed;
        assert_eq!(template_rows(&state).len(), 1, "no FEED templates yet");
        assert_eq!(
            voltrus_scada_native::api::eng_system::protocol(state.draft.protocol).port,
            502
        );
    }
}
