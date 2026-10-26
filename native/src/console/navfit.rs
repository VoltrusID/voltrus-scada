//! The navbar's fit: what it shows at a window's width. It steps down, a
//! step at a time and keeping the steps before, from the widest window:
//!
//! 1. [`Fit::Full`]: everything, with labels.
//! 2. [`Fit::IconTabs`]: the tabs and the More button as icons (with
//!    tooltips).
//! 3. [`Fit::NoNames`]: no wordmark beside the logo, no user name beside
//!    the avatar (its tooltip names the user).
//! 4. [`Fit::SearchIcon`]: the Search box as one button (⌘K still works).
//! 5. [`Fit::Folded`]: the secondary controls (notifications, settings,
//!    alarm sound, HUD, appearance) folded into one menu button.
//! 6. [`Fit::NoHost`]: no server host or edition badge.
//!
//! The widths come from the constants the navbar draws with and its labels'
//! widths as the text system measures them, so the step chosen is the
//! widest whose parts fit, the avatar and sign-out keeping [`EDGE`] from
//! the window's right edge.

use vt_ui::theme::{CONTROL_SM, ICON_MD, ICON_SM, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6};

/// What the navbar shows, widest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    Full,
    IconTabs,
    NoNames,
    SearchIcon,
    Folded,
    NoHost,
}

impl Fit {
    pub const ALL: [Self; 6] = [
        Self::Full,
        Self::IconTabs,
        Self::NoNames,
        Self::SearchIcon,
        Self::Folded,
        Self::NoHost,
    ];

    /// The tabs and the More button with their labels.
    pub fn tab_labels(self) -> bool {
        self == Self::Full
    }

    /// The wordmark beside the logo and the user's name and role beside the
    /// avatar.
    pub fn names(self) -> bool {
        self <= Self::IconTabs
    }

    /// The Search box (else a search button).
    pub fn search_box(self) -> bool {
        self <= Self::NoNames
    }

    /// The secondary controls in one menu.
    pub fn folds_controls(self) -> bool {
        self >= Self::Folded
    }

    /// The server's host and the edition badge.
    pub fn host(self) -> bool {
        self <= Self::Folded
    }
}

/// One tab: its label's width and its badge's text width, if it shows one.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tab {
    pub label: f32,
    pub badge: Option<f32>,
}

/// What the navbar holds, with its texts' widths in pixels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parts {
    /// Before the logo (the traffic lights on macOS).
    pub left: f32,
    pub wordmark: f32,
    pub host: f32,
    /// The edition badge's text.
    pub edition: Option<f32>,
    pub tabs: Vec<Tab>,
    /// The More button's label (the page shown, or "More"); `None` without
    /// a More menu.
    pub more: Option<f32>,
    pub search_label: f32,
    /// The ⌘K hint in the Search box.
    pub search_keys: f32,
    /// Secondary controls on the navbar when not folded.
    pub controls: usize,
    /// The wider of the user's name and role.
    pub name: f32,
    pub sign_out: bool,
}

/// A part of the navbar, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Brand,
    Host,
    Edition,
    Separator,
    Tabs,
    Spacer,
    Search,
    Control(usize),
    Menu,
    User,
    SignOut,
}

/// Between the navbar's parts.
pub fn gap() -> f32 {
    f32::from(SPACE_6)
}
/// Right of the sign-out button (or the avatar): the room it keeps from the
/// window's edge.
pub const EDGE: f32 = 12.0;
/// Room for rounding in the measured texts.
const SLACK: f32 = 6.0;
pub const LOGO: f32 = 22.0;
pub const HOST_MIN: f32 = 40.0;
pub const HOST_MAX: f32 = 200.0;
pub const SPACER_MIN: f32 = 8.0;
/// A control's transparent border, each side.
const BORDER: f32 = 1.0;

/// The padding each side of a tab.
pub fn tab_padding(fit: Fit) -> f32 {
    f32::from(if fit.tab_labels() { SPACE_5 } else { SPACE_4 })
}

fn badge(text: f32) -> f32 {
    (text + 2.0 * f32::from(SPACE_2)).max(18.0)
}

