//! The Flows section's dialogs: the settings form (a new flow: ID, name,
//! enabled, trigger; an existing one: name and enabled), the runs history
//! (a sheet) and the test run (a JSON payload, the steps' results).

use std::collections::{HashMap, HashSet};

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, EntityInputHandler, FontWeight, Hsla, KeyDownEvent, Subscription,
    Window, div, px,
};
use voltrus_scada_native::api::eng_flows::{
    Flow, FlowGraph, FlowRun, TEST_PAYLOAD, TRIGGER_KINDS, TestResult, TriggerKind, label_of,
    parse_json, pretty, validate_meta,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, empty_state, field, kbd, pill, segmented};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::{State, editor, now_millis, now_secs, refresh, with_window};
use crate::console::Console;
use crate::console::overlay::Overlay;
use crate::console::text_area::TextArea;
use crate::console::views::ops_kit::notice;
use crate::console::views::{LIST_TAIL, ListKeys, focus_ring, keyboard_list, local_time};

const FORM: &str = "data-flow-form";
const RUNS: &str = "data-flow-runs";
const RUNS_LIST: &str = "data.flows.runs";
const TEST: &str = "data-flow-test";

// ── Settings form ────────────────────────────────────────────────────────

/// The settings form while it is open.
pub(crate) struct MetaForm {
    /// The flow being edited; `None` for a new one.
    original: Option<Flow>,
    id: Entity<TextInput>,
    name: Entity<TextInput>,
    enabled: bool,
    /// A new flow's trigger.
    trigger: &'static TriggerKind,
    error: Option<String>,
    saving: bool,
    /// The save the form waits for.
    seq: i64,
    _subscriptions: Vec<Subscription>,
}

