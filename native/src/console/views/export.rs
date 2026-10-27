//! The data export dialog (the palette's "Export data…"): telemetry rows
//! from `GET /api/v1/data/export` for a time range, device and metric, raw
//! or aggregated per interval, as JSON or CSV, saved to a file the operator
//! picks. Its REST calls are scoped to the Audit page, whose `on_reply`
//! hands the answer back here.

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, Context, Entity, FontWeight, Subscription, Window, div, px,
};
use time::OffsetDateTime;
use voltrus_scada_native::api::ops::{
    self, AGGREGATIONS, EXPORT_MAX, ExportQuery, Format, INTERVALS, Saved,
};
use voltrus_scada_native::runtime::{CallKey, Reply};
use voltrus_scada_native::zone::LocalZone;
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, segmented};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::capitalized;
use super::ops_kit::{close_overlay_in, labelled, on_field, save_download, saved_message, value};
use crate::console::Console;
use crate::console::overlay::Overlay;

/// The dialog's overlay id.
const DIALOG: &str = "data-export";

/// Quick ranges: a label and how far back from now, in seconds.
const PRESETS: [(&str, i64); 4] = [
    ("Last hour", 3600),
    ("24 h", 86_400),
    ("7 days", 7 * 86_400),
    ("30 days", 30 * 86_400),
];

struct Form {
    from: Entity<TextInput>,
    to: Entity<TextInput>,
    device: Entity<TextInput>,
    metric: Entity<TextInput>,
    limit: Entity<TextInput>,
    /// The quick range the times came from, until edited by hand.
    preset: Option<usize>,
    /// 0 for raw rows, else `AGGREGATIONS[aggregation - 1]`.
    aggregation: usize,
    /// Index into `INTERVALS`.
    interval: usize,
    format: Format,
    busy: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(crate) struct State {
    form: Option<Form>,
    window: Option<AnyWindowHandle>,
}

/// Open the dialog, keeping what was filled in last time.
pub fn open(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().form.is_none() {
        let form = new_form(window, cx);
        console.state::<State>().form = Some(form);
        set_preset(console, 1, cx);
    }
    console.state::<State>().window = Some(window.window_handle());
    let overlay = Overlay::modal(DIALOG, px(580.0), dialog, cx);
    console.open_overlay(overlay, window, cx);
    if let Some(form) = &console.state::<State>().form {
        form.from.update(cx, |field, _| field.focus(window));
    }
}

fn new_form(window: &mut Window, cx: &mut Context<Console>) -> Form {
    let field = |placeholder: &'static str, cx: &mut Context<Console>| {
        cx.new(|cx| TextInput::new(cx).placeholder(placeholder).mono())
    };
    let from = field("YYYY-MM-DD HH:MM", cx);
    let to = field("YYYY-MM-DD HH:MM", cx);
    let device = field("Every device, or one device id", cx);
    let metric = field("Every metric, or one tag key", cx);
    let limit = cx.new(|cx| {
        TextInput::new(cx)
            .placeholder("1000")
            .text("1000")
            .kind(InputKind::Unsigned)
            .mono()
    });
    let mut subscriptions = Vec::new();
    for time in [&from, &to] {
        subscriptions.push(on_field(time, time_event, window, cx));
    }
    for other in [&device, &metric, &limit] {
        subscriptions.push(on_field(other, field_event, window, cx));
    }
    Form {
        from,
        to,
        device,
        metric,
        limit,
        preset: None,
        aggregation: 0,
        interval: 1,
        format: Format::Json,
        busy: false,
        error: None,
        _subscriptions: subscriptions,
    }
}

/// A time typed by hand ends the quick range.
fn time_event(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if *event == InputEvent::Changed
        && let Some(form) = &mut console.state::<State>().form
    {
        form.preset = None;
    }
    field_event(console, event, window, cx);
}

/// Enter exports; Esc closes (the form stays for next time).
fn field_event(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Changed => {
            if let Some(form) = &mut console.state::<State>().form {
                form.error = None;
            }
            cx.notify();
        }
        InputEvent::Submitted => export(console, cx),
        InputEvent::Cancelled => console.close_overlay(window, cx),
        InputEvent::Blurred => {}
    }
}

