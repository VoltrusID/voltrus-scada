//! The audit log: who did what on the server, searchable and filtered by
//! action, user and time, sorted by time, action or user, a page of 50 at
//! a time, and exported as CSV to a file the operator picks. The log is an
//! operator route, so the page is too. Replies to the data export dialog
//! (`views::export`) come here as well: its calls are scoped to this page.

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Entity, FontWeight, Pixels, SharedString, Subscription, Task,
    Window, div, px, uniform_list,
};
use voltrus_scada_native::api::ops::{
    self, AuditEntry, AuditPage, AuditQuery, AuditSort, Format, Saved,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, empty_state, pill};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use super::ops_kit::{
    filter_bar, labelled, notice, on_field, pager, save_download, saved_message, text_field, value,
};
use super::{
    ListKeys, export, focus_ring, keyboard_list, page_header, rows_on_screen, tail_row, tail_rows,
};
use crate::console::keys::{
    Dismiss, OpsNextPage, OpsPreviousPage, OpsRefresh, OpsSortNext, OpsSortReverse,
};
use crate::console::{Console, Page};

/// The audit log's keyboard list.
pub(crate) const LIST: &str = "audit";
const KEY_CONTEXT: &str = "OpsAudit OpsPaged OpsRefreshable";
const ROW_HEIGHT_PX: f32 = 34.0;
const PANEL: f32 = 360.0;
/// Typing pauses this long before the log is asked again.
const DEBOUNCE: Duration = Duration::from_millis(250);

const TIME: f32 = 132.0;
const ACTION: f32 = 150.0;
const USER: f32 = 110.0;
const RESOURCE: f32 = 180.0;

struct Fields {
    search: Entity<TextInput>,
    action: Entity<TextInput>,
    user: Entity<TextInput>,
    from: Entity<TextInput>,
    to: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(crate) struct State {
    fields: Option<Fields>,
    /// The page, sort and filters last asked for.
    pub(crate) query: AuditQuery,
    page: Option<Rc<AuditPage>>,
    error: Option<String>,
    /// A filter that does not read (a date), shown instead of asking.
    filter_error: Option<&'static str>,
    seq: i64,
    /// The entry open in the side panel.
    open: Option<AuditEntry>,
    exporting: bool,
    debounce: Option<Task<()>>,
}

impl State {
    fn rows(&self) -> &[AuditEntry] {
        match &self.page {
            Some(page) => &page.entries,
            None => &[],
        }
    }

    fn total_pages(&self) -> u32 {
        self.page.as_ref().map_or(1, |page| page.total_pages.max(1))
    }
}

pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().fields.is_none() {
        let fields = Fields {
            search: text_field("Search actions, users, resources, details  /", false, cx),
            action: text_field("e.g. login", false, cx),
            user: text_field("Username", false, cx),
            from: text_field("YYYY-MM-DD HH:MM", true, cx),
            to: text_field("YYYY-MM-DD HH:MM", true, cx),
            _subscriptions: Vec::new(),
        };
        let subscriptions = [
            &fields.search,
            &fields.action,
            &fields.user,
            &fields.from,
            &fields.to,
        ]
        .into_iter()
        .map(|field| on_field(field, field_event, window, cx))
        .collect();
        console.state::<State>().fields = Some(Fields {
            _subscriptions: subscriptions,
            ..fields
        });
    }
    if console.state::<State>().page.is_none() {
        fetch(console, cx);
    }
    console.focus_list(LIST, window, cx);
}

/// `/`: the search field.
pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    console
        .state_ref::<State>()?
        .fields
        .as_ref()
        .map(|fields| fields.search.clone())
}

pub fn side_panel(console: &Console) -> Pixels {
    let open = console
        .state_ref::<State>()
        .is_some_and(|state| state.open.is_some());
    px(if open { PANEL } else { 0.0 })
}

