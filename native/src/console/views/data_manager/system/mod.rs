//! Data Manager › System: the web client's `SystemPanel.tsx`, one tab at a
//! time, each in its own module:
//! - System ([`overview`]): host, CPU, memory, disk, database and the
//!   server process, polled every 5 s while on screen.
//! - Documentation ([`docs`]): topology, tag cross-reference and
//!   automation, and the HTML export.
//! - Templates ([`templates`]): the vertical templates, their detail, a
//!   preview and (operator) an instance after a confirmation.
//! - Wizard ([`wizard`]): the quick start and the full project wizard.
//! - Health ([`health`]): the project health check, run again on demand.
//! - Config: the web tab's configuration snapshots are their own section;
//!   this tab points there.
//! - SMTP ([`smtp`], operator): the mail server, and a test email.
//! - Tools ([`tools`]): Voltrus PLC and HMI by deep link, and the docs.
//!
//! Keys: t and ⇧T switch tabs, r refreshes, [ and ] switch a tab's view
//! (documentation view, template industry, wizard mode or template kind,
//! SMTP encryption), e exports the documentation, `/` searches the tab's
//! list, ↵ acts on a row, Esc steps back inside a tab before leaving it.

mod docs;
mod health;
mod overview;
mod smtp;
mod templates;
mod tools;
mod wizard;

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, ElementId, Entity, FocusHandle, FontWeight, Hsla,
    SharedString, Subscription, Window, div, px, relative, uniform_list,
};
use voltrus_scada_native::api::eng_system::{HealthSeverity, Level};
use voltrus_scada_native::runtime::{CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, kbd, segmented};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::{NAV, SECTIONS, choose, current_id, visible};
use crate::console::keys::{
    EngSystemExportDocs, EngSystemPreviousTab, OpsNextFilter, OpsNextTab, OpsPreviousFilter,
    OpsRefresh,
};
use crate::console::nav::Move;
use crate::console::pages::Role;
use crate::console::views::ops_kit::poll_while_shown;
use crate::console::views::{
    ListKeys, card, focus_ring, keyboard_list, page_header, rows_on_screen, tail_row, tail_rows,
};
use crate::console::{Console, Page};

/// The section's own keys (see `keys::COMMANDS`), and the operations
/// pages' tab, refresh and filter keys.
const KEY_CONTEXT: &str = "EngSystem OpsTabbed OpsRefreshable OpsFiltered";
const ID: &str = "system";

/// The System panel's tabs, in the web client's order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Overview,
    Docs,
    Templates,
    Wizard,
    Health,
    Config,
    Smtp,
    Tools,
}

impl Tab {
    const ALL: [Self; 8] = [
        Self::Overview,
        Self::Docs,
        Self::Templates,
        Self::Wizard,
        Self::Health,
        Self::Config,
        Self::Smtp,
        Self::Tools,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "System",
            Self::Docs => "Documentation",
            Self::Templates => "Templates",
            Self::Wizard => "Wizard",
            Self::Health => "Health",
            Self::Config => "Config",
            Self::Smtp => "SMTP",
            Self::Tools => "Tools",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Overview => Icon::Gauge,
            Self::Docs => Icon::ScrollText,
            Self::Templates => Icon::Layers,
            Self::Wizard => Icon::Zap,
            Self::Health => Icon::Activity,
            Self::Config => Icon::History,
            Self::Smtp => Icon::Send,
            Self::Tools => Icon::Plug,
        }
    }

    fn step(self, forward: bool) -> Self {
        let at = Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0);
        let len = Self::ALL.len();
        Self::ALL[if forward {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        }]
    }

    /// The tabs whose list `/` searches.
    fn searchable(self) -> bool {
        matches!(self, Self::Docs | Self::Templates | Self::Health)
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub tab: Tab,
    /// The tab's body, focused where a tab has no list of its own, so
    /// its keys still work.
    body: Option<FocusHandle>,
    filter: Option<Entity<TextInput>>,
    _filter: Option<Subscription>,
}

/// The tab shown.
pub(super) fn shown_tab(console: &Console) -> Tab {
    console
        .state_ref::<State>()
        .map_or(Tab::Overview, |state| state.tab)
}

/// The search text of the tab's list.
pub(super) fn query(console: &Console, cx: &App) -> String {
    console
        .state_ref::<State>()
        .and_then(|state| state.filter.as_ref())
        .map(|field| field.read(cx).value().trim().to_owned())
        .unwrap_or_default()
}

