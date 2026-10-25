//! The faceplate (`hmi/Faceplate.tsx`, ISA-101): a side panel for the
//! selected equipment with its identity, state and alarm, the process value
//! against its limits, a trend, related tags, and its commands, armed first
//! and confirmed second. An armed command lapses after
//! [`voltrus_scada_native::mimic::ARM_TIMEOUT`].

use std::time::Instant;

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, Hsla, SharedString, div, px, relative};
use voltrus_scada_native::mimic::{
    self, Binding, EquipState, Level, Widget, limit, low_limits, related, related_text, state_text,
};
use vt_ui::Icon;
use vt_ui::chart::sparkline;
use vt_ui::components::{Button, IconButton, kbd};
use vt_ui::theme::*;

use super::paint::{Hmi, fade};
use super::scene::{Item, StoreValues, TagView, motion};
use super::{FACEPLATE_WIDTH, act, armed, level_color, setpoint, write_status};
use crate::console::Console;
use crate::console::views::now_ms;

/// The trend's reach.
const TREND_MS: f64 = 30.0 * 60_000.0;

/// A state or alarm pill: a small square, upper-case words, tinted.
fn pill(text: impl Into<SharedString>, color: Hsla, alpha: f32) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_2)
        .px(SPACE_3)
        .py(px(2.0))
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(fade(color, 0.55 * alpha))
        .bg(fade(color, 0.12 * alpha))
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::BOLD)
        .text_color(fade(color, alpha))
        .child(div().size(px(6.0)).rounded(px(1.0)).bg(fade(color, alpha)))
        .child(text.into())
}

fn state_color(hmi: &Hmi, state: EquipState) -> Hsla {
    match state {
        EquipState::Running => hmi.run,
        EquipState::Fault => hmi.fault,
        EquipState::Stale => hmi.stale,
        EquipState::Idle | EquipState::Stopped => hmi.fg_muted,
    }
}

/// The analog indicator: range, alarm zones, a pointer at the value.
fn analog_bar(tag: &TagView, low: (Option<f64>, Option<f64>), hmi: &Hmi) -> gpui::Div {
    let (min, max) = (tag.min, tag.max);
    #[allow(clippy::cast_possible_truncation)]
    let at = |value: f64| mimic::fraction(value, min, max) as f32;
    let warn = limit(tag.warn, min, max);
    let crit = limit(tag.crit, min, max);
    let (low_warn, low_crit) = low;
    let zone = |from: f64, to: f64, color: Hsla, alpha: f32| {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(relative(at(from)))
            .w(relative((at(to) - at(from)).max(0.0)))
            .bg(fade(color, alpha))
    };
    let mut bar = div()
        .relative()
        .h(px(12.0))
        .rounded(px(2.0))
        .overflow_hidden()
        .bg(hmi.badge)
        .border_1()
        .border_color(hmi.border);
    if let Some(value) = tag.value {
        bar = bar.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(relative(at(value)))
                .bg(fade(hmi.stroke, 0.28)),
        );
    }
    if let Some(lc) = low_crit {
        bar = bar.child(zone(min, lc, hmi.fault, 0.4));
    }
    if let Some(lw) = low_warn {
        bar = bar.child(zone(low_crit.unwrap_or(min), lw, hmi.warn, 0.32));
    }
    if let Some(w) = warn {
        bar = bar.child(zone(w, crit.unwrap_or(max), hmi.warn, 0.32));
    }
    if let Some(c) = crit {
        bar = bar.child(zone(c, max, hmi.fault, 0.4));
    }
    if let Some(value) = tag.value {
        bar = bar.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(at(value)))
                .ml(px(-1.5))
                .w(px(3.0))
                .bg(hmi.fg),
        );
    }
    let marks: Vec<(String, Hsla)> = [
        low_crit.map(|v| (format!("LL {}", trim(v)), hmi.fault)),
        low_warn.map(|v| (format!("L {}", trim(v)), hmi.warn)),
        warn.map(|v| (format!("H {}", trim(v)), hmi.warn)),
        crit.map(|v| (format!("HH {}", trim(v)), hmi.fault)),
    ]
    .into_iter()
    .flatten()
    .collect();
    div()
        .flex()
        .flex_col()
        .gap(SPACE_2)
        .mt(SPACE_4)
        .child(bar)
        .child(
            div()
                .flex()
                .justify_between()
                .gap(SPACE_4)
                .font_family(MONO)
                .text_size(FONT_CAPTION)
                .text_color(hmi.fg_dim)
                .child(format!("{}–{} {}", trim(min), trim(max), tag.unit))
                .child(
                    div().flex().gap(SPACE_4).children(
                        marks
                            .into_iter()
                            .map(|(text, color)| div().text_color(color).child(text)),
                    ),
                ),
        )
}