fn tab(tab: Tab, fit: Fit) -> f32 {
    let inner = f32::from(SPACE_3);
    2.0 * BORDER
        + 2.0 * tab_padding(fit)
        + f32::from(ICON_MD)
        + if fit.tab_labels() {
            inner + tab.label
        } else {
            0.0
        }
        + tab.badge.map_or(0.0, |text| inner + badge(text))
}

fn more(label: f32, fit: Fit) -> f32 {
    let inner = f32::from(SPACE_3);
    2.0 * BORDER
        + 2.0 * f32::from(SPACE_5)
        + f32::from(ICON_MD)
        + if fit.tab_labels() { inner + label } else { 0.0 }
        + inner
        + f32::from(ICON_SM)
}

fn search(parts: &Parts, fit: Fit) -> f32 {
    if !fit.search_box() {
        return f32::from(CONTROL_SM);
    }
    let inner = f32::from(SPACE_3);
    let keys = 2.0 * BORDER + 2.0 * f32::from(SPACE_2) + parts.search_keys;
    2.0 * BORDER
        + 2.0 * f32::from(SPACE_4)
        + f32::from(ICON_SM)
        + inner
        + parts.search_label
        + inner
        + keys
}

/// Each part's span `[start, end)` at its narrowest (the spacer at its
/// minimum), left to right, as the navbar lays them out at `fit`.
pub fn spans(parts: &Parts, fit: Fit) -> Vec<(Part, f32, f32)> {
    let mut widths: Vec<(Part, f32)> = Vec::new();
    let names_gap = f32::from(SPACE_4);
    widths.push((
        Part::Brand,
        LOGO + if fit.names() {
            names_gap + parts.wordmark
        } else {
            0.0
        },
    ));
    if fit.host() {
        widths.push((Part::Host, parts.host.clamp(HOST_MIN, HOST_MAX)));
        if let Some(edition) = parts.edition {
            widths.push((Part::Edition, edition + 2.0 * f32::from(SPACE_3)));
        }
    }
    widths.push((Part::Separator, 1.0));
    let mut tabs: Vec<f32> = parts.tabs.iter().map(|t| tab(*t, fit)).collect();
    if let Some(label) = parts.more {
        tabs.push(more(label, fit));
    }
    if !tabs.is_empty() {
        let between = f32::from(SPACE_2) * (tabs.len() - 1) as f32;
        widths.push((Part::Tabs, tabs.iter().sum::<f32>() + between));
    }
    widths.push((Part::Spacer, SPACER_MIN));
    widths.push((Part::Search, search(parts, fit)));
    if fit.folds_controls() {
        widths.push((Part::Menu, f32::from(CONTROL_SM)));
    } else {
        for control in 0..parts.controls {
            widths.push((Part::Control(control), f32::from(CONTROL_SM)));
        }
    }
    widths.push((Part::Separator, 1.0));
    widths.push((
        Part::User,
        LOGO + if fit.names() {
            names_gap + parts.name
        } else {
            0.0
        },
    ));
    if parts.sign_out {
        widths.push((Part::SignOut, f32::from(CONTROL_SM)));
    }
    let mut x = parts.left;
    widths
        .into_iter()
        .map(|(part, width)| {
            let span = (part, x, x + width);
            x += width + gap();
            span
        })
        .collect()
}

/// The window width the navbar needs at `fit`.
pub fn needed(parts: &Parts, fit: Fit) -> f32 {
    spans(parts, fit)
        .last()
        .map_or(parts.left, |(_, _, end)| *end)
        + EDGE
}