/// Fill the range with quick range `index`, ending now.
fn set_preset(console: &mut Console, index: usize, cx: &mut Context<Console>) {
    let zone = console.zone.clone();
    let Some(form) = &mut console.state::<State>().form else {
        return;
    };
    let Some((_, span)) = PRESETS.get(index) else {
        return;
    };
    let now = OffsetDateTime::now_utc().unix_timestamp();
    form.preset = Some(index);
    form.error = None;
    let (from, to) = (
        ops::format_local(now - span, &zone),
        ops::format_local(now, &zone),
    );
    form.from.update(cx, |field, cx| field.set_text(from, cx));
    form.to.update(cx, |field, cx| field.set_text(to, cx));
    cx.notify();
}

/// What the form asks the server for; why not, when something does not
/// read.
fn query(form: &Form, zone: &LocalZone, cx: &gpui::App) -> Result<ExportQuery, String> {
    let time = |field: &Entity<TextInput>, end: bool, name: &str| {
        let text = value(field, cx);
        if text.is_empty() {
            Ok(None)
        } else {
            ops::parse_local(&text, zone, end)
                .map(Some)
                .ok_or_else(|| format!("{name} reads YYYY-MM-DD or YYYY-MM-DD HH:MM"))
        }
    };
    let from = time(&form.from, false, "From")?;
    let to = time(&form.to, true, "To")?;
    if let (Some(from), Some(to)) = (from, to)
        && from > to
    {
        return Err("From comes after To".into());
    }
    let limit_text = value(&form.limit, cx);
    let limit = if limit_text.is_empty() {
        1000
    } else {
        match limit_text.parse::<usize>() {
            Ok(limit) if (1..=EXPORT_MAX).contains(&limit) => limit,
            _ => return Err(format!("The limit is 1 to {EXPORT_MAX} rows")),
        }
    };
    Ok(ExportQuery {
        from,
        to,
        device: value(&form.device, cx),
        metric: value(&form.metric, cx),
        aggregation: form
            .aggregation
            .checked_sub(1)
            .and_then(|index| AGGREGATIONS.get(index).copied()),
        interval: INTERVALS.get(form.interval).copied().unwrap_or("5m"),
        format: form.format,
        limit,
    })
}

/// Ask where to save, then fetch the rows into the file.
fn export(console: &mut Console, cx: &mut Context<Console>) {
    let zone = console.zone.clone();
    let Some(form) = &mut console.state::<State>().form else {
        return;
    };
    if form.busy {
        return;
    }
    let query = match query(form, &zone, cx) {
        Ok(query) => query,
        Err(problem) => {
            form.error = Some(problem);
            cx.notify();
            return;
        }
    };
    form.busy = true;
    form.error = None;
    let now = zone.now();
    let name = format!(
        "telemetry-{}-{:02}{:02}.{}",
        ops::format_date(now.date()),
        now.hour(),
        now.minute(),
        query.format.param()
    );
    save_download(
        console,
        CallKey::new("audit.data-export"),
        query.path(),
        name,
        query.format,
        |console, cx| {
            if let Some(form) = &mut console.state::<State>().form {
                form.busy = false;
            }
            cx.notify();
        },
        cx,
    );
    cx.notify();
}

/// The saved file, or why not (routed here by the Audit page).
pub fn on_reply(console: &mut Console, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let window = state.window;
    if let Some(form) = &mut state.form {
        form.busy = false;
    }
    match reply.take::<Saved>() {
        Ok(saved) => {
            if let Some(window) = window {
                close_overlay_in(window, DIALOG, cx);
            }
            console.show_toast(saved_message(&saved, "rows"), false, cx);
        }
        Err(error) => {
            if let Some(form) = &mut console.state::<State>().form {
                form.error = Some(error.clone());
            }
            if !console.overlay_is(DIALOG) {
                console.show_toast(format!("Data export failed: {error}"), true, cx);
            }
        }
    }
}

