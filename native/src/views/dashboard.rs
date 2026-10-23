//! Live tag dashboard: sortable table of live values + trend chart of the
//! selected tag (300-point ring, mirroring the web client's LIVE_PTS).

use std::collections::HashMap;
use std::collections::VecDeque;

use gpui::*;
use gpui_component::{
    ActiveTheme, Sizable,
    chart::AreaChart,
    h_flex,
    table::{Column, ColumnSort, Table, TableDelegate, TableEvent, TableState},
    v_flex,
};

use crate::api::DataSourceSummary;
use crate::ws_types::TagUpdateEntry;

const LIVE_PTS: usize = 300;

#[derive(Clone)]
pub struct ChartPoint {
    pub t: String,
    pub v: f64,
}

#[derive(Clone)]
pub struct SensorLive {
    pub meta: DataSourceSummary,
    pub value: f64,
    pub quality: String,
    pub ts: i64,
    pub ring: VecDeque<ChartPoint>,
}

impl SensorLive {
    fn new(meta: DataSourceSummary) -> Self {
        Self {
            meta,
            value: f64::NAN,
            quality: "—".into(),
            ts: 0,
            ring: VecDeque::with_capacity(LIVE_PTS),
        }
    }
}

// ── Table delegate ─────────────────────────────────────────────────────────

