//! System › Documentation, after the web client's `DocsPanel.tsx`: the
//! generated documentation for handover and audits in three views (the
//! topology, the tag cross-reference, the scripts and flow rules), each a
//! keyboard list searched with `/`, and the whole of it as one HTML file
//! (`GET /api/v1/docs/export?format=html`, saved where the operator picks).

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, Hsla, SharedString, Window, div, px};
use voltrus_scada_native::api::eng_system::{
    AUTOMATION_PATH, Automation, AutomationRow, DOCS_EXPORT_FILE, DOCS_EXPORT_PATH, TAG_XREF_PATH,
    TOPOLOGY_PATH, TagXref, Topology, script_status,
};
use voltrus_scada_native::api::ops::{Format, Saved};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, kbd, pill, segmented};
use vt_ui::theme::*;

use super::{
    Look, cell, click_row, header_row, heading_row, nothing, query, row_shell, rows, stat,
};
use crate::console::Console;
use crate::console::views::ListKeys;
use crate::console::views::ops_kit::{notice, save_download};

const TOPOLOGY_LIST: &str = "data.system.docs.topology";
const TAGS_LIST: &str = "data.system.docs.tags";
const AUTOMATION_LIST: &str = "data.system.docs.automation";
const ROW: f32 = 32.0;

/// The documentation's views, as the web panel's tabs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum View {
    #[default]
    Topology,
    Tags,
    Automation,
}

impl View {
    const ALL: [Self; 3] = [Self::Topology, Self::Tags, Self::Automation];

    fn label(self) -> &'static str {
        match self {
            Self::Topology => "Topology",
            Self::Tags => "Tag xref",
            Self::Automation => "Automation",
        }
    }

    fn list(self) -> &'static str {
        match self {
            Self::Topology => TOPOLOGY_LIST,
            Self::Tags => TAGS_LIST,
            Self::Automation => AUTOMATION_LIST,
        }
    }
}

#[derive(Default)]
struct State {
    view: View,
    topology: Option<Result<Topology, String>>,
    tags: Option<Result<TagXref, String>>,
    automation: Option<Result<Automation, String>>,
    asked: bool,
    /// The last refresh; older answers are dropped.
    seq: i64,
    exporting: bool,
    export_seq: i64,
}

/// The list the view on screen shows.
pub(super) fn list(console: &Console) -> &'static str {
    console
        .state_ref::<State>()
        .map_or(View::Topology, |state| state.view)
        .list()
}

pub(super) fn subtitle(_: &Console) -> String {
    "Generated documentation for handover, audits and compliance · [ ] view · / search · e export HTML"
        .to_owned()
}

pub(super) fn open(console: &mut Console) {
    if !console.state::<State>().asked {
        refresh(console);
    }
}

/// The three views, as the web panel asks for them together.
pub(super) fn refresh(console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    let state = console.state::<State>();
    state.asked = true;
    state.seq += 1;
    let seq = state.seq;
    console.call(
        CallKey::new("data.system.docs.topology").with(seq),
        Call::get::<Topology>(TOPOLOGY_PATH),
    );
    console.call(
        CallKey::new("data.system.docs.tags").with(seq),
        Call::get::<TagXref>(TAG_XREF_PATH),
    );
    console.call(
        CallKey::new("data.system.docs.automation").with(seq),
        Call::get::<Automation>(AUTOMATION_PATH),
    );
}

pub(super) fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    let list = list(console);
    console.focus_list(list, window, cx);
    true
}

pub(super) fn cycle(
    console: &mut Console,
    forward: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let state = console.state::<State>();
    let at = View::ALL
        .iter()
        .position(|view| *view == state.view)
        .unwrap_or(0);
    let len = View::ALL.len();
    state.view = View::ALL[if forward {
        (at + 1) % len
    } else {
        (at + len - 1) % len
    }];
    focus(console, window, cx);
}

fn set_view(console: &mut Console, view: View, window: &mut Window, cx: &mut Context<Console>) {
    console.state::<State>().view = view;
    focus(console, window, cx);
    cx.notify();
}

