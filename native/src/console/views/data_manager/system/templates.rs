//! System › Templates, after the web client's `TemplatesPanel.tsx`: the
//! vertical templates (`GET /api/v1/templates[?industry=]`) by industry,
//! a template's tag patterns, alarm rules and flows (`GET /templates/{id}`,
//! operator), the expanded preview (`POST /templates/preview`, anyone) and
//! an instance of it (`POST /templates/{id}/instantiate`, operator) after a
//! confirmation, since it creates a device, tags, alarm limits, flow
//! rules and a screen.
//!
//! Keys: ↵ on a template opens it, ↵ in the detail previews, ↵ in the
//! name field instantiates (after the confirmation), ↵ in the instances
//! field previews again, Esc steps back, [ and ] change the industry.

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, SharedString,
    Subscription, Window, div, px,
};
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::eng_system::{
    Industry, InstantiateBody, Instantiated, PreviewBody, TEMPLATE_PREVIEW_PATH, TemplateList,
    TemplatePreview, TemplateSummary, VerticalTemplate, instance_device_id, instantiate_path,
    matches_any, template_path, templates_path,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, field, kbd, pill, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::{
    Look, cell, click_row, form_note, heading_row, needs, query, row_shell, rows, severity_color,
};
use crate::console::Console;
use crate::console::keys::Dismiss;
use crate::console::overlay::Overlay;
use crate::console::pages::Role;
use crate::console::views::ops_kit::{notice, on_field};
use crate::console::views::{ListKeys, card};

const BROWSE_LIST: &str = "data.system.templates";
const DETAIL_LIST: &str = "data.system.templates.detail";
const PREVIEW_LIST: &str = "data.system.templates.preview";
const CONFIRM: &str = "system-template-instantiate";
const CARD_ROW: f32 = 64.0;
const ROW: f32 = 30.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum View {
    #[default]
    Browse,
    Detail,
    Instantiate,
}

struct Fields {
    name: Entity<TextInput>,
    instances: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
struct State {
    view: View,
    /// The industry the list shows; `None` is every industry.
    industry: Option<String>,
    list: Option<Result<TemplateList, String>>,
    /// Every industry, from the last answer (the server lists them all
    /// whatever the filter).
    industries: Vec<Industry>,
    list_asked: bool,
    list_seq: i64,
    selected: Option<TemplateSummary>,
    detail: Option<Result<VerticalTemplate, String>>,
    detail_seq: i64,
    preview: Option<Result<TemplatePreview, String>>,
    preview_seq: i64,
    previewing: bool,
    fields: Option<Fields>,
    note: Option<Result<String, String>>,
    /// The instance waiting for the confirmation.
    pending: Option<InstantiateBody>,
    instantiating: bool,
    confirm_focus: Option<FocusHandle>,
}

pub(super) fn list(console: &Console) -> Option<&'static str> {
    Some(
        match console
            .state_ref::<State>()
            .map_or(View::Browse, |state| state.view)
        {
            View::Browse => BROWSE_LIST,
            View::Detail => DETAIL_LIST,
            View::Instantiate => PREVIEW_LIST,
        },
    )
}

pub(super) fn subtitle(console: &Console) -> String {
    let view = console
        .state_ref::<State>()
        .map_or(View::Browse, |state| state.view);
    match view {
        View::Browse => {
            "Industry templates · select → preview → instantiate · ↵ open · [ ] industry · / search"
                .to_owned()
        }
        View::Detail => "Tag patterns, alarm rules and flows · ↵ preview · Esc back".to_owned(),
        View::Instantiate => {
            "The expanded template · ↵ in the name instantiates · Esc back".to_owned()
        }
    }
}

pub(super) fn open(console: &mut Console) {
    if !console.state::<State>().list_asked {
        fetch_list(console);
    }
}

/// `r`: the list, and the open template's detail and preview.
pub(super) fn refresh(console: &mut Console, cx: &gpui::App) {
    fetch_list(console);
    let state = console.state::<State>();
    let (view, selected) = (state.view, state.selected.clone());
    if let Some(selected) = selected {
        match view {
            View::Detail => fetch_detail(console, &selected.id),
            View::Instantiate => send_preview(console, cx),
            View::Browse => {}
        }
    }
}

fn fetch_list(console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    state.list_asked = true;
    state.list_seq += 1;
    let (seq, path) = (state.list_seq, templates_path(state.industry.as_deref()));
    console.call(
        CallKey::new("data.system.templates.list").with(seq),
        Call::get::<TemplateList>(path),
    );
}

fn fetch_detail(console: &mut Console, id: &str) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    state.detail_seq += 1;
    state.detail = None;
    let seq = state.detail_seq;
    console.call(
        CallKey::new("data.system.templates.detail").with(seq),
        Call::get::<VerticalTemplate>(template_path(id)),
    );
}