/// A number as JavaScript prints it: no trailing zeros.
fn trim(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn section_title(text: &'static str, hmi: &Hmi) -> gpui::Div {
    div()
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::BOLD)
        .text_color(hmi.fg_dim)
        .child(text)
}

/// The panel for `item`; `can_command` says whether the user's role may
/// send commands at all.
pub(crate) fn render(
    console: &mut Console,
    item: &Item,
    hmi: &Hmi,
    can_command: bool,
    t: f32,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let widget: &Widget = &item.widget;
    let resolved = item.symbol.map(|(resolved, _)| resolved);
    let live = item.symbol.map(|(_, live)| live);
    let def = resolved.map(|resolved| resolved.def);
    let tag = item.primary.as_ref();
    let blink = motion::blink(t);

    let title: String = widget
        .text("label")
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .or_else(|| tag.map(|tag| tag.name.to_string()))
        .or_else(|| def.map(|def| def.name.to_owned()))
        .unwrap_or_else(|| "Tag".to_owned());
    let subtitle = [
        def.map(|def| def.name.to_owned()),
        tag.map(|tag| tag.key.to_string()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let state_bound = def.is_some_and(|def| def.binding == Binding::State);
    let alarm = live.and_then(|live| live.alarm).or_else(|| {
        tag.filter(|tag| tag.value.is_some())
            .map(|tag| tag.level)
            .filter(|level| *level != Level::Normal)
    });

    // State and alarm pills.
    let mut pills: Vec<AnyElement> = Vec::new();
    if let (Some(live), Some(def)) = (live, def)
        && state_bound
    {
        let alpha = if live.state == EquipState::Fault {
            blink
        } else {
            1.0
        };
        pills.push(
            pill(
                state_text(def, live.state),
                state_color(hmi, live.state),
                alpha,
            )
            .into_any_element(),
        );
    }
    let faulted = live.is_some_and(|live| live.state == EquipState::Fault);
    if faulted && !state_bound {
        pills.push(pill("FAULT", hmi.fault, blink).into_any_element());
    }
    if let Some(level) = alarm {
        let (text, alpha) = match level {
            Level::Critical => ("CRITICAL ALARM", blink),
            _ => ("WARNING", 1.0),
        };
        pills.push(pill(text, level_color(hmi, level), alpha).into_any_element());
    }
    let has_value = tag.is_some_and(|tag| tag.value.is_some());
    if widget.tag().is_some() && !has_value {
        pills.push(pill("NO DATA", hmi.stale, 1.0).into_any_element());
    }
    if has_value && alarm.is_none() && !faulted {
        pills.push(pill("QUALITY GOOD", hmi.fg_muted, 1.0).into_any_element());
    }

    // The process value, for anything but on/off equipment.
    let value_block = tag.filter(|_| !state_bound).map(|tag| {
        let source_config = console
            .store
            .get(tag.id)
            .map(|known| known.info.source_config.clone())
            .unwrap_or_default();
        let analog = tag.max != tag.min;
        div()
            .flex()
            .flex_col()
            .px(SPACE_6)
            .pt(SPACE_5)
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(SPACE_3)
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(28.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(if tag.value.is_some() {
                                hmi.fg
                            } else {
                                hmi.stale
                            })
                            .child(tag.text().unwrap_or_else(|| "----".to_owned())),
                    )
                    .child(div().text_color(hmi.fg_dim).child(tag.unit.clone())),
            )
            .when(analog, |this| {
                this.child(analog_bar(tag, low_limits(&source_config), hmi))
            })
    });

    // The trend, from the tag's history in the store.
    let trend = tag.and_then(|tag| console.store.get(tag.id)).map(|known| {
        let now = now_ms();
        let summary = known.history.summary(now - TREND_MS);
        // How far back the samples in the window reach, as the web shows.
        let minutes = known
            .samples()
            .find(|(at, _)| *at >= now - TREND_MS)
            .map(|(at, _)| ((now - at) / 60_000.0).round().max(1.0));
        let decimals = usize::try_from(known.info.decimals.clamp(0, 6)).unwrap_or(1);
        div()
            .flex()
            .flex_col()
            .gap(SPACE_2)
            .px(SPACE_6)
            .pt(SPACE_5)
            .pb(SPACE_5)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(FONT_CAPTION)
                    .text_color(hmi.fg_dim)
                    .child(div().font_weight(FontWeight::BOLD).child(match minutes {
                        Some(minutes) => format!("TREND · LAST {minutes:.0} MIN"),
                        None => "TREND".to_owned(),
                    }))
                    .when_some(summary.filter(|_| !state_bound), |this, summary| {
                        this.child(div().font_family(MONO).child(format!(
                            "min {:.*} · max {:.*}",
                            decimals, summary.min, decimals, summary.max
                        )))
                    }),
            )
            .child(
                div()
                    .relative()
                    .h(px(70.0))
                    .rounded(RADIUS_SM)
                    .border_1()
                    .border_color(hmi.border)
                    .bg(hmi.badge)
                    .p(px(2.0))
                    .child(if known.history.len() < 2 {
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(FONT_SMALL)
                            .text_color(hmi.fg_dim)
                            .child("Collecting samples…")
                            .into_any_element()
                    } else {
                        sparkline(known.history.clone(), hmi.accent, now, TREND_MS)
                            .into_any_element()
                    }),
            )
    });

    // Related tags: run feedback, run tag, fault tag.
    let rows: Vec<AnyElement> = related(widget)
        .into_iter()
        .map(|row| {
            let known = console.store.get(row.tag);
            let value = known.and_then(|known| known.value.is_finite().then_some(known.value));
            let on = value.is_some_and(|value| value > 0.5);
            let color = match value {
                None => hmi.stale,
                Some(_) if on && row.label == "Fault" => hmi.fault,
                Some(_) if on => hmi.run,
                Some(_) => hmi.fg_muted,
            };
            let key = known.map_or_else(|| format!("#{}", row.tag), |known| known.info.key.clone());
            div()
                .flex()
                .justify_between()
                .py(SPACE_3)
                .border_t_1()
                .border_color(hmi.border)
                .child(
                    div()
                        .flex()
                        .gap(SPACE_3)
                        .text_color(hmi.fg_muted)
                        .child(row.label)
                        .child(div().font_family(MONO).text_color(hmi.fg_dim).child(key)),
                )
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::BOLD)
                        .text_color(color)
                        .child(if value.is_some() {
                            related_text(row.label, on)
                        } else {
                            "NO DATA"
                        }),
                )
                .into_any_element()
        })
        .collect();

    // Commands.
    let commands = mimic::commands(widget, &StoreValues(&console.store));
    let now = Instant::now();
    let armed_now = armed(console, &widget.id).filter(|armed| !armed.expired(now));
    let status = widget
        .tag()
        .and_then(|tag| write_status(console, &widget.id, tag));
    let setpoint_value = setpoint(console, &widget.id);
    let command_block = commands.map(|commands| {
        let mut block = div()
            .flex()
            .flex_col()
            .gap(SPACE_3)
            .px(SPACE_6)
            .py(SPACE_5)
            .border_t_1()
            .border_color(hmi.border)
            .child(section_title("OPERATOR COMMAND", hmi));
        if !can_command {
            return block.child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(hmi.fg_dim)
                    .child("Commands need the operator role (admin, demo or integration)."),
            );
        }
        if let Some(armed) = &armed_now {
            let left = armed.left(now).as_secs_f32().ceil();
            block = block
                .child(
                    div()
                        .flex()
                        .gap(SPACE_3)
                        .child(
                            div().flex_1().child(
                                Button::new("mimic-confirm", format!("Confirm {}", armed.label))
                                    .primary()
                                    .icon(Icon::Check)
                                    .tooltip("Send the command  ↵")
                                    .on_click(cx.listener(|console, _, _, cx| {
                                        act::confirm(console, cx);
                                    })),
                            ),
                        )
                        .child(
                            Button::new("mimic-disarm", "Cancel")
                                .tooltip("Esc")
                                .on_click(cx.listener(|console, _, _, cx| {
                                    act::disarm(console, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .text_size(FONT_CAPTION)
                        .text_color(hmi.warn)
                        .child(format!("Armed: confirm within {left:.0} s")),
                );
        } else if commands.setpoint.is_some() {
            let value = setpoint_value.unwrap_or(commands.on.value);
            let decimals = tag.map_or(1, |tag| tag.decimals);
            block = block.child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_3)
                    .child(
                        IconButton::new("mimic-step-down", Icon::ChevronDown)
                            .tooltip("Lower  ↓")
                            .on_click(
                                cx.listener(|console, _, _, cx| act::step(console, -1.0, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_center()
                            .font_family(MONO)
                            .text_size(FONT_TITLE)
                            .font_weight(FontWeight::BOLD)
                            .child(format!(
                                "{value:.decimals$} {}",
                                tag.map(|tag| tag.unit.to_string()).unwrap_or_default()
                            )),
                    )
                    .child(
                        IconButton::new("mimic-step-up", Icon::ChevronUp)
                            .tooltip("Raise  ↑")
                            .on_click(cx.listener(|console, _, _, cx| act::step(console, 1.0, cx))),
                    )
                    .child(
                        Button::new("mimic-set", "SET")
                            .tooltip("Arm the setpoint  1")
                            .on_click(cx.listener(|console, _, _, cx| act::arm(console, true, cx))),
                    ),
            );
        } else {
            let on = commands.on.clone();
            let off = commands.off.clone();
            let busy = status
                .as_ref()
                .is_some_and(|status| status.as_deref() == Ok("Sending…"));
            block = block.child(
                div()
                    .flex()
                    .gap(SPACE_3)
                    .child(
                        div().flex_1().child(
                            Button::new(ElementId::Name("mimic-on".into()), on.label)
                                .icon(Icon::Play)
                                .disabled(!on.enabled || busy)
                                .tooltip("Arm  1")
                                .on_click(
                                    cx.listener(|console, _, _, cx| act::arm(console, true, cx)),
                                ),
                        ),
                    )
                    .child(
                        div().flex_1().child(
                            Button::new(ElementId::Name("mimic-off".into()), off.label)
                                .icon(Icon::Square)
                                .disabled(!off.enabled || busy)
                                .tooltip("Arm  0")
                                .on_click(cx.listener(|console, _, _, cx| {
                                    act::arm(console, false, cx);
                                })),
                        ),
                    ),
            );
        }
        if faulted {
            block =
                block.child(div().text_size(FONT_CAPTION).text_color(hmi.fault).child(
                    "Equipment is tripped. Reset the fault in the field before restarting.",
                ));
        }
        if let Some(status) = &status {
            let (text, color) = match status {
                Ok(text) => (text.clone(), hmi.fg_muted),
                Err(error) => (format!("Refused: {error}"), hmi.fault),
            };
            block = block.child(
                div()
                    .font_family(MONO)
                    .text_size(FONT_CAPTION)
                    .text_color(color)
                    .truncate()
                    .child(text),
            );
        }
        block
    });

    let detail = widget.tag().map(|tag| {
        div()
            .id("mimic-open-tag")
            .flex()
            .items_center()
            .justify_between()
            .px(SPACE_6)
            .py(SPACE_4)
            .border_t_1()
            .border_color(hmi.border)
            .cursor_pointer()
            .text_color(hmi.accent)
            .font_weight(FontWeight::SEMIBOLD)
            .hover(|style| style.bg(theme.hover))
            .child("Open tag history and details →")
            .on_click(cx.listener(move |console, _, window, cx| {
                act::open_tag(console, tag, window, cx);
            }))
    });

    div()
        .id("mimic-faceplate")
        .flex()
        .flex_col()
        .flex_none()
        .w(FACEPLATE_WIDTH)
        .h_full()
        .overflow_y_scroll()
        .bg(theme.surface)
        .border_l_1()
        .border_color(theme.border)
        .text_size(FONT_SMALL)
        .text_color(theme.fg)
        .child(
            div()
                .flex()
                .items_start()
                .gap(SPACE_4)
                .px(SPACE_6)
                .py(SPACE_5)
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
                                .font_weight(FontWeight::BOLD)
                                .truncate()
                                .child(title),
                        )
                        .child(
                            div()
                                .font_family(MONO)
                                .text_size(FONT_CAPTION)
                                .text_color(theme.fg_dim)
                                .truncate()
                                .child(subtitle),
                        ),
                )
                .child(
                    IconButton::new("mimic-close-faceplate", Icon::X)
                        .tooltip("Close  Esc")
                        .on_click(cx.listener(|console, _, _, cx| act::close(console, cx))),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(SPACE_3)
                .px(SPACE_6)
                .pt(SPACE_5)
                .children(pills),
        )
        .children(value_block)
        .children(trend)
        .when(!rows.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .px(SPACE_6)
                    .pb(SPACE_4)
                    .children(rows),
            )
        })
        .children(command_block)
        .children(detail)
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(SPACE_2)
                .px(SPACE_6)
                .py(SPACE_5)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .child(kbd("1", cx))
                .child(kbd("0", cx))
                .child("arm")
                .child(kbd("↵", cx))
                .child("confirm")
                .child(kbd("Esc", cx))
                .child("close"),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::trim;

    #[test]
    fn limits_print_like_javascript() {
        assert_eq!(trim(85.0), "85");
        assert_eq!(trim(50.15), "50.15");
        assert_eq!(trim(0.0), "0");
    }
}