pub fn on_open(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        return;
    }
    open_tab(console);
    poll_while_shown(console, Page::DataManager, overview::EVERY, tick, cx);
}

/// Every 5 s while the Data Manager is on screen: the host figures again,
/// when this section and its System tab are the ones shown.
fn tick(console: &mut Console, _: &mut Context<Console>) {
    if current_id(console) == Some(ID) && shown_tab(console) == Tab::Overview {
        overview::fetch(console);
    }
}

/// Fetch what the tab on screen has not got yet.
fn open_tab(console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    match shown_tab(console) {
        Tab::Overview => overview::open(console),
        Tab::Docs => docs::open(console),
        Tab::Templates => templates::open(console),
        Tab::Wizard => wizard::open(console),
        Tab::Health => health::open(console),
        Tab::Smtp => smtp::open(console),
        Tab::Config | Tab::Tools => {}
    }
}

/// `r`: the tab's data again (the health check runs again).
fn refresh(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        return;
    }
    match shown_tab(console) {
        Tab::Overview => overview::fetch(console),
        Tab::Docs => docs::refresh(console),
        Tab::Templates => templates::refresh(console, cx),
        Tab::Wizard => wizard::refresh(console),
        Tab::Health => health::run(console),
        Tab::Smtp => smtp::fetch(console),
        Tab::Config | Tab::Tools => {}
    }
    cx.notify();
}

pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let focused = match shown_tab(console) {
        Tab::Overview | Tab::Config => false,
        Tab::Docs => docs::focus(console, window, cx),
        Tab::Templates => templates::focus(console, window, cx),
        Tab::Wizard => wizard::focus(console, window, cx),
        Tab::Health => health::focus(console, window, cx),
        Tab::Smtp => smtp::focus(console, window, cx),
        Tab::Tools => tools::focus(console, window, cx),
    };
    if !focused {
        focus_body(console, window, cx);
    }
    cx.notify();
}

/// Focus the tab's body (its keys work, no row is current).
pub(super) fn focus_body(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let handle = console
        .state::<State>()
        .body
        .get_or_insert_with(|| cx.focus_handle())
        .clone();
    window.focus(&handle);
}

pub(super) fn set_tab(
    console: &mut Console,
    tab: Tab,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    console.state::<State>().tab = tab;
    open_tab(console);
    focus(console, window, cx);
}

/// `[` and `]`: the tab's other view.
fn cycle(console: &mut Console, forward: bool, window: &mut Window, cx: &mut Context<Console>) {
    match shown_tab(console) {
        Tab::Docs => docs::cycle(console, forward, window, cx),
        Tab::Templates => templates::cycle(console, forward, cx),
        Tab::Wizard => wizard::cycle(console, window, cx),
        Tab::Smtp => smtp::cycle(console, forward, cx),
        _ => {}
    }
    cx.notify();
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    let state = console.state_ref::<State>()?;
    state.filter.clone().filter(|_| state.tab.searchable())
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let name = key.name;
    if name.starts_with("data.system.overview") {
        overview::on_reply(console, key, reply, cx);
    } else if name.starts_with("data.system.docs.") {
        docs::on_reply(console, key, reply, cx);
    } else if name.starts_with("data.system.templates.") {
        templates::on_reply(console, key, reply, cx);
    } else if name.starts_with("data.system.wizard.") {
        wizard::on_reply(console, key, reply, cx);
    } else if name.starts_with("data.system.health") {
        health::on_reply(console, key, reply, cx);
    } else if name.starts_with("data.system.smtp.") {
        smtp::on_reply(console, key, reply, cx);
    } else if let Some(error) = reply.error() {
        console.show_toast(format!("{name} failed: {error}"), true, cx);
    }
    cx.notify();
}

/// The Config tab's way to the Config snapshots section, as the sidebar
/// would go there.
pub(super) fn open_snapshots(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !console.is_operator() {
        console.show_toast("Config snapshots need the operator role", true, cx);
        return;
    }
    if let Some(row) = visible(SECTIONS, console.role())
        .iter()
        .position(|spec| spec.id == "snapshots")
    {
        console.list(NAV, cx).select(row);
    }
    choose(console, "snapshots", cx);
    super::snapshots::focus(console, window, cx);
}