/// The widest step whose parts fit `available` pixels; the narrowest when
/// none does.
pub fn choose(parts: &Parts, available: f32) -> Fit {
    Fit::ALL
        .into_iter()
        .find(|fit| needed(parts, *fit) + SLACK <= available)
        .unwrap_or(Fit::NoHost)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The console's navbar for an admin, as the system font measures it.
    fn plant() -> Parts {
        let label = |chars: usize| chars as f32 * 7.2;
        Parts {
            left: 78.0,
            wordmark: 96.0,
            host: 92.0,
            edition: Some(62.0),
            tabs: [8, 4, 6, 6, 8, 9, 5]
                .into_iter()
                .enumerate()
                .map(|(index, chars)| Tab {
                    label: label(chars),
                    // The Alarms tab's count.
                    badge: (index == 2).then_some(14.0),
                })
                .collect(),
            more: Some(label(4)),
            search_label: 38.0,
            search_keys: 16.0,
            controls: 5,
            name: 64.0,
            sign_out: true,
        }
    }

    #[test]
    fn the_navbar_steps_down_in_order_as_the_window_narrows() {
        let parts = plant();
        let mut last = Fit::Full;
        let mut seen = Vec::new();
        for width in (360..=2400).rev().step_by(4) {
            let fit = choose(&parts, width as f32);
            assert!(fit >= last, "{width}: {fit:?} after {last:?}");
            last = fit;
            if seen.last() != Some(&fit) {
                seen.push(fit);
            }
        }
        assert_eq!(seen, Fit::ALL, "every step, in order");
        // Each step needs less room than the one before.
        for pair in Fit::ALL.windows(2) {
            assert!(
                needed(&parts, pair[1]) < needed(&parts, pair[0]),
                "{pair:?}"
            );
        }
    }

    #[test]
    fn at_every_width_the_parts_do_not_overlap_and_the_avatar_is_inside() {
        let parts = plant();
        let narrowest = needed(&parts, Fit::NoHost) + SLACK;
        for width in [
            narrowest, 640.0, 800.0, 1024.0, 1100.0, 1280.0, 1440.0, 1728.0, 2560.0,
        ] {
            if width < narrowest {
                continue;
            }
            let fit = choose(&parts, width);
            let spans = spans(&parts, fit);
            for pair in spans.windows(2) {
                let ((a, _, a_end), (b, b_start, _)) = (pair[0], pair[1]);
                assert!(
                    a_end + gap() <= b_start + 0.001,
                    "{width}: {a:?} runs into {b:?}"
                );
            }
            for part in [Part::User, Part::SignOut] {
                let (_, start, end) = spans
                    .iter()
                    .copied()
                    .find(|(p, _, _)| *p == part)
                    .expect("the avatar and sign-out are always there");
                assert!(start >= parts.left, "{width}: {part:?} starts at {start}");
                assert!(
                    end + EDGE <= width,
                    "{width}: {part:?} ends at {end}, clipped"
                );
            }
            // The step is the widest that fits: the one before does not.
            if let Some(wider) = Fit::ALL.iter().rev().find(|f| **f < fit) {
                assert!(
                    needed(&parts, *wider) + SLACK > width,
                    "{width}: {wider:?} would fit"
                );
            }
        }
    }

    #[test]
    fn the_steps_hide_what_they_name() {
        let parts = plant();
        let has = |fit: Fit, part: Part| spans(&parts, fit).iter().any(|(p, _, _)| *p == part);
        assert!(has(Fit::Full, Part::Control(4)) && !has(Fit::Full, Part::Menu));
        assert!(has(Fit::Folded, Part::Menu) && !has(Fit::Folded, Part::Control(0)));
        assert!(has(Fit::Folded, Part::Host) && has(Fit::Folded, Part::Edition));
        assert!(!has(Fit::NoHost, Part::Host) && !has(Fit::NoHost, Part::Edition));
        assert!(Fit::Full.tab_labels() && !Fit::IconTabs.tab_labels());
        assert!(Fit::IconTabs.names() && !Fit::NoNames.names());
        assert!(Fit::NoNames.search_box() && !Fit::SearchIcon.search_box());
        // The kiosk lock has no sign-out.
        let kiosk = Parts {
            sign_out: false,
            ..plant()
        };
        assert!(
            !spans(&kiosk, Fit::Full)
                .iter()
                .any(|(p, _, _)| *p == Part::SignOut)
        );
        assert!(needed(&kiosk, Fit::Full) < needed(&parts, Fit::Full));
    }
}
