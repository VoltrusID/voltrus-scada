//! Local station mode's screen, shown instead of the sign-in card while the
//! console connects to the station's server, and the sign-in card's way
//! back to the station or to setting one up (`console::station`).

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, div, px};
use voltrus_scada_native::station::{Service, Step};
use vt_ui::Icon;
use vt_ui::components::{Button, brand_mark};
use vt_ui::theme::*;

use super::window_controls;
use crate::console::Console;
use crate::console::station::{self, Local, Setup, Status};

pub fn render(console: &mut Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(local) = console.station.local.as_ref() else {
        return div().into_any_element();
    };
    let dir = local.dir.display().to_string();
    let (title, reason) = match &local.status {
        Status::Connecting | Status::Live => ("Connecting to this station…", None),
        Status::SigningIn => ("Signing in to this station…", None),
        Status::Unreachable(reason) => ("This station is not answering", Some(reason.clone())),
        Status::Refused(reason) => ("This station refused to sign in", Some(reason.clone())),
    };
    let idle = local.idle();
    let signing_in = local.status == Status::SigningIn;
    let fix = fix_row(local, &theme, cx);

    let card = div()
        .id("station")
        .w(px(440.0))
        .flex()
        .flex_col()
        .gap(SPACE_8)
        .p(SPACE_10)
        .rounded(RADIUS_XL)
        .border_1()
        .border_color(theme.border)
        .bg(theme.elevated)
        .shadow(theme.shadow(Elevation::Modal))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .child(
                    div()
                        .text_size(FONT_DISPLAY)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(FONT_BODY)
                        .text_color(theme.fg_muted)
                        .child("The Voltrus SCADA server on this computer runs as a service; the console signs in to it by itself."),
                ),
        )
        .when_some(reason, |this, reason| {
            this.child(
                div()
                    .flex()
                    .items_start()
                    .gap(SPACE_4)
                    .px(SPACE_6)
                    .py(SPACE_5)
                    .rounded(RADIUS_MD)
                    .bg(theme.danger.alpha(0.1))
                    .border_1()
                    .border_color(theme.danger.alpha(0.3))
                    .text_size(FONT_SMALL)
                    .text_color(theme.danger)
                    .child(vt_ui::icon(Icon::CircleAlert, ICON_MD, theme.danger))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .debug_selector(|| "station-reason".into())
                            .child(reason),
                    ),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child("Station folder")
                .child(
                    div()
                        .font_family(MONO)
                        .text_color(theme.fg_muted)
                        .debug_selector(|| "station-dir".into())
                        .child(dir),
                ),
        )
        .children(fix)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_4)
                .child(
                    div().debug_selector(|| "station-retry".into()).child(
                        Button::new("station-retry", if idle { "Retry" } else { "Connecting…" })
                            .primary()
                            .disabled(!idle)
                            .on_click(cx.listener(|console, _, _, cx| station::retry(console, cx))),
                    ),
                )
                .child(
                    Button::new("station-leave", "Sign in to another server")
                        .ghost()
                        .disabled(signing_in)
                        .on_click(cx.listener(|console, _, window, cx| {
                            station::leave(console, cx);
                            console
                                .inputs
                                .username
                                .update(cx, |field, _| field.focus(window));
                        })),
                ),
        );

    div()
        .id("station-screen")
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(SPACE_8)
        .when(window_controls::SHOWN, |this| this.pt(TITLE_BAR_HEIGHT))
        .bg(theme.chrome)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(SPACE_5)
                .child(brand_mark(px(64.0)))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Voltrus SCADA"),
                ),
        )
        .child(card)
        .child(
            div()
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(concat!("Voltrus SCADA console ", env!("CARGO_PKG_VERSION"))),
        )
        .into_any_element()
}

/// The sign-in card's station row: back to the station, or setting this
/// computer up as one (with its progress or why it failed); nothing when
/// neither applies.
pub fn card_row(console: &mut Console, cx: &mut Context<Console>) -> Option<AnyElement> {
    let theme = theme(cx).clone();
    if console.station.offers_return() {
        return Some(
            div()
                .debug_selector(|| "station-return".into())
                .child(
                    Button::new("station-return", "Use this station")
                        .icon(Icon::Server)
                        .on_click(cx.listener(|console, _, _, cx| {
                            station::open(console, cx);
                        })),
                )
                .into_any_element(),
        );
    }
    if !console.station.offers_setup() {
        return None;
    }
    let running = match &console.station.setup {
        Some(Setup::Running(step)) => Some(step.label()),
        _ => None,
    };
    let failed = match &console.station.setup {
        Some(Setup::Failed(error)) => Some(error.clone()),
        _ => None,
    };
    Some(
        div()
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .debug_selector(|| "station-setup".into())
            .child(
                Button::new(
                    "station-setup",
                    running.unwrap_or("Set up this computer as a station"),
                )
                .icon(Icon::Server)
                .disabled(running.is_some())
                .on_click(cx.listener(|console, _, _, cx| station::set_up(console, cx))),
            )
            .when_some(failed, |this, error| {
                this.child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.danger)
                        .debug_selector(|| "station-setup-error".into())
                        .child(error),
                )
            })
            .into_any_element(),
    )
}

/// The station screen's way past a station that does not answer because
/// its service is not installed (a setup that stopped after `station
/// init`) or stopped, as the bundled server reported it; with the
/// progress, or why the bundled server could not tell or do it.
fn fix_row(local: &Local, theme: &Theme, cx: &mut Context<Console>) -> Option<AnyElement> {
    let (label, steps): (&'static str, &'static [Step]) = match local.service.as_ref()? {
        Ok(Service::Running) => return None,
        Ok(Service::NotInstalled) => ("Finish setting up this station", Step::FINISH),
        Ok(Service::Stopped) => ("Start the station server", Step::START),
        Err(error) => {
            return Some(
                div()
                    .text_size(FONT_CAPTION)
                    .text_color(theme.danger)
                    .debug_selector(|| "station-fix-error".into())
                    .child(error.clone())
                    .into_any_element(),
            );
        }
    };
    let running = match &local.fix {
        Some(Setup::Running(step)) => Some(step.label()),
        _ => None,
    };
    let failed = match &local.fix {
        Some(Setup::Failed(error)) => Some(error.clone()),
        _ => None,
    };
    Some(
        div()
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .debug_selector(move || format!("station-fix: {label}"))
            .child(
                Button::new("station-fix", running.unwrap_or(label))
                    .icon(Icon::Server)
                    .disabled(running.is_some())
                    .on_click(cx.listener(move |console, _, _, cx| {
                        station::repair(console, steps, cx);
                    })),
            )
            .when_some(failed, |this, error| {
                this.child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(theme.danger)
                        .debug_selector(|| "station-fix-error".into())
                        .child(error),
                )
            })
            .into_any_element(),
    )
}