fn ensure_filter(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().filter.is_some() {
        return;
    }
    let field = cx.new(|cx| TextInput::new(cx).placeholder("Search  /"));
    let subscription = cx.subscribe_in(
        &field,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                if let Some(list) = current_list(console) {
                    console.list(list, cx).cursor = None;
                }
                cx.notify();
            }
            InputEvent::Submitted | InputEvent::Cancelled => {
                match current_list(console) {
                    Some(list) => {
                        let nav = console.list(list, cx);
                        nav.step(Move::First);
                        window.focus(&nav.focus);
                    }
                    None => focus(console, window, cx),
                }
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    );
    let state = console.state::<State>();
    state.filter = Some(field);
    state._filter = Some(subscription);
}

/// The keyboard list the search filters, on the tabs that have one.
fn current_list(console: &Console) -> Option<&'static str> {
    match shown_tab(console) {
        Tab::Docs => Some(docs::list(console)),
        Tab::Templates => templates::list(console),
        Tab::Health => Some(health::LIST),
        _ => None,
    }
}

fn subtitle(console: &Console) -> String {
    match shown_tab(console) {
        Tab::Overview => {
            "Host, storage and the server process · every 5 s while shown · t tab · r refresh"
                .to_owned()
        }
        Tab::Docs => docs::subtitle(console),
        Tab::Templates => templates::subtitle(console),
        Tab::Wizard => wizard::subtitle(console),
        Tab::Health => {
            "Audit the configuration for common issues · ↵ details · r run again · / search"
                .to_owned()
        }
        Tab::Config => "Configuration snapshots have their own section".to_owned(),
        Tab::Smtp => smtp::subtitle(console),
        Tab::Tools => {
            "Voltrus desktop apps with this server as their context, and the docs · ↵ open"
                .to_owned()
        }
    }
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    ensure_filter(console, window, cx);
    let theme = theme(cx).clone();
    let state = console.state::<State>();
    let tab = state.tab;
    let body_focus = state.body.get_or_insert_with(|| cx.focus_handle()).clone();
    let filter = state.filter.clone().filter(|_| tab.searchable());

    let mut controls: Vec<AnyElement> = Vec::new();
    if let Some(field) = filter {
        controls.push(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .w(px(220.0))
                .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
                .child(div().flex_1().child(field))
                .into_any_element(),
        );
    }
    if tab == Tab::Docs {
        controls.push(docs::export_button(console, cx));
    }
    if matches!(
        tab,
        Tab::Overview | Tab::Docs | Tab::Templates | Tab::Smtp | Tab::Wizard
    ) {
        controls.push(
            Button::new("system-refresh", "Refresh")
                .icon(Icon::RefreshCw)
                .tooltip("R")
                .on_click(cx.listener(|console, _, _, cx| refresh(console, cx)))
                .into_any_element(),
        );
    }
    if tab == Tab::Health {
        controls.push(health::run_button(console, cx));
    }

    let tabs = Tab::ALL
        .into_iter()
        .map(|each| {
            Segment::new(
                each.label(),
                each == tab,
                cx.listener(move |console, _, window, cx| set_tab(console, each, window, cx)),
            )
            .icon(each.icon())
        })
        .collect();
    let bar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .px(SPACE_9)
        .py(SPACE_4)
        .border_b_1()
        .border_color(theme.border)
        .child(segmented("system-tab", tabs, cx))
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(kbd("T", cx))
                .child(kbd("⇧T", cx))
                .child("tabs"),
        );

    let body = match tab {
        Tab::Overview => overview::render(console, window, cx),
        Tab::Docs => docs::render(console, window, cx),
        Tab::Templates => templates::render(console, window, cx),
        Tab::Wizard => wizard::render(console, window, cx),
        Tab::Health => health::render(console, window, cx),
        Tab::Config => config(console, cx),
        Tab::Smtp => smtp::render(console, window, cx),
        Tab::Tools => tools::render(console, window, cx),
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("System", subtitle(console), controls, cx))
        .child(
            div()
                .key_context(KEY_CONTEXT)
                .track_focus(&body_focus)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(|console, _: &OpsNextTab, window, cx| {
                    let next = shown_tab(console).step(true);
                    set_tab(console, next, window, cx);
                }))
                .on_action(
                    cx.listener(|console, _: &EngSystemPreviousTab, window, cx| {
                        let previous = shown_tab(console).step(false);
                        set_tab(console, previous, window, cx);
                    }),
                )
                .on_action(cx.listener(|console, _: &OpsRefresh, _, cx| refresh(console, cx)))
                .on_action(cx.listener(|console, _: &OpsNextFilter, window, cx| {
                    cycle(console, true, window, cx);
                }))
                .on_action(cx.listener(|console, _: &OpsPreviousFilter, window, cx| {
                    cycle(console, false, window, cx);
                }))
                .on_action(cx.listener(|console, _: &EngSystemExportDocs, _, cx| {
                    docs::export(console, cx);
                }))
                .child(bar)
                .child(div().flex().flex_col().flex_1().min_h_0().child(body)),
        )
        .into_any_element()
}