fn instances_text(console: &Console, cx: &gpui::App) -> String {
    console
        .state_ref::<State>()
        .and_then(|state| state.fields.as_ref())
        .map(|fields| fields.instances.read(cx).value().to_owned())
        .unwrap_or_default()
}

fn name_text(console: &Console, cx: &gpui::App) -> String {
    console
        .state_ref::<State>()
        .and_then(|state| state.fields.as_ref())
        .map(|fields| fields.name.read(cx).value().to_owned())
        .unwrap_or_default()
}

/// Ask for the preview with the instance count typed (2 until typed).
fn send_preview(console: &mut Console, cx: &gpui::App) {
    if console.stress.is_some() {
        return;
    }
    let text = instances_text(console, cx);
    let state = console.state::<State>();
    let Some(selected) = state.selected.clone() else {
        return;
    };
    match PreviewBody::new(&selected.id, &text) {
        Ok(body) => {
            state.preview_seq += 1;
            state.previewing = true;
            state.note = None;
            let seq = state.preview_seq;
            console.call(
                CallKey::new("data.system.templates.preview").with(seq),
                Call::send::<TemplatePreview>(
                    Method::POST,
                    TEMPLATE_PREVIEW_PATH,
                    serde_json::to_value(body).unwrap_or_default(),
                ),
            );
        }
        Err(error) => state.note = Some(Err(error)),
    }
}

pub(super) fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    let view = console.state::<State>().view;
    match view {
        View::Browse => console.focus_list(BROWSE_LIST, window, cx),
        View::Detail => console.focus_list(DETAIL_LIST, window, cx),
        View::Instantiate => {
            let name = console
                .state_ref::<State>()
                .and_then(|state| state.fields.as_ref())
                .map(|fields| fields.name.clone());
            match name {
                Some(name) => name.read(cx).focus(window),
                None => console.focus_list(PREVIEW_LIST, window, cx),
            }
        }
    }
    true
}

fn set_view(console: &mut Console, view: View, window: &mut Window, cx: &mut Context<Console>) {
    console.state::<State>().view = view;
    focus(console, window, cx);
    cx.notify();
}

/// `[` and `]`: the next or previous industry (every industry first).
pub(super) fn cycle(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let mut options: Vec<Option<String>> = vec![None];
    options.extend(
        state
            .industries
            .iter()
            .map(|industry| Some(industry.id.clone())),
    );
    let at = options
        .iter()
        .position(|option| *option == state.industry)
        .unwrap_or(0);
    let len = options.len();
    let next = options[if forward {
        (at + 1) % len
    } else {
        (at + len - 1) % len
    }]
    .clone();
    set_industry(console, next, cx);
}

fn set_industry(console: &mut Console, industry: Option<String>, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    if state.industry == industry {
        return;
    }
    state.industry = industry;
    state.view = View::Browse;
    console.list(BROWSE_LIST, cx).cursor = None;
    fetch_list(console);
    cx.notify();
}

/// The templates the search leaves.
fn shown(list: &TemplateList, search: &str) -> Vec<usize> {
    list.templates
        .iter()
        .enumerate()
        .filter(|(_, template)| {
            matches_any(
                search,
                &[
                    template.name.as_str(),
                    template.description.as_str(),
                    template.industry.as_str(),
                    template.id.as_str(),
                ],
            )
        })
        .map(|(index, _)| index)
        .collect()
}

