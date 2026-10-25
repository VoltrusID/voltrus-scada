//! The add and edit form of Data Manager › Sources, a modal after the web
//! client's `EditPanel.tsx` and `SourceConfigFields.tsx`:
//! - General: name, key, unit, decimals, range, limits, base value, card
//!   kind, colour, image or text content, the low-alarm limits and
//!   deadband, the ISA-18.2 alarm delays and (editing) the per-tag shortcut.
//! - Source: the source type and its settings: the simulations'
//!   parameters, the API poll, or a Modbus device with one register or an
//!   expression over its registers.
//!
//! Enter in any field saves, Esc closes, Tab walks the fields; the form
//! stays open with the server's answer when saving fails.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Keystroke, SharedString, Subscription, Window,
    div, px, uniform_list,
};
use serde::de::IgnoredAny;
use voltrus_scada_native::api::{
    DataSource, Method, ModbusDevice, ModbusDeviceDetail, ModbusRegister, TagShortcuts,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, field, kbd, pill, section_label, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::source_model::{
    self as model, API_METHODS, CHART_TYPES, COLOR_PRESETS, Category, Draft, FieldKind, ModbusMode,
    SOURCE_TYPES,
};
use super::sources::{self, State, category_color, tag_color};
use crate::console::Console;
use crate::console::overlay::Overlay;
use crate::console::shortcuts;
use crate::console::views::{ListKeys, OnActivate, focus_ring, keyboard_list};

/// The overlay's id.
pub(crate) const FORM: &str = "data-source-form";
const DEVICES: &str = "data.form.devices";
const REGISTERS: &str = "data.form.registers";
const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 540.0;
const PICK_ROW: f32 = 28.0;

/// What a save tells the operator.
const SETTINGS_DROPPED: &str = "Saved, but the server kept the old source settings and low limits: its PUT /api/v1/data-sources/{id} ignores source_config and device_id";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Source,
}

/// The form while it is open.
pub(crate) struct Form {
    /// The source being edited; `None` for a new one.
    original: Option<DataSource>,
    /// The choices (types, card, method, device, register); text fields
    /// live in `inputs` until saving.
    draft: Draft,
    tab: Tab,
    inputs: HashMap<&'static str, Entity<TextInput>>,
    shortcut: Option<String>,
    shortcut_before: Option<String>,
    recording: bool,
    /// The operator an expression's next register is appended with.
    op: char,
    error: Option<String>,
    saving: bool,
    /// This form among the ones opened, the save's key argument: a late
    /// answer for an earlier form leaves this one alone.
    seq: i64,
    _subscriptions: Vec<Subscription>,
}

/// The general tab's text fields: key, placeholder, what they accept,
/// monospace.
const GENERAL: [(&str, &str, InputKind, bool); 18] = [
    ("name", "Line 1 temperature", InputKind::Text, false),
    ("key", "line1_temp", InputKind::Text, true),
    ("unit", "°C", InputKind::Text, false),
    ("decimals", "1", InputKind::Unsigned, true),
    ("min", "0", InputKind::Number, true),
    ("max", "100", InputKind::Number, true),
    ("warn", "80", InputKind::Number, true),
    ("crit", "90", InputKind::Number, true),
    ("base", "50", InputKind::Number, true),
    ("color", "#8b5cf6", InputKind::Text, true),
    (
        "image_url",
        "https://example.com/image.png",
        InputKind::Text,
        false,
    ),
    (
        "text_content",
        "# Title — **bold**, *italic*",
        InputKind::Text,
        true,
    ),
    ("low_warn", "—", InputKind::Number, true),
    ("low_crit", "—", InputKind::Number, true),
    ("deadband", "0", InputKind::Number, true),
    ("on_delay", "0", InputKind::Unsigned, true),
    ("off_delay", "0", InputKind::Unsigned, true),
    (
        "expression",
        "{Reg_A} + {Reg_B} * 0.5",
        InputKind::Text,
        true,
    ),
];

fn text_of<'a>(draft: &'a Draft, key: &str) -> &'a str {
    match key {
        "name" => &draft.name,
        "key" => &draft.key,
        "unit" => &draft.unit,
        "decimals" => &draft.decimals,
        "min" => &draft.min,
        "max" => &draft.max,
        "warn" => &draft.warn,
        "crit" => &draft.crit,
        "base" => &draft.base,
        "color" => &draft.color,
        "image_url" => &draft.image_url,
        "text_content" => &draft.text_content,
        "low_warn" => &draft.low_warn,
        "low_crit" => &draft.low_crit,
        "deadband" => &draft.deadband,
        "on_delay" => &draft.on_delay,
        "off_delay" => &draft.off_delay,
        "expression" => &draft.expression,
        other => draft.settings.get(other).map_or("", String::as_str),
    }
}