/// Typing asks again after a pause, from page 1; Enter asks at once and
/// goes to the list, Esc just goes to the list.
fn field_event(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Changed => {
            console.state::<State>().query.page = 1;
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DEBOUNCE).await;
                let _ = this.update(cx, |console, cx| {
                    fetch(console, cx);
                    cx.notify();
                });
            });
            console.state::<State>().debounce = Some(task);
        }
        InputEvent::Submitted => {
            console.state::<State>().debounce = None;
            fetch(console, cx);
            console.focus_list(LIST, window, cx);
            cx.notify();
        }
        InputEvent::Cancelled => console.focus_list(LIST, window, cx),
        InputEvent::Blurred => {}
    }
}

/// The query the fields, the sort and the page make; why not, when a date
/// does not read.
fn build_query(console: &mut Console, cx: &gpui::App) -> Result<AuditQuery, &'static str> {
    let offset = console.utc_offset;
    let state = console.state::<State>();
    let mut query = state.query.clone();
    let Some(fields) = &state.fields else {
        return Ok(query);
    };
    let time = |field: &Entity<TextInput>, end: bool, problem: &'static str| {
        let text = value(field, cx);
        if text.is_empty() {
            Ok(None)
        } else {
            ops::parse_local(&text, offset, end)
                .map(Some)
                .ok_or(problem)
        }
    };
    query.search = value(&fields.search, cx);
    query.action = value(&fields.action, cx);
    query.username = value(&fields.user, cx);
    query.from = time(
        &fields.from,
        false,
        "From reads YYYY-MM-DD or YYYY-MM-DD HH:MM",
    )?;
    query.to = time(&fields.to, true, "To reads YYYY-MM-DD or YYYY-MM-DD HH:MM")?;
    Ok(query)
}

fn fetch(console: &mut Console, cx: &gpui::App) {
    let query = build_query(console, cx);
    let state = console.state::<State>();
    match query {
        Ok(query) => {
            state.filter_error = None;
            state.seq += 1;
            let seq = state.seq;
            let path = query.path();
            state.query = query;
            console.call(
                CallKey::new("audit.page").with(seq),
                Call::get::<AuditPage>(path),
            );
        }
        Err(problem) => state.filter_error = Some(problem),
    }
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "audit.page" => {
            let state = console.state::<State>();
            if key.arg != state.seq {
                return;
            }
            match reply.take::<AuditPage>() {
                Ok(page) => {
                    state.page = Some(Rc::new(page));
                    state.error = None;
                }
                Err(error) => state.error = Some(error),
            }
        }
        "audit.export" => {
            console.state::<State>().exporting = false;
            match reply.take::<Saved>() {
                Ok(saved) => console.show_toast(saved_message(&saved, "entries"), false, cx),
                Err(error) => console.show_toast(format!("Audit export failed: {error}"), true, cx),
            }
        }
        "audit.data-export" => export::on_reply(console, reply, cx),
        _ => return,
    }
    cx.notify();
}

/// E, the palette or the button: the log with the filters on screen, as
/// CSV, to a file the operator picks.
pub fn export_csv(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if !console.is_operator() {
        console.show_toast("The audit log needs an operator role", true, cx);
        return;
    }
    if console.page != Page::Audit {
        console.show(Page::Audit, window, cx);
    }
    if console.state::<State>().exporting {
        return;
    }
    let query = match build_query(console, cx) {
        Ok(query) => query,
        Err(problem) => {
            console.state::<State>().filter_error = Some(problem);
            cx.notify();
            return;
        }
    };
    console.state::<State>().exporting = true;
    let today = ops::format_date(
        time::OffsetDateTime::now_utc()
            .to_offset(console.utc_offset)
            .date(),
    );
    save_download(
        CallKey::new("audit.export"),
        query.export_path(),
        format!("audit-log-{today}.csv"),
        Format::Csv,
        |console, cx| {
            console.state::<State>().exporting = false;
            cx.notify();
        },
        cx,
    );
    cx.notify();
}