/// ↵ on a template: its detail (operator), or straight to the preview.
fn open_template(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let search = query(console, cx);
    let state = console.state::<State>();
    let Some(Ok(list)) = &state.list else {
        return;
    };
    let Some(template) = shown(list, &search)
        .get(row)
        .map(|index| list.templates[*index].clone())
    else {
        return;
    };
    let changed = state.selected.as_ref().map(|selected| &selected.id) != Some(&template.id);
    state.selected = Some(template.clone());
    if changed {
        state.preview = None;
        state.note = None;
        state.detail = None;
    }
    if console.is_operator() {
        if changed || !matches!(console.state::<State>().detail, Some(Ok(_))) {
            fetch_detail(console, &template.id);
        }
        console.list(DETAIL_LIST, cx).cursor = None;
        set_view(console, View::Detail, window, cx);
    } else {
        preview(console, window, cx);
    }
}

/// "Preview & instantiate": the expanded template and the form.
fn preview(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    send_preview(console, cx);
    console.list(PREVIEW_LIST, cx).cursor = None;
    set_view(console, View::Instantiate, window, cx);
}

fn preview_row(
    console: &mut Console,
    _: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    preview(console, window, cx);
}

/// Esc: back a view; on the list, out to the sidebar.
fn back(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    let view = console.state::<State>().view;
    let to = match view {
        View::Browse => return false,
        View::Detail => View::Browse,
        View::Instantiate => {
            if console.is_operator() {
                View::Detail
            } else {
                View::Browse
            }
        }
    };
    set_view(console, to, window, cx);
    true
}

/// The name and instances: the confirmation, then the server.
fn ask_instantiate(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.state::<State>().note = Some(Err(format!(
            "Instantiating: {}",
            needs(Role::Operator).to_lowercase()
        )));
        cx.notify();
        return;
    }
    let (name, instances) = (name_text(console, cx), instances_text(console, cx));
    let state = console.state::<State>();
    if state.instantiating {
        return;
    }
    let Some(selected) = state.selected.clone() else {
        return;
    };
    match InstantiateBody::new(&selected.id, &name, &instances) {
        Ok(body) => {
            state.pending = Some(body);
            state.note = None;
            let focus = cx.focus_handle();
            state.confirm_focus = Some(focus.clone());
            let overlay = Overlay::modal(CONFIRM, px(460.0), confirm_dialog, cx);
            console.open_overlay(overlay, window, cx);
            window.focus(&focus);
        }
        Err(error) => state.note = Some(Err(error)),
    }
    cx.notify();
}

fn instantiate_confirmed(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(body) = console.state::<State>().pending.take() else {
        return;
    };
    console.close_overlay(window, cx);
    let state = console.state::<State>();
    state.instantiating = true;
    let path = instantiate_path(&body.template_id);
    console.call(
        CallKey::new("data.system.templates.instantiate"),
        Call::send::<Instantiated>(
            Method::POST,
            path,
            serde_json::to_value(body).unwrap_or_default(),
        ),
    );
    cx.notify();
}

pub(super) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    let state = console.state::<State>();
    match key.name {
        "data.system.templates.list" => {
            if key.arg != state.list_seq {
                return;
            }
            let list = reply.take::<TemplateList>();
            if let Ok(list) = &list {
                state.industries = list.industries.clone();
            }
            state.list = Some(list);
        }
        "data.system.templates.detail" => {
            if key.arg == state.detail_seq {
                state.detail = Some(reply.take::<VerticalTemplate>());
            }
        }
        "data.system.templates.preview" => {
            if key.arg == state.preview_seq {
                state.previewing = false;
                state.preview = Some(reply.take::<TemplatePreview>());
            }
        }
        "data.system.templates.instantiate" => {
            state.instantiating = false;
            match reply.take::<Instantiated>() {
                Ok(done) => {
                    let summary = done.summary();
                    state.note = Some(Ok(summary.clone()));
                    console.refresh_tags();
                    console.show_toast(summary, false, cx);
                }
                Err(error) => {
                    state.note = Some(Err(error.clone()));
                    console.show_toast(format!("Instantiating failed: {error}"), true, cx);
                }
            }
        }
        _ => {}
    }
}

fn on_name(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Submitted => ask_instantiate(console, window, cx),
        InputEvent::Cancelled => {
            back(console, window, cx);
        }
        InputEvent::Changed => cx.notify(),
        InputEvent::Blurred => {}
    }
}

fn on_instances(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Submitted => {
            send_preview(console, cx);
            cx.notify();
        }
        InputEvent::Cancelled => {
            back(console, window, cx);
        }
        InputEvent::Changed | InputEvent::Blurred => {}
    }
}

