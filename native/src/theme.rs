//! Voltrus brand theme — mirrors the web client's zinc-950 dark palette
//! (`client/src/index.css`) so the native console and the web dashboard read
//! as one product.

use gpui::{App, px, rgb};
use gpui_component::Theme;

// ── Palette (see client/src/index.css `:root`) ──
pub const BG: u32 = 0x09090b; // --bg (zinc-950)
pub const BG_ELEVATED: u32 = 0x111113; // --card
const BG_TITLE: u32 = 0x0c0c0e; // between bg and card
pub const BORDER: u32 = 0x27272a; // --border (zinc-800)
const BORDER_LIGHT: u32 = 0x3f3f46; // --border-light (zinc-700)
pub const TEXT: u32 = 0xe4e4e7; // --text (zinc-200)
pub const TEXT_DIM: u32 = 0xa1a1aa; // --text-dim (zinc-400)
pub const TEXT_MUTED: u32 = 0x71717a; // --text-muted (zinc-500)
pub const WARN: u32 = 0xf59e0b; // --warn (amber-500)
pub const CRIT: u32 = 0xef4444; // --crit (red-500)
pub const OK: u32 = 0x22c55e; // --ok (green-500)
const OK_DARK: u32 = 0x16a34a; // green-600
const OK_DARKER: u32 = 0x15803d; // green-700
const INFO: u32 = 0x3b82f6; // web `info` (blue-500)

/// Apply after `gpui_component::init` and `Theme::change(ThemeMode::Dark, ..)`
/// — `change` resets colors, so brand overrides must come last.
pub fn apply(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    let c = &mut theme.colors;

    // Surfaces
    c.background = rgb(BG).into();
    c.popover = rgb(BG_ELEVATED).into();
    c.secondary = rgb(BG_ELEVATED).into();
    c.secondary_hover = rgb(0x18181b).into(); // zinc-900
    c.border = rgb(BORDER).into();
    c.title_bar = rgb(BG_TITLE).into();
    c.title_bar_border = rgb(BORDER).into();
    c.tab_bar = rgb(BG_TITLE).into();
    c.tab_bar_segmented = rgb(BG_ELEVATED).into();
    c.sidebar = rgb(BG_TITLE).into();
    c.sidebar_border = rgb(BORDER).into();
    c.input = rgb(BG).into();
    c.ring = rgb(BORDER_LIGHT).into();
    c.selection = rgb(BORDER).into();
    c.group_box = rgb(BG_ELEVATED).into();

    // Text
    c.foreground = rgb(TEXT).into();
    c.muted_foreground = rgb(TEXT_MUTED).into();
    c.popover_foreground = rgb(TEXT).into();

    // Voltrus green as the interactive accent (live/ok color).
    c.accent = rgb(OK).into();
    c.accent_foreground = rgb(BG).into();
    c.primary = rgb(OK).into();
    c.primary_foreground = rgb(BG).into();
    c.primary_hover = rgb(OK_DARK).into();
    c.primary_active = rgb(OK_DARKER).into();

    // Semantic — same values the web client uses.
    c.danger = rgb(CRIT).into();
    c.danger_hover = rgb(0xdc2626).into(); // red-600
    c.danger_foreground = rgb(0xffffff).into();
    c.red = rgb(CRIT).into();
    c.yellow = rgb(WARN).into();
    c.green = rgb(OK).into();
    c.blue = rgb(INFO).into();

    // Charts: first series is the Voltrus green trend line.
    c.chart_1 = rgb(OK).into();
    c.chart_2 = rgb(INFO).into();
    c.chart_3 = rgb(WARN).into();
    c.chart_4 = rgb(0xa78bfa).into(); // violet-400, web low-alarm accent
    c.chart_5 = rgb(TEXT_DIM).into();

    // Typography — macOS system stack, matching the web client's UI spec:
    // SF Pro Text 13px for UI sans, SF Mono 12px for data/mono readouts
    // (Menlo is the fallback on machines without SF Mono).
    theme.font_family = "SF Pro Text".into();
    theme.mono_font_family = "SF Mono".into();
    theme.font_size = px(13.);
}
