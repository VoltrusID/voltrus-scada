//! Views are functions over `&mut Console`. Each part of the window is
//! drawn by its own cached view ([`cached::CachedView`]): the navbar, the
//! page, the status bar, the toasts, the overlay and the HUD. This root is a
//! thin shell around them: it redraws every frame, but a part that did not
//! change replays its last frame, so an animating page costs only itself.

pub(crate) mod alarms;
pub(crate) mod audit;
pub(crate) mod cached;
pub(crate) mod commands;
pub(crate) mod export;
pub(crate) mod hud;
pub(crate) mod navbar;
pub(crate) mod ops_kit;
pub(crate) mod overlays;
pub(crate) mod overview;
pub(crate) mod performance;
pub(crate) mod shift;
mod sign_in;
pub(crate) mod status_bar;
pub(crate) mod tags;
pub(crate) mod toasts;
pub(crate) mod trends;

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
    Action, Animation, AnimationExt as _, AnyElement, AnyView, App, Context, Div, ElementId,
    FontWeight, Hsla, SharedString, Window, canvas, div, px,
};
use time::{OffsetDateTime, UtcOffset};
use voltrus_scada_native::api::Alarm;
use vt_ui::chart::format_value;
use vt_ui::theme::*;

use super::keys::{
    AcknowledgeAll, Back, CONSOLE, Dismiss, FocusNext, FocusPrevious, FocusSearch, Forward, LIST,
    ListActivate, ListDown, ListFirst, ListLast, ListPageDown, ListPageUp, ListToggle, ListUp,
    Quit, ShowMore, ShowPage, SignOut, StartStress, StopStress, ToggleAppearance, ToggleHelp,
    TogglePalette, TogglePerfHud,
};
use super::keys::{OpsExportAudit, OpsExportData, OpsNewHandover};
use super::nav::Move;
use super::store::Status;
use super::{Console, Page, Phase, anim, pages, severity_rank};

/// Room below a list's last row, so it scrolls clear of the toast stack in
/// the bottom-right corner (three toasts and their gaps). Plain scrolling
/// areas take it as bottom padding; a `uniform_list` (whose padding shrinks
/// its viewport instead) gets [`tail_rows`] empty rows after its last.
pub(crate) const LIST_TAIL: gpui::Pixels = px(200.0);

/// Empty rows of `row_height` that make up [`LIST_TAIL`] after a
/// `uniform_list`'s last row; none for an empty list.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn tail_rows(len: usize, row_height: f32) -> usize {
    if len == 0 {
        0
    } else {
        (f32::from(LIST_TAIL) / row_height).ceil() as usize
    }
}

/// One of the [`tail_rows`].
pub(crate) fn tail_row(row_height: f32) -> AnyElement {
    div().w_full().h(px(row_height)).into_any_element()
}

impl Render for Console {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.meter.borrow_mut().frame_start(Instant::now());
        let theme = theme(cx).clone();
        let in_console = self.phase == Phase::Console;
        let body = if in_console {
            let page = self.page_view(self.page, cx);
            div()
                .flex()
                .flex_col()
                .size_full()
                .child(
                    AnyView::from(self.views.navbar.clone()).cached(
                        div()
                            .flex_none()
                            .w_full()
                            .h(TITLE_BAR_HEIGHT)
                            .style()
                            .clone(),
                    ),
                )
                .child(
                    AnyView::from(page).cached(div().flex_1().w_full().min_h_0().style().clone()),
                )
                .child(
                    AnyView::from(self.views.status_bar.clone()).cached(
                        div()
                            .flex_none()
                            .w_full()
                            .h(STATUS_BAR_HEIGHT)
                            .style()
                            .clone(),
                    ),
                )
                .into_any_element()
        } else {
            sign_in::render(self, cx)
        };

        // Bottom right, above the status bar and left of the page's side
        // panel (tag detail, trend picker): clear of headers and controls.
        let toast_right = if in_console {
            (pages::spec(self.page).side_panel)(self) + SPACE_7
        } else {
            SPACE_7
        };
        let toast_bottom = if in_console {
            STATUS_BAR_HEIGHT + SPACE_7
        } else {
            SPACE_7
        };
        let meter = self.meter.clone();