fn ensure_fields(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().fields.is_some() {
        return;
    }
    let name = cx.new(|cx| TextInput::new(cx).placeholder("e.g. My Plant Pump Station"));
    let instances = cx.new(|cx| {
        TextInput::new(cx)
            .placeholder("2")
            .text("2")
            .kind(InputKind::Unsigned)
            .mono()
    });
    let subscriptions = vec![
        on_field(&name, on_name, window, cx),
        on_field(&instances, on_instances, window, cx),
    ];
    console.state::<State>().fields = Some(Fields {
        name,
        instances,
        _subscriptions: subscriptions,
    });
}

fn element_id(name: &'static str, row: usize) -> ElementId {
    ElementId::NamedInteger(name.into(), u64::try_from(row).unwrap_or(0))
}

pub(super) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    ensure_fields(console, window, cx);
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let (view, industry, industries, selected) = (
        state.view,
        state.industry.clone(),
        state.industries.clone(),
        state.selected.clone(),
    );

    let views = [
        (View::Browse, "Browse"),
        (View::Detail, "Detail"),
        (View::Instantiate, "Instantiate"),
    ]
    .into_iter()
    .filter(|(each, _)| {
        *each == View::Browse
            || (selected.is_some() && (*each != View::Detail || console.is_operator()))
    })
    .map(|(each, label)| {
        Segment::new(
            label,
            each == view,
            cx.listener(move |console, _, window, cx| {
                if each == View::Instantiate && console.state::<State>().preview.is_none() {
                    preview(console, window, cx);
                } else {
                    set_view(console, each, window, cx);
                }
            }),
        )
    })
    .collect();
    let mut choices = vec![Segment::new(
        "All industries",
        industry.is_none(),
        cx.listener(|console, _, _, cx| set_industry(console, None, cx)),
    )];
    choices.extend(industries.into_iter().map(|each| {
        let id = each.id.clone();
        Segment::new(
            each.name,
            industry.as_deref() == Some(id.as_str()),
            cx.listener(move |console, _, _, cx| set_industry(console, Some(id.clone()), cx)),
        )
    }));
    let bar = div()
        .flex()
        .flex_none()
        .flex_wrap()
        .items_center()
        .gap(SPACE_4)
        .px(SPACE_9)
        .py(SPACE_4)
        .child(segmented("system-templates-view", views, cx))
        .when(view == View::Browse, |this| {
            this.child(segmented("system-templates-industry", choices, cx))
                .child(kbd("[  ]", cx))
        })
        .when_some(
            selected.filter(|_| view != View::Browse),
            |this, selected| {
                this.child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(selected.id),
                )
            },
        );

    let body = match view {
        View::Browse => browse(console, window, cx),
        View::Detail => detail(console, window, cx),
        View::Instantiate => instantiate(console, window, cx),
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .on_action(cx.listener(|console, _: &Dismiss, window, cx| {
            if !back(console, window, cx) {
                cx.propagate();
            }
        }))
        .child(bar)
        .child(div().flex().flex_col().flex_1().min_h_0().child(body))
        .into_any_element()
}