pub struct TagTableDelegate {
    rows: Vec<SensorLive>,
    columns: Vec<Column>,
    sort_field: SortField,
    sort_order: ColumnSort,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SortField {
    #[default]
    Name,
    Value,
    Quality,
}

impl TagTableDelegate {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            columns: vec![
                Column::new("name", "Tag").width(240.),
                Column::new("value", "Value").width(110.).sortable(),
                Column::new("unit", "Unit").width(70.),
                Column::new("quality", "Quality").width(90.).sortable(),
                Column::new("updated", "Updated").width(90.),
            ],
            sort_field: SortField::default(),
            sort_order: ColumnSort::Ascending,
        }
    }

    fn sort_rows(&mut self) {
        let desc = matches!(self.sort_order, ColumnSort::Descending);
        match self.sort_field {
            SortField::Name => self.rows.sort_by(|a, b| {
                let c = a.meta.name.to_lowercase().cmp(&b.meta.name.to_lowercase());
                if desc { c.reverse() } else { c }
            }),
            SortField::Value => self.rows.sort_by(|a, b| {
                a.value
                    .partial_cmp(&b.value)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortField::Quality => self.rows.sort_by(|a, b| a.quality.cmp(&b.quality)),
        }
    }
}

impl TableDelegate for TagTableDelegate {
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
        let Some(row) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };
        let decimals = row.meta.decimals.clamp(0, 6) as usize;

        match col_ix {
            0 => h_flex()
                .gap_1()
                .child(
                    div()
                        .truncate()
                        .text_color(cx.theme().foreground)
                        .child(row.meta.name.clone()),
                )
                .into_any_element(),
            1 => {
                let color = value_color(
                    row,
                    cx.theme().red,
                    cx.theme().yellow,
                    cx.theme().foreground,
                );
                let mono = cx.theme().mono_font_family.clone();
                h_flex()
                    .justify_between()
                    .child(div().text_xs().font_family(mono).text_color(color).child(
                        if !row.value.is_finite() {
                            "—".to_string()
                        } else {
                            format!("{:.*}", decimals, row.value)
                        },
                    ))
                    .into_any_element()
            }
            2 => div()
                .text_color(cx.theme().muted_foreground)
                .child(row.meta.unit.clone())
                .into_any_element(),
            3 => {
                let ok = row.quality == "good" || row.quality == "normal";
                let mono = cx.theme().mono_font_family.clone();
                div()
                    .text_xs()
                    .font_family(mono)
                    .text_color(if ok { cx.theme().green } else { cx.theme().red })
                    .child(row.quality.clone())
                    .into_any_element()
            }
            4 => {
                let mono = cx.theme().mono_font_family.clone();
                div()
                    .text_xs()
                    .font_family(mono)
                    .text_color(cx.theme().muted_foreground)
                    .child(age_label(row.ts))
                    .into_any_element()
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
        self.sort_order = sort;
        self.sort_field = match col_ix {
            1 => SortField::Value,
            3 => SortField::Quality,
            _ => SortField::Name,
        };
        self.sort_rows();
    }
}

fn value_color(row: &SensorLive, red: Hsla, yellow: Hsla, normal: Hsla) -> Hsla {
    if !row.value.is_finite() {
        return normal;
    }
    if row.meta.crit_val != 0.0 && row.value >= row.meta.crit_val {
        red
    } else if row.meta.warn_val != 0.0 && row.value >= row.meta.warn_val {
        yellow
    } else {
        normal
    }
}

fn age_label(ts: i64) -> String {
    if ts == 0 {
        return "—".into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let age = now.saturating_sub(ts.max(1_000_000_000)); // server timestamps are seconds
    if age < 0 {
        "now".into()
    } else if age < 60 {
        format!("{age}s ago")
    } else {
        format!("{}m ago", age / 60)
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

// ── View ───────────────────────────────────────────────────────────────────

pub struct Dashboard {
    rows: Vec<SensorLive>,
    index: HashMap<String, usize>,
    table: Entity<TableState<TagTableDelegate>>,
    chart_key: Option<String>,
    seq: u64,
    msg_counter: u64,
    msg_rate: u64,
}

impl Dashboard {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(TagTableDelegate::new(), window, cx)
                .row_selectable(true)
                .col_selectable(false)
                .col_movable(false)
        });

        // Row click → chart that tag.
        cx.subscribe_in(&table, window, |this, _table, event, _window, cx| {
            if let TableEvent::SelectRow(row_ix) = event {
                let key = this.rows.get(*row_ix).map(|r| r.meta.key.clone());
                if let Some(k) = key {
                    this.chart_key = Some(k);
                    cx.notify();
                }
            }
        })
        .detach();

        // Update-rate meter (updates/s) — the "blazing fast" proof.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                smol::Timer::after(std::time::Duration::from_secs(1)).await;
                let Ok(()) = this.update(cx, |this, cx| {
                    this.msg_rate = this.msg_counter;
                    this.msg_counter = 0;
                    cx.notify();
                }) else {
                    break;
                };
            }
        })
        .detach();

        Self {
            rows: Vec::new(),
            index: HashMap::new(),
            table,
            chart_key: None,
            seq: 0,
            msg_counter: 0,
            msg_rate: 0,
        }
    }

    /// Replace sensor metadata (login / config change) — resets live state.
    pub fn set_sensors(&mut self, sensors: Vec<DataSourceSummary>, cx: &mut Context<Self>) {
        self.rows = sensors.into_iter().map(SensorLive::new).collect();
        self.index = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.meta.key.clone(), i))
            .collect();
        self.chart_key = self.rows.first().map(|r| r.meta.key.clone());
        self.sync_table(cx);
        cx.notify();
    }

    /// Apply a batch of live updates.
    pub fn apply_updates(&mut self, updates: &[TagUpdateEntry], cx: &mut Context<Self>) {
        for u in updates {
            let Some(&i) = self.index.get(&u.tag_id) else {
                continue;
            };
            let row = &mut self.rows[i];
            row.value = u.value;
            row.quality = u.quality.clone();
            row.ts = u.ts;
            self.seq += 1;
            let label = hhmmss(u.ts);
            // Non-finite floats (bad register reads) must never reach the
            // chart — lyon's path builder asserts on NaN coordinates.
            if u.value.is_finite() {
                row.ring.push_back(ChartPoint {
                    t: label,
                    v: u.value,
                });
                if row.ring.len() > LIVE_PTS {
                    row.ring.pop_front();
                }
            }
            self.msg_counter += 1;
        }
        self.sync_table(cx);
        cx.notify();
    }

    fn sync_table(&mut self, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().rows = self.rows.clone();
            cx.notify();
        });
    }

    fn render_chart(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(key) = &self.chart_key else {
            return v_flex()
                .flex_1()
                .min_h(px(180.))
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child("Select a tag row to trend it"),
                )
                .into_any_element();
        };
        let Some(&i) = self.index.get(key) else {
            return div().into_any_element();
        };
        let row = &self.rows[i];
        let color = cx.theme().green;
        let unit = row.meta.unit.clone();
        let name = row.meta.name.clone();
        let decimals = row.meta.decimals.clamp(0, 6) as usize;
        let data: Vec<ChartPoint> = row
            .ring
            .iter()
            .filter(|p| p.v.is_finite())
            .cloned()
            .collect();
        let current = row.value;
        let mono = cx.theme().mono_font_family.clone();

        v_flex()
            .flex_1()
            .min_h(px(180.))
            .gap_1()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .p_2()
            .child(
                h_flex()
                    .justify_between()
                    .px_1()
                    .child(div().text_color(cx.theme().foreground).child(name))
                    .child(div().text_xs().font_family(mono).text_color(color).child(
                        if !current.is_finite() {
                            format!("— {unit}")
                        } else {
                            format!("{:.*} {unit}", decimals, current)
                        },
                    )),
            )
            .child(
                AreaChart::new(data)
                    .x(|d| d.t.clone())
                    .y(|d| d.v)
                    .stroke(color)
                    .tick_margin(12),
            )
            .into_any_element()
    }
}

impl Render for Dashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_2()
            .gap_2()
            .child(self.render_chart(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_h(px(160.))
                    .child(Table::new(&self.table).bordered(false).stripe(true).small()),
            )
            .child(
                h_flex()
                    .gap_4()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} tags", self.rows.len()))
                    .child(format!("{} updates/s", self.msg_rate))
                    .child(format!("trend window: {LIVE_PTS} pts")),
            )
    }
}