/// Save the documentation as HTML (e, the header's button).
pub(super) fn export(console: &mut Console, cx: &mut Context<Console>) {
    if console.stress.is_some() {
        console.show_toast("A stress test has no server to export from", true, cx);
        return;
    }
    let state = console.state::<State>();
    if state.exporting {
        return;
    }
    state.exporting = true;
    state.export_seq += 1;
    let seq = state.export_seq;
    save_download(
        CallKey::new("data.system.docs.export").with(seq),
        DOCS_EXPORT_PATH.to_owned(),
        DOCS_EXPORT_FILE.to_owned(),
        Format::Csv,
        export_cancelled,
        cx,
    );
    cx.notify();
}

/// The operator closed the save panel.
fn export_cancelled(console: &mut Console, cx: &mut Context<Console>) {
    console.state::<State>().exporting = false;
    cx.notify();
}

pub(super) fn export_button(console: &mut Console, cx: &mut Context<Console>) -> AnyElement {
    let exporting = console.state::<State>().exporting;
    Button::new(
        "system-docs-export",
        if exporting {
            "Exporting…"
        } else {
            "Export HTML"
        },
    )
    .icon(Icon::Download)
    .tooltip("E")
    .disabled(exporting)
    .on_click(cx.listener(|console, _, _, cx| export(console, cx)))
    .into_any_element()
}

pub(super) fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    let stale = console.state_ref::<State>().is_some_and(|state| {
        let current = if key.name == "data.system.docs.export" {
            state.export_seq
        } else {
            state.seq
        };
        key.arg != current
    });
    if stale {
        return;
    }
    match key.name {
        "data.system.docs.topology" => {
            console.state::<State>().topology = Some(reply.take::<Topology>());
        }
        "data.system.docs.tags" => {
            console.state::<State>().tags = Some(reply.take::<TagXref>());
        }
        "data.system.docs.automation" => {
            console.state::<State>().automation = Some(reply.take::<Automation>());
        }
        "data.system.docs.export" => {
            console.state::<State>().exporting = false;
            match reply.take::<Saved>() {
                Ok(saved) => {
                    let name = saved.file.file_name().map_or_else(
                        || saved.file.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    );
                    console.show_toast(
                        format!("Saved the system documentation to {name}"),
                        false,
                        cx,
                    );
                }
                Err(error) => console.show_toast(format!("Export failed: {error}"), true, cx),
            }
        }
        _ => {}
    }
}

/// A protocol's dot colour: the chart series in the web panel's order,
/// dim for the simulation and unknown ones.
fn protocol_color(protocol: &str, theme: &Theme) -> Hsla {
    let index = match protocol {
        "Modbus TCP" => 0,
        "OPC-UA" => 1,
        "Siemens S7" => 2,
        "EtherNet/IP" => 3,
        "DNP3" => 4,
        "BACnet/IP" => 5,
        "MQTT" => return theme.warning,
        _ => return theme.fg_dim,
    };
    theme.series[index]
}

fn dot(color: Hsla) -> impl IntoElement {
    div().flex_none().size(px(8.0)).rounded(px(4.0)).bg(color)
}

fn element_id(name: &'static str, row: usize) -> ElementId {
    ElementId::NamedInteger(name.into(), u64::try_from(row).unwrap_or(0))
}

pub(super) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let view = console.state::<State>().view;
    let views = View::ALL
        .into_iter()
        .map(|each| {
            Segment::new(
                each.label(),
                each == view,
                cx.listener(move |console, _, window, cx| set_view(console, each, window, cx)),
            )
        })
        .collect();
    let bar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .px(SPACE_9)
        .py(SPACE_4)
        .child(segmented("system-docs-view", views, cx))
        .child(kbd("[  ]", cx));
    let body = match view {
        View::Topology => topology(console, window, cx),
        View::Tags => tags(console, window, cx),
        View::Automation => automation(console, window, cx),
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(bar)
        .child(div().flex().flex_col().flex_1().min_h_0().child(body))
        .into_any_element()
}

fn loading_or_error<T>(
    answer: Option<&Result<T, String>>,
    what: &str,
    theme: &Theme,
) -> Option<AnyElement> {
    match answer {
        None => Some(notice("Loading…", false, theme)),
        Some(Err(error)) => Some(notice(format!("{what}: {error}"), true, theme)),
        Some(Ok(_)) => None,
    }
}

