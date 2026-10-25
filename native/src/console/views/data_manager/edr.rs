//! Data Manager › EDR compatibility: the web client's `EdrCompatPanel.tsx`
//! — the report (detected security products, I/O optimisation, warnings,
//! recommendations), the recommended exclusions, the Windows Defender
//! script (operators: shown, copied, saved as `.ps1`) and the Group Policy
//! deployment guide.
//!
//! Every tab is one keyboard list of fixed-height rows (j/k, PgUp/PgDn);
//! Enter copies the row's text (an exclusion's path, a script line). `[`
//! and `]` switch tabs, R refreshes, C copies the whole script or guide, S
//! saves the script. `/` filters the exclusions.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, ClipboardItem, Context, Entity, FontWeight, Hsla, SharedString, Subscription,
    Window, div, px, uniform_list,
};
use voltrus_scada_native::api::eng_edr::{
    self as model, EdrReport, Exclusions, IoStatus, PS1, PS1_FILE,
};
use voltrus_scada_native::api::ops::{Format, Saved};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, pill, segmented};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use crate::console::Console;
use crate::console::keys::{EngEdrCopy, EngEdrNextTab, EngEdrPrevTab, EngEdrRefresh, EngEdrSave};
use crate::console::nav::Move;
use crate::console::views::{
    ListKeys, focus_ring, keyboard_list, ops_kit, page_header, rows_on_screen, tail_row, tail_rows,
};

const LIST: &str = "data.edr.rows";
const ROW: f32 = 34.0;
const PRIORITY: f32 = 108.0;
const KIND: f32 = 96.0;
const PATH: f32 = 320.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Report,
    Exclusions,
    Script,
    Guide,
}

impl Tab {
    const ALL: [Self; 4] = [Self::Report, Self::Exclusions, Self::Script, Self::Guide];

    fn label(self) -> &'static str {
        match self {
            Self::Report => "Report",
            Self::Exclusions => "Exclusions",
            Self::Script => "Defender script",
            Self::Guide => "Deploy guide",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Report => Icon::Gauge,
            Self::Exclusions => Icon::List,
            Self::Script => Icon::Terminal,
            Self::Guide => Icon::ScrollText,
        }
    }
}

/// A text answer: its lines for the list and the whole for copying.
#[derive(Clone)]
struct Text {
    raw: String,
    lines: Rc<[String]>,
}

impl Text {
    fn new(raw: String) -> Self {
        let lines = model::text_lines(&raw).into();
        Self { raw, lines }
    }
}

#[derive(Default)]
struct State {
    tab: Tab,
    report: Option<Result<EdrReport, String>>,
    io: Option<IoStatus>,
    exclusions: Option<Result<Exclusions, String>>,
    script: Option<Result<Text, String>>,
    guide: Option<Result<Text, String>>,
    asked: bool,
    exclusions_asked: bool,
    script_asked: bool,
    guide_asked: bool,
    saving: bool,
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
}

pub fn on_open(console: &mut Console, _: &mut Context<Console>) {
    if console.stress.is_some() || console.state::<State>().asked {
        return;
    }
    console.state::<State>().asked = true;
    fetch_report(console);
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    console.focus_list(LIST, window, cx);
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console.state_ref::<State>()?.search.clone()
}

fn fetch_report(console: &mut Console) {
    console.call(
        CallKey::new("data.edr.report"),
        Call::new(|api| async move { api.edr_report().await }),
    );
    console.call(
        CallKey::new("data.edr.io"),
        Call::new(|api| async move { api.edr_io_status().await }),
    );
}

/// Fetch what `tab` shows if it is not there yet (or `again`).
fn fetch_tab(console: &mut Console, tab: Tab, again: bool) {
    if console.stress.is_some() {
        return;
    }
    let operator = console.is_operator();
    let state = console.state::<State>();
    match tab {
        Tab::Report => {
            if again {
                fetch_report(console);
            }
        }
        Tab::Exclusions => {
            if !state.exclusions_asked || again {
                state.exclusions_asked = true;
                console.call(
                    CallKey::new("data.edr.exclusions"),
                    Call::new(|api| async move { api.edr_exclusions().await }),
                );
            }
        }
        Tab::Script => {
            if operator && (!state.script_asked || again) {
                state.script_asked = true;
                console.call(
                    CallKey::new("data.edr.script"),
                    Call::new(|api| async move { api.edr_defender_ps1().await }),
                );
            }
        }
        Tab::Guide => {
            if !state.guide_asked || again {
                state.guide_asked = true;
                console.call(
                    CallKey::new("data.edr.guide"),
                    Call::new(|api| async move { api.edr_gpo_guide().await }),
                );
            }
        }
    }
}

