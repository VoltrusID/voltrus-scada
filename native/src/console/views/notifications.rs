//! The navbar's bell and its popover: alarms to acknowledge, and the
//! user's notifications (mentions in shift handovers and annotations),
//! polled from the ticker. Enter marks the cursor's notification read,
//! Space dismisses it.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, Pixels, SharedString, Window, canvas, div, px};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, empty_state, kbd};
use vt_ui::theme::*;

use super::{ListKeys, age, focus_ring, keyboard_list, text_on};
use crate::console::keys::{ShellToggleNotifications, ShowPage, display, hint, page_keys};
use crate::console::overlay::OverlayKind;
use crate::console::pages::{self, Page};
use crate::console::shell::{self, Shell};
use crate::console::{Console, time_now_secs};

pub(crate) const LIST: &str = "notifications";
pub(crate) const WIDTH: Pixels = px(380.0);
/// Room for the type and age line and two lines of the message.
const ROW: f32 = 68.0;
/// Notifications shown before the list scrolls.
const VISIBLE: usize = 6;

/// The bell, with the unread count; it records where it is for the
/// popover.
pub fn bell(console: &mut Console, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let unread = console.unread_notifications();
    let open = console.overlay_is("notifications");
    let anchor = console.state::<Shell>().bell_anchor.clone();
    // Opened before the navbar was drawn (at launch) or the window
    // resized: move the popover under the bell.
    if let (Some(bounds), Some(overlay)) = (anchor.get(), console.overlay.as_mut())
        && overlay.id == "notifications"
        && let OverlayKind::Popover { at, .. } = &mut overlay.kind
        && *at != shell::popover_at(bounds)
    {
        *at = shell::popover_at(bounds);
        cx.notify();
    }
    let tooltip = SharedString::from(format!(
        "Notifications  {}",
        hint(&ShellToggleNotifications).unwrap_or_default()
    ));
    div()
        .relative()
        .flex_none()
        .child(
            IconButton::new("notifications", Icon::Inbox)
                .active(open)
                .color(if unread > 0 {
                    theme.warning
                } else {
                    theme.fg_muted
                })
                .tooltip(tooltip)
                .on_click(cx.listener(|_, _, window, cx| {
                    window.dispatch_action(Box::new(ShellToggleNotifications), cx);
                })),
        )
        .when(unread > 0, |this| {
            this.child(
                div()
                    .absolute()
                    .top(px(-3.0))
                    .right(px(-4.0))
                    .min_w(px(14.0))
                    .h(px(14.0))
                    .px(px(3.0))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.warning)
                    .text_color(text_on(theme.warning, &theme))
                    .text_size(FONT_CAPTION)
                    .font_weight(FontWeight::BOLD)
                    .font_family(MONO)
                    .child(if unread > 99 {
                        "99+".to_owned()
                    } else {
                        unread.to_string()
                    }),
            )
        })
        .child(
            canvas(
                move |bounds, _, _| anchor.set(Some(bounds)),
                |_, (), _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .into_any_element()
}

fn activate(
    console: &mut Console,
    row: usize,
    dismiss: bool,
    _: &mut Window,
    _: &mut Context<Console>,
) {
    let Some(notification) = console.state::<Shell>().notifications.get(row).cloned() else {
        return;
    };
    if dismiss {
        shell::dismiss(console, notification.id);
    } else if !notification.read {
        shell::mark_read(console, notification.id);
    }
}

/// What a notification is about, from its type.
fn kind_label(kind: &str) -> &'static str {
    match kind {
        "shift_handover" => "Shift handover",
        "annotation" => "Annotation",
        _ => "Notification",
    }
}