fn subscribe(
    input: &Entity<TextInput>,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> Subscription {
    cx.subscribe_in(
        input,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted => save_meta(console, cx),
            InputEvent::Cancelled => console.close_overlay(window, cx),
            InputEvent::Changed => {
                if let Some(form) = console.state::<State>().form.as_mut() {
                    form.error = None;
                }
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    )
}

/// Open the form for `original`, or for a new flow (the web client's
/// `flow_<ms>`, "New Flow", enabled, a Modbus trigger).
pub(super) fn open_meta(
    console: &mut Console,
    original: Option<Flow>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let id_text = original
        .as_ref()
        .map_or_else(|| format!("flow_{}", now_millis()), |flow| flow.id.clone());
    let name_text = original
        .as_ref()
        .map_or_else(|| "New Flow".to_owned(), |flow| flow.name.clone());
    let id = cx.new(|cx| {
        TextInput::new(cx)
            .text(id_text)
            .placeholder("flow_…")
            .mono()
    });
    let name = cx.new(|cx| TextInput::new(cx).text(name_text).placeholder("Flow name"));
    let subscriptions = vec![subscribe(&id, window, cx), subscribe(&name, window, cx)];
    let enabled = original.as_ref().is_none_or(|flow| flow.enabled);
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.form = Some(MetaForm {
        original,
        id,
        name: name.clone(),
        enabled,
        trigger: &TRIGGER_KINDS[0],
        error: None,
        saving: false,
        seq: 0,
        _subscriptions: subscriptions,
    });
    let overlay = Overlay::modal(FORM, px(480.0), render_meta, cx);
    console.open_overlay(overlay, window, cx);
    name.read(cx).focus(window);
}

fn save_meta(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    let flows = match &state.flows {
        Some(Ok(flows)) => flows.clone(),
        _ => Vec::new(),
    };
    let Some(form) = state.form.as_mut() else {
        return;
    };
    if form.saving {
        return;
    }
    let id = form.id.read(cx).value().trim().to_owned();
    let name = form.name.read(cx).value().trim().to_owned();
    let taken = form.original.is_none().then_some(flows.as_slice());
    if let Err(error) = validate_meta(&id, &name, taken) {
        form.error = Some(error);
        cx.notify();
        return;
    }
    let now = now_secs();
    let (key, flow, created) = match &form.original {
        None => (
            "data.flows.create",
            Flow::blank(&id, &name, form.enabled, form.trigger, now),
            true,
        ),
        Some(original) => (
            "data.flows.meta",
            original.with_meta(&name, form.enabled, now),
            false,
        ),
    };
    form.saving = true;
    form.seq = seq;
    form.error = None;
    console.call(
        CallKey::new(key).with(seq),
        Call::new(move |api| async move {
            if created {
                let id = api.create_flow(&flow).await?;
                anyhow::Ok(Flow { id, ..flow })
            } else {
                api.update_flow(&flow).await?;
                anyhow::Ok(flow)
            }
        }),
    );
    cx.notify();
}

fn form_saved(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    opens: bool,
    cx: &mut Context<Console>,
) {
    match reply.take::<Flow>() {
        Ok(flow) => {
            let state = console.state::<State>();
            state.focus_id = Some(flow.id.clone());
            let verb = if opens { "Created" } else { "Saved" };
            console.show_toast(format!("{verb} {}", flow.name), false, cx);
            refresh(console);
            with_window(console, cx, move |console, window, cx| {
                if console.overlay_is(FORM) {
                    console.close_overlay(window, cx);
                }
                console.state::<State>().form = None;
                // A new flow opens on the canvas, as the web client's does.
                if opens {
                    editor::open(console, flow, window, cx);
                }
            });
        }
        Err(error) => match console
            .state::<State>()
            .form
            .as_mut()
            .filter(|form| form.seq == key.arg)
        {
            Some(form) => {
                form.saving = false;
                form.error = Some(error);
            }
            None => console.show_toast(format!("Saving the flow failed: {error}"), true, cx),
        },
    }
}

fn render_meta(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(form) = console.state::<State>().form.as_ref() else {
        return div().into_any_element();
    };
    let editing = form.original.as_ref().map(|flow| flow.id.clone());
    let (id, name, enabled, trigger, error, saving) = (
        form.id.clone(),
        form.name.clone(),
        form.enabled,
        form.trigger.id,
        form.error.clone(),
        form.saving,
    );
    let set_enabled = |on: bool, cx: &mut Context<Console>| {
        cx.listener(move |console: &mut Console, _: &gpui::ClickEvent, _, cx| {
            if let Some(form) = console.state::<State>().form.as_mut() {
                form.enabled = on;
            }
            cx.notify();
        })
    };
    let note = |text: &'static str| {
        div()
            .text_size(FONT_SMALL)
            .text_color(theme.fg_dim)
            .child(text)
    };

    let mut body: Vec<AnyElement> = Vec::new();
    body.push(match &editing {
        None => field("ID", id, cx).into_any_element(),
        Some(id) => field(
            "ID",
            div()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(id.clone()),
            cx,
        )
        .into_any_element(),
    });
    body.push(field("Name", name, cx).into_any_element());
    body.push(
        field(
            "Enabled",
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .child(segmented(
                    "data-flow-form-enabled",
                    vec![
                        Segment::new("Off", !enabled, set_enabled(false, cx)),
                        Segment::new("On", enabled, set_enabled(true, cx)),
                    ],
                    cx,
                ))
                .child(note(
                    "Stored: the engine loads enabled flows at start-up. Start and Stop in the list act on the running engine now.",
                )),
            cx,
        )
        .into_any_element(),
    );
    if editing.is_none() {
        let triggers = TRIGGER_KINDS
            .iter()
            .map(|kind| {
                Segment::new(
                    kind.label,
                    kind.id == trigger,
                    cx.listener(move |console, _, _, cx| {
                        if let Some(form) = console.state::<State>().form.as_mut() {
                            form.trigger = kind;
                        }
                        cx.notify();
                    }),
                )
            })
            .collect();
        body.push(
            field(
                "Trigger",
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2)
                    .child(segmented("data-flow-form-trigger", triggers, cx))
                    .child(note(
                        "Its settings and the steps are edited on the canvas next.",
                    )),
                cx,
            )
            .into_any_element(),
        );
    }

    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(56.0))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border)
        .child(vt_ui::icon(
            if editing.is_some() {
                Icon::Pencil
            } else {
                Icon::Plus
            },
            ICON_LG,
            theme.accent,
        ))
        .child(
            div()
                .flex_1()
                .text_size(FONT_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child(if editing.is_some() {
                    "Flow settings"
                } else {
                    "New flow"
                }),
        );
    let footer = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(56.0))
        .px(SPACE_8)
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .map(|this| match &error {
                    Some(error) => this.child(error_line(error.clone(), &theme)),
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
            Button::new("data-flow-form-cancel", "Cancel")
                .ghost()
                .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx))),
        )
        .child(
            Button::new(
                "data-flow-form-save",
                match (saving, editing.is_some()) {
                    (true, _) => "Saving…",
                    (false, true) => "Save",
                    (false, false) => "Create",
                },
            )
            .primary()
            .icon(Icon::Check)
            .disabled(saving)
            .on_click(cx.listener(|console, _, _, cx| save_meta(console, cx))),
        );

    div()
        .flex()
        .flex_col()
        .child(header)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .children(body),
        )
        .child(footer)
        .into_any_element()
}