fn step_page(console: &mut Console, forward: bool, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let (page, last) = (state.query.page, state.total_pages());
    let next = if forward {
        page + 1
    } else {
        page.saturating_sub(1)
    };
    if !(1..=last).contains(&next) || next == page {
        return;
    }
    state.query.page = next;
    fetch(console, cx);
    console.list(LIST, cx).cursor = Some(0);
    cx.notify();
}

fn previous_page(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    step_page(console, false, cx);
}

fn next_page(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    step_page(console, true, cx);
}

/// Sort by `sort`: the same column again reverses; a new one starts
/// newest (time) or A–Z (action, user).
fn sort_by(console: &mut Console, sort: AuditSort, cx: &mut Context<Console>) {
    let query = &mut console.state::<State>().query;
    if query.sort == sort {
        query.descending = !query.descending;
    } else {
        query.sort = sort;
        query.descending = sort == AuditSort::Time;
    }
    query.page = 1;
    fetch(console, cx);
    cx.notify();
}

fn activate_row(
    console: &mut Console,
    row: usize,
    toggle: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let state = console.state::<State>();
    let Some(entry) = state.rows().get(row).cloned() else {
        return;
    };
    state.open = if toggle && state.open.as_ref().is_some_and(|open| open.id == entry.id) {
        None
    } else {
        Some(entry)
    };
    cx.notify();
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let offset = console.utc_offset;
    let state = console.state::<State>();
    let page = state.page.clone();
    let (error, filter_error) = (state.error.clone(), state.filter_error);
    let (sort, descending, page_number) =
        (state.query.sort, state.query.descending, state.query.page);
    let total_pages = state.total_pages();
    let exporting = state.exporting;
    let open = state.open.clone();
    let fields = state.fields.as_ref().map(|fields| {
        (
            fields.search.clone(),
            fields.action.clone(),
            fields.user.clone(),
            fields.from.clone(),
            fields.to.clone(),
        )
    });
    let total = page.as_ref().map_or(0, |page| page.total);
    let shown = page.as_ref().map_or(0, |page| page.entries.len());

    let controls = vec![
        IconButton::new("audit-refresh", Icon::RefreshCw)
            .tooltip("Refresh  R")
            .on_click(cx.listener(|console, _, _, cx| {
                fetch(console, cx);
                cx.notify();
            }))
            .into_any_element(),
        Button::new(
            "audit-export",
            if exporting {
                "Exporting…"
            } else {
                "Export CSV"
            },
        )
        .icon(Icon::Download)
        .disabled(exporting)
        .tooltip("E")
        .on_click(cx.listener(|console, _, window, cx| export_csv(console, window, cx)))
        .into_any_element(),
    ];
    let subtitle = format!(
        "{total} entries · / searches, S sorts, ⇧S reverses, E exports CSV, ↵ opens an entry"
    );

    let bar = fields.map(|(search, action, user, from, to)| {
        filter_bar(&theme)
            .child(
                labelled(
                    "Search",
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
                        .child(div().flex_1().child(search)),
                    &theme,
                )
                .flex_1()
                .min_w(px(220.0)),
            )
            .child(labelled("Action", div().w(px(130.0)).child(action), &theme))
            .child(labelled("User", div().w(px(120.0)).child(user), &theme))
            .child(labelled("From", div().w(px(150.0)).child(from), &theme))
            .child(labelled("To", div().w(px(150.0)).child(to), &theme))
    });

    let arrow = if descending { "↓" } else { "↑" };
    let column =
        |label: &'static str, column: AuditSort, width: Option<f32>, cx: &mut Context<Console>| {
            let active = sort == column;
            div()
                .id(SharedString::from(format!("audit-sort-{label}")))
                .flex_none()
                .when_some(width, |this, width| this.w(px(width)))
                .cursor_pointer()
                .when(active, |this| this.text_color(theme.fg))
                .child(if active {
                    format!("{label} {arrow}")
                } else {
                    label.to_owned()
                })
                .on_click(cx.listener(move |console, _, _, cx| sort_by(console, column, cx)))
        };
    let header = div()
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
        .child(column("TIME", AuditSort::Time, Some(TIME), cx))
        .child(column("ACTION", AuditSort::Action, Some(ACTION), cx))
        .child(column("USER", AuditSort::User, Some(USER), cx))
        .child(div().w(px(RESOURCE)).flex_none().child("RESOURCE"))
        .child(div().flex_1().min_w_0().child("DETAILS"));

    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate_row,
        },
        shown,
        rows_on_screen(window, ROW_HEIGHT_PX),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let cursor = nav.cursor;
    let focused = nav.focus.is_focused(window);
    let scroll = nav.scroll.clone();
    let open_id = open.as_ref().map(|entry| entry.id);
    let body = match (&page, filter_error, error) {
        (_, Some(problem), _) => notice(problem, true, &theme),
        (_, None, Some(error)) => notice(
            format!("Could not load the audit log: {error}"),
            true,
            &theme,
        ),
        (None, None, None) => notice("Loading the audit log…", false, &theme),
        (Some(_), None, None) if shown == 0 => empty_state(
            Icon::ScrollText,
            "No entries match",
            "Clear a filter, or search for less.",
            Vec::new(),
            cx,
        )
        .into_any_element(),
        (Some(page), None, None) => {
            let page = page.clone();
            uniform_list(
                "audit",
                shown + tail_rows(shown, ROW_HEIGHT_PX),
                cx.processor(
                    move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                        let theme = vt_ui::theme::theme(cx).clone();
                        range
                            .map(|row| match page.entries.get(row) {
                                Some(entry) => audit_row(
                                    entry,
                                    row,
                                    (focused && cursor == Some(row), open_id == Some(entry.id)),
                                    offset,
                                    &theme,
                                    cx,
                                ),
                                None => tail_row(ROW_HEIGHT_PX),
                            })
                            .collect::<Vec<_>>()
                    },
                ),
            )
            .track_scroll(scroll)
            .size_full()
            .into_any_element()
        }
    };

    let label = if shown == 0 {
        format!("Page {page_number} of {total_pages}")
    } else {
        let first = page_number.saturating_sub(1) * ops::AUDIT_PAGE + 1;
        format!(
            "Page {page_number} of {total_pages} · entries {first}–{} of {total}",
            first + u32::try_from(shown).unwrap_or(0) - 1
        )
    };
    let table = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(header)
        .child(list.flex_1().min_h_0().child(body))
        .child(pager(
            "audit",
            label,
            (page_number > 1, page_number < total_pages),
            (previous_page, next_page),
            cx,
        ));

    div()
        .flex()
        .flex_col()
        .size_full()
        .key_context(KEY_CONTEXT)
        .on_action(cx.listener(|console, _: &OpsPreviousPage, _, cx| step_page(console, false, cx)))
        .on_action(cx.listener(|console, _: &OpsNextPage, _, cx| step_page(console, true, cx)))
        .on_action(cx.listener(|console, _: &OpsRefresh, _, cx| {
            fetch(console, cx);
            cx.notify();
        }))
        .on_action(cx.listener(|console, _: &OpsSortNext, _, cx| {
            let next = console.state::<State>().query.sort.next();
            sort_by(console, next, cx);
        }))
        .on_action(cx.listener(|console, _: &OpsSortReverse, _, cx| {
            let sort = console.state::<State>().query.sort;
            sort_by(console, sort, cx);
        }))
        // Esc closes the side panel first.
        .on_action(cx.listener(|console, _: &Dismiss, _, cx| {
            let open = console.state::<State>().open.is_some();
            if open && console.overlay.is_none() {
                console.state::<State>().open = None;
                cx.notify();
            } else {
                cx.propagate();
            }
        }))
        .child(page_header("Audit log", subtitle, controls, cx))
        .children(bar)
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(table)
                .when_some(open, |this, entry| this.child(detail(&entry, offset, cx))),
        )
        .into_any_element()
}