fn set_tab(console: &mut Console, tab: Tab, cx: &mut Context<Console>) {
    console.state::<State>().tab = tab;
    let nav = console.list(LIST, cx);
    nav.cursor = None;
    nav.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
    fetch_tab(console, tab, false);
    cx.notify();
}

fn step_tab(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    let at = Tab::ALL
        .iter()
        .position(|tab| *tab == console.state::<State>().tab)
        .unwrap_or(0);
    let next = if forward {
        (at + 1) % Tab::ALL.len()
    } else {
        (at + Tab::ALL.len() - 1) % Tab::ALL.len()
    };
    set_tab(console, Tab::ALL[next], cx);
}

fn refresh(console: &mut Console, cx: &mut Context<Console>) {
    let tab = console.state::<State>().tab;
    fetch_tab(console, tab, true);
    console.show_toast(
        format!("Refreshing {}", tab.label().to_lowercase()),
        false,
        cx,
    );
}

fn copy_text(console: &mut Console, text: String, what: &str, cx: &mut Context<Console>) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    console.show_toast(format!("Copied {what}"), false, cx);
}

/// C: the whole script or guide on the clipboard (the web "Copy Script").
fn copy_all(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let (text, what) = match state.tab {
        Tab::Script => (state.script.as_ref(), "the script"),
        Tab::Guide => (state.guide.as_ref(), "the guide"),
        _ => {
            console.show_toast("C copies the script or the guide", false, cx);
            return;
        }
    };
    match text {
        Some(Ok(text)) => {
            let raw = text.raw.clone();
            copy_text(console, raw, what, cx);
        }
        _ => console.show_toast("Nothing to copy yet", true, cx),
    }
}

/// S: save the Defender script where the operator picks.
fn save_script(console: &mut Console, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.show_toast("Only operators can download the Defender script", true, cx);
        return;
    }
    let state = console.state::<State>();
    if state.saving {
        return;
    }
    state.saving = true;
    // `Format` only decides how rows are counted; the text is written as
    // the server sent it.
    ops_kit::save_download(
        CallKey::new("data.edr.save"),
        PS1.to_owned(),
        PS1_FILE.to_owned(),
        Format::Csv,
        |console, cx| {
            console.state::<State>().saving = false;
            cx.notify();
        },
        cx,
    );
    cx.notify();
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    match key.name {
        "data.edr.report" => state.report = Some(reply.take::<EdrReport>()),
        "data.edr.io" => {
            if let Ok(io) = reply.take::<IoStatus>() {
                state.io = Some(io);
            }
        }
        "data.edr.exclusions" => state.exclusions = Some(reply.take::<Exclusions>()),
        "data.edr.script" => state.script = Some(reply.take::<String>().map(Text::new)),
        "data.edr.guide" => state.guide = Some(reply.take::<String>().map(Text::new)),
        "data.edr.save" => {
            state.saving = false;
            match reply.take::<Saved>() {
                Ok(saved) => {
                    let name = saved.file.file_name().map_or_else(
                        || saved.file.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    );
                    console.show_toast(format!("Saved the Defender script to {name}"), false, cx);
                }
                Err(error) => console.show_toast(format!("Saving failed: {error}"), true, cx),
            }
        }
        _ => {}
    }
    cx.notify();
}

