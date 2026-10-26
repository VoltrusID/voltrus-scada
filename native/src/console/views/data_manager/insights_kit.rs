//! What the Insights sections (OEE, Analytics, Recipes) share: the plant's
//! devices from the tag list, a row of device chips, a figure card, a bar,
//! a timer that runs only while a section is on screen, and saving a text
//! file where the operator picks.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, Hsla, SharedString, div};
use vt_ui::components::{Button, ButtonKind};
use vt_ui::theme::*;

use super::section_shown;
use crate::console::{Console, Phase};

/// The devices of the tag list, kept until the list changes.
#[derive(Default)]
struct Devices {
    /// The store's every-row answer the list was made from; a new tag list
    /// gives a new one.
    from: Option<Rc<[usize]>>,
    ids: Rc<[SharedString]>,
}

/// Every device id the plant's tags name, each once, in list order. Made
/// again only when the tag list changes, never per frame.
pub(crate) fn devices(console: &mut Console) -> Rc<[SharedString]> {
    let all = console.store.filter("");
    if let Some(kept) = console.state_ref::<Devices>()
        && kept
            .from
            .as_ref()
            .is_some_and(|from| Rc::ptr_eq(from, &all))
    {
        return Rc::clone(&kept.ids);
    }
    let mut seen = HashSet::new();
    let ids: Rc<[SharedString]> = console
        .store
        .tags
        .iter()
        .map(|tag| tag.info.device_id.as_str())
        .filter(|device| !device.is_empty() && seen.insert(*device))
        .map(|device| SharedString::from(device.to_owned()))
        .collect();
    let state = console.state::<Devices>();
    state.from = Some(all);
    state.ids = Rc::clone(&ids);
    ids
}

/// The positions of the tags of `device`, in list order.
pub(crate) fn device_tags(console: &Console, device: &str) -> Vec<usize> {
    console
        .store
        .tags
        .iter()
        .enumerate()
        .filter(|(_, tag)| tag.info.device_id == device)
        .map(|(index, _)| index)
        .collect()
}

/// A chosen device as text.
pub(crate) fn text(value: &Option<SharedString>) -> Option<&str> {
    value.as_ref().map(SharedString::as_ref)
}

/// What a chip row does when a chip is picked: `None` is "All".
pub(crate) type OnPick = fn(&mut Console, Option<SharedString>, &mut Context<Console>);

/// A row of compact buttons, one per option (and "All" first when `all`
/// holds), the chosen one filled; it wraps when there are many. Tab
/// reaches each and Enter or Space picks it.
pub(crate) fn chips(
    id: &'static str,
    options: &[SharedString],
    chosen: Option<&str>,
    all: Option<&'static str>,
    on_pick: OnPick,
    cx: &mut Context<Console>,
) -> AnyElement {
    let chip = |index: usize,
                label: SharedString,
                value: Option<SharedString>,
                active: bool,
                cx: &mut Context<Console>| {
        Button::new(
            ElementId::NamedInteger(SharedString::from(id), u64::try_from(index).unwrap_or(0)),
            label,
        )
        .compact()
        .kind(if active {
            ButtonKind::Primary
        } else {
            ButtonKind::Secondary
        })
        .on_click(cx.listener(move |console, _, _, cx| {
            on_pick(console, value.clone(), cx);
        }))
        .into_any_element()
    };
    let mut row = Vec::with_capacity(options.len() + 1);
    if let Some(label) = all {
        row.push(chip(0, label.into(), None, chosen.is_none(), cx));
    }
    for (index, option) in options.iter().enumerate() {
        let active = chosen == Some(option.as_ref());
        row.push(chip(
            index + 1,
            option.clone(),
            Some(option.clone()),
            active,
            cx,
        ));
    }
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(SPACE_2)
        .min_w_0()
        .children(row)
        .into_any_element()
}

/// The next option after `current` (or before it), wrapping; `[` and `]`
/// step through the devices this way.
pub(crate) fn step<'a>(
    options: &'a [SharedString],
    current: Option<&str>,
    forward: bool,
) -> Option<&'a SharedString> {
    if options.is_empty() {
        return None;
    }
    let at = current.and_then(|current| options.iter().position(|option| option == current));
    let len = options.len();
    let next = match (at, forward) {
        (None, true) => 0,
        (None, false) => len - 1,
        (Some(at), true) => (at + 1) % len,
        (Some(at), false) => (at + len - 1) % len,
    };
    options.get(next)
}

/// A horizontal bar `fraction` full (clamped to 0–1).
pub(crate) fn bar(fraction: f64, color: Hsla, height: gpui::Pixels, theme: &Theme) -> AnyElement {
    #[allow(clippy::cast_possible_truncation)]
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    div()
        .flex_1()
        .min_w_0()
        .h(height)
        .rounded(RADIUS_SM)
        .bg(theme.border)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(gpui::relative(fraction))
                .rounded(RADIUS_SM)
                .bg(color),
        )
        .into_any_element()
}