/// The web panel's Config tab is the Config snapshots section here.
fn config(console: &mut Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let operator = console.is_operator();
    div()
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_9)
        .child(
            card(&theme)
                .p(SPACE_8)
                .gap(SPACE_5)
                .max_w(px(560.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .child(vt_ui::icon(Icon::History, ICON_LG, theme.accent))
                        .child(
                            div()
                                .text_size(FONT_TITLE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Config snapshots"),
                        ),
                )
                .child(div().text_color(theme.fg_muted).child(
                    "Snapshots of the whole configuration, their differences and a restore \
                     are in Engineering › Config snapshots, a section of its own.",
                ))
                .child(
                    div().flex().child(
                        Button::new("system-open-snapshots", "Open Config snapshots")
                            .primary()
                            .icon(Icon::History)
                            .disabled(!operator)
                            .tooltip(if operator {
                                "Go to the section"
                            } else {
                                "Needs the operator role"
                            })
                            .on_click(cx.listener(|console, _, window, cx| {
                                open_snapshots(console, window, cx);
                            })),
                    ),
                ),
        )
        .into_any_element()
}

// ── Shared drawing ───────────────────────────────────────────────────────

/// Where a list's cursor is, for drawing its rows.
#[derive(Clone, Copy)]
pub(super) struct Look {
    pub cursor: Option<usize>,
    pub focused: bool,
}

impl Look {
    pub fn current(self, row: usize) -> bool {
        self.cursor == Some(row)
    }

    pub fn ring(self, row: usize) -> bool {
        self.focused && self.cursor == Some(row)
    }
}

/// A keyboard list of `len` rows of `row_height`, virtualised: only the
/// rows on screen are drawn (by `row`), with room below the last for the
/// toasts. `empty` stands in for no rows.
#[allow(clippy::too_many_arguments)]
pub(super) fn rows(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
    keys: ListKeys,
    len: usize,
    row_height: f32,
    empty: AnyElement,
    row: impl Fn(&mut Console, usize, Look, &Theme, &mut Context<Console>) -> AnyElement + 'static,
) -> AnyElement {
    let list = keyboard_list(keys, len, rows_on_screen(window, row_height), console, cx);
    let nav = console.list(keys.id, cx);
    let look = Look {
        cursor: nav.cursor,
        focused: nav.focus.is_focused(window),
    };
    let scroll = nav.scroll.clone();
    let body = if len == 0 {
        empty
    } else {
        uniform_list(
            SharedString::from(format!("{}-rows", keys.id)),
            len + tail_rows(len, row_height),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let colors = theme(cx).clone();
                    range
                        .map(|index| {
                            if index < len {
                                row(console, index, look, &colors, cx)
                            } else {
                                tail_row(row_height)
                            }
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .size_full()
        .into_any_element()
    };
    list.flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(body)
        .into_any_element()
}

/// Enter on a list whose rows only show something.
pub(super) fn nothing(
    _: &mut Console,
    _: usize,
    _: bool,
    _: &mut Window,
    _: &mut Context<Console>,
) {
}

/// A list row: cursor background, hover and focus ring.
pub(super) fn row_shell(
    id: ElementId,
    row: usize,
    look: Look,
    height: f32,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    let hover = theme.hover;
    div()
        .id(id)
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(height))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(look.current(row), |this| this.bg(theme.selected))
        .when(!look.current(row), |this| {
            this.hover(move |style| style.bg(hover))
        })
        .when(look.ring(row), |this| this.child(focus_ring(theme)))
}

/// A click on row `row` of list `list`: the cursor goes there and the list
/// takes focus; `activate` (if any) runs as Enter would.
pub(super) fn click_row(
    list: &'static str,
    row: usize,
    activate: Option<crate::console::views::OnActivate>,
    cx: &mut Context<Console>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |console, _: &ClickEvent, window, cx| {
        let nav = console.list(list, cx);
        nav.select(row);
        let focus = nav.focus.clone();
        window.focus(&focus);
        if let Some(activate) = activate {
            activate(console, row, false, window, cx);
        }
        cx.notify();
    })
}

/// A heading between groups of rows in one list.
pub(super) fn heading_row(text: String, height: f32, theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .h(px(height))
        .flex()
        .items_end()
        .px(SPACE_8)
        .pb(SPACE_3)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .border_b_1()
        .border_color(theme.border)
        .child(text.to_uppercase())
        .into_any_element()
}

/// A table's column titles; `None` widths share what is left.
pub(super) fn header_row(cells: &[(&'static str, Option<f32>)], theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(30.0))
        .px(SPACE_8)
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .children(cells.iter().map(|(label, width)| {
            div()
                .truncate()
                .map(|this| match width {
                    Some(width) => this.w(px(*width)).flex_none(),
                    None => this.flex_1().min_w_0(),
                })
                .child(*label)
        }))
}

/// One table cell.
pub(super) fn cell(
    text: impl Into<SharedString>,
    width: Option<f32>,
    mono: bool,
    color: Hsla,
) -> Div {
    div()
        .truncate()
        .text_size(FONT_SMALL)
        .text_color(color)
        .when(mono, |this| this.font_family(MONO))
        .map(|this| match width {
            Some(width) => this.w(px(width)).flex_none(),
            None => this.flex_1().min_w_0(),
        })
        .child(text.into())
}

/// A figure with its caption, in a card.
pub(super) fn stat(label: &'static str, value: String, color: Hsla, theme: &Theme) -> Div {
    card(theme)
        .p(SPACE_7)
        .gap(SPACE_2)
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .whitespace_nowrap()
                .child(value),
        )
        .child(
            div()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child(label.to_uppercase()),
        )
}

/// A usage bar, filled to `fraction` (0–1).
pub(super) fn meter(fraction: f32, color: Hsla, theme: &Theme) -> Div {
    div()
        .h(px(6.0))
        .w_full()
        .rounded(px(3.0))
        .bg(theme.border)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(fraction))
                .rounded(px(3.0))
                .bg(color),
        )
}

pub(super) fn level_color(level: Level, theme: &Theme) -> Hsla {
    match level {
        Level::Ok => theme.success,
        Level::Warning => theme.warning,
        Level::Critical => theme.danger,
    }
}

pub(super) fn severity_color(severity: HealthSeverity, theme: &Theme) -> Hsla {
    match severity {
        HealthSeverity::Critical => theme.danger,
        HealthSeverity::Warning => theme.warning,
        HealthSeverity::Info => theme.info,
        HealthSeverity::Pass => theme.success,
    }
}

/// The line under a form: why it cannot go yet, what the server said, or
/// the keys.
pub(super) fn form_note(
    note: Option<Result<String, String>>,
    hints: &[(&'static str, &'static str)],
    cx: &Context<Console>,
) -> AnyElement {
    let theme = theme(cx);
    match note {
        Some(Err(error)) => div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .min_w_0()
            .text_size(FONT_SMALL)
            .text_color(theme.danger)
            .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.danger))
            .child(div().min_w_0().child(error))
            .into_any_element(),
        Some(Ok(message)) => div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .min_w_0()
            .text_size(FONT_SMALL)
            .text_color(theme.success)
            .child(vt_ui::icon(Icon::CircleCheck, ICON_SM, theme.success))
            .child(div().min_w_0().child(message))
            .into_any_element(),
        None => div()
            .flex()
            .items_center()
            .gap(SPACE_2)
            .text_size(FONT_CAPTION)
            .text_color(theme.fg_dim)
            .children(hints.iter().flat_map(|(keys, what)| {
                [
                    kbd(*keys, cx).into_any_element(),
                    div().child(*what).into_any_element(),
                ]
            }))
            .into_any_element(),
    }
}

/// Why a control is off for this user.
pub(super) fn needs(role: Role) -> &'static str {
    match role {
        Role::Admin => "Needs the admin role",
        _ => "Needs the operator role",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_cycle_both_ways_in_the_web_order() {
        assert_eq!(Tab::Overview.step(true), Tab::Docs);
        assert_eq!(Tab::Tools.step(true), Tab::Overview);
        assert_eq!(Tab::Overview.step(false), Tab::Tools);
        assert_eq!(Tab::Smtp.step(false), Tab::Config);
        let labels: Vec<&str> = Tab::ALL.iter().map(|tab| tab.label()).collect();
        assert_eq!(
            labels,
            [
                "System",
                "Documentation",
                "Templates",
                "Wizard",
                "Health",
                "Config",
                "SMTP",
                "Tools"
            ]
        );
        assert!(Tab::Docs.searchable() && Tab::Health.searchable());
        assert!(!Tab::Smtp.searchable());
    }
}
