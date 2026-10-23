//! Operator console: Dashboard / Alarms tabs + connection status bar.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme, TitleBar, h_flex,
    tab::{Tab, TabBar},
    v_flex,
};

use crate::api::{Alarm, DataSourceSummary};
use crate::runtime::{AlarmEvent, RuntimeHandle, UiCmd};
use crate::views::alarms::AlarmsView;
use crate::views::dashboard::Dashboard;
use crate::ws_types::TagUpdateEntry;

pub struct Console {
    pub dashboard: Entity<Dashboard>,
    pub alarms: Entity<AlarmsView>,
    tab: usize, // 0 = dashboard, 1 = alarms
    conn: SharedString,
    user: SharedString,
    alarm_count: usize,
    handle: RuntimeHandle,
}

impl Console {
    pub fn new(handle: RuntimeHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dashboard = cx.new(|cx| Dashboard::new(window, cx));
        let alarms = cx.new(|cx| AlarmsView::new(handle.clone(), window, cx));
        Self {
            dashboard,
            alarms,
            tab: 0,
            conn: "connecting…".into(),
            user: String::new().into(),
            alarm_count: 0,
            handle,
        }
    }

    pub fn set_user(&mut self, user: &str, cx: &mut Context<Self>) {
        self.user = user.to_string().into();
        cx.notify();
    }

    pub fn on_conn(&mut self, state: String, cx: &mut Context<Self>) {
        self.conn = state.into();
        cx.notify();
    }

    pub fn on_sensors(&mut self, sensors: Vec<DataSourceSummary>, cx: &mut Context<Self>) {
        let keys: Vec<String> = sensors.iter().map(|s| s.key.clone()).collect();
        self.dashboard
            .update(cx, |d, cx| d.set_sensors(sensors, cx));
        self.handle.send(UiCmd::Subscribe(keys));
    }

    pub fn on_tags(&mut self, updates: Vec<TagUpdateEntry>, cx: &mut Context<Self>) {
        self.dashboard
            .update(cx, |d, cx| d.apply_updates(&updates, cx));
    }

    pub fn on_alarms(&mut self, alarms: Vec<Alarm>, cx: &mut Context<Self>) {
        self.alarm_count = alarms.len();
        self.alarms.update(cx, |v, cx| v.set_alarms(alarms, cx));
    }

    pub fn on_alarm(&mut self, _ev: AlarmEvent, cx: &mut Context<Self>) {
        // Full list refresh follows the event from the runtime.
        cx.notify();
    }

    fn conn_color(&self, cx: &Context<Self>) -> Hsla {
        match self.conn.as_ref() {
            "live" => cx.theme().green,
            "connected" => cx.theme().blue,
            s if s.starts_with("reconn") || s.starts_with("auth") => cx.theme().yellow,
            _ => cx.theme().muted_foreground,
        }
    }
}

impl Render for Console {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_ix = self.tab;
        let alarm_count = self.alarm_count;
        v_flex()
            .size_full()
            .child(
                TitleBar::new().child(
                    TabBar::new("console-tabs")
                        .mt(px(1.))
                        .segmented()
                        .px_0()
                        .py(px(2.))
                        .selected_index(tab_ix)
                        .on_click(cx.listener(|this, ix: &usize, _window, cx| {
                            this.tab = *ix;
                            cx.notify();
                        }))
                        .child(Tab::new().label("Dashboard"))
                        .child(Tab::new().label(format!("Alarms ({alarm_count})"))),
                ),
            )
            .child(
                div()
                    .id("tab-content")
                    .flex_1()
                    .overflow_hidden()
                    .map(|this| match self.tab {
                        0 => this.child(self.dashboard.clone()),
                        _ => this.child(self.alarms.clone()),
                    }),
            )
            .child(
                h_flex()
                    .px_3()
                    .gap_4()
                    .h_7()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().tab_bar)
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        h_flex().gap_4().child(
                            h_flex()
                                .gap_2()
                                .child(div().size_2().rounded_full().bg(self.conn_color(cx)))
                                .child(self.conn.clone()),
                        ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(self.user.clone())
                            .child("·")
                            .child(div().text_color(cx.theme().green).child("Voltrus Native"))
                            .child("GPU-accelerated (GPUI)"),
                    ),
            )
    }
}