fn ensure_search(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().search.is_some() {
        return;
    }
    let field = ops_kit::text_field("Filter exclusions  /", false, cx);
    let subscription = cx.subscribe_in(
        &field,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                if console.state::<State>().tab != Tab::Exclusions {
                    set_tab(console, Tab::Exclusions, cx);
                }
                console.list(LIST, cx).cursor = None;
                cx.notify();
            }
            InputEvent::Submitted | InputEvent::Cancelled => {
                let nav = console.list(LIST, cx);
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

// ── Rows ─────────────────────────────────────────────────────────────────

/// One fixed-height line of a tab.
#[derive(Clone)]
enum Row {
    /// The compatibility banner.
    Banner {
        compatible: bool,
        line: String,
    },
    Heading(&'static str, Option<usize>),
    Product(model::EdrProduct),
    Io {
        label: &'static str,
        value: String,
        good: Option<bool>,
    },
    Warning(String),
    Recommendation(String),
    Exclusion(model::EdrExclusion),
    Line(String),
    Note(String, bool),
}

impl Row {
    /// What Enter copies.
    fn text(&self) -> Option<String> {
        match self {
            Self::Banner { line, .. } => Some(line.clone()),
            Self::Product(product) => Some(format!("{} ({})", product.name, product.vendor)),
            Self::Io { label, value, .. } => Some(format!("{label}: {value}")),
            Self::Warning(text) | Self::Recommendation(text) | Self::Line(text) => {
                Some(text.clone())
            }
            Self::Exclusion(exclusion) => Some(exclusion.path.clone()),
            Self::Heading(..) | Self::Note(..) => None,
        }
    }
}

fn io_rows(io: &IoStatus) -> Vec<Row> {
    let flag = |label, on: bool| Row::Io {
        label,
        value: if on { "Enabled" } else { "Disabled" }.to_owned(),
        good: Some(on),
    };
    vec![
        flag("Batch writes", io.batch_writes_enabled),
        flag("WAL mode", io.wal_mode_active),
        flag("Checkpoint optimized", io.checkpoint_optimized),
        Row::Io {
            label: "Batch size",
            value: io.batch_size.to_string(),
            good: None,
        },
        Row::Io {
            label: "Write amplification",
            value: model::amplification_label(io.write_amplification),
            good: None,
        },
        Row::Io {
            label: "Sequential writes",
            value: model::percent_label(io.sequential_write_pct),
            good: None,
        },
    ]
}

fn report_rows(report: &EdrReport, io: Option<&IoStatus>) -> Vec<Row> {
    let mut rows = vec![Row::Banner {
        compatible: report.compatible,
        line: format!(
            "{} — {}",
            if report.compatible {
                "System is EDR-compatible"
            } else {
                "EDR compatibility issues detected"
            },
            report.banner()
        ),
    }];
    rows.push(Row::Heading(
        "Detected security products",
        Some(report.detected_products.len()),
    ));
    if report.detected_products.is_empty() {
        rows.push(Row::Note(
            "No EDR/antivirus products detected on this system.".to_owned(),
            false,
        ));
    }
    rows.extend(report.detected_products.iter().cloned().map(Row::Product));
    rows.push(Row::Heading("I/O optimization", None));
    rows.extend(io_rows(io.unwrap_or(&report.io_optimization)));
    if !report.warnings.is_empty() {
        rows.push(Row::Heading("Warnings", Some(report.warnings.len())));
        rows.extend(report.warnings.iter().cloned().map(Row::Warning));
    }
    if !report.recommendations.is_empty() {
        rows.push(Row::Heading(
            "Recommendations",
            Some(report.recommendations.len()),
        ));
        rows.extend(
            report
                .recommendations
                .iter()
                .cloned()
                .map(Row::Recommendation),
        );
    }
    rows
}

fn text_rows(text: Option<&Result<Text, String>>, loading: &str) -> Vec<Row> {
    match text {
        None => vec![Row::Note(loading.to_owned(), false)],
        Some(Err(error)) => vec![Row::Note(error.clone(), true)],
        Some(Ok(text)) if text.lines.is_empty() => {
            vec![Row::Note("The server sent nothing.".to_owned(), false)]
        }
        Some(Ok(text)) => text.lines.iter().cloned().map(Row::Line).collect(),
    }
}

fn rows(console: &Console, query: &str) -> Vec<Row> {
    let Some(state) = console.state_ref::<State>() else {
        return Vec::new();
    };
    match state.tab {
        Tab::Report => match &state.report {
            None => vec![Row::Note(
                "Scanning the system for EDR/antivirus products…".to_owned(),
                false,
            )],
            Some(Err(error)) => vec![Row::Note(error.clone(), true)],
            Some(Ok(report)) => report_rows(report, state.io.as_ref()),
        },
        Tab::Exclusions => {
            // The lighter endpoint, else the report's copy while it loads.
            let list = match (&state.exclusions, &state.report) {
                (Some(Ok(answer)), _) => &answer.exclusions,
                (_, Some(Ok(report))) => &report.recommended_exclusions,
                (Some(Err(error)), _) => return vec![Row::Note(error.clone(), true)],
                _ => return vec![Row::Note("Loading exclusions…".to_owned(), false)],
            };
            let found = model::filter_exclusions(list, query);
            if found.is_empty() {
                return vec![Row::Note(
                    if list.is_empty() {
                        "The server recommends no exclusions.".to_owned()
                    } else {
                        "No exclusion matches the filter.".to_owned()
                    },
                    false,
                )];
            }
            found.into_iter().cloned().map(Row::Exclusion).collect()
        }
        Tab::Script => {
            if console.is_operator() {
                text_rows(state.script.as_ref(), "Loading the Defender script…")
            } else {
                vec![Row::Note(
                    "The Windows Defender script is for operators (admin, demo, integration)."
                        .to_owned(),
                    false,
                )]
            }
        }
        Tab::Guide => text_rows(state.guide.as_ref(), "Loading the deployment guide…"),
    }
}

fn activate(console: &mut Console, row: usize, _: bool, _: &mut Window, cx: &mut Context<Console>) {
    let query = query(console, cx);
    if let Some(text) = rows(console, &query).get(row).and_then(Row::text) {
        copy_text(console, text, "the line", cx);
    }
}

fn query(console: &Console, cx: &gpui::App) -> String {
    console
        .state_ref::<State>()
        .and_then(|state| state.search.as_ref())
        .map(|field| ops_kit::value(field, cx))
        .unwrap_or_default()
}

// ── Drawing ──────────────────────────────────────────────────────────────

fn priority_color(priority: &str, theme: &Theme) -> Hsla {
    match model::priority_rank(priority) {
        0 => theme.danger,
        1 => theme.warning,
        _ => theme.fg_dim,
    }
}

#[allow(clippy::too_many_lines)]
fn draw_row(row: &Row, index: usize, ring: bool, current: bool, theme: &Theme) -> AnyElement {
    let base = div()
        .id(("edr-row", index))
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(ROW))
        .px(SPACE_9)
        .when(current, |this| this.bg(theme.selected))
        .when(ring, |this| this.child(focus_ring(theme)));
    let text = |text: String, color: Hsla| {
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_color(color)
            .child(text)
    };
    let element = match row {
        Row::Banner { compatible, line } => {
            let color = if *compatible {
                theme.success
            } else {
                theme.danger
            };
            base.bg(color.alpha(0.08))
                .child(vt_ui::icon(
                    if *compatible {
                        Icon::CircleCheck
                    } else {
                        Icon::CircleAlert
                    },
                    ICON_MD,
                    color,
                ))
                .child(text(line.clone(), color).font_weight(FontWeight::SEMIBOLD))
        }
        Row::Heading(title, count) => base
            .pt(SPACE_4)
            .text_size(FONT_CAPTION)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.fg_dim)
            .child(title.to_uppercase())
            .when_some(*count, |this, count| {
                this.child(div().font_family(MONO).child(count.to_string()))
            }),
        Row::Product(product) => base
            .child(pill(model::category_label(&product.category), theme.info))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .gap(SPACE_3)
                    .child(div().flex_none().child(product.name.clone()))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.fg_muted)
                            .child(format!("({})", product.vendor)),
                    )
                    .when_some(product.notes.clone(), |this, notes| {
                        this.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(notes),
                        )
                    }),
            )
            .child(pill(
                if product.active { "Active" } else { "Inactive" },
                if product.active {
                    theme.success
                } else {
                    theme.fg_dim
                },
            )),
        Row::Io { label, value, good } => {
            base.child(text((*label).to_owned(), theme.fg_muted)).child(
                div()
                    .font_family(MONO)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(match good {
                        Some(true) => theme.success,
                        Some(false) => theme.warning,
                        None => theme.fg,
                    })
                    .child(value.clone()),
            )
        }
        Row::Warning(warning) => base
            .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.warning))
            .child(text(warning.clone(), theme.fg)),
        Row::Recommendation(line) => base
            .child(vt_ui::icon(Icon::Info, ICON_SM, theme.accent))
            .child(text(line.clone(), theme.fg)),
        Row::Exclusion(exclusion) => base
            .child(div().w(px(PRIORITY)).flex_none().flex().child(pill(
                exclusion.priority.to_uppercase(),
                priority_color(&exclusion.priority, theme),
            )))
            .child(
                div()
                    .w(px(KIND))
                    .flex_none()
                    .truncate()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(exclusion.exclusion_type.clone()),
            )
            .child(
                div()
                    .w(px(PATH))
                    .flex_none()
                    .truncate()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .child(exclusion.path.clone()),
            )
            .child(text(exclusion.reason.clone(), theme.fg_muted)),
        Row::Line(line) => base.h(px(ROW)).child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .child(if line.is_empty() {
                    " ".to_owned()
                } else {
                    line.clone()
                }),
        ),
        Row::Note(note, error) => base
            .child(vt_ui::icon(
                if *error {
                    Icon::CircleAlert
                } else {
                    Icon::Clock
                },
                ICON_SM,
                if *error { theme.danger } else { theme.fg_dim },
            ))
            .child(text(
                note.clone(),
                if *error { theme.danger } else { theme.fg_muted },
            )),
    };
    element.into_any_element()
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    ensure_search(console, window, cx);
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    let query = query(console, cx);
    let rows: Rc<[Row]> = rows(console, &query).into();
    let state = console.state::<State>();
    let tab = state.tab;
    let saving = state.saving;
    let search_field = state.search.clone();
    let script_ready = matches!(state.script, Some(Ok(_)));
    let guide_ready = matches!(state.guide, Some(Ok(_)));
    let subtitle = match &state.report {
        Some(Ok(report)) => format!(
            "{} · {} · [ ] tabs · ↵ copy line · R refresh",
            if report.compatible {
                "Compatible"
            } else {
                "Issues detected"
            },
            report.banner()
        ),
        Some(Err(_)) => "The report failed · R retry".to_owned(),
        None => "Scanning…".to_owned(),
    };

    let segments = Tab::ALL
        .iter()
        .map(|each| {
            let each = *each;
            Segment::new(
                each.label(),
                tab == each,
                cx.listener(move |console, _, _, cx| set_tab(console, each, cx)),
            )
            .icon(each.icon())
        })
        .collect();

    let mut controls = vec![
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .w(px(220.0))
            .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
            .children(search_field.map(|field| div().flex_1().child(field)))
            .into_any_element(),
        Button::new("edr-refresh", "Refresh")
            .icon(Icon::RefreshCw)
            .tooltip("R")
            .on_click(cx.listener(|console, _, _, cx| refresh(console, cx)))
            .into_any_element(),
    ];
    if matches!(tab, Tab::Script | Tab::Guide) {
        controls.push(
            Button::new("edr-copy", "Copy all")
                .icon(Icon::Copy)
                .tooltip("C")
                .disabled(if tab == Tab::Script {
                    !script_ready
                } else {
                    !guide_ready
                })
                .on_click(cx.listener(|console, _, _, cx| copy_all(console, cx)))
                .into_any_element(),
        );
    }
    if tab == Tab::Script {
        controls.push(
            Button::new("edr-save", if saving { "Saving…" } else { "Save .ps1…" })
                .primary()
                .icon(Icon::Download)
                .tooltip(if operator {
                    "S"
                } else {
                    "Operators only (admin, demo, integration)"
                })
                .disabled(!operator || saving)
                .on_click(cx.listener(|console, _, _, cx| save_script(console, cx)))
                .into_any_element(),
        );
    }

    let count = rows.len();
    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate,
        },
        count,
        rows_on_screen(window, ROW),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());

    let header = (tab == Tab::Exclusions).then(|| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(SPACE_5)
            .h(px(30.0))
            .px(SPACE_9)
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            .text_size(FONT_CAPTION)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.fg_dim)
            .child(div().w(px(PRIORITY)).flex_none().child("PRIORITY"))
            .child(div().w(px(KIND)).flex_none().child("TYPE"))
            .child(div().w(px(PATH)).flex_none().child("PATH"))
            .child(div().flex_1().child("REASON"))
    });
    let intro = match tab {
        Tab::Exclusions => Some(
            "Add these paths to the EDR/antivirus policy; critical exclusions are needed for acceptable performance.",
        ),
        Tab::Guide => Some(
            "OT network deployment guide for IT teams deploying Voltrus to SCADA environments.",
        ),
        Tab::Script => {
            Some("PowerShell that adds every recommended exclusion to Windows Defender.")
        }
        Tab::Report => None,
    };

    let body = uniform_list(
        "edr-rows",
        count + tail_rows(count, ROW),
        cx.processor(
            move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = vt_ui::theme::theme(cx).clone();
                range
                    .map(|index| match rows.get(index) {
                        Some(row) => draw_row(
                            row,
                            index,
                            focused && cursor == Some(index),
                            cursor == Some(index),
                            &theme,
                        ),
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
        .size_full()
        .child(page_header("EDR compatibility", subtitle, controls, cx))
        .child(
            div()
                .key_context("EngEdr")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &EngEdrNextTab, _, cx| {
                    step_tab(console, true, cx);
                }))
                .on_action(cx.listener(|console, _: &EngEdrPrevTab, _, cx| {
                    step_tab(console, false, cx);
                }))
                .on_action(cx.listener(|console, _: &EngEdrRefresh, _, cx| refresh(console, cx)))
                .on_action(cx.listener(|console, _: &EngEdrCopy, _, cx| copy_all(console, cx)))
                .on_action(cx.listener(|console, _: &EngEdrSave, _, cx| {
                    if console.state::<State>().tab == Tab::Script {
                        save_script(console, cx);
                    }
                }))
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
                        .child(segmented("edr-tabs", segments, cx))
                        .when_some(intro, |this, intro| {
                            this.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(FONT_SMALL)
                                    .text_color(theme.fg_muted)
                                    .child(SharedString::from(intro)),
                            )
                        }),
                )
                .children(header)
                .child(list.flex_1().min_h_0().child(body)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_lists_every_part_the_web_panel_shows() {
        let report = EdrReport {
            compatible: false,
            warnings: vec!["w".into()],
            recommendations: vec!["r1".into(), "r2".into()],
            ..EdrReport::default()
        };
        let rows = report_rows(&report, None);
        // Banner, products heading + empty note, I/O heading + 6, warnings
        // heading + 1, recommendations heading + 2.
        assert_eq!(rows.len(), 1 + 2 + 7 + 2 + 3);
        assert!(matches!(
            &rows[0],
            Row::Banner {
                compatible: false,
                ..
            }
        ));
        assert_eq!(
            rows[0].text().unwrap(),
            "EDR compatibility issues detected — 0 product(s) detected / 1 warning(s)"
        );
        assert!(rows[1].text().is_none(), "headings copy nothing");
        // A fresher I/O status replaces the report's.
        let io = IoStatus {
            wal_mode_active: true,
            ..IoStatus::default()
        };
        let rows = report_rows(&report, Some(&io));
        assert!(rows.iter().any(|row| matches!(
            row,
            Row::Io {
                label: "WAL mode",
                good: Some(true),
                ..
            }
        )));
    }

    #[test]
    fn tabs_cycle_and_texts_load() {
        assert_eq!(Tab::ALL.len(), 4);
        let rows = text_rows(None, "loading");
        assert!(matches!(&rows[0], Row::Note(text, false) if text == "loading"));
        let text = Text::new("a\r\nb".into());
        let rows = text_rows(Some(&Ok(text)), "");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].text().unwrap(), "b");
    }
}