/// A failure in a dialog's footer.
pub(super) fn error_line(error: String, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_3)
        .min_w_0()
        .text_size(FONT_SMALL)
        .text_color(theme.danger)
        .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.danger))
        .child(div().min_w_0().truncate().child(error))
        .into_any_element()
}

// ── Runs ─────────────────────────────────────────────────────────────────

/// The runs sheet while it is open.
pub(crate) struct Runs {
    id: String,
    name: String,
    runs: Option<Result<Vec<FlowRun>, String>>,
    seq: i64,
    /// Runs shown in full, by run id.
    open: HashSet<i64>,
}

/// Open the runs of `flow` (`GET /api/v1/flows/{id}/runs`).
pub(super) fn open_runs(
    console: &mut Console,
    flow: &Flow,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    console.state::<State>().runs = Some(Runs {
        id: flow.id.clone(),
        name: flow.name.clone(),
        runs: None,
        seq: 0,
        open: HashSet::new(),
    });
    fetch_runs(console);
    let nav = console.list(RUNS_LIST, cx);
    nav.cursor = None;
    let overlay = Overlay::sheet(RUNS, px(560.0), render_runs, cx);
    console.open_overlay(overlay, window, cx);
    console.focus_list(RUNS_LIST, window, cx);
}

fn fetch_runs(console: &mut Console) {
    let state = console.state::<State>();
    state.seq += 1;
    let seq = state.seq;
    let Some(runs) = state.runs.as_mut() else {
        return;
    };
    runs.seq = seq;
    let id = runs.id.clone();
    console.call(
        CallKey::new("data.flows.runs").with(seq),
        Call::new(move |api| async move { api.flow_runs(&id).await }),
    );
}

fn toggle_run_detail(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(runs) = console.state::<State>().runs.as_mut()
        && let Some(Ok(list)) = &runs.runs
        && let Some(run) = list.get(row)
    {
        let id = run.id;
        if !runs.open.remove(&id) {
            runs.open.insert(id);
        }
    }
    cx.notify();
}

/// A run's stored output, pretty when it is JSON.
fn output_text(output: &str) -> String {
    parse_json(output).map_or_else(|_| output.to_owned(), |value| pretty(&value))
}