fn topology(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let answer = console.state::<State>().topology.clone();
    if let Some(element) = loading_or_error(answer.as_ref(), "Topology", &theme) {
        return element;
    }
    let Some(Ok(data)) = answer else {
        return div().into_any_element();
    };
    let search = query(console, cx);
    let devices: Vec<usize> = data
        .devices
        .iter()
        .enumerate()
        .filter(|(_, device)| {
            voltrus_scada_native::api::eng_system::matches_any(
                &search,
                &[
                    device.name.as_str(),
                    device.id.as_str(),
                    device.protocol.as_str(),
                    device.address.as_str(),
                ],
            )
        })
        .map(|(index, _)| index)
        .collect();
    let summary = &data.summary;
    let stats = div()
        .grid()
        .grid_cols(4)
        .gap(SPACE_6)
        .child(stat(
            "Devices",
            summary.total_devices.to_string(),
            theme.accent,
            &theme,
        ))
        .child(stat(
            "Tags",
            summary.total_tags.to_string(),
            theme.accent,
            &theme,
        ))
        .child(stat(
            "Online",
            summary.online_count.to_string(),
            theme.success,
            &theme,
        ))
        .child(stat(
            "Offline",
            summary.offline_count.to_string(),
            theme.danger,
            &theme,
        ));
    let protocols = div()
        .flex()
        .flex_wrap()
        .gap(SPACE_6)
        .children(summary.protocols.iter().map(|protocol| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .text_size(FONT_SMALL)
                .child(dot(protocol_color(&protocol.protocol, &theme)))
                .child(protocol.protocol.clone())
                .child(
                    div()
                        .font_family(MONO)
                        .text_color(theme.fg_muted)
                        .child(format!(
                            "{} devices · {} tags",
                            protocol.device_count, protocol.tag_count
                        )),
                )
        }));
    let len = devices.len();
    let empty = notice(
        if data.devices.is_empty() {
            "No devices yet"
        } else {
            "No device matches the search"
        },
        false,
        &theme,
    );
    let header = header_row(
        &[
            ("DEVICE", None),
            ("PROTOCOL", Some(130.0)),
            ("ADDRESS", Some(170.0)),
            ("TAGS", Some(56.0)),
            ("STATUS", Some(110.0)),
        ],
        &theme,
    );
    let list = rows(
        console,
        window,
        cx,
        ListKeys {
            id: TOPOLOGY_LIST,
            on_move: None,
            on_activate: nothing,
        },
        len,
        ROW,
        empty,
        move |_, row, look: Look, theme, cx| {
            let device = &data.devices[devices[row]];
            row_shell(element_id("system-docs-device", row), row, look, ROW, theme)
                .on_click(click_row(TOPOLOGY_LIST, row, None, cx))
                .child(dot(if device.online {
                    theme.success
                } else {
                    theme.danger
                }))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .gap(SPACE_3)
                        .child(div().truncate().child(device.name.clone()))
                        .child(
                            div()
                                .truncate()
                                .font_family(MONO)
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child(device.id.clone()),
                        ),
                )
                .child(
                    div()
                        .w(px(130.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .child(dot(protocol_color(&device.protocol, theme)))
                        .child(cell(device.protocol.clone(), None, false, theme.fg_muted)),
                )
                .child(cell(
                    if device.address.is_empty() {
                        "—".to_owned()
                    } else {
                        device.address.clone()
                    },
                    Some(170.0),
                    true,
                    theme.fg_muted,
                ))
                .child(cell(
                    device.tag_count.to_string(),
                    Some(56.0),
                    true,
                    theme.fg,
                ))
                .child(
                    div()
                        .w(px(110.0))
                        .flex_none()
                        .flex()
                        .gap(SPACE_2)
                        .child(pill(
                            if device.online { "Online" } else { "Offline" },
                            if device.online {
                                theme.success
                            } else {
                                theme.danger
                            },
                        ))
                        .when(!device.enabled, |this| {
                            this.child(pill("Disabled", theme.fg_dim))
                        }),
                )
                .into_any_element()
        },
    );
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(SPACE_6)
                .px(SPACE_9)
                .pb(SPACE_6)
                .child(stats)
                .child(protocols),
        )
        .child(header)
        .child(list)
        .into_any_element()
}

