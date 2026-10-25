//! What the operations pages (Commands, Shift handover, Audit log and the
//! data export dialog) share: fetching on a timer only while a page is on
//! screen, a pager, labelled filter fields, the command status colours,
//! and saving a download where the operator picks.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, Context, Entity, FontWeight, Hsla, SharedString, Subscription,
    Window, div, px,
};
use voltrus_scada_native::api::ops::{Format, Saved};
use voltrus_scada_native::runtime::{Call, CallKey};
use vt_ui::Icon;
use vt_ui::components::{Button, kbd};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use crate::console::{Console, Page, Phase, pages};

/// The pages whose poll runs now.
#[derive(Default)]
struct Polls(HashSet<&'static str>);

/// Run `tick` every `every` while `page` is on screen in a signed-in
/// console (not in a stress test), from the page's `on_show`. The loop ends
/// on its own when the page goes off screen; the next show starts it again.
/// It fetches; it never redraws by itself.
pub(crate) fn poll_while_shown(
    console: &mut Console,
    page: Page,
    every: Duration,
    tick: fn(&mut Console, &mut Context<Console>),
    cx: &mut Context<Console>,
) {
    let id = pages::spec(page).id;
    if !console.state::<Polls>().0.insert(id) {
        return;
    }
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(every).await;
            let shown = this.update(cx, |console, cx| {
                let shown = console.page == page
                    && console.phase == Phase::Console
                    && console.stress.is_none();
                if shown {
                    tick(console, cx);
                } else {
                    console.state::<Polls>().0.remove(id);
                }
                shown
            });
            if !matches!(shown, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

/// A text field for a filter or a form.
pub(crate) fn text_field(
    placeholder: &'static str,
    mono: bool,
    cx: &mut Context<Console>,
) -> Entity<TextInput> {
    cx.new(|cx| {
        let field = TextInput::new(cx).placeholder(placeholder);
        if mono { field.mono() } else { field }
    })
}

/// `on_event` for every event of `field`.
pub(crate) fn on_field(
    field: &Entity<TextInput>,
    on_event: fn(&mut Console, &InputEvent, &mut Window, &mut Context<Console>),
    window: &mut Window,
    cx: &mut Context<Console>,
) -> Subscription {
    cx.subscribe_in(field, window, move |console, _, event, window, cx| {
        on_event(console, event, window, cx);
    })
}

/// A field's text, trimmed.
pub(crate) fn value(field: &Entity<TextInput>, cx: &gpui::App) -> String {
    field.read(cx).value().trim().to_owned()
}

/// A small label over a control, for filter bars and forms.
pub(crate) fn labelled(label: &'static str, control: impl IntoElement, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_2)
        .min_w_0()
        .child(
            div()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child(label.to_uppercase()),
        )
        .child(control)
}

/// A bar of filters below a page's header.
pub(crate) fn filter_bar(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_end()
        .flex_wrap()
        .gap(SPACE_5)
        .px(SPACE_9)
        .py(SPACE_5)
        .border_b_1()
        .border_color(theme.border)
}

type Step = fn(&mut Console, &mut Window, &mut Context<Console>);

/// The bar under a paged list: where it is, and Previous/Next (also ← and
/// →).
pub(crate) fn pager(
    id: &'static str,
    label: String,
    (previous, next): (bool, bool),
    (on_previous, on_next): (Step, Step),
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(40.0))
        .px(SPACE_9)
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(label),
        )
        .child(kbd("←  →", cx))
        .child(
            Button::new(SharedString::from(format!("{id}-previous")), "Previous")
                .compact()
                .icon(Icon::ChevronLeft)
                .disabled(!previous)
                .on_click(
                    cx.listener(move |console, _, window, cx| on_previous(console, window, cx)),
                ),
        )
        .child(
            Button::new(SharedString::from(format!("{id}-next")), "Next")
                .compact()
                .icon(Icon::ChevronRight)
                .disabled(!next)
                .on_click(cx.listener(move |console, _, window, cx| on_next(console, window, cx))),
        )
        .into_any_element()
}

/// The colour of a command's status: amber pending, blue queued, accent
/// sent, green acknowledged, red failed, dim when expired or cancelled.
pub(crate) fn command_status_color(status: &str, theme: &Theme) -> Hsla {
    match status {
        "pending" => theme.warning,
        "queued" => theme.info,
        "sent" => theme.accent,
        "acked" => theme.success,
        "failed" => theme.danger,
        _ => theme.fg_dim,
    }
}

/// Ask where to save a download (`suggested` as the name, in Downloads),
/// then fetch `path` into it off the UI thread. The answer comes back to
/// the page of `key` as a [`Saved`]; `cancelled` runs if the operator
/// closes the save panel instead.
pub(crate) fn save_download(
    key: CallKey,
    path: String,
    suggested: String,
    format: Format,
    cancelled: fn(&mut Console, &mut Context<Console>),
    cx: &mut Context<Console>,
) {
    let directory = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    let prompt = cx.prompt_for_new_path(&directory, Some(&suggested));
    cx.spawn(async move |this, cx| {
        let chosen = prompt.await;
        let _ = this.update(cx, |console, cx| match chosen {
            Ok(Ok(Some(file))) => console.call(
                key,
                Call::new(move |api| async move { api.download(&path, &file, format).await }),
            ),
            Ok(Err(error)) => {
                cancelled(console, cx);
                console.show_toast(format!("No save panel: {error}"), true, cx);
            }
            _ => cancelled(console, cx),
        });
    })
    .detach();
}

/// Close the overlay `id` in `window`, if it is still the one open, from
/// code that has no window at hand (a REST answer), focus going back where
/// it was.
pub(crate) fn close_overlay_in(
    window: AnyWindowHandle,
    id: &'static str,
    cx: &mut Context<Console>,
) {
    cx.spawn(async move |this, cx| {
        let _ = window.update(cx, |_, window, cx| {
            let _ = this.update(cx, |console, cx| {
                if console.overlay_is(id) {
                    console.close_overlay(window, cx);
                }
            });
        });
    })
    .detach();
}

/// `Saved 1 000 rows of 52 340 to telemetry.json`.
pub(crate) fn saved_message(saved: &Saved, rows: &str) -> String {
    let name = saved.file.file_name().map_or_else(
        || saved.file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    match saved.total {
        Some(total) if usize::try_from(total).is_ok_and(|total| total > saved.rows) => {
            format!("Saved {} {rows} of {} to {name}", saved.rows, total)
        }
        _ => format!("Saved {} {rows} to {name}", saved.rows),
    }
}

/// A one-line message where a list would be: loading, or why it failed.
pub(crate) fn notice(text: impl Into<SharedString>, error: bool, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(SPACE_3)
        .size_full()
        .p(SPACE_10)
        .text_size(FONT_BODY)
        .text_color(if error { theme.danger } else { theme.fg_muted })
        .child(vt_ui::icon(
            if error {
                Icon::CircleAlert
            } else {
                Icon::Clock
            },
            ICON_MD,
            if error { theme.danger } else { theme.fg_dim },
        ))
        .child(text.into())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_messages_say_what_the_server_held_back() {
        let saved = Saved {
            file: PathBuf::from("/tmp/telemetry.json"),
            rows: 1000,
            total: Some(52_340),
        };
        assert_eq!(
            saved_message(&saved, "rows"),
            "Saved 1000 rows of 52340 to telemetry.json"
        );
        let all = Saved {
            total: Some(1000),
            ..saved.clone()
        };
        assert_eq!(
            saved_message(&all, "rows"),
            "Saved 1000 rows to telemetry.json"
        );
        let csv = Saved {
            total: None,
            rows: 3,
            ..saved
        };
        assert_eq!(
            saved_message(&csv, "entries"),
            "Saved 3 entries to telemetry.json"
        );
    }

    #[test]
    fn operator_pages_stay_out_of_a_viewers_reach() {
        use crate::console::overlay::{entries, more_pages};
        use crate::console::pages::{Role, spec};
        // The server serves commands to anyone, the handover log and the
        // audit log to operators only (`demo_protected`).
        assert_eq!(spec(Page::Commands).role, Role::Any);
        assert_eq!(spec(Page::Shift).role, Role::Operator);
        assert_eq!(spec(Page::Audit).role, Role::Operator);
        let viewer = more_pages(Some(Role::Any));
        assert!(viewer.contains(&Page::Commands));
        assert!(!viewer.contains(&Page::Shift) && !viewer.contains(&Page::Audit));
        let operator = more_pages(Some(Role::Operator));
        assert!(operator.contains(&Page::Shift) && operator.contains(&Page::Audit));
        // The palette offers the operator commands only to operators; the
        // data export (any user) to everyone.
        let titles = |role| -> Vec<String> {
            entries(Some(role))
                .iter()
                .map(|entry| entry.title.to_string())
                .collect()
        };
        let viewer = titles(Role::Any);
        assert!(viewer.iter().any(|title| title == "Export data…"));
        assert!(
            !viewer
                .iter()
                .any(|title| title == "New shift handover entry")
        );
        assert!(!viewer.iter().any(|title| title == "Go to Audit log"));
        let admin = titles(Role::Admin);
        assert!(
            admin
                .iter()
                .any(|title| title == "New shift handover entry")
        );
        assert!(
            admin
                .iter()
                .any(|title| title == "Export the audit log as CSV")
        );
    }

    #[test]
    fn the_dialogs_open_from_the_command_line_by_action_name() {
        use crate::console::keys::COMMANDS;
        for name in ["OpsExportData", "OpsNewHandover", "OpsExportAudit"] {
            assert!(
                COMMANDS
                    .iter()
                    .any(|command| (command.action)().name().rsplit("::").next() == Some(name)),
                "--overlay {name} names a command"
            );
        }
    }
}