fn set_text(draft: &mut Draft, key: &'static str, value: String) {
    let slot = match key {
        "name" => &mut draft.name,
        "key" => &mut draft.key,
        "unit" => &mut draft.unit,
        "decimals" => &mut draft.decimals,
        "min" => &mut draft.min,
        "max" => &mut draft.max,
        "warn" => &mut draft.warn,
        "crit" => &mut draft.crit,
        "base" => &mut draft.base,
        "color" => &mut draft.color,
        "image_url" => &mut draft.image_url,
        "text_content" => &mut draft.text_content,
        "low_warn" => &mut draft.low_warn,
        "low_crit" => &mut draft.low_crit,
        "deadband" => &mut draft.deadband,
        "on_delay" => &mut draft.on_delay,
        "off_delay" => &mut draft.off_delay,
        "expression" => &mut draft.expression,
        setting => {
            if value.trim().is_empty() {
                draft.settings.remove(setting);
            } else {
                draft.settings.insert(setting, value);
            }
            return;
        }
    };
    *slot = value;
}

fn form_mut(console: &mut Console) -> Option<&mut Form> {
    console.state::<State>().form.as_mut()
}

/// Copy the text fields into the draft.
fn collect(form: &mut Form, cx: &App) {
    for (&key, input) in &form.inputs {
        set_text(&mut form.draft, key, input.read(cx).value().to_owned());
    }
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
            InputEvent::Submitted => save(console, window, cx),
            InputEvent::Cancelled => console.close_overlay(window, cx),
            // The colour swatch and the expression preview follow typing.
            InputEvent::Changed => cx.notify(),
            InputEvent::Blurred => {}
        },
    )
}

/// Open the form for `original`, or for a new source.
pub(crate) fn open(
    console: &mut Console,
    original: Option<DataSource>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let draft = original
        .as_ref()
        .map_or_else(Draft::new, Draft::from_source);
    let mut inputs = HashMap::new();
    let mut subscriptions = Vec::new();
    let fields = GENERAL
        .iter()
        .copied()
        .chain(model::all_config_fields().map(|setting| {
            let kind = match setting.kind {
                FieldKind::Number => InputKind::Number,
                FieldKind::Integer => InputKind::Unsigned,
                FieldKind::Text | FieldKind::Json => InputKind::Text,
            };
            (
                setting.key,
                setting.placeholder,
                kind,
                setting.kind != FieldKind::Text,
            )
        }));
    for (key, placeholder, kind, mono) in fields {
        let text = text_of(&draft, key).to_owned();
        let input = cx.new(|cx| {
            let input = TextInput::new(cx)
                .text(text)
                .placeholder(placeholder)
                .kind(kind);
            if mono { input.mono() } else { input }
        });
        subscriptions.push(subscribe(&input, window, cx));
        inputs.insert(key, input);
    }
    // Recording a shortcut takes the next keystroke before any binding.
    let weak = cx.entity().downgrade();
    subscriptions.push(cx.intercept_keystrokes(move |event, _, cx| {
        let taken = weak
            .update(cx, |console, cx| record(console, &event.keystroke, cx))
            .unwrap_or(false);
        if taken {
            cx.stop_propagation();
        }
    }));
    let state = console.state::<State>();
    state.form_seq += 1;
    let seq = state.form_seq;
    let shortcut = original.as_ref().and_then(|source| {
        state
            .shortcuts
            .as_ref()
            .and_then(|shortcuts| shortcuts.get(&source.id).cloned())
    });
    let modbus_device = (draft.source_type == "modbus")
        .then(|| draft.modbus_device.clone())
        .flatten();
    let modbus = draft.source_type == "modbus";
    state.window = Some(window.window_handle());
    state.form = Some(Form {
        original,
        draft,
        tab: Tab::General,
        inputs,
        shortcut_before: shortcut.clone(),
        shortcut,
        recording: false,
        op: '+',
        error: None,
        saving: false,
        seq,
        _subscriptions: subscriptions,
    });
    if modbus {
        ensure_devices(console);
    }
    if let Some(device) = modbus_device {
        ensure_registers(console, device);
    }
    let overlay = Overlay::modal(FORM, px(WIDTH), render, cx);
    console.open_overlay(overlay, window, cx);
    focus_input(console, "name", window, cx);
}

fn focus_input(console: &mut Console, key: &str, window: &mut Window, cx: &mut Context<Console>) {
    if let Some(input) = form_mut(console).and_then(|form| form.inputs.get(key).cloned()) {
        input.read(cx).focus(window);
    }
}

fn set_input(console: &mut Console, key: &str, text: String, cx: &mut Context<Console>) {
    if let Some(input) = form_mut(console).and_then(|form| form.inputs.get(key).cloned()) {
        input.update(cx, |field, cx| field.set_text(text, cx));
    }
}

fn input_text(console: &mut Console, key: &str, cx: &App) -> String {
    form_mut(console)
        .and_then(|form| form.inputs.get(key))
        .map(|input| input.read(cx).value().to_owned())
        .unwrap_or_default()
}

/// The shortcut recorder's keystroke: taken (and kept from the bindings)
/// while recording.
fn record(console: &mut Console, keystroke: &Keystroke, cx: &mut Context<Console>) -> bool {
    let open = console.overlay_is(FORM);
    let Some(form) = form_mut(console) else {
        return false;
    };
    if !form.recording {
        return false;
    }
    if !open {
        form.recording = false;
        return false;
    }
    if keystroke.key == "escape" {
        form.recording = false;
        cx.notify();
        return true;
    }
    match shortcuts::from_keystroke(keystroke) {
        Some(combo) => {
            form.shortcut = Some(combo);
            form.recording = false;
            cx.notify();
            true
        }
        None => false,
    }
}