fn tags(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let answer = console.state::<State>().tags.clone();
    if let Some(element) = loading_or_error(answer.as_ref(), "Tag cross-reference", &theme) {
        return element;
    }
    let Some(Ok(data)) = answer else {
        return div().into_any_element();
    };
    let search = query(console, cx);
    let shown: Vec<usize> = data
        .tags
        .iter()
        .enumerate()
        .filter(|(_, tag)| tag.matches(&search))
        .map(|(index, _)| index)
        .collect();
    let len = shown.len();
    let line = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_3)
        .px(SPACE_9)
        .pb(SPACE_4)
        .text_size(FONT_BODY)
        .text_color(theme.fg_muted)
        .child(
            div()
                .font_family(MONO)
                .text_color(theme.fg)
                .child(if search.is_empty() {
                    data.total_tags.to_string()
                } else {
                    format!("{len} of {}", data.total_tags)
                }),
        )
        .child("tags across all devices")
        .children(data.by_device.iter().take(6).map(|device| {
            div()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(format!(
                    "· {} {} ({} alarmed)",
                    if device.device_name.is_empty() {
                        &device.device_id
                    } else {
                        &device.device_name
                    },
                    device.tag_count,
                    device.alarmed_count
                ))
        }));
    let header = header_row(
        &[
            ("KEY", Some(180.0)),
            ("NAME", None),
            ("DEVICE", Some(140.0)),
            ("PROTOCOL", Some(110.0)),
            ("UNIT", Some(56.0)),
            ("ALARM", Some(80.0)),
            ("WARN", Some(80.0)),
            ("CRIT", Some(80.0)),
        ],
        &theme,
    );
    let empty = notice(
        if data.tags.is_empty() {
            "No tags yet"
        } else {
            "No tag matches the search"
        },
        false,
        &theme,
    );
    let list = rows(
        console,
        window,
        cx,
        ListKeys {
            id: TAGS_LIST,
            on_move: None,
            on_activate: nothing,
        },
        len,
        ROW,
        empty,
        move |_, row, look: Look, theme, cx| {
            let tag = &data.tags[shown[row]];
            let alarm_color = match tag.alarm_severity.as_deref() {
                Some("critical") => theme.danger,
                _ if tag.has_alarm => theme.warning,
                _ => theme.fg_dim,
            };
            row_shell(element_id("system-docs-tag", row), row, look, ROW, theme)
                .on_click(click_row(TAGS_LIST, row, None, cx))
                .child(cell(tag.key.clone(), Some(180.0), true, theme.fg_muted))
                .child(cell(tag.name.clone(), None, false, theme.fg))
                .child(cell(
                    SharedString::from(tag.device_name.clone()),
                    Some(140.0),
                    false,
                    theme.fg_muted,
                ))
                .child(cell(
                    tag.protocol.clone(),
                    Some(110.0),
                    false,
                    theme.fg_muted,
                ))
                .child(cell(
                    if tag.unit.is_empty() {
                        "—".to_owned()
                    } else {
                        tag.unit.clone()
                    },
                    Some(56.0),
                    false,
                    theme.fg_muted,
                ))
                .child(div().w(px(80.0)).flex_none().flex().map(|this| {
                    if tag.has_alarm {
                        this.child(pill(tag.alarm_label().to_owned(), alarm_color))
                    } else {
                        this.child(cell("—", None, false, theme.fg_dim))
                    }
                }))
                .child(cell(
                    format_limit(tag.warn_val),
                    Some(80.0),
                    true,
                    theme.warning,
                ))
                .child(cell(
                    format_limit(tag.crit_val),
                    Some(80.0),
                    true,
                    theme.danger,
                ))
                .into_any_element()
        },
    );
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(line)
        .child(header)
        .child(list)
        .into_any_element()
}

