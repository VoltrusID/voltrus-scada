//! The bottom bar: the link to the server, how fast values arrive, counts.

use gpui::prelude::*;
use gpui::{Context, div};
use voltrus_scada_native::runtime::Link;
use vt_ui::components::status_dot;
use vt_ui::theme::*;

use super::host;
use crate::console::Console;
use crate::console::store::Status;

pub fn render(console: &Console, rate: usize, cx: &mut Context<Console>) -> impl IntoElement {
    let theme = theme(cx).clone();
    let (color, label) = match &console.link {
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
    let attention = console.store.count(Status::Warning)
        + console.store.count(Status::Critical)
        + console.store.count(Status::Offline);
    let alarms = console.unacknowledged();

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_7)
        .h(STATUS_BAR_HEIGHT)
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
        .child(div().font_family(MONO).child(format!("{rate} updates/s")))
        .child(format!("{tags} tags"))
        .when(attention > 0, |this| {
            this.child(
                div()
                    .text_color(theme.warning)
                    .child(format!("{attention} need attention")),
            )
        })
        .child(
            div()
                .text_color(if alarms > 0 {
                    theme.danger
                } else {
                    theme.fg_muted
                })
                .child(match alarms {
                    0 => "No alarms to acknowledge".to_owned(),
                    1 => "1 alarm to acknowledge".to_owned(),
                    count => format!("{count} alarms to acknowledge"),
                }),
        )
        .child(div().flex_1())
        .child(div().text_color(theme.fg_dim).child(theme.name.clone()))
        .child(concat!("Voltrus SCADA console ", env!("CARGO_PKG_VERSION")))
}