/// The dialog's content; the overlay draws its frame.
fn dialog(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(form) = &console.state::<State>().form else {
        return div().into_any_element();
    };
    let (from, to, device, metric, limit) = (
        form.from.clone(),
        form.to.clone(),
        form.device.clone(),
        form.metric.clone(),
        form.limit.clone(),
    );
    let (preset, aggregation, interval, format, busy, error) = (
        form.preset,
        form.aggregation,
        form.interval,
        form.format,
        form.busy,
        form.error.clone(),
    );

    let presets = PRESETS
        .iter()
        .enumerate()
        .map(|(index, (label, _))| {
            Segment::new(
                *label,
                preset == Some(index),
                cx.listener(move |console, _, _, cx| set_preset(console, index, cx)),
            )
        })
        .collect();
    let aggregations = std::iter::once("raw")
        .chain(AGGREGATIONS)
        .enumerate()
        .map(|(index, name)| {
            Segment::new(
                capitalized(name),
                aggregation == index,
                cx.listener(move |console, _, _, cx| {
                    if let Some(form) = &mut console.state::<State>().form {
                        form.aggregation = index;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();
    let intervals = INTERVALS
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Segment::new(
                *name,
                interval == index,
                cx.listener(move |console, _, _, cx| {
                    if let Some(form) = &mut console.state::<State>().form {
                        form.interval = index;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();
    let formats = [(Format::Json, "JSON"), (Format::Csv, "CSV")]
        .into_iter()
        .map(|(choice, label)| {
            Segment::new(
                label,
                format == choice,
                cx.listener(move |console, _, _, cx| {
                    if let Some(form) = &mut console.state::<State>().form {
                        form.format = choice;
                    }
                    cx.notify();
                }),
            )
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(48.0))
                .px(SPACE_8)
                .border_b_1()
                .border_color(theme.border)
                .child(vt_ui::icon(Icon::Download, ICON_MD, theme.accent))
                .child(
                    div()
                        .flex_1()
                        .text_size(FONT_EMPHASIS)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Export data"),
                )
                .child(
                    IconButton::new("export-close", Icon::X)
                        .tooltip("Close  Esc")
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.close_overlay(window, cx);
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .child(labelled("Range", segmented("export-preset", presets, cx), &theme))
                .child(
                    div()
                        .flex()
                        .gap(SPACE_5)
                        .child(labelled("From, your clock", from, &theme).flex_1())
                        .child(labelled("To", to, &theme).flex_1()),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_5)
                        .child(labelled("Device", device, &theme).flex_1())
                        .child(labelled("Metric", metric, &theme).flex_1()),
                )
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(SPACE_5)
                        .child(labelled(
                            "Aggregation",
                            segmented("export-aggregation", aggregations, cx),
                            &theme,
                        ))
                        .when(aggregation != 0, |this| {
                            this.child(labelled(
                                "Per",
                                segmented("export-interval", intervals, cx),
                                &theme,
                            ))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(SPACE_5)
                        .child(labelled("Format", segmented("export-format", formats, cx), &theme))
                        .child(labelled("Rows, at most", div().w(px(110.0)).child(limit), &theme))
                        .child(
                            div()
                                .pb(SPACE_2)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(format!("up to {EXPORT_MAX}")),
                        ),
                )
                .child(
                    div()
                        .min_h(px(18.0))
                        .text_size(FONT_SMALL)
                        .map(|this| match &error {
                            Some(error) => this.text_color(theme.danger).child(error.clone()),
                            None => this
                                .text_color(theme.fg_dim)
                                .child("Times are read on this computer's clock; leave a field empty for no limit."),
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(SPACE_4)
                .px(SPACE_8)
                .py(SPACE_6)
                .border_t_1()
                .border_color(theme.border)
                .child(
                    Button::new("export-cancel", "Cancel")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new("export-save", if busy { "Exporting…" } else { "Export…  ↵" })
                        .primary()
                        .icon(Icon::Download)
                        .disabled(busy)
                        .on_click(cx.listener(|console, _, _, cx| export(console, cx))),
                ),
        )
        .into_any_element()
}