/// A path segment with everything but unreserved characters escaped.
fn path_segment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn ensure_devices(console: &mut Console) {
    if console.stress.is_some() {
        console.state::<State>().devices = Some(Err("A stress test has no server to ask".into()));
        return;
    }
    let state = console.state::<State>();
    if state.devices_asked {
        return;
    }
    state.devices_asked = true;
    console.call(
        CallKey::new("data.sources.devices"),
        Call::get::<Vec<ModbusDevice>>("/api/v1/modbus-devices"),
    );
}

fn ensure_registers(console: &mut Console, device: String) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    if state.registers.contains_key(&device)
        || state.registers_asked.as_deref() == Some(device.as_str())
    {
        return;
    }
    state.registers_asked = Some(device.clone());
    let path = format!("/api/v1/modbus-devices/{}", path_segment(&device));
    console.call(
        CallKey::new("data.sources.registers"),
        Call::new(move |api| async move {
            let detail: ModbusDeviceDetail = api.get_json(&path).await?;
            anyhow::Ok((device, detail.registers))
        }),
    );
}

/// What the server answered a save with.
struct Saved {
    id: i64,
    key: String,
    warning: Option<String>,
    shortcut_changed: bool,
}

fn save(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(form) = form_mut(console) else {
        return;
    };
    if form.saving {
        return;
    }
    collect(form, cx);
    let call = match build_save(form) {
        Ok(call) => call,
        Err(error) => {
            form.error = Some(error);
            cx.notify();
            return;
        }
    };
    form.saving = true;
    form.error = None;
    let seq = form.seq;
    console.call(CallKey::new("data.sources.save").with(seq), call);
    cx.notify();
}

fn build_save(form: &Form) -> Result<Call, String> {
    form.draft.validate()?;
    let key = form.draft.key.trim().to_owned();
    let Some(original) = &form.original else {
        let body = form.draft.create_body()?;
        let delays = form.draft.delays_body()?;
        return Ok(Call::new(move |api| async move {
            // The row exists once created: a failed delays PUT is a
            // warning, never a failure that would invite creating it again.
            let (id, failed) = sources::create(&api, &body, delays.as_ref()).await?;
            let warning = failed.map(|error| {
                format!("Created {key}, but its alarm delays were not saved: {error}")
            });
            anyhow::Ok(Saved {
                id,
                key,
                warning,
                shortcut_changed: false,
            })
        }));
    };
    let id = original.id;
    let body = form.draft.update_body(original)?;
    let shortcut = (form.shortcut != form.shortcut_before).then(|| form.shortcut.clone());
    Ok(Call::new(move |api| async move {
        api.send_json::<_, IgnoredAny>(Method::PUT, &format!("/api/v1/data-sources/{id}"), &body)
            .await?;
        // The server answers ok without storing the settings; check the row.
        let mut warning = None;
        if body.get("source_config").is_some() || body.get("device_id").is_some() {
            let stored: Vec<DataSource> = api.get_json("/api/v1/data-sources").await?;
            if let Some(row) = stored.iter().find(|row| row.id == id)
                && !model::settings_kept(&body, row)
            {
                warning = Some(SETTINGS_DROPPED.to_owned());
            }
        }
        // The server replaces the whole map: start from what it has now.
        if let Some(combo) = &shortcut {
            let current: TagShortcuts = api.get_json("/api/v1/shortcuts").await?;
            let map = model::shortcuts_body(&current.by_id(), id, combo.as_deref());
            api.send_json::<_, IgnoredAny>(Method::PUT, "/api/v1/shortcuts", &map)
                .await?;
        }
        anyhow::Ok(Saved {
            id,
            key,
            warning,
            shortcut_changed: shortcut.is_some(),
        })
    }))
}

/// Close the form once the server has saved it: replies come without a
/// window, so the close runs right after this update.
fn close_later(console: &mut Console, cx: &mut Context<Console>) {
    let Some(handle) = console.state::<State>().window else {
        return;
    };
    let entity = cx.entity();
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            entity.update(cx, |console, cx| {
                if console.overlay_is(FORM) {
                    console.close_overlay(window, cx);
                }
                console.state::<State>().form = None;
            });
        });
    });
}