pub fn popover(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let unacknowledged = console.unacknowledged();
    let (notifications, error) = {
        let shell = console.state::<Shell>();
        (
            shell.notifications.clone(),
            shell.notifications_error.clone(),
        )
    };
    let unread = notifications.iter().filter(|n| !n.read).count();
    let count = notifications.len();
    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate,
        },
        count,
        VISIBLE,
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));
    let alarms_keys = pages::spec(Page::Alarms)
        .digit
        .map(|digit| display(&page_keys(digit)))
        .unwrap_or_default();
    let now = time_now_secs();

    let alarms = div()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .mx(SPACE_3)
        .px(SPACE_4)
        .py(SPACE_4)
        .rounded(RADIUS_MD)
        .bg(if unacknowledged > 0 {
            theme.danger.alpha(0.08)
        } else {
            theme.surface
        })
        .child(vt_ui::icon(
            if unacknowledged > 0 {
                Icon::CircleAlert
            } else {
                Icon::CircleCheck
            },
            ICON_MD,
            if unacknowledged > 0 {
                theme.danger
            } else {
                theme.success
            },
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(FONT_SMALL)
                .child(match unacknowledged {
                    0 => "No alarms to acknowledge".to_owned(),
                    1 => "1 alarm to acknowledge".to_owned(),
                    count => format!("{count} alarms to acknowledge"),
                }),
        )
        .child(
            Button::new("bell-open-alarms", "Open Alarms")
                .compact()
                .ghost()
                .tooltip(alarms_keys)
                .on_click(cx.listener(|console, _, window, cx| {
                    console.close_overlay(window, cx);
                    window.dispatch_action(Box::new(ShowPage(Page::Alarms)), cx);
                })),
        );

    let rows: Vec<AnyElement> =
        notifications
            .iter()
            .enumerate()
            .map(|(row, notification)| {
                let ring = focused && cursor == Some(row);
                let hover = theme.hover;
                let id = notification.id;
                let read = notification.read;
                div()
                    .id(("notification", row))
                    .relative()
                    .flex()
                    .items_start()
                    .gap(SPACE_4)
                    .h(px(ROW))
                    .mx(SPACE_3)
                    .px(SPACE_4)
                    .py(SPACE_3)
                    .rounded(RADIUS_MD)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |console, _, window, cx| {
                        console.list(LIST, cx).select(row);
                        activate(console, row, false, window, cx);
                    }))
                    .child(div().pt(px(5.0)).flex_none().child(
                        div().size(px(7.0)).rounded_full().bg(if read {
                            gpui::transparent_black()
                        } else {
                            theme.accent
                        }),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(SPACE_1)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(SPACE_3)
                                    .text_size(FONT_CAPTION)
                                    .text_color(theme.fg_dim)
                                    .child(kind_label(&notification.notification_type))
                                    .child("·")
                                    .child(format!("{} ago", age(now - notification.created_at))),
                            )
                            .child(
                                div()
                                    .text_size(FONT_SMALL)
                                    .line_clamp(2)
                                    .text_color(if read { theme.fg_muted } else { theme.fg })
                                    .when(!read, |this| this.font_weight(FontWeight::MEDIUM))
                                    .child(notification.message.clone()),
                            ),
                    )
                    .child(
                        IconButton::new(("dismiss-notification", row), Icon::X)
                            .size(CONTROL_SM)
                            .tooltip("Dismiss  Space")
                            .on_click(cx.listener(move |console, _, _, cx| {
                                cx.stop_propagation();
                                shell::dismiss(console, id);
                            })),
                    )
                    .when(ring, |this| this.child(focus_ring(&theme)))
                    .into_any_element()
            })
            .collect();

    let body = if count == 0 {
        div()
            .h(px(150.0))
            .child(empty_state(
                Icon::Inbox,
                "Nothing new",
                error.map_or_else(
                    || "Mentions in shift handovers and annotations appear here.".to_owned(),
                    |error| format!("Not updated: {error}"),
                ),
                Vec::new(),
                cx,
            ))
            .into_any_element()
    } else {
        div()
            .id("notification-rows")
            .max_h(px(ROW * VISIBLE as f32 + 8.0))
            .overflow_y_scroll()
            .track_scroll(&console.scroll("notifications"))
            .flex()
            .flex_col()
            .gap(SPACE_1)
            .children(rows)
            .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .gap(SPACE_3)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .px(SPACE_6)
                .pt(SPACE_2)
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Notifications"),
                )
                .when(unread > 0, |this| {
                    this.child(
                        div()
                            .font_family(MONO)
                            .text_size(FONT_SMALL)
                            .text_color(theme.warning)
                            .child(format!("{unread} unread")),
                    )
                })
                .child(div().flex_1())
                .when(unread > 0, |this| {
                    this.child(
                        Button::new("mark-all-read", "Mark all read")
                            .compact()
                            .ghost()
                            .on_click(
                                cx.listener(|console, _, _, _| shell::mark_all_read(console)),
                            ),
                    )
                }),
        )
        .child(alarms)
        .child(list.child(body))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_5)
                .px(SPACE_6)
                .pt(SPACE_2)
                .border_t_1()
                .border_color(theme.border)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .child(kbd("↵", cx))
                        .child("Read"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .child(kbd("Space", cx))
                        .child("Dismiss"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_2)
                        .child(kbd("Esc", cx))
                        .child("Close"),
                ),
        )
        .into_any_element()
}