fn render_runs(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let offset = console.utc_offset;
    let Some(runs) = console.state::<State>().runs.as_ref() else {
        return div().into_any_element();
    };
    let (name, list, open) = (runs.name.clone(), runs.runs.clone(), runs.open.clone());
    let len = match &list {
        Some(Ok(list)) => list.len(),
        _ => 0,
    };
    let keys = keyboard_list(
        ListKeys {
            id: RUNS_LIST,
            on_move: None,
            on_activate: toggle_run_detail,
        },
        len,
        10,
        console,
        cx,
    );
    let nav = console.list(RUNS_LIST, cx);
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));

    let body = match list {
        None => notice("Loading runs…", false, &theme),
        Some(Err(error)) => notice(format!("Runs could not be loaded: {error}"), true, &theme),
        Some(Ok(list)) if list.is_empty() => empty_state(
            Icon::History,
            "No runs yet",
            "The engine logs a run each time the trigger fires and the flow runs.",
            Vec::new(),
            cx,
        )
        .into_any_element(),
        Some(Ok(list)) => div()
            .flex()
            .flex_col()
            .pb(LIST_TAIL)
            .children(list.into_iter().enumerate().map(|(row, run)| {
                let expanded = open.contains(&run.id);
                let summary = match (&run.error, &run.output) {
                    (Some(error), _) => error.clone(),
                    (None, Some(output)) => output.clone(),
                    (None, None) => "Filtered out".to_owned(),
                };
                let color = if run.success {
                    theme.success
                } else {
                    theme.danger
                };
                div()
                    .id(("data-flow-run-row", row))
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(SPACE_2)
                    .px(SPACE_8)
                    .py(SPACE_4)
                    .border_b_1()
                    .border_color(theme.border.alpha(0.6))
                    .cursor_pointer()
                    .when(cursor == Some(row), |this| this.bg(theme.selected))
                    .on_click(cx.listener(move |console, _, window, cx| {
                        console.list(RUNS_LIST, cx).select(row);
                        toggle_run_detail(console, row, false, window, cx);
                    }))
                    .when(focused && cursor == Some(row), |this| {
                        this.child(focus_ring(&theme))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_4)
                            .child(vt_ui::icon(
                                if run.success {
                                    Icon::CircleCheck
                                } else {
                                    Icon::CircleX
                                },
                                ICON_MD,
                                color,
                            ))
                            .child(
                                div()
                                    .w(px(96.0))
                                    .flex_none()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .child(local_time(run.started_at, offset)),
                            )
                            .child(
                                div()
                                    .w(px(64.0))
                                    .flex_none()
                                    .text_right()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child(format!("{} ms", run.duration_ms)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(if run.success {
                                        theme.fg_muted
                                    } else {
                                        theme.danger
                                    })
                                    .child(summary),
                            ),
                    )
                    .when(expanded, |this| {
                        let detail = match (&run.error, &run.output) {
                            (Some(error), _) => error.clone(),
                            (None, Some(output)) => output_text(output),
                            (None, None) => {
                                "A filter stopped this run: nothing reached the outputs.".to_owned()
                            }
                        };
                        this.child(
                            div()
                                .p(SPACE_4)
                                .rounded(RADIUS_MD)
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.border)
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .child(detail),
                        )
                    })
            }))
            .into_any_element(),
    };

    div()
        .on_key_down(cx.listener(|console, event: &KeyDownEvent, _, cx| {
            let modifiers = event.keystroke.modifiers;
            if event.keystroke.key == "r" && !modifiers.modified() {
                fetch_runs(console);
                cx.stop_propagation();
                cx.notify();
            }
        }))
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_5)
                .h(px(56.0))
                .px(SPACE_8)
                .border_b_1()
                .border_color(theme.border)
                .child(vt_ui::icon(Icon::History, ICON_LG, theme.accent))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_TITLE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("Runs · {name}")),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child("The latest 20, newest first · ↵ details · R refresh · Esc close"),
                        ),
                )
                .child(
                    IconButton::new("data-flow-runs-refresh", Icon::RefreshCw)
                        .tooltip("Refresh  R")
                        .on_click(cx.listener(|console, _, _, cx| {
                            fetch_runs(console);
                            cx.notify();
                        })),
                )
                .child(
                    IconButton::new("data-flow-runs-close", Icon::X)
                        .tooltip("Close  Esc")
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.close_overlay(window, cx);
                        })),
                ),
        )
        .child(
            keys.flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(body),
        )
        .into_any_element()
}