/// Answers to the form's calls: the save, the Modbus devices and their
/// registers.
pub(crate) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    match key.name {
        "data.sources.save" => {
            let current = form_mut(console).is_some_and(|form| form.seq == key.arg);
            match reply.take::<Saved>() {
                Ok(saved) => {
                    console.state::<State>().focus_id = Some(saved.id);
                    console.refresh_tags();
                    if saved.shortcut_changed {
                        sources::shortcuts_saved(console);
                    }
                    match saved.warning {
                        Some(warning) => console.show_toast(warning, true, cx),
                        None => console.show_toast(format!("Saved {}", saved.key), false, cx),
                    }
                    // A form opened since then keeps its edits.
                    if current {
                        close_later(console, cx);
                    }
                }
                Err(error) => match form_mut(console).filter(|_| current) {
                    Some(form) => {
                        form.saving = false;
                        form.error = Some(error);
                    }
                    None => console.show_toast(format!("Saving failed: {error}"), true, cx),
                },
            }
        }
        "data.sources.devices" => {
            console.state::<State>().devices = Some(reply.take::<Vec<ModbusDevice>>());
        }
        "data.sources.registers" => {
            let state = console.state::<State>();
            match reply.take::<(String, Vec<ModbusRegister>)>() {
                Ok((device, registers)) => {
                    state.registers.insert(device, Ok(registers));
                }
                Err(error) => {
                    if let Some(device) = state.registers_asked.clone() {
                        state.registers.insert(device, Err(error));
                    }
                }
            }
            state.registers_asked = None;
        }
        _ => {}
    }
    cx.notify();
}

// ── Choices ──────────────────────────────────────────────────────────────

fn set_tab(console: &mut Console, tab: Tab, cx: &mut Context<Console>) {
    if let Some(form) = form_mut(console) {
        form.tab = tab;
    }
    cx.notify();
}

fn set_source_type(console: &mut Console, id: &'static str, cx: &mut Context<Console>) {
    if let Some(form) = form_mut(console) {
        form.draft.set_source_type(id);
    }
    if id == "modbus" {
        ensure_devices(console);
    }
    cx.notify();
}

fn pick_device(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let state = console.state::<State>();
    let Some(Ok(devices)) = &state.devices else {
        return;
    };
    let Some(device) = devices.get(row).map(|device| device.id.clone()) else {
        return;
    };
    if let Some(form) = state.form.as_mut()
        && form.draft.modbus_device.as_deref() != Some(device.as_str())
    {
        form.draft.modbus_device = Some(device.clone());
        form.draft.register = None;
    }
    ensure_registers(console, device);
    cx.notify();
}

fn registers_of(console: &mut Console) -> Option<Result<Vec<ModbusRegister>, String>> {
    let state = console.state::<State>();
    let device = state.form.as_ref()?.draft.modbus_device.clone()?;
    state.registers.get(&device).cloned()
}

fn pick_register(
    console: &mut Console,
    row: usize,
    _: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(Ok(registers)) = registers_of(console) else {
        return;
    };
    let Some(name) = registers.get(row).map(|register| register.name.clone()) else {
        return;
    };
    if let Some(form) = form_mut(console) {
        form.draft.pick_register(&name);
    }
    // Like the web form, the key follows the register.
    set_input(console, "key", name, cx);
    cx.notify();
}

fn append_register(console: &mut Console, name: &str, cx: &mut Context<Console>) {
    let op = form_mut(console).map_or('+', |form| form.op);
    let expression = input_text(console, "expression", cx);
    let next = model::append_term(&expression, op, &model::register_operand(name));
    set_input(console, "expression", next, cx);
    cx.notify();
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// One row of a picker list.
struct PickRow {
    label: SharedString,
    detail: SharedString,
    chosen: bool,
}

/// A short keyboard list inside the form: j/k or ↑↓ move, Enter or a click
/// picks.
fn picker(
    console: &mut Console,
    id: &'static str,
    rows: Vec<PickRow>,
    on_activate: OnActivate,
    cx: &mut Context<Console>,
    window: &mut Window,
) -> AnyElement {
    let theme = theme(cx).clone();
    let len = rows.len();
    let rows = Rc::new(rows);
    let list = keyboard_list(
        ListKeys {
            id,
            on_move: None,
            on_activate,
        },
        len,
        5,
        console,
        cx,
    );
    let nav = console.list(id, cx);
    if nav.cursor.is_none() {
        nav.cursor = rows.iter().position(|row| row.chosen);
    }
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    #[allow(clippy::cast_precision_loss)]
    let height = len.clamp(1, 5) as f32 * PICK_ROW;
    list.h(px(height + 2.0))
        .rounded(RADIUS_MD)
        .border_1()
        .border_color(if focused {
            theme.accent
        } else {
            theme.border_strong
        })
        .bg(theme.bg)
        .overflow_hidden()
        .child(
            uniform_list(
                id,
                len,
                cx.processor(
                    move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                        let theme = vt_ui::theme::theme(cx).clone();
                        range
                            .filter_map(|row| {
                                let item = rows.get(row)?;
                                let hover = theme.hover;
                                Some(
                                    div()
                                        .id((id, row))
                                        .relative()
                                        .flex()
                                        .items_center()
                                        .gap(SPACE_4)
                                        .h(px(PICK_ROW))
                                        .px(SPACE_5)
                                        .cursor_pointer()
                                        .when(item.chosen, |this| this.bg(theme.selected))
                                        .when(!item.chosen, |this| {
                                            this.hover(move |style| style.bg(hover))
                                        })
                                        .on_click(cx.listener(move |console, _, window, cx| {
                                            let nav = console.list(id, cx);
                                            nav.select(row);
                                            let focus = nav.focus.clone();
                                            window.focus(&focus);
                                            on_activate(console, row, false, window, cx);
                                        }))
                                        .when(focused && cursor == Some(row), |this| {
                                            this.child(focus_ring(&theme))
                                        })
                                        .child(vt_ui::icon(
                                            if item.chosen {
                                                Icon::CircleCheck
                                            } else {
                                                Icon::Circle
                                            },
                                            ICON_SM,
                                            if item.chosen {
                                                theme.accent
                                            } else {
                                                theme.fg_dim
                                            },
                                        ))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .child(item.label.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_none()
                                                .font_family(MONO)
                                                .text_size(FONT_SMALL)
                                                .text_color(theme.fg_dim)
                                                .child(item.detail.clone()),
                                        ),
                                )
                            })
                            .collect::<Vec<_>>()
                    },
                ),
            )
            .track_scroll(scroll)
            .size_full(),
        )
        .into_any_element()
}

