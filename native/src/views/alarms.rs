//! Active alarm list with per-row + bulk acknowledge.

use gpui::*;
use gpui_component::{
    ActiveTheme, Sizable,
    button::Button,
    h_flex,
    table::{Column, ColumnSort, Table, TableDelegate, TableState},
    v_flex,
};

use crate::api::Alarm;
use crate::runtime::{RuntimeHandle, UiCmd};

pub struct AlarmTableDelegate {
    rows: Vec<Alarm>,
    columns: Vec<Column>,
    handle: RuntimeHandle,
}

impl AlarmTableDelegate {
    fn new(handle: RuntimeHandle) -> Self {
        Self {
            rows: Vec::new(),
            columns: vec![
                Column::new("time", "Time").width(80.),
                Column::new("tag", "Tag").width(200.),
                Column::new("severity", "Severity").width(90.).sortable(),
                Column::new("message", "Message").width(320.),
                Column::new("value", "Value").width(90.),
                Column::new("state", "State").width(100.),
                Column::new("ack", "").width(80.),
            ],
            handle,
        }
    }
}

fn severity_color(severity: &str, cx: &App) -> Hsla {
    match severity {
        "critical" => cx.theme().red,
        "high" | "warning" => cx.theme().yellow,
        _ => cx.theme().blue,
    }
}

fn hhmmss(ts: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(ts)
        .map(|t| {
            t.format(&time::macros::format_description!(
                "[hour]:[minute]:[second]"
            ))
            .unwrap_or_default()
        })
        .unwrap_or_default()
}

impl TableDelegate for AlarmTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(alarm) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };
        match col_ix {
            0 => {
                let mono = cx.theme().mono_font_family.clone();
                div()
                    .text_xs()
                    .font_family(mono)
                    .text_color(cx.theme().muted_foreground)
                    .child(hhmmss(alarm.triggered_at))
                    .into_any_element()
            }
            1 => div()
                .truncate()
                .text_color(cx.theme().foreground)
                .child(alarm.source_name.clone())
                .into_any_element(),
            2 => div()
                .font_weight(FontWeight::BOLD)
                .text_color(severity_color(&alarm.severity, cx))
                .child(alarm.severity.clone())
                .into_any_element(),
            3 => div()
                .text_color(cx.theme().foreground)
                .truncate()
                .child(alarm.message.clone())
                .into_any_element(),
            4 => {
                let mono = cx.theme().mono_font_family.clone();
                div()
                    .text_xs()
                    .font_family(mono)
                    .text_color(cx.theme().foreground)
                    .child(format!("{:.2}", alarm.value))
                    .into_any_element()
            }
            5 => div()
                .text_color(if alarm.acknowledged_at.is_some() {
                    cx.theme().muted_foreground
                } else {
                    cx.theme().red
                })
                .child(alarm.state.clone())
                .into_any_element(),
            6 => {
                if alarm.acknowledged_at.is_some() {
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("acked")
                        .into_any_element()
                } else {
                    let handle = self.handle.clone();
                    let id = alarm.id;
                    Button::new(SharedString::from(format!("ack-{id}")))
                        .small()
                        .label("Ack")
                        .on_click(move |_, _, _| {
                            handle.send(UiCmd::AckAlarm(id));
                        })
                        .into_any_element()
                }
            }
            _ => div().into_any_element(),
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) {
        if col_ix == 2 {
            let desc = matches!(sort, ColumnSort::Descending);
            self.rows.sort_by(|a, b| {
                let rank = |s: &str| match s {
                    "critical" => 0,
                    "high" | "warning" => 1,
                    _ => 2,
                };
                let c = rank(&a.severity).cmp(&rank(&b.severity));
                if desc { c.reverse() } else { c }
            });
        }
    }
}

pub struct AlarmsView {
    table: Entity<TableState<AlarmTableDelegate>>,
    handle: RuntimeHandle,
}

impl AlarmsView {
    pub fn new(handle: RuntimeHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let h = handle.clone();
        let table =
            cx.new(|cx| TableState::new(AlarmTableDelegate::new(h), window, cx).col_movable(false));
        Self { table, handle }
    }

    pub fn set_alarms(&mut self, alarms: Vec<Alarm>, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().rows = alarms;
            cx.notify();
        });
        cx.notify();
    }
}

impl Render for AlarmsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.handle.clone();
        v_flex()
            .size_full()
            .p_2()
            .gap_2()
            .child(
                h_flex().justify_end().child(
                    Button::new("ack-all")
                        .label("Acknowledge All")
                        .on_click(move |_, _, _| {
                            handle.send(UiCmd::AckAll);
                        }),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .child(Table::new(&self.table).bordered(false).stripe(true).small()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Live alarm feed via WebSocket (MessagePack)"),
            )
    }
}