        div()
            .id("console")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.fg)
            .font_family(SANS)
            .text_size(FONT_BODY)
            .track_focus(&self.focus)
            .key_context(CONSOLE)
            .on_action(cx.listener(|console, _: &Quit, _, cx| {
                console.shutdown();
                cx.quit();
            }))
            .on_action(cx.listener(|console, _: &SignOut, window, cx| {
                if console.phase != Phase::SignIn {
                    console.sign_out(window, cx);
                }
            }))
            .on_action(cx.listener(|console, action: &ShowPage, window, cx| {
                if console.phase == Phase::Console {
                    console.show(action.0, window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &Back, window, cx| {
                if console.phase == Phase::Console {
                    console.history_step(false, window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &Forward, window, cx| {
                if console.phase == Phase::Console {
                    console.history_step(true, window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &AcknowledgeAll, window, cx| {
                if console.phase == Phase::Console && console.is_signed_in() {
                    alarms::confirm_acknowledge_all(console, window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &ToggleAppearance, _, cx| {
                console.toggle_appearance(cx);
            }))
            .on_action(cx.listener(|console, _: &TogglePalette, window, cx| {
                if console.overlay_is("palette") {
                    console.close_overlay(window, cx);
                } else if console.phase == Phase::Console {
                    console.open_palette(window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &ToggleHelp, window, cx| {
                if console.overlay_is("help") {
                    console.close_overlay(window, cx);
                } else if console.phase == Phase::Console {
                    console.open_help(window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &ShowMore, window, cx| {
                if console.overlay_is("more") {
                    console.close_overlay(window, cx);
                } else if console.phase == Phase::Console {
                    console.open_more(window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &TogglePerfHud, _, cx| {
                console.set_hud(!console.hud, cx);
            }))
            .on_action(cx.listener(|console, _: &FocusSearch, window, cx| {
                if console.phase == Phase::Console && console.overlay.is_none() {
                    console.focus_search(window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &FocusNext, window, cx| {
                console.focus_step(true, window, cx);
            }))
            .on_action(cx.listener(|console, _: &FocusPrevious, window, cx| {
                console.focus_step(false, window, cx);
            }))
            .on_action(cx.listener(|console, _: &Dismiss, window, cx| {
                if console.phase == Phase::Console {
                    console.dismiss(window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &StartStress, window, cx| {
                let plan = performance::plan(console);
                console.start_stress(plan, false, window, cx);
            }))
            .on_action(cx.listener(|console, _: &StopStress, window, cx| {
                if console.stress.is_some() {
                    console.sign_out(window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &OpsExportData, window, cx| {
                if console.phase == Phase::Console {
                    export::open(console, window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &OpsNewHandover, window, cx| {
                if console.phase == Phase::Console {
                    shift::new_entry(console, window, cx);
                }
            }))
            .on_action(cx.listener(|console, _: &OpsExportAudit, window, cx| {
                if console.phase == Phase::Console {
                    audit::export_csv(console, window, cx);
                }
            }))
            .map(|root| alarms::root_actions(root, cx))
            .child(body)
            .when(!self.toasts.is_empty(), |this| {
                this.child(
                    AnyView::from(self.views.toasts.clone()).cached(
                        div()
                            .absolute()
                            .bottom(toast_bottom)
                            .right(toast_right)
                            .w(toasts::WIDTH)
                            .h(toasts::HEIGHT)
                            .style()
                            .clone(),
                    ),
                )
            })
            .when(self.hud && in_console, |this| {
                this.child(
                    AnyView::from(self.views.hud.clone()).cached(
                        div()
                            .absolute()
                            .top(TITLE_BAR_HEIGHT + SPACE_6)
                            .right(SPACE_7)
                            .w(hud::WIDTH)
                            .h(hud::HEIGHT)
                            .style()
                            .clone(),
                    ),
                )
            })
            .when(self.overlay.is_some() && in_console, |this| {
                this.child(
                    AnyView::from(self.views.overlay.clone())
                        .cached(div().absolute().inset_0().style().clone()),
                )
            })
            // Painted last: the frame's end, for the meter.
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, (), _, _| {
                        meter.borrow_mut().frame_end(Instant::now(), now_ms());
                    },
                )
                .absolute()
                .w(px(0.0))
                .h(px(0.0)),
            )
    }
}

/// A page as its cached view draws it: the registry's render function,
/// entering with a short fade and slide after each switch.
pub(crate) fn page(
    console: &mut Console,
    page: Page,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let content = (pages::spec(page).render)(console, window, cx);
    if anim::progress(console.switched_at, Instant::now(), anim::PAGE_IN).is_some() {
        console.meter.borrow_mut().expect_frame(Instant::now());
    }
    div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .child(content)
        .with_animation(
            ElementId::NamedInteger("page-in".into(), console.switch_seq),
            Animation::new(anim::PAGE_IN).with_easing(anim::ease_out),
            |this, t| this.opacity(0.3 + 0.7 * t).top(px((1.0 - t) * 6.0)),
        )
        .into_any_element()
}

/// The cursor of a keyboard list moved to a row.
pub(crate) type OnMove = fn(&mut Console, usize, &mut Window, &mut Context<Console>);
/// Enter (`toggle` false) or Space (`toggle` true) on a keyboard list's row.
pub(crate) type OnActivate = fn(&mut Console, usize, bool, &mut Window, &mut Context<Console>);

/// What a keyboard list does with its rows.
#[derive(Clone, Copy)]
pub(crate) struct ListKeys {
    /// The list's id, for [`Console::list`].
    pub id: &'static str,
    /// The cursor moved to a row (the Tags page opens its detail).
    pub on_move: Option<OnMove>,
    pub on_activate: OnActivate,
}

fn mover<A: Action>(
    keys: ListKeys,
    step: Move,
    cx: &mut Context<Console>,
) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    cx.listener(move |console: &mut Console, _: &A, window, cx| {
        if let Some(row) = console.list_move(keys.id, step, cx)
            && let Some(on_move) = keys.on_move
        {
            on_move(console, row, window, cx);
        }
    })
}

fn activator<A: Action>(
    keys: ListKeys,
    toggle: bool,
    cx: &mut Context<Console>,
) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    cx.listener(move |console: &mut Console, _: &A, window, cx| {
        if let Some(row) = console.list(keys.id, cx).cursor {
            (keys.on_activate)(console, row, toggle, window, cx);
        }
    })
}

/// A container that makes the list inside it keyboard-driven: it takes
/// focus (a click on a row gives it), and j/k, the arrows, PgUp/PgDn,
/// Home/End, Enter and Space act on its cursor row. `len` is how many rows
/// this render shows; `page` how many fit on screen.
pub(crate) fn keyboard_list(
    keys: ListKeys,
    len: usize,
    page: usize,
    console: &mut Console,
    cx: &mut Context<Console>,
) -> gpui::Stateful<Div> {
    let nav = console.list(keys.id, cx);
    nav.set_len(len);
    nav.page = page;
    let focus = nav.focus.clone();
    div()
        .id(SharedString::from(keys.id))
        .track_focus(&focus)
        .key_context(LIST)
        .on_action(mover::<ListUp>(keys, Move::Up, cx))
        .on_action(mover::<ListDown>(keys, Move::Down, cx))
        .on_action(mover::<ListPageUp>(keys, Move::PageUp, cx))
        .on_action(mover::<ListPageDown>(keys, Move::PageDown, cx))
        .on_action(mover::<ListFirst>(keys, Move::First, cx))
        .on_action(mover::<ListLast>(keys, Move::Last, cx))
        .on_action(activator::<ListActivate>(keys, false, cx))
        .on_action(activator::<ListToggle>(keys, true, cx))
}

/// The keyboard cursor's ring over a row; it does not move the row's
/// content.
pub(crate) fn focus_ring(theme: &Theme) -> Div {
    div()
        .absolute()
        .inset_0()
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(theme.accent)
}

/// How many rows of `row_height` fit in the window's height, for PgUp/PgDn.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn rows_on_screen(window: &Window, row_height: f32) -> usize {
    let height = f32::from(window.viewport_size().height) - 160.0;
    (height / row_height).floor().max(1.0) as usize
}

/// Wall-clock now in milliseconds, the time base of every chart.
pub(crate) fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
}

/// The colour of a value of this status: plain when normal, so problems
/// stand out.
pub(crate) fn value_color(status: Status, theme: &Theme) -> Hsla {
    match status {
        Status::Normal => theme.fg,
        Status::Warning => theme.warning,
        Status::Critical => theme.danger,
        Status::Offline | Status::Unknown => theme.fg_dim,
    }
}

/// The colour of a status pill or dot.
pub(crate) fn status_color(status: Status, theme: &Theme) -> Hsla {
    match status {
        Status::Normal => theme.success,
        Status::Warning => theme.warning,
        Status::Critical => theme.danger,
        Status::Offline | Status::Unknown => theme.fg_dim,
    }
}

/// Text that reads on a `fill` background: the theme's background or
/// foreground, whichever contrasts more with it (WCAG relative luminance).
pub(crate) fn text_on(fill: Hsla, theme: &Theme) -> Hsla {
    let luminance = |color: Hsla| {
        let rgb = color.to_rgb();
        let linear = |channel: f32| {
            if channel <= 0.039_28 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
    };
    let contrast = |a: f32, b: f32| (a.max(b) + 0.05) / (a.min(b) + 0.05);
    let fill = luminance(fill);
    if contrast(fill, luminance(theme.bg)) >= contrast(fill, luminance(theme.fg)) {
        theme.bg
    } else {
        theme.fg
    }
}

pub(crate) fn severity_color(severity: &str, theme: &Theme) -> Hsla {
    match severity_rank(severity) {
        0 => theme.danger,
        1 | 2 => theme.warning,
        _ => theme.info,
    }
}

/// `12 s`, `4 min`, `3 h`, `2 d`.
pub(crate) fn age(seconds: i64) -> String {
    match seconds.max(0) {
        seconds @ 0..60 => format!("{seconds} s"),
        seconds @ 60..3600 => format!("{} min", seconds / 60),
        seconds @ 3600..86_400 => format!("{} h", seconds / 3600),
        seconds => format!("{} d", seconds / 86_400),
    }
}

/// A span of time for a caption: `45 s`, `12 min`, `6 h`.
pub(crate) fn span_label(span_ms: f64) -> String {
    #[allow(clippy::cast_possible_truncation)]
    let seconds = (span_ms / 1000.0).round() as i64;
    age(seconds)
}

/// The span a small chart covers: as far back as its data reaches, so a
/// young history still fills it, within `[shortest, longest]`.
pub(crate) fn fit_span(reach_ms: Option<f64>, shortest: f64, longest: f64) -> f64 {
    reach_ms.map_or(longest, |reach| reach.clamp(shortest, longest))
}

/// The tag an alarm is about, by the name the console shows for it.
pub(crate) fn alarm_source(console: &Console, alarm: &Alarm) -> String {
    console
        .store
        .by_key(&alarm.source_key)
        .map(|index| console.store.tags[index].info.name.clone())
        .filter(|name| !name.is_empty())
        .or_else(|| (!alarm.source_name.is_empty()).then(|| alarm.source_name.clone()))
        .unwrap_or_else(|| alarm.source_key.clone())
}

/// What crossed which limit, with readable numbers: `Critical limit 85,
/// reached 86.40`. The server's own message carries full float precision.
pub(crate) fn alarm_summary(alarm: &Alarm) -> String {
    let severity = capitalized(&alarm.severity);
    let verb = if alarm.alarm_type.eq_ignore_ascii_case("low") {
        "fell to"
    } else {
        "reached"
    };
    format!(
        "{severity} limit {}, {verb} {}",
        format_value(alarm.threshold),
        format_value(alarm.value)
    )
}

pub(crate) fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars.as_str().chars()).collect()
    })
}

/// A Unix time on the operator's clock: `14:05:09`, or `3 Mar 14:05` when
/// it is not today.
pub(crate) fn local_time(unix_secs: i64, offset: UtcOffset) -> String {
    let Ok(at) = OffsetDateTime::from_unix_timestamp(unix_secs) else {
        return "—".into();
    };
    let at = at.to_offset(offset);
    let today = OffsetDateTime::now_utc().to_offset(offset).date();
    if at.date() == today {
        format!("{:02}:{:02}:{:02}", at.hour(), at.minute(), at.second())
    } else {
        let month = at.month().to_string();
        format!(
            "{} {} {:02}:{:02}",
            at.day(),
            month.get(..3).unwrap_or(&month),
            at.hour(),
            at.minute()
        )
    }
}

/// `plant.local:3000` of `http://plant.local:3000`.
pub(crate) fn host(url: &str) -> &str {
    url.split_once("://").map_or(url, |(_, rest)| rest)
}

/// The page's top bar: title, a line about it, and controls on the right.
pub(crate) fn page_header(
    title: &'static str,
    subtitle: impl Into<SharedString>,
    controls: Vec<AnyElement>,
    cx: &Context<Console>,
) -> Div {
    let theme = theme(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_6)
        .h(px(56.0))
        .px(SPACE_9)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(subtitle.into()),
                ),
        )
        .children(controls)
}

/// A bordered surface for a group of content.
pub(crate) fn card(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .rounded(RADIUS_LG)
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
}

/// A card's title row, with an optional count and control.
pub(crate) fn card_title(
    title: &'static str,
    count: Option<usize>,
    control: Option<AnyElement>,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_3)
        .h(px(40.0))
        .px(SPACE_7)
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(FONT_EMPHASIS)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .when_some(count, |this, count| {
            this.child(
                div()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(count.to_string()),
            )
        })
        .child(div().flex_1())
        .children(control)
}

/// A value with its unit, the unit smaller and dimmer.
pub(crate) fn value_with_unit(
    value: String,
    unit: &str,
    size: gpui::Pixels,
    color: Hsla,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(SPACE_2)
        .min_w_0()
        .child(
            div()
                .font_family(MONO)
                .text_size(size)
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .whitespace_nowrap()
                .child(value),
        )
        .when(!unit.is_empty(), |this| {
            this.child(
                div()
                    .text_size(if size > FONT_TITLE {
                        FONT_BODY
                    } else {
                        FONT_CAPTION
                    })
                    .text_color(theme.fg_dim)
                    .whitespace_nowrap()
                    .child(unit.to_owned()),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_pick_the_largest_whole_unit() {
        assert_eq!(age(-3), "0 s");
        assert_eq!(age(59), "59 s");
        assert_eq!(age(61), "1 min");
        assert_eq!(age(7200), "2 h");
        assert_eq!(age(200_000), "2 d");
    }

    #[test]
    fn times_show_on_the_given_clock() {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let offset = UtcOffset::from_hms(7, 0, 0).unwrap();
        assert_eq!(
            local_time(now, offset).len(),
            8,
            "today shows the time only"
        );
        assert_eq!(local_time(0, UtcOffset::UTC), "1 Jan 00:00");
        assert_eq!(host("https://scada.plant:3000"), "scada.plant:3000");
    }

    #[test]
    fn spans_and_alarm_lines_read_well() {
        assert_eq!(span_label(15.0 * 60_000.0), "15 min");
        assert_eq!(fit_span(Some(30_000.0), 120_000.0, 900_000.0), 120_000.0);
        assert_eq!(fit_span(Some(400_000.0), 120_000.0, 900_000.0), 400_000.0);
        assert_eq!(fit_span(None, 120_000.0, 900_000.0), 900_000.0);
        let alarm = Alarm {
            severity: "critical".into(),
            alarm_type: "high".into(),
            threshold: 85.0,
            value: 86.395_149_230_957_03,
            ..Alarm::default()
        };
        assert_eq!(alarm_summary(&alarm), "Critical limit 85, reached 86.40");
        assert_eq!(capitalized(""), "");
    }
}