fn note(text: impl Into<SharedString>, color: gpui::Hsla) -> AnyElement {
    div()
        .text_size(FONT_SMALL)
        .text_color(color)
        .child(text.into())
        .into_any_element()
}

fn grid(columns: u16, children: Vec<AnyElement>) -> AnyElement {
    div()
        .grid()
        .grid_cols(columns)
        .gap(SPACE_6)
        .children(children)
        .into_any_element()
}

/// The form's content; the overlay frame draws the card around it.
#[allow(clippy::too_many_lines)]
pub(crate) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let Some(form) = state.form.as_ref() else {
        return div().into_any_element();
    };
    let draft = form.draft.clone();
    let inputs = form.inputs.clone();
    let (tab, error, saving, recording, op) = (
        form.tab,
        form.error.clone(),
        form.saving,
        form.recording,
        form.op,
    );
    let shortcut = form.shortcut.clone();
    let editing = form.original.as_ref().map(|source| source.id);
    let devices = state.devices.clone();
    let registers = draft
        .modbus_device
        .as_ref()
        .and_then(|device| state.registers.get(device).cloned());
    let input = |key: &str| {
        inputs
            .get(key)
            .cloned()
            .map(IntoElement::into_any_element)
            .unwrap_or_else(|| div().into_any_element())
    };
    let labelled = |label: &str, key: &str, cx: &Context<Console>| {
        field(label.to_owned(), input(key), cx).into_any_element()
    };

    let kind = model::source_type(&draft.source_type);
    let kind_color = category_color(&draft.source_type, &theme);
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
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if editing.is_some() {
                            "Edit source"
                        } else {
                            "New source"
                        }),
                )
                .child(
                    div()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(match &form_key(console, cx) {
                            key if key.is_empty() => "no key yet".to_owned(),
                            key => key.clone(),
                        }),
                ),
        )
        .child(pill(
            format!(
                "{} · {}",
                kind.map_or("?", |kind| kind.category.tag()),
                model::type_label(&draft.source_type)
            ),
            kind_color,
        ));

    let tabs = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .px(SPACE_8)
        .pt(SPACE_6)
        .child(segmented(
            "data-form-tab",
            vec![
                Segment::new(
                    "General",
                    tab == Tab::General,
                    cx.listener(|console, _, _, cx| set_tab(console, Tab::General, cx)),
                )
                .icon(Icon::Settings),
                Segment::new(
                    "Source",
                    tab == Tab::Source,
                    cx.listener(|console, _, _, cx| set_tab(console, Tab::Source, cx)),
                )
                .icon(Icon::Database),
            ],
            cx,
        ));

    let mut body: Vec<AnyElement> = Vec::new();
    match tab {
        Tab::General => {
            let numbers = model::shows_numbers(&draft.chart_type);
            body.push(grid(
                2,
                vec![labelled("Name", "name", cx), labelled("Key", "key", cx)],
            ));
            if numbers {
                body.push(grid(
                    4,
                    vec![
                        labelled("Unit", "unit", cx),
                        labelled("Decimals", "decimals", cx),
                        labelled("Minimum", "min", cx),
                        labelled("Maximum", "max", cx),
                    ],
                ));
                body.push(grid(
                    4,
                    vec![
                        labelled("Warning at", "warn", cx),
                        labelled("Critical at", "crit", cx),
                        labelled("Base value", "base", cx),
                        div().into_any_element(),
                    ],
                ));
            }
            let charts = CHART_TYPES
                .into_iter()
                .map(|chart| {
                    Segment::new(
                        chart,
                        draft.chart_type == chart,
                        cx.listener(move |console, _, _, cx| {
                            if let Some(form) = form_mut(console) {
                                form.draft.chart_type = chart.to_owned();
                            }
                            cx.notify();
                        }),
                    )
                })
                .collect();
            body.push(
                field("Card", segmented("data-form-chart", charts, cx), cx).into_any_element(),
            );
            let color_text = inputs
                .get("color")
                .map(|input| input.read(cx).value().to_owned())
                .unwrap_or_default();
            let swatches = COLOR_PRESETS.into_iter().enumerate().map(|(index, hex)| {
                let chosen = color_text.eq_ignore_ascii_case(hex);
                div()
                    .id(("data-form-color", index))
                    .size(px(18.0))
                    .rounded(px(9.0))
                    .bg(tag_color(hex, &theme))
                    .border_2()
                    .border_color(if chosen { theme.fg } else { theme.bg })
                    .cursor_pointer()
                    .tab_index(0)
                    .focus(|style| style.border_color(theme.accent))
                    .on_click(cx.listener(move |console, _, _, cx| {
                        set_input(console, "color", hex.to_owned(), cx);
                        cx.notify();
                    }))
            });
            body.push(
                field(
                    "Colour",
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .child(
                            div()
                                .flex_none()
                                .size(px(22.0))
                                .rounded(RADIUS_SM)
                                .border_1()
                                .border_color(theme.border_strong)
                                .bg(tag_color(&color_text, &theme)),
                        )
                        .child(div().w(px(110.0)).child(input("color")))
                        .children(swatches),
                    cx,
                )
                .into_any_element(),
            );
            match draft.chart_type.as_str() {
                "image" => body.push(labelled("Image URL", "image_url", cx)),
                "text" => body.push(labelled("Text (Markdown)", "text_content", cx)),
                _ => {}
            }
            if numbers {
                body.push(
                    section_label("Low-alarm limits and deadband (optional)", cx)
                        .into_any_element(),
                );
                body.push(grid(
                    3,
                    vec![
                        labelled("Low warning", "low_warn", cx),
                        labelled("Low critical", "low_crit", cx),
                        labelled("Deadband", "deadband", cx),
                    ],
                ));
                body.push(section_label("ISA-18.2 alarm delays (seconds)", cx).into_any_element());
                body.push(grid(
                    2,
                    vec![
                        labelled("On delay", "on_delay", cx),
                        labelled("Off delay", "off_delay", cx),
                    ],
                ));
            }
            if editing.is_some() {
                body.push(section_label("Shortcut", cx).into_any_element());
                body.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .map(|this| match (&shortcut, recording) {
                            (_, true) => {
                                this.child(pill("Press a key…  Esc cancels", theme.accent))
                            }
                            (Some(combo), false) => {
                                this.child(kbd(model::display_combo(combo), cx))
                            }
                            (None, false) => this.child(note("None", theme.fg_dim)),
                        })
                        .child(
                            Button::new(
                                "data-form-record",
                                if recording { "Recording…" } else { "Record" },
                            )
                            .compact()
                            .icon(Icon::Keyboard)
                            .on_click(cx.listener(
                                |console, _, _, cx| {
                                    if let Some(form) = form_mut(console) {
                                        form.recording = true;
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .when(shortcut.is_some() && !recording, |this| {
                            this.child(
                                Button::new("data-form-clear-shortcut", "Clear")
                                    .compact()
                                    .ghost()
                                    .on_click(cx.listener(|console, _, _, cx| {
                                        if let Some(form) = form_mut(console) {
                                            form.shortcut = None;
                                        }
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(note(
                            "Opens this tag from anywhere; saved with the source.",
                            theme.fg_dim,
                        ))
                        .into_any_element(),
                );
            }
        }
        Tab::Source => {
            let groups: [(&str, &[Category]); 3] = [
                (Category::Real.title(), &[Category::Real]),
                (Category::Simulated.title(), &[Category::Simulated]),
                (Category::Api.title(), &[Category::Api, Category::Static]),
            ];
            for (index, (title, categories)) in groups.into_iter().enumerate() {
                let segments = SOURCE_TYPES
                    .iter()
                    .filter(|kind| categories.contains(&kind.category))
                    .map(|kind| {
                        let id = kind.id;
                        Segment::new(
                            kind.label,
                            draft.source_type == id,
                            cx.listener(move |console, _, _, cx| set_source_type(console, id, cx)),
                        )
                    })
                    .collect();
                body.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_5)
                        .child(
                            div()
                                .w(px(76.0))
                                .flex_none()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(title),
                        )
                        .child(segmented(("data-form-type", index), segments, cx))
                        .into_any_element(),
                );
            }
            if kind.is_none() {
                body.push(note(
                    format!(
                        "This source reads “{}”, which this console does not edit; its settings are kept as they are.",
                        draft.source_type
                    ),
                    theme.fg_muted,
                ));
            }
            let settings: Vec<AnyElement> = model::config_fields(&draft.source_type)
                .iter()
                .map(|setting| {
                    let label = match (setting.min, setting.max) {
                        (Some(min), Some(max)) => format!(
                            "{} ({}–{})",
                            setting.label,
                            model::format_number(min),
                            model::format_number(max)
                        ),
                        (Some(min), None) => {
                            format!("{} (≥ {})", setting.label, model::format_number(min))
                        }
                        _ => setting.label.to_owned(),
                    };
                    field(label, input(setting.key), cx).into_any_element()
                })
                .collect();
            if !settings.is_empty() {
                body.push(section_label("Settings", cx).into_any_element());
                body.push(grid(2, settings));
            }
            if draft.source_type == "api" {
                let methods = API_METHODS
                    .into_iter()
                    .map(|method| {
                        Segment::new(
                            method,
                            draft.method == method,
                            cx.listener(move |console, _, _, cx| {
                                if let Some(form) = form_mut(console) {
                                    form.draft.method = method.to_owned();
                                }
                                cx.notify();
                            }),
                        )
                    })
                    .collect();
                body.push(
                    field("Method", segmented("data-form-method", methods, cx), cx)
                        .into_any_element(),
                );
            }
            if let Some(text) = model::type_note(&draft.source_type) {
                body.push(note(text, theme.fg_muted));
            }
            if draft.source_type == "modbus" {
                body.extend(modbus(
                    console, &draft, devices, registers, op, &input, window, cx,
                ));
            }
        }
    }

    let footer = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(56.0))
        .px(SPACE_8)
        .border_t_1()
        .border_color(theme.border)
        .when_some(editing, |this, id| {
            this.child(
                Button::new("data-form-delete", "Delete")
                    .danger()
                    .compact()
                    .icon(Icon::Trash)
                    .on_click(cx.listener(move |console, _, window, cx| {
                        sources::ask_delete(console, id, window, cx);
                    })),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .map(|this| match &error {
                    Some(error) => this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_3)
                            .min_w_0()
                            .text_size(FONT_SMALL)
                            .text_color(theme.danger)
                            .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.danger))
                            .child(div().min_w_0().truncate().child(error.clone())),
                    ),
                    None => this
                        .text_size(FONT_CAPTION)
                        .text_color(theme.fg_dim)
                        .child(kbd("↵", cx))
                        .child("save")
                        .child(kbd("Tab", cx))
                        .child("next field")
                        .child(kbd("Esc", cx))
                        .child("close"),
                }),
        )
        .child(
            Button::new("data-form-cancel", "Cancel")
                .ghost()
                .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx))),
        )
        .child(
            Button::new(
                "data-form-save",
                match (saving, editing.is_some()) {
                    (true, _) => "Saving…",
                    (false, true) => "Save",
                    (false, false) => "Create",
                },
            )
            .primary()
            .icon(Icon::Check)
            .disabled(saving)
            .on_click(cx.listener(|console, _, window, cx| save(console, window, cx))),
        );

    div()
        .flex()
        .flex_col()
        .h(px(HEIGHT))
        .child(header)
        .child(tabs)
        .child(
            div()
                .id("data-form-body")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .overflow_y_scroll()
                .children(body),
        )
        .child(footer)
        .into_any_element()
}

/// The key as typed, for the header.
fn form_key(console: &mut Console, cx: &App) -> String {
    input_text(console, "key", cx).trim().to_owned()
}

/// The Modbus part of the Source tab: the device, then one register or an
/// expression over the device's registers.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn modbus(
    console: &mut Console,
    draft: &Draft,
    devices: Option<Result<Vec<ModbusDevice>, String>>,
    registers: Option<Result<Vec<ModbusRegister>, String>>,
    op: char,
    input: &dyn Fn(&str) -> AnyElement,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> Vec<AnyElement> {
    let theme = theme(cx).clone();
    let mut out = vec![section_label("Modbus device", cx).into_any_element()];
    match devices {
        None => out.push(note("Loading the Modbus devices…", theme.fg_dim)),
        Some(Err(error)) => out.push(note(format!("Modbus devices: {error}"), theme.danger)),
        Some(Ok(list)) if list.is_empty() => out.push(note(
            "No Modbus devices yet: add one in the Modbus device manager.",
            theme.fg_muted,
        )),
        Some(Ok(list)) => {
            let rows = list
                .iter()
                .map(|device| PickRow {
                    label: if device.name.is_empty() {
                        device.id.clone().into()
                    } else {
                        device.name.clone().into()
                    },
                    detail: format!(
                        "{}:{} · {} registers",
                        device.ip, device.port, device.register_count
                    )
                    .into(),
                    chosen: draft.modbus_device.as_deref() == Some(device.id.as_str()),
                })
                .collect();
            out.push(picker(console, DEVICES, rows, pick_device, cx, window));
        }
    }
    if draft.modbus_device.is_none() {
        out.push(note(
            "Select a Modbus device to read live data from its registers.",
            theme.fg_muted,
        ));
        return out;
    }
    let expression = draft.modbus_mode == ModbusMode::Expression;
    let mode = |label: &'static str, wanted: ModbusMode| {
        Segment::new(
            label,
            draft.modbus_mode == wanted,
            cx.listener(move |console, _, _, cx| {
                if let Some(form) = form_mut(console) {
                    form.draft.modbus_mode = wanted;
                }
                cx.notify();
            }),
        )
    };
    let modes = vec![
        mode("Single register", ModbusMode::Single),
        mode("Expression", ModbusMode::Expression),
    ];
    out.push(segmented("data-form-modbus-mode", modes, cx));
    let list = match registers {
        None => {
            out.push(note("Loading the device's registers…", theme.fg_dim));
            return out;
        }
        Some(Err(error)) => {
            out.push(note(format!("Registers: {error}"), theme.danger));
            return out;
        }
        Some(Ok(list)) => list,
    };
    if expression {
        out.push(field("Expression", input("expression"), cx).into_any_element());
        let ops = ['+', '-', '*', '/']
            .into_iter()
            .map(|symbol| {
                Segment::new(
                    symbol.to_string(),
                    op == symbol,
                    cx.listener(move |console, _, _, cx| {
                        if let Some(form) = form_mut(console) {
                            form.op = symbol;
                        }
                        cx.notify();
                    }),
                )
            })
            .collect();
        out.push(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(SPACE_3)
                .child(note("Insert", theme.fg_muted))
                .child(segmented("data-form-op", ops, cx))
                .children(list.iter().enumerate().map(|(index, register)| {
                    let name = register.name.clone();
                    Button::new(("data-form-register", index), register.name.clone())
                        .compact()
                        .tooltip(format!(
                            "addr {}, {} · appends {op} {{{}}}",
                            register.address, register.register_type, register.name
                        ))
                        .on_click(cx.listener(move |console, _, _, cx| {
                            append_register(console, &name, cx);
                        }))
                        .into_any_element()
                }))
                .into_any_element(),
        );
        let preview = input_text(console, "expression", cx);
        if !preview.trim().is_empty() {
            out.push(
                div()
                    .px(SPACE_5)
                    .py(SPACE_3)
                    .rounded(RADIUS_MD)
                    .bg(theme.accent_soft)
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .child(format!("= {}", preview.trim()))
                    .into_any_element(),
            );
        }
    } else if list.is_empty() {
        out.push(note("This device has no registers yet.", theme.fg_muted));
    } else {
        let rows = list
            .iter()
            .map(|register| PickRow {
                label: register.name.clone().into(),
                detail: format!("addr {} · {}", register.address, register.register_type).into(),
                chosen: draft.register.as_deref() == Some(register.name.as_str()),
            })
            .collect();
        out.push(section_label("Register", cx).into_any_element());
        out.push(picker(console, REGISTERS, rows, pick_register, cx, window));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::testing::Harness;

    fn saved(id: i64) -> Saved {
        Saved {
            id,
            key: "line1_temp".into(),
            warning: None,
            shortcut_changed: false,
        }
    }

    fn form_seq(h: &mut Harness) -> Option<i64> {
        h.update(|console, _, _| console.state::<State>().form.as_ref().map(|form| form.seq))
    }

    #[gpui::test]
    fn a_late_save_answer_leaves_a_newer_form_open(cx: &mut gpui::TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| open(console, None, window, cx));
        let first = form_seq(&mut h).expect("the first form");
        // Esc while it saves, then another form.
        h.update(|console, window, cx| {
            console.close_overlay(window, cx);
            open(console, None, window, cx);
        });
        let second = form_seq(&mut h).expect("the second form");
        assert_ne!(first, second);
        h.reply(
            CallKey::new("data.sources.save").with(first),
            Reply::ok(saved(7)),
        );
        h.draw();
        assert!(
            h.update(|console, _, _| console.overlay_is(FORM)),
            "still open"
        );
        assert_eq!(form_seq(&mut h), Some(second), "with its edits");
        // Its own answer closes it.
        h.reply(
            CallKey::new("data.sources.save").with(second),
            Reply::ok(saved(8)),
        );
        h.draw();
        assert_eq!(form_seq(&mut h), None);
    }

    #[gpui::test]
    fn a_recorded_shortcut_reads_back_as_the_same_keys(cx: &mut gpui::TestAppContext) {
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| {
            open(console, Some(Harness::tag(7, "plts_1.p")), window, cx);
        });
        h.draw();
        h.update(|console, _, _| {
            if let Some(form) = form_mut(console) {
                form.recording = true;
            }
        });
        h.keys("cmd-shift-up");
        let recorded =
            h.update(|console, _, _| form_mut(console).and_then(|form| form.shortcut.clone()));
        assert_eq!(recorded.as_deref(), Some("Mod+Shift+ArrowUp"));
        assert_eq!(
            recorded
                .as_deref()
                .and_then(shortcuts::to_keystroke)
                .as_deref(),
            Some("secondary-shift-up")
        );
    }

    #[test]
    fn every_text_field_round_trips_through_the_draft() {
        let mut draft = Draft::new();
        for (key, ..) in GENERAL {
            set_text(&mut draft, key, format!("{key}!"));
            assert_eq!(text_of(&draft, key), format!("{key}!"), "{key}");
        }
        for setting in model::all_config_fields() {
            set_text(&mut draft, setting.key, "7".into());
            assert_eq!(text_of(&draft, setting.key), "7", "{}", setting.key);
            set_text(&mut draft, setting.key, "  ".into());
            assert!(!draft.settings.contains_key(setting.key), "blank is absent");
        }
        let keys: std::collections::HashSet<&str> = GENERAL
            .iter()
            .map(|(key, ..)| *key)
            .chain(model::all_config_fields().map(|setting| setting.key))
            .collect();
        assert_eq!(
            keys.len(),
            GENERAL.len() + model::all_config_fields().count(),
            "no two fields share a key"
        );
    }

    #[test]
    fn device_ids_are_escaped_in_paths() {
        assert_eq!(path_segment("inverter-1"), "inverter-1");
        assert_eq!(path_segment("line 2/a"), "line%202%2Fa");
    }
}