// ── Test ─────────────────────────────────────────────────────────────────

/// The test dialog while it is open.
pub(crate) struct Test {
    id: String,
    name: String,
    payload: Entity<TextArea>,
    result: Option<Result<TestResult, String>>,
    running: bool,
    seq: i64,
    /// Step labels by id, to name the results.
    labels: HashMap<String, String>,
    /// The editor has changes the server does not have yet.
    unsaved: bool,
    _subscription: Subscription,
}

/// Open the test of `flow` with the last payload typed (or the web
/// client's default).
pub(super) fn open_test(
    console: &mut Console,
    flow: &Flow,
    unsaved: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let text = console
        .state::<State>()
        .test_payload
        .clone()
        .unwrap_or_else(|| TEST_PAYLOAD.to_owned());
    let payload = cx.new(|cx| TextArea::new(6, cx).placeholder("{\"metrics\": {…}}   ⌘↵ run"));
    payload.update(cx, |area, cx| {
        area.replace_text_in_range(None, &text, window, cx);
    });
    let subscription = cx.subscribe_in(
        &payload,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted => run_test(console, cx),
            InputEvent::Cancelled => console.close_overlay(window, cx),
            InputEvent::Changed | InputEvent::Blurred => {}
        },
    );
    let labels = FlowGraph::of(flow)
        .map(|graph| {
            graph
                .nodes
                .iter()
                .map(|node| {
                    let id = node.get("id").and_then(|id| id.as_str()).unwrap_or("");
                    (id.to_owned(), label_of(node))
                })
                .collect()
        })
        .unwrap_or_default();
    let state = console.state::<State>();
    state.window = Some(window.window_handle());
    state.test = Some(Test {
        id: flow.id.clone(),
        name: flow.name.clone(),
        payload: payload.clone(),
        result: None,
        running: false,
        seq: 0,
        labels,
        unsaved,
        _subscription: subscription,
    });
    let overlay = Overlay::modal(TEST, px(640.0), render_test, cx);
    console.open_overlay(overlay, window, cx);
    payload.read(cx).focus(window);
}

/// `POST /api/v1/flows/{id}/test` with the payload typed.
fn run_test(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let Some(test) = state.test.as_ref() else {
        return;
    };
    if test.running {
        return;
    }
    let text = test.payload.read(cx).value().to_owned();
    state.test_payload = Some(text.clone());
    state.seq += 1;
    let seq = state.seq;
    let Some(test) = state.test.as_mut() else {
        return;
    };
    let payload = match parse_json(&text) {
        Ok(payload) => payload,
        Err(error) => {
            test.result = Some(Err(format!("The payload is {error}")));
            cx.notify();
            return;
        }
    };
    test.running = true;
    test.seq = seq;
    test.result = None;
    let id = test.id.clone();
    console.call(
        CallKey::new("data.flows.test").with(seq),
        Call::new(move |api| async move { api.test_flow(&id, payload).await }),
    );
    cx.notify();
}

fn outcome_color(outcome: &str, theme: &Theme) -> Hsla {
    match outcome {
        "continue" => theme.success,
        "filter_reject" => theme.warning,
        "error" => theme.danger,
        _ => theme.fg_dim,
    }
}