fn audit_row(
    entry: &AuditEntry,
    row: usize,
    (ring, open): (bool, bool),
    offset: time::UtcOffset,
    theme: &Theme,
    cx: &Context<Console>,
) -> AnyElement {
    let hover = theme.hover;
    div()
        .id(ElementId::NamedInteger(
            "audit".into(),
            u64::try_from(row).unwrap_or(0),
        ))
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(ROW_HEIGHT_PX))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .cursor_pointer()
        .when(open, |this| this.bg(theme.selected))
        .when(!open, |this| this.hover(move |style| style.bg(hover)))
        .on_click(cx.listener(move |console, _, window, cx| {
            let nav = console.list(LIST, cx);
            nav.select(row);
            window.focus(&nav.focus);
            activate_row(console, row, false, window, cx);
        }))
        .when(ring, |this| this.child(focus_ring(theme)))
        .child(
            div()
                .w(px(TIME))
                .flex_none()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .child(ops::format_local(entry.created_at, offset)),
        )
        .child(
            div()
                .w(px(ACTION))
                .flex_none()
                .overflow_hidden()
                .child(pill(entry.action.clone(), theme.info)),
        )
        .child(
            div()
                .w(px(USER))
                .flex_none()
                .truncate()
                .child(entry.username.clone()),
        )
        .child(
            div()
                .w(px(RESOURCE))
                .flex_none()
                .truncate()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(entry.resource.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(entry.details.clone().unwrap_or_else(|| "—".to_owned())),
        )
        .into_any_element()
}