/// A labelled figure: the caption above, the value in mono below.
pub(crate) fn figure(label: &'static str, value: String, color: Hsla, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_1)
        .min_w_0()
        .child(
            div()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child(label.to_uppercase()),
        )
        .child(
            div()
                .truncate()
                .font_family(MONO)
                .text_size(FONT_EMPHASIS)
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(value),
        )
        .into_any_element()
}

/// The section timers that run now: section id → the running loop's
/// number. A loop whose number is gone (the session's state was dropped
/// at sign-out) ends at its next tick, so a later session never runs two.
#[derive(Default)]
struct Timers {
    running: HashMap<&'static str, u64>,
}

/// Numbers the loops across sessions (a session's [`Timers`] starts
/// empty).
static NEXT_TIMER: AtomicU64 = AtomicU64::new(1);

/// Run `tick` every `every` while section `id` is on screen in a
/// signed-in console, until `tick` answers false. Started again by the
/// next call once it has ended.
pub(crate) fn every_while_shown(
    console: &mut Console,
    id: &'static str,
    every: Duration,
    tick: fn(&mut Console, &mut Context<Console>) -> bool,
    cx: &mut Context<Console>,
) {
    if console.stress.is_some() {
        return;
    }
    let timers = console.state::<Timers>();
    if timers.running.contains_key(id) {
        return;
    }
    let number = NEXT_TIMER.fetch_add(1, Ordering::Relaxed);
    timers.running.insert(id, number);
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(every).await;
            let going = this.update(cx, |console, cx| {
                let mine = console.state::<Timers>().running.get(id) == Some(&number);
                let going = mine
                    && console.phase == Phase::Console
                    && console.stress.is_none()
                    && section_shown(console, id)
                    && tick(console, cx);
                if !going && mine {
                    console.state::<Timers>().running.remove(id);
                }
                going
            });
            if !matches!(going, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

/// Ask where to save `text` (`suggested` as the name, in Downloads) and
/// write it there; a toast says how it went.
pub(crate) fn save_text(suggested: String, text: String, cx: &mut Context<Console>) {
    let directory = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    let prompt = cx.prompt_for_new_path(&directory, Some(&suggested));
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(file))) = prompt.await else {
            return;
        };
        let written = cx
            .background_executor()
            .spawn({
                let file = file.clone();
                async move { std::fs::write(&file, text) }
            })
            .await;
        let _ = this.update(cx, |console, cx| match written {
            Ok(()) => {
                let name = file.file_name().map_or_else(
                    || file.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                console.show_toast(format!("Saved {name}"), false, cx);
            }
            Err(error) => console.show_toast(format!("Saving failed: {error}"), true, cx),
        });
    })
    .detach();
}

/// Test helpers the Insights sections share.
#[cfg(test)]
pub(crate) mod testing {
    use voltrus_scada_native::runtime::{CallKey, Event};

    use crate::console::Page;
    use crate::console::testing::Harness;

    /// The plant `plts_1` with tags `p` and `q`, signed in as `role`.
    pub(crate) fn signed_in(cx: &mut gpui::TestAppContext, role: &str) -> Harness {
        let mut h = Harness::new(cx);
        h.sign_in(role);
        let tag = |id, key| {
            let mut tag = Harness::tag(id, key);
            tag.device_id = "plts_1".into();
            tag
        };
        h.event(Event::Tags(vec![tag(1, "p"), tag(2, "q")]));
        h.calls();
        h
    }

    /// The Data Manager on section `id` (as `--page data --section id`).
    pub(crate) fn open(h: &mut Harness, id: &'static str) {
        h.update(|console, window, cx| {
            super::super::launch(console, id);
            console.show(Page::DataManager, window, cx);
        });
        h.draw();
    }

    /// The call named `name` among those sent since the last look.
    pub(crate) fn sent(calls: &[CallKey], name: &str) -> Option<CallKey> {
        calls.iter().copied().find(|key| key.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_steps_wrap_both_ways() {
        let options: Vec<SharedString> = vec!["a".into(), "b".into(), "c".into()];
        let at = |current, forward| step(&options, current, forward).map(|s| s.to_string());
        assert_eq!(at(None, true).as_deref(), Some("a"));
        assert_eq!(at(None, false).as_deref(), Some("c"));
        assert_eq!(at(Some("c"), true).as_deref(), Some("a"));
        assert_eq!(at(Some("a"), false).as_deref(), Some("c"));
        assert_eq!(at(Some("gone"), true).as_deref(), Some("a"));
        assert!(step(&[], None, true).is_none());
    }
}