fn render_test(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(test) = console.state::<State>().test.as_ref() else {
        return div().into_any_element();
    };
    let (name, payload, result, running, labels, unsaved) = (
        test.name.clone(),
        test.payload.clone(),
        test.result.clone(),
        test.running,
        test.labels.clone(),
        test.unsaved,
    );

    let result_view = match result {
        None if running => Some(notice("Running the stored steps…", false, &theme)),
        None => None,
        Some(Err(error)) => Some(
            div()
                .p(SPACE_5)
                .rounded(RADIUS_MD)
                .bg(theme.danger.opacity(0.08))
                .text_size(FONT_SMALL)
                .text_color(theme.danger)
                .child(error)
                .into_any_element(),
        ),
        Some(Ok(result)) => {
            let steps = result.node_results.len();
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_4)
                    .child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(match steps {
                                0 => "No steps ran: the flow has none.".to_owned(),
                                1 => "1 step ran, in the stored order:".to_owned(),
                                steps => format!("{steps} steps ran, in the stored order:"),
                            }),
                    )
                    .children(result.node_results.iter().enumerate().map(|(index, step)| {
                        let outcome = step.outcome().to_owned();
                        let label = labels
                            .get(&step.node_id)
                            .cloned()
                            .unwrap_or_else(|| "Step".to_owned());
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_4)
                            .child(
                                div()
                                    .w(px(20.0))
                                    .flex_none()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_dim)
                                    .child(format!("{}", index + 1)),
                            )
                            .child(div().w(px(120.0)).flex_none().truncate().child(label))
                            .child(
                                div()
                                    .w(px(140.0))
                                    .flex_none()
                                    .truncate()
                                    .font_family(MONO)
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child(step.node_id.clone()),
                            )
                            .child(pill(outcome.clone(), outcome_color(&outcome, &theme)))
                            .when_some(step.error.clone(), |this, error| {
                                this.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.danger)
                                        .child(error),
                                )
                            })
                    }))
                    .child(
                        div()
                            .text_size(FONT_CAPTION)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.fg_dim)
                            .child("OUTPUT"),
                    )
                    .child(
                        div()
                            .id("data-flow-test-output")
                            .max_h(px(200.0))
                            .overflow_y_scroll()
                            .p(SPACE_4)
                            .rounded(RADIUS_MD)
                            .bg(theme.surface)
                            .border_1()
                            .border_color(theme.border)
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .child(pretty(&result.output)),
                    )
                    .into_any_element(),
            )
        }
    };

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_5)
                .h(px(56.0))
                .px(SPACE_8)
                .border_b_1()
                .border_color(theme.border)
                .child(vt_ui::icon(Icon::Send, ICON_LG, theme.accent))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Test {name}")),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .child(field("Payload JSON", payload, cx))
                .when(unsaved, |this| {
                    this.child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.warning)
                            .child("The editor has unsaved changes: the server tests the stored steps. Save with ⌘S first."),
                    )
                })
                .children(result_view),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .h(px(56.0))
                .px(SPACE_8)
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(kbd("⌘↵", cx))
                        .child("run")
                        .child(kbd("Esc", cx))
                        .child("close"),
                )
                .child(
                    Button::new("data-flow-test-close", "Close")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new(
                        "data-flow-test-run",
                        if running { "Running…" } else { "Run test" },
                    )
                    .primary()
                    .icon(Icon::Play)
                    .disabled(running)
                    .on_click(cx.listener(|console, _, _, cx| run_test(console, cx))),
                ),
        )
        .into_any_element()
}

/// Answers to the dialogs' calls.
pub(super) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    match key.name {
        "data.flows.create" => form_saved(console, key, reply, true, cx),
        "data.flows.meta" => form_saved(console, key, reply, false, cx),
        "data.flows.runs" => {
            if let Some(runs) = console.state::<State>().runs.as_mut()
                && runs.seq == key.arg
            {
                runs.runs = Some(reply.take::<Vec<FlowRun>>());
            }
        }
        "data.flows.test" => {
            if let Some(test) = console.state::<State>().test.as_mut()
                && test.seq == key.arg
            {
                test.running = false;
                test.result = Some(reply.take::<TestResult>());
            }
        }
        _ => {}
    }
}