/// The side panel: one entry in full.
fn detail(entry: &AuditEntry, offset: time::UtcOffset, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let at = time::OffsetDateTime::from_unix_timestamp(entry.created_at)
        .map(|at| at.to_offset(offset))
        .map_or_else(
            |_| "—".to_owned(),
            |at| {
                format!(
                    "{} {:02}:{:02}:{:02}",
                    ops::format_date(at.date()),
                    at.hour(),
                    at.minute(),
                    at.second()
                )
            },
        );
    let fields = [
        ("Time", at),
        ("Action", entry.action.clone()),
        ("User", entry.username.clone()),
        ("Resource", entry.resource.clone()),
        ("Entry", format!("#{}", entry.id)),
    ];
    div()
        .id("audit-detail")
        .flex()
        .flex_col()
        .flex_none()
        .w(px(PANEL))
        .h_full()
        .overflow_y_scroll()
        .border_l_1()
        .border_color(theme.border)
        .bg(theme.chrome)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(48.0))
                .px(SPACE_7)
                .border_b_1()
                .border_color(theme.border)
                .child(pill(entry.action.clone(), theme.info))
                .child(div().flex_1())
                .child(
                    IconButton::new("audit-close", Icon::X)
                        .tooltip("Close  Esc")
                        .on_click(cx.listener(|console, _, _, cx| {
                            console.state::<State>().open = None;
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .p(SPACE_7)
                .children(fields.into_iter().map(|(label, value)| {
                    div()
                        .flex()
                        .gap(SPACE_4)
                        .child(
                            div()
                                .w(px(80.0))
                                .flex_none()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .child(value),
                        )
                })),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .px(SPACE_7)
                .pb(SPACE_9)
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.fg_dim)
                        .child("DETAILS"),
                )
                .child(
                    div()
                        .p(SPACE_5)
                        .rounded(RADIUS_MD)
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(
                            entry
                                .details
                                .clone()
                                .unwrap_or_else(|| "No details.".to_owned()),
                        ),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_count_from_one_and_never_below() {
        let mut state = State::default();
        assert_eq!(state.total_pages(), 1, "no answer yet is one page");
        assert!(state.rows().is_empty());
        state.page = Some(Rc::new(AuditPage {
            entries: vec![AuditEntry::default(); 3],
            total: 103,
            page: 1,
            limit: 50,
            total_pages: 3,
        }));
        assert_eq!((state.total_pages(), state.rows().len()), (3, 3));
        state.page = Some(Rc::new(AuditPage::default()));
        assert_eq!(state.total_pages(), 1, "an empty log is one page");
    }
}