fn browse(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let list = match console.state::<State>().list.clone() {
        None => return notice("Loading the templates…", false, &theme),
        Some(Err(error)) => return notice(format!("Templates: {error}"), true, &theme),
        Some(Ok(list)) => list,
    };
    let search = query(console, cx);
    let shown = shown(&list, &search);
    let selected = console
        .state::<State>()
        .selected
        .as_ref()
        .map(|selected| selected.id.clone());
    let empty = notice(
        if list.templates.is_empty() {
            "No templates for this industry"
        } else {
            "No template matches the search"
        },
        false,
        &theme,
    );
    rows(
        console,
        window,
        cx,
        ListKeys {
            id: BROWSE_LIST,
            on_move: None,
            on_activate: open_template,
        },
        shown.len(),
        CARD_ROW,
        empty,
        move |_, row, look: Look, theme, cx| {
            let template = &list.templates[shown[row]];
            let chosen = selected.as_deref() == Some(template.id.as_str());
            row_shell(
                element_id("system-template", row),
                row,
                look,
                CARD_ROW,
                theme,
            )
            .on_click(click_row(BROWSE_LIST, row, Some(open_template), cx))
            .child(vt_ui::icon(
                Icon::Layers,
                ICON_LG,
                if chosen { theme.accent } else { theme.fg_dim },
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(SPACE_1)
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(template.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(template.description.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(SPACE_2)
                    .child(pill(
                        list.industry_name(&template.industry).to_owned(),
                        theme.accent,
                    ))
                    .child(pill(format!("{} tags", template.tag_count), theme.success))
                    .child(pill(format!("{} alarms", template.alarm_count), theme.info))
                    .child(pill(
                        format!("{} flows", template.flow_count),
                        theme.series[2],
                    ))
                    .when(template.has_oee, |this| {
                        this.child(pill("OEE", theme.warning))
                    }),
            )
            .into_any_element()
        },
    )
}

/// A row of the detail or preview list.
#[derive(Clone)]
enum Line {
    Heading(String),
    Tag(usize),
    Alarm(usize),
    Flow(usize),
}

fn detail(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let template = match state.detail.clone() {
        None => return notice("Loading the template…", false, &theme),
        Some(Err(error)) => return notice(format!("Template: {error}"), true, &theme),
        Some(Ok(template)) => template,
    };
    let industry_name = match &state.list {
        Some(Ok(list)) => list.industry_name(&template.industry).to_owned(),
        _ => template.industry.clone(),
    };
    let search = query(console, cx);
    let mut lines = Vec::new();
    let tags: Vec<usize> = (0..template.tags.len())
        .filter(|index| {
            let tag = &template.tags[*index];
            matches_any(
                &search,
                &[tag.name_pattern.as_str(), tag.description.as_str()],
            )
        })
        .collect();
    let alarms: Vec<usize> = (0..template.alarms.len())
        .filter(|index| {
            let alarm = &template.alarms[*index];
            matches_any(
                &search,
                &[alarm.tag_pattern.as_str(), alarm.message_template.as_str()],
            )
        })
        .collect();
    let flows: Vec<usize> = (0..template.flows.len())
        .filter(|index| {
            let flow = &template.flows[*index];
            matches_any(&search, &[flow.name.as_str(), flow.description.as_str()])
        })
        .collect();
    lines.push(Line::Heading(format!(
        "Tag patterns (ISA-5.1) · {} · pattern · description · unit · type · group",
        tags.len()
    )));
    lines.extend(tags.into_iter().map(Line::Tag));
    lines.push(Line::Heading(format!(
        "Alarm rules · {} · tag · severity · condition · threshold · message",
        alarms.len()
    )));
    lines.extend(alarms.into_iter().map(Line::Alarm));
    if !template.flows.is_empty() {
        lines.push(Line::Heading(format!(
            "Flow rules · {} · name · trigger · description",
            flows.len()
        )));
        lines.extend(flows.into_iter().map(Line::Flow));
    }

    let facts = [
        ("Industry", industry_name),
        ("Equipment", template.equipment_type.clone()),
        ("Tags", format!("{} patterns", template.tags.len())),
        ("Alarms", format!("{} rules", template.alarms.len())),
        ("Flows", format!("{} rules", template.flows.len())),
        (
            "OEE",
            if template.oee_config.is_some() {
                "Yes"
            } else {
                "No"
            }
            .to_owned(),
        ),
        ("Screen", template.dashboard.screen_name.clone()),
    ];
    let info = card(&theme)
        .p(SPACE_7)
        .gap(SPACE_4)
        .child(
            div()
                .text_size(FONT_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child(template.name.clone()),
        )
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(template.description.clone()),
        )
        .child(
            div()
                .grid()
                .grid_cols(7)
                .gap(SPACE_5)
                .children(facts.into_iter().map(|(label, value)| {
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(FONT_CAPTION)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.fg_dim)
                                .child(label.to_uppercase()),
                        )
                        .child(div().truncate().font_family(MONO).child(value))
                })),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    Button::new("system-template-preview", "Preview & instantiate")
                        .primary()
                        .icon(Icon::Eye)
                        .tooltip("↵")
                        .on_click(cx.listener(|console, _, window, cx| {
                            preview(console, window, cx);
                        })),
                )
                .child(
                    Button::new("system-template-back", "Back")
                        .ghost()
                        .icon(Icon::ChevronLeft)
                        .tooltip("Esc")
                        .on_click(cx.listener(|console, _, window, cx| {
                            back(console, window, cx);
                        })),
                ),
        );

    let len = lines.len();
    let list = rows(
        console,
        window,
        cx,
        ListKeys {
            id: DETAIL_LIST,
            on_move: None,
            on_activate: preview_row,
        },
        len,
        ROW,
        notice("Nothing matches the search", false, &theme),
        move |_, row, look: Look, theme, cx| match &lines[row] {
            Line::Heading(text) => heading_row(text.clone(), ROW, theme),
            Line::Tag(index) => {
                let tag = &template.tags[*index];
                row_shell(
                    element_id("system-template-tag", row),
                    row,
                    look,
                    ROW,
                    theme,
                )
                .on_click(click_row(DETAIL_LIST, row, None, cx))
                .child(cell(tag.name_pattern.clone(), Some(160.0), true, theme.fg))
                .child(cell(tag.description.clone(), None, false, theme.fg_muted))
                .child(cell(dash(&tag.unit), Some(60.0), false, theme.fg_muted))
                .child(cell(
                    tag.data_type.clone(),
                    Some(80.0),
                    true,
                    theme.fg_muted,
                ))
                .child(cell(tag.group.clone(), Some(100.0), false, theme.fg_muted))
                .into_any_element()
            }
            Line::Alarm(index) => {
                let alarm = &template.alarms[*index];
                row_shell(
                    element_id("system-template-alarm", row),
                    row,
                    look,
                    ROW,
                    theme,
                )
                .on_click(click_row(DETAIL_LIST, row, None, cx))
                .child(cell(alarm.tag_pattern.clone(), Some(160.0), true, theme.fg))
                .child(div().w(px(80.0)).flex_none().flex().child(pill(
                    alarm.severity.clone(),
                    alarm_color(&alarm.severity, theme),
                )))
                .child(cell(
                    alarm.comparison.clone(),
                    Some(90.0),
                    false,
                    theme.fg_muted,
                ))
                .child(cell(number(alarm.threshold), Some(80.0), true, theme.fg))
                .child(cell(
                    alarm.message_template.clone(),
                    None,
                    false,
                    theme.fg_muted,
                ))
                .into_any_element()
            }
            Line::Flow(index) => {
                let flow = &template.flows[*index];
                row_shell(
                    element_id("system-template-flow", row),
                    row,
                    look,
                    ROW,
                    theme,
                )
                .on_click(click_row(DETAIL_LIST, row, None, cx))
                .child(cell(flow.name.clone(), Some(240.0), false, theme.fg))
                .child(cell(
                    flow.trigger_type.clone(),
                    Some(120.0),
                    true,
                    theme.fg_muted,
                ))
                .child(cell(flow.description.clone(), None, false, theme.fg_muted))
                .into_any_element()
            }
        },
    );
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(div().flex_none().px(SPACE_9).pb(SPACE_6).child(info))
        .child(list)
        .into_any_element()
}

