//! The bottom bar: the link to the server, how fast values arrive, counts.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Window, div};
use voltrus_scada_native::runtime::Link;
use vt_ui::components::status_dot;
use vt_ui::theme::*;

use super::host;
use crate::console::Console;

pub fn render(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let rate = console.rate.per_second(Instant::now());
    let unacknowledged = console.unacknowledged();
    let (color, label) = match &console.link {
        Link::Live if console.stress.is_some() => (theme.info, "Stress test".to_owned()),
        Link::Live => (theme.success, "Live".to_owned()),
        Link::SigningIn => (theme.warning, "Signing in".to_owned()),
        Link::Reconnecting {
            attempt,
            retry_in_secs,
            reason,
        } => (
            theme.warning,
            format!("Reconnecting in {retry_in_secs} s (attempt {attempt}): {reason}"),
        ),
        Link::Failed(reason) => (theme.danger, reason.clone()),
        Link::SignedOut => (theme.fg_dim, "Signed out".to_owned()),
    };
    let tags = console.store.tags.len();
    let attention = console.store.attention_count();
    let offline_devices = console
        .devices
        .values()
        .filter(|device| !device.online)
        .count();

    div()
        .size_full()
        .flex()
        .items_center()
        .gap(SPACE_7)
        .px(SPACE_6)
        .bg(theme.chrome)
        .border_t_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_muted)
        .whitespace_nowrap()
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .min_w_0()
                .child(status_dot(color))
                .child(div().truncate().child(label)),
        )
        .child(
            div()
                .font_family(MONO)
                .text_color(theme.fg_dim)
                .child(host(&console.server).to_owned()),
        )
        .when_some(console.server_version.as_ref(), |this, version| {
            this.child(
                div()
                    .font_family(MONO)
                    .text_color(theme.fg_dim)
                    .child(format!("server {version}")),
            )
        })
        .child(div().font_family(MONO).child(format!("{rate} updates/s")))
        .child(format!("{tags} tags"))
        .when(attention > 0, |this| {
            this.child(
                div()
                    .text_color(theme.warning)
                    .child(format!("{attention} need attention")),
            )
        })
        .when(offline_devices > 0, |this| {
            this.child(
                div()
                    .text_color(theme.warning)
                    .child(match offline_devices {
                        1 => "1 device offline".to_owned(),
                        count => format!("{count} devices offline"),
                    }),
            )
        })
        .when_some(console.alarms_error.as_ref(), |this, error| {
            this.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.warning)
                    .child(format!("Alarm list not updated: {error}")),
            )
        })
        .child(
            div()
                .text_color(if unacknowledged > 0 {
                    theme.danger
                } else {
                    theme.fg_muted
                })
                .child(match unacknowledged {
                    0 => "No alarms to acknowledge".to_owned(),
                    1 => "1 alarm to acknowledge".to_owned(),
                    count => format!("{count} alarms to acknowledge"),
                }),
        )
        .child(div().flex_1())
        .child(div().text_color(theme.fg_dim).child(theme.name.clone()))
        .child(concat!("Voltrus SCADA console ", env!("CARGO_PKG_VERSION")))
        .into_any_element()
}