/// A limit as the server stores it, without float noise.
fn format_limit(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        let text = format!("{value:.4}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// A row of the Automation list: a heading, or a flow or script.
#[derive(Clone, Copy)]
enum AutoRow {
    Heading(&'static str),
    Item(AutomationRow),
}

fn automation(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let answer = console.state::<State>().automation.clone();
    if let Some(element) = loading_or_error(answer.as_ref(), "Automation", &theme) {
        return element;
    }
    let Some(Ok(data)) = answer else {
        return div().into_any_element();
    };
    let search = query(console, cx);
    let items = data.rows(&search);
    let mut shown: Vec<AutoRow> = Vec::with_capacity(items.len() + 2);
    let flows: Vec<AutoRow> = items
        .iter()
        .filter(|item| matches!(item, AutomationRow::Flow(_)))
        .map(|item| AutoRow::Item(*item))
        .collect();
    let scripts: Vec<AutoRow> = items
        .iter()
        .filter(|item| matches!(item, AutomationRow::Script(_)))
        .map(|item| AutoRow::Item(*item))
        .collect();
    if !flows.is_empty() {
        shown.push(AutoRow::Heading(
            "Flow rules · name · trigger · actions · status",
        ));
        shown.extend(flows);
    }
    if !scripts.is_empty() {
        shown.push(AutoRow::Heading(
            "Scripts · name · description · timeout · status",
        ));
        shown.extend(scripts);
    }
    let summary = &data.summary;
    let stats = div()
        .grid()
        .grid_cols(3)
        .gap(SPACE_6)
        .px(SPACE_9)
        .pb(SPACE_6)
        .flex_none()
        .child(stat(
            "Scripts",
            summary.total_scripts.to_string(),
            theme.accent,
            &theme,
        ))
        .child(stat(
            "Flow rules",
            summary.total_flows.to_string(),
            theme.accent,
            &theme,
        ))
        .child(stat(
            "Enabled",
            (summary.enabled_scripts + summary.enabled_flows).to_string(),
            theme.success,
            &theme,
        ));
    let len = shown.len();
    let empty = notice(
        if data.flows.is_empty() && data.scripts.is_empty() {
            "No scripts or flow rules yet"
        } else {
            "Nothing matches the search"
        },
        false,
        &theme,
    );
    let list = rows(
        console,
        window,
        cx,
        ListKeys {
            id: AUTOMATION_LIST,
            on_move: None,
            on_activate: nothing,
        },
        len,
        ROW,
        empty,
        move |_, row, look: Look, theme, cx| match shown[row] {
            AutoRow::Heading(text) => heading_row(text.to_owned(), ROW, theme),
            AutoRow::Item(AutomationRow::Flow(index)) => {
                let flow = &data.flows[index];
                row_shell(element_id("system-docs-flow", row), row, look, ROW, theme)
                    .on_click(click_row(AUTOMATION_LIST, row, None, cx))
                    .child(cell(flow.name.clone(), Some(200.0), false, theme.fg))
                    .child(cell(
                        flow.trigger_description.clone(),
                        Some(240.0),
                        false,
                        theme.fg_muted,
                    ))
                    .child(cell(flow.actions(), None, false, theme.fg_muted))
                    .child(div().w(px(56.0)).flex_none().flex().child(pill(
                        if flow.enabled { "ON" } else { "OFF" },
                        if flow.enabled {
                            theme.success
                        } else {
                            theme.danger
                        },
                    )))
                    .into_any_element()
            }
            AutoRow::Item(AutomationRow::Script(index)) => {
                let script = &data.scripts[index];
                let status = script_status(script);
                row_shell(element_id("system-docs-script", row), row, look, ROW, theme)
                    .on_click(click_row(AUTOMATION_LIST, row, None, cx))
                    .child(cell(script.name.clone(), Some(200.0), false, theme.fg))
                    .child(cell(
                        script.description.clone(),
                        None,
                        false,
                        theme.fg_muted,
                    ))
                    .child(cell(
                        format!("{} ms", script.cpu_limit_ms),
                        Some(80.0),
                        true,
                        theme.fg_muted,
                    ))
                    .child(div().w(px(56.0)).flex_none().flex().child(pill(
                        status,
                        match status {
                            "ON" => theme.success,
                            _ => theme.danger,
                        },
                    )))
                    .into_any_element()
            }
        },
    );
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(stats)
        .child(list)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_read_without_float_noise() {
        assert_eq!(format_limit(59900.0), "59900");
        assert_eq!(format_limit(50.15), "50.15");
        assert_eq!(format_limit(0.1 + 0.2), "0.3");
    }

    #[test]
    fn views_cycle_in_the_web_order() {
        assert_eq!(
            View::ALL.map(View::label),
            ["Topology", "Tag xref", "Automation"]
        );
        assert_eq!(View::Tags.list(), TAGS_LIST);
    }
}