fn dash(text: &str) -> String {
    if text.is_empty() {
        "—".to_owned()
    } else {
        text.to_owned()
    }
}

fn number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

fn alarm_color(severity: &str, theme: &Theme) -> gpui::Hsla {
    use voltrus_scada_native::api::eng_system::HealthSeverity;
    severity_color(
        match severity {
            "critical" => HealthSeverity::Critical,
            "warning" => HealthSeverity::Warning,
            _ => HealthSeverity::Info,
        },
        theme,
    )
}

#[allow(clippy::too_many_lines)]
fn instantiate(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let state = console.state::<State>();
    let Some(fields) = &state.fields else {
        return div().into_any_element();
    };
    let (name, instances) = (fields.name.clone(), fields.instances.clone());
    let (preview, previewing, note, instantiating, selected) = (
        state.preview.clone(),
        state.previewing,
        state.note.clone(),
        state.instantiating,
        state.selected.clone(),
    );
    let named = !name.read(cx).value().trim().is_empty();
    let title = selected.map_or_else(String::new, |selected| selected.name);

    let form =
        card(&theme)
            .p(SPACE_7)
            .gap(SPACE_5)
            .child(
                div()
                    .text_size(FONT_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Instantiate {title}")),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(4)
                    .gap(SPACE_6)
                    .child(div().col_span(3).child(field("Name", name, cx)))
                    .child(field("Instances (1–20)", instances, cx)),
            )
            .when(!operator, |this| {
                this.child(div().text_size(FONT_SMALL).text_color(theme.warning).child(
                    "Anyone may preview a template; instantiating it needs the operator role.",
                ))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .child(
                        Button::new(
                            "system-template-instantiate",
                            if instantiating {
                                "Creating…"
                            } else {
                                "Instantiate template"
                            },
                        )
                        .primary()
                        .icon(Icon::Plus)
                        .disabled(!operator || instantiating || !named)
                        .tooltip(if operator {
                            "↵ in the name field"
                        } else {
                            needs(Role::Operator)
                        })
                        .on_click(cx.listener(|console, _, window, cx| {
                            ask_instantiate(console, window, cx);
                        })),
                    )
                    .child(
                        Button::new("system-template-repreview", "Preview again")
                            .icon(Icon::Eye)
                            .tooltip("↵ in the instances field")
                            .disabled(previewing)
                            .on_click(cx.listener(|console, _, _, cx| {
                                send_preview(console, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("system-template-form-back", "Back")
                            .ghost()
                            .icon(Icon::ChevronLeft)
                            .tooltip("Esc")
                            .on_click(cx.listener(|console, _, window, cx| {
                                back(console, window, cx);
                            })),
                    )
                    .child(div().flex_1().min_w_0().child(form_note(
                        note,
                        &[("↵", "instantiate"), ("Tab", "instances"), ("Esc", "back")],
                        cx,
                    ))),
            );

    let body = match preview {
        None => notice(
            if previewing {
                "Expanding the template…"
            } else {
                "No preview yet"
            },
            false,
            &theme,
        ),
        Some(Err(error)) => notice(format!("Preview: {error}"), true, &theme),
        Some(Ok(preview)) => {
            let search = query(console, cx);
            let mut lines = Vec::new();
            let tags: Vec<usize> = (0..preview.expanded_tags.len())
                .filter(|index| {
                    let tag = &preview.expanded_tags[*index];
                    matches_any(&search, &[tag.name.as_str(), tag.description.as_str()])
                })
                .collect();
            let alarms: Vec<usize> = (0..preview.expanded_alarms.len())
                .filter(|index| {
                    let alarm = &preview.expanded_alarms[*index];
                    matches_any(
                        &search,
                        &[alarm.tag_pattern.as_str(), alarm.message_template.as_str()],
                    )
                })
                .collect();
            let flows: Vec<usize> = (0..preview.flows.len())
                .filter(|index| {
                    let flow = &preview.flows[*index];
                    matches_any(&search, &[flow.name.as_str(), flow.description.as_str()])
                })
                .collect();
            lines.push(Line::Heading(format!(
                "Preview: {} tags · name · description · unit · type · widget",
                preview.expanded_tags.len()
            )));
            lines.extend(tags.into_iter().map(Line::Tag));
            lines.push(Line::Heading(format!(
                "{} alarm limits · tag · severity · condition · threshold · message",
                preview.expanded_alarms.len()
            )));
            lines.extend(alarms.into_iter().map(Line::Alarm));
            lines.push(Line::Heading(format!(
                "{} flow rules, created disabled · name · trigger · description",
                preview.flows.len()
            )));
            lines.extend(flows.into_iter().map(Line::Flow));
            let len = lines.len();
            rows(
                console,
                window,
                cx,
                ListKeys {
                    id: PREVIEW_LIST,
                    on_move: None,
                    on_activate: super::nothing,
                },
                len,
                ROW,
                notice("Nothing matches the search", false, &theme),
                move |_, row, look: Look, theme, cx| match &lines[row] {
                    Line::Heading(text) => heading_row(text.clone(), ROW, theme),
                    Line::Tag(index) => {
                        let tag = &preview.expanded_tags[*index];
                        row_shell(element_id("system-preview-tag", row), row, look, ROW, theme)
                            .on_click(click_row(PREVIEW_LIST, row, None, cx))
                            .child(cell(tag.name.clone(), Some(160.0), true, theme.fg))
                            .child(cell(tag.description.clone(), None, false, theme.fg_muted))
                            .child(cell(dash(&tag.unit), Some(60.0), false, theme.fg_muted))
                            .child(cell(
                                tag.data_type.clone(),
                                Some(80.0),
                                true,
                                theme.fg_muted,
                            ))
                            .child(cell(tag.widget.clone(), Some(80.0), false, theme.fg_muted))
                            .into_any_element()
                    }
                    Line::Alarm(index) => {
                        let alarm = &preview.expanded_alarms[*index];
                        row_shell(
                            element_id("system-preview-alarm", row),
                            row,
                            look,
                            ROW,
                            theme,
                        )
                        .on_click(click_row(PREVIEW_LIST, row, None, cx))
                        .child(cell(alarm.tag_pattern.clone(), Some(160.0), true, theme.fg))
                        .child(div().w(px(80.0)).flex_none().flex().child(pill(
                            alarm.severity.clone(),
                            alarm_color(&alarm.severity, theme),
                        )))
                        .child(cell(
                            alarm.comparison.clone(),
                            Some(90.0),
                            false,
                            theme.fg_muted,
                        ))
                        .child(cell(number(alarm.threshold), Some(80.0), true, theme.fg))
                        .child(cell(
                            alarm.message_template.clone(),
                            None,
                            false,
                            theme.fg_muted,
                        ))
                        .into_any_element()
                    }
                    Line::Flow(index) => {
                        let flow = &preview.flows[*index];
                        row_shell(
                            element_id("system-preview-flow", row),
                            row,
                            look,
                            ROW,
                            theme,
                        )
                        .on_click(click_row(PREVIEW_LIST, row, None, cx))
                        .child(cell(flow.name.clone(), Some(240.0), false, theme.fg))
                        .child(cell(
                            flow.trigger_type.clone(),
                            Some(120.0),
                            true,
                            theme.fg_muted,
                        ))
                        .child(cell(flow.description.clone(), None, false, theme.fg_muted))
                        .into_any_element()
                    }
                },
            )
        }
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(div().flex_none().px(SPACE_9).pb(SPACE_6).child(form))
        .child(div().flex().flex_col().flex_1().min_h_0().child(body))
        .into_any_element()
}

/// The instantiate confirmation: ↵ or the button creates, Esc keeps.
fn confirm_dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let Some(body) = state.pending.clone() else {
        return div().into_any_element();
    };
    let template = state.selected.as_ref().map_or_else(
        || body.template_id.clone(),
        |selected| selected.name.clone(),
    );
    let counts = match &state.preview {
        Some(Ok(preview)) => format!(
            "{} tags, {} alarm limits and {} flow rules (disabled)",
            preview.expanded_tags.len(),
            preview.expanded_alarms.len(),
            preview.flows.len()
        ),
        _ => "the template's tags, alarm limits and flow rules (disabled)".to_owned(),
    };
    let focus = state.confirm_focus.clone();
    let device = instance_device_id(&body.name);
    div()
        .when_some(focus, |this, focus| this.track_focus(&focus))
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
            let own = console
                .state_ref::<State>()
                .and_then(|state| state.confirm_focus.as_ref())
                .is_some_and(|focus| focus.is_focused(window));
            if own && event.keystroke.key == "enter" {
                instantiate_confirmed(console, window, cx);
                cx.stop_propagation();
            }
        }))
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(vt_ui::icon(Icon::Layers, ICON_LG, theme.accent))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Instantiate {template}?")),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .text_color(theme.fg_muted)
                .child(format!(
                    "Creates the device {device} named \"{}\" with {counts}, and its dashboard \
                     screen. Templates with OEE add a day shift when none exists.",
                    body.name
                ))
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(SharedString::from(format!(
                            "instances: {}",
                            body.instance_counts.get("n").copied().unwrap_or(2)
                        ))),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(kbd("↵", cx))
                        .child("create")
                        .child(kbd("Esc", cx))
                        .child("keep"),
                )
                .child(
                    Button::new("system-template-confirm-cancel", "Keep")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.state::<State>().pending = None;
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new("system-template-confirm", "Instantiate")
                        .primary()
                        .icon(Icon::Plus)
                        .on_click(cx.listener(|console, _, window, cx| {
                            instantiate_confirmed(console, window, cx);
                        })),
                ),
        )
        .into_any_element()
}
