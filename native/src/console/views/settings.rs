//! The settings sheet (⌘,), the theme picker it opens and the keyboard
//! shortcut editor. The state and the server calls are in
//! `console::shell`.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, SharedString, Window, div, px,
    uniform_list,
};
use voltrus_scada_native::api::shell::{LANGUAGES, ThemeMode};
use vt_ui::Icon;
use vt_ui::components::{Button, Segment, kbd, pill, section_label, segmented};
use vt_ui::theme::*;

use super::{ListKeys, focus_ring, host, keyboard_list};
use crate::console::Console;
use crate::console::keys::{Dismiss, FocusSearch, ShellOpenSettings, hint};
use crate::console::prefs::AGGREGATIONS;
use crate::console::shell::{self, OS_NOTIFICATIONS, Shell};
use crate::console::shortcuts::{self, Entry, NAV_TARGETS, Skipped};

pub(crate) const THEMES_LIST: &str = "settings-themes";
pub(crate) const SHORTCUTS_LIST: &str = "settings-shortcuts";

const PICKER_ROW: f32 = 34.0;
const SHORTCUT_ROW: f32 = 34.0;
/// Rows the picker and the editor show at once.
const VISIBLE_ROWS: f32 = 10.0;

/// A segmented control's click, as a console listener.
fn on_pick(
    cx: &mut Context<Console>,
    pick: impl Fn(&mut Console, &mut Window, &mut Context<Console>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static {
    cx.listener(move |console, _: &ClickEvent, window, cx| pick(console, window, cx))
}

/// One setting: its name and a line about it on the left, the control on
/// the right.
fn setting(
    title: &'static str,
    about: impl Into<SharedString>,
    control: AnyElement,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(SPACE_7)
        .py(SPACE_5)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(SPACE_1)
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(about.into()),
                ),
        )
        .child(div().flex_none().child(control))
}

/// A setting whose control is too wide to sit beside its name.
fn wide_setting(
    title: &'static str,
    about: impl Into<SharedString>,
    control: AnyElement,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_3)
        .py(SPACE_5)
        .child(div().font_weight(FontWeight::MEDIUM).child(title))
        .child(control)
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(about.into()),
        )
}

fn section(title: &'static str, rows: Vec<Div>, cx: &Context<Console>) -> Div {
    let theme = theme(cx);
    div()
        .flex()
        .flex_col()
        .child(div().pb(SPACE_2).child(section_label(title, cx)))
        .child(
            div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.border)
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    row.when(index > 0, |row| row.border_t_1().border_color(theme.border))
                })),
        )
}

/// Off and On.
fn switch(
    id: &'static str,
    on: bool,
    cx: &mut Context<Console>,
    set: fn(&mut Console, bool, &mut Context<Console>),
) -> AnyElement {
    segmented(
        id,
        vec![
            Segment::new(
                "Off",
                !on,
                on_pick(cx, move |console, _, cx| set(console, false, cx)),
            ),
            Segment::new(
                "On",
                on,
                on_pick(cx, move |console, _, cx| set(console, true, cx)),
            ),
        ],
        cx,
    )
}

/// A theme's colours in a row of small squares.
fn swatches(colors: &[Hsla], theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(3.0))
        .children(colors.iter().map(|color| {
            div()
                .size(px(10.0))
                .rounded(px(2.0))
                .bg(*color)
                .border_1()
                .border_color(theme.border)
        }))
}

fn theme_choice(console: &mut Console, dark: bool, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let name = console.state::<Shell>().prefs.theme_for(dark).to_owned();
    let colors = shell::themes(console)
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.swatches.to_vec())
        .unwrap_or_default();
    div()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .child(swatches(&colors, &theme))
        .child(
            Button::new(
                if dark {
                    "pick-dark-theme"
                } else {
                    "pick-light-theme"
                },
                "Change…",
            )
            .compact()
            .on_click(cx.listener(move |console, _, window, cx| {
                shell::open_picker(console, dark, window, cx);
            })),
        )
        .into_any_element()
}

fn header(title: &'static str, keys: Option<String>, cx: &Context<Console>) -> Div {
    let theme = theme(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .px(SPACE_8)
        .h(px(52.0))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(FONT_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .when_some(keys, |this, keys| this.child(kbd(keys, cx)))
}

fn footer(keys: &[(&'static str, &'static str)], note: String, cx: &Context<Console>) -> Div {
    let theme = theme(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_6)
        .px(SPACE_6)
        .h(px(34.0))
        .border_t_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_dim)
        .children(keys.iter().map(|(keys, label)| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_2)
                .child(kbd(*keys, cx))
                .child(*label)
        }))
        .child(div().flex_1())
        .child(note)
}

/// ⌘,: everything the operator can set, top to bottom.
pub fn render(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let username = console
        .user
        .as_ref()
        .map_or_else(String::new, |(name, _)| name.clone());
    let operator = console.is_operator();
    let history_agg = console.history_agg();
    let (mode, pinned, lang, cooldown, cooldown_input, prefs, license, bound, kept) = {
        let shell = console.state::<Shell>();
        let kept = shell.plan.skipped.len();
        (
            shell.mode,
            shell.pinned,
            shell.lang.clone(),
            shell.cooldown,
            shell.cooldown_input.clone(),
            shell.prefs.clone(),
            shell.license.clone(),
            shell.plan.bound.len(),
            kept,
        )
    };
    let shown_mode = mode.unwrap_or(if theme.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    });

    let appearance = segmented(
        "settings-appearance",
        ThemeMode::ALL
            .into_iter()
            .map(|choice| {
                Segment::new(
                    choice.label(),
                    choice == shown_mode,
                    on_pick(cx, move |console, _, cx| {
                        shell::set_mode(console, choice, cx)
                    }),
                )
                .icon(match choice {
                    ThemeMode::Light => Icon::Sun,
                    ThemeMode::Dark => Icon::Moon,
                    ThemeMode::System => Icon::Gauge,
                })
            })
            .collect(),
        cx,
    );
    let appearance_about = match pinned {
        Some(dark) if mode.is_some() => format!(
            "Started with --appearance {}; your choice on the server is {}",
            if dark { "dark" } else { "light" },
            shown_mode.label().to_lowercase()
        ),
        _ => format!("Kept on the server for {username}"),
    };
    let appearance_rows = vec![
        setting("Appearance", appearance_about, appearance, &theme),
        setting(
            "Dark theme",
            prefs.dark_theme.clone(),
            theme_choice(console, true, cx),
            &theme,
        ),
        setting(
            "Light theme",
            prefs.light_theme.clone(),
            theme_choice(console, false, cx),
            &theme,
        ),
    ];

    let language = segmented(
        "settings-language",
        LANGUAGES
            .iter()
            .map(|&(code, name)| {
                Segment::new(
                    name,
                    lang.as_deref() == Some(code),
                    on_pick(cx, move |console, _, cx| shell::set_lang(console, code, cx)),
                )
            })
            .collect(),
        cx,
    );

    let cooldown_control = if operator {
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .children(cooldown_input.map(|input| div().w(px(84.0)).child(input)))
            .child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child("s"),
            )
            .child(
                Button::new("save-cooldown", "Save")
                    .compact()
                    .on_click(cx.listener(|console, _, _, cx| shell::save_cooldown(console, cx))),
            )
            .into_any_element()
    } else {
        div()
            .font_family(MONO)
            .child(cooldown.map_or_else(|| "—".to_owned(), |secs| format!("{secs} s")))
            .into_any_element()
    };
    let mut alarm_rows = vec![setting(
        "Alarm cooldown",
        if operator {
            "After an acknowledgement the same alarm waits this long before it is raised again; 0 turns it off. For every user."
        } else {
            "After an acknowledgement the same alarm waits this long before it is raised again. Operators (admin, demo, integration) change it."
        },
        cooldown_control,
        &theme,
    )];
    if OS_NOTIFICATIONS {
        alarm_rows.push(setting(
            "System notifications",
            "A notification for each new critical alarm, also while the console is in the background.",
            switch("settings-os-notifications", prefs.os_notifications, cx, shell::set_os_notifications),
            &theme,
        ));
    }

    let aggregation = segmented(
        "settings-aggregation",
        AGGREGATIONS
            .iter()
            .map(|&(id, label, _)| {
                Segment::new(
                    label,
                    history_agg == id,
                    on_pick(cx, move |console, _, cx| {
                        shell::set_history_agg(console, id, cx)
                    }),
                )
            })
            .collect(),
        cx,
    );
    let aggregation_about = AGGREGATIONS
        .iter()
        .find(|(id, _, _)| *id == history_agg)
        .map_or("", |(_, _, about)| about);

    let shortcuts_row = setting(
        "Keyboard shortcuts",
        match kept {
            0 => format!("{bound} of yours work here; kept on the server with the web client's"),
            kept => format!("{bound} of yours work here, {kept} only in the web client or taken"),
        },
        Button::new("edit-shortcuts", "Edit…")
            .compact()
            .icon(Icon::Keyboard)
            .on_click(cx.listener(|console, _, window, cx| {
                shell::open_shortcuts(console, window, cx);
            }))
            .into_any_element(),
        &theme,
    );
    let kiosk_row = setting(
        "Kiosk lock",
        "Blocks quitting, signing out and closing the window on this workstation.",
        switch("settings-kiosk", prefs.kiosk, cx, shell::set_kiosk),
        &theme,
    );

    let about_line = |label: &'static str, value: String| {
        div()
            .flex()
            .items_center()
            .gap(SPACE_6)
            .py(SPACE_3)
            .child(
                div()
                    .w(px(120.0))
                    .flex_none()
                    .text_color(theme.fg_muted)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(MONO)
                    .child(value),
            )
    };
    let edition = license.as_ref().map_or_else(
        || "—".to_owned(),
        |license| match license.note() {
            Some(note) => format!("{} · {note}", license.tier().label()),
            None => license.tier().label().to_owned(),
        },
    );
    let unlocked = license
        .as_ref()
        .map(|license| license.unlocked().join(", "))
        .filter(|list| !list.is_empty())
        .unwrap_or_else(|| "none (open core)".to_owned());
    let about = div()
        .flex()
        .flex_col()
        .pt(SPACE_3)
        .border_t_1()
        .border_color(theme.border)
        .text_size(FONT_SMALL)
        .child(about_line("Console", env!("CARGO_PKG_VERSION").to_owned()))
        .child(about_line(
            "Server",
            console.server_version.as_ref().map_or_else(
                || host(&console.server).to_owned(),
                |version| format!("{version} · {}", host(&console.server)),
            ),
        ))
        .child(about_line("Edition", edition))
        .child(about_line("Add-on features", unlocked));

    let body = div()
        .id("settings-body")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&console.scroll("settings"))
        .flex()
        .flex_col()
        .gap(SPACE_8)
        .px(SPACE_8)
        .py(SPACE_7)
        .child(section("Appearance", appearance_rows, cx))
        .child(section(
            "Language",
            vec![wide_setting(
                "Language",
                "Kept on the server for the web client; this console's own text stays in English.",
                language,
                &theme,
            )],
            cx,
        ))
        .child(section("Alarms", alarm_rows, cx))
        .child(section(
            "History",
            vec![wide_setting(
                "Aggregation",
                format!("{aggregation_about} The default for history this workstation asks for."),
                aggregation,
                &theme,
            )],
            cx,
        ))
        .child(section(
            "Keyboard and kiosk",
            vec![shortcuts_row, kiosk_row],
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .child(section_label("About", cx))
                .child(about),
        );

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(header("Settings", hint(&ShellOpenSettings), cx))
        .child(body)
        .child(footer(
            &[("Tab", "Next control"), ("Esc", "Close")],
            String::new(),
            cx,
        ))
        .into_any_element()
}

// ── Theme picker ──────────────────────────────────────────────────────────

fn look_at_theme(console: &mut Console, row: usize, _: &mut Window, cx: &mut Context<Console>) {
    shell::preview_theme(console, row, cx);
}

fn choose_theme(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    shell::choose_theme(console, row, window, cx);
}

/// Every bundled theme of one brightness: moving the cursor shows it, Enter
/// chooses it, Esc goes back with the theme before.
pub fn themes(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some((dark, filter)) = console
        .state::<Shell>()
        .picker
        .as_ref()
        .map(|picker| (picker.dark, picker.filter.clone()))
    else {
        return div().into_any_element();
    };
    let field = filter.clone();
    let chosen = console.state::<Shell>().prefs.theme_for(dark).to_owned();
    let rows = shell::picker_rows(console, cx);
    let entries: Vec<(SharedString, [Hsla; 5])> = {
        let all = shell::themes(console);
        rows.iter()
            .map(|index| {
                (
                    SharedString::from(all[*index].name.clone()),
                    all[*index].swatches,
                )
            })
            .collect()
    };
    let count = entries.len();
    let list = keyboard_list(
        ListKeys {
            id: THEMES_LIST,
            on_move: Some(look_at_theme),
            on_activate: choose_theme,
        },
        count,
        VISIBLE_ROWS as usize,
        console,
        cx,
    );
    let nav = console.list(THEMES_LIST, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let rows_view = uniform_list(
        "theme-rows",
        count,
        cx.processor(
            move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = vt_ui::theme::theme(cx).clone();
                range
                    .filter_map(|row| {
                        let (name, colors) = entries.get(row)?;
                        let active = cursor == Some(row);
                        let hover = theme.hover;
                        Some(
                            div()
                                .id(("theme-row", row))
                                .relative()
                                .flex()
                                .items_center()
                                .gap(SPACE_5)
                                .h(px(PICKER_ROW))
                                .w_full()
                                .px(SPACE_5)
                                .rounded(RADIUS_MD)
                                .cursor_pointer()
                                .when(active, |this| this.bg(theme.selected))
                                .when(!active, |this| this.hover(move |style| style.bg(hover)))
                                .on_click(cx.listener(move |console, _, window, cx| {
                                    shell::choose_theme(console, row, window, cx);
                                }))
                                .child(swatches(colors, &theme))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(name.clone()),
                                )
                                .when(**name == *chosen, |this| {
                                    this.child(vt_ui::icon(Icon::Check, ICON_MD, theme.accent))
                                })
                                .when(focused && active, |this| this.child(focus_ring(&theme))),
                        )
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .px(SPACE_3)
    .h(px(PICKER_ROW * VISIBLE_ROWS));

    div()
        .flex()
        .flex_col()
        // Esc anywhere in the picker goes back to the settings, putting
        // the theme before back; `/` finds.
        .on_action(cx.listener(|console, _: &Dismiss, window, cx| {
            shell::cancel_picker(console, window, cx);
        }))
        .on_action(cx.listener(move |_, _: &FocusSearch, window, cx| {
            field.update(cx, |field, _| field.focus(window));
        }))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .p(SPACE_6)
                .border_b_1()
                .border_color(theme.border)
                .child(vt_ui::icon(Icon::Palette, ICON_LG, theme.fg_dim))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if dark { "Dark theme" } else { "Light theme" }),
                )
                .child(div().flex_1().child(filter)),
        )
        .child(list.py(SPACE_3).child(if count == 0 {
            div()
                .h(px(PICKER_ROW * 2.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_dim)
                .child("No theme matches")
                .into_any_element()
        } else {
            rows_view.into_any_element()
        }))
        .child(footer(
            &[
                ("↑↓", "Look"),
                ("↵", "Choose"),
                ("/", "Find"),
                ("Esc", "Back"),
            ],
            format!("{count} {} themes", if dark { "dark" } else { "light" }),
            cx,
        ))
        .into_any_element()
}

// ── Shortcut editor ───────────────────────────────────────────────────────

/// One editor row.
pub struct ShortcutRow {
    pub entry: Entry,
    pub kind: &'static str,
    pub label: String,
    pub detail: String,
    pub combo: String,
    pub skipped: Option<Skipped>,
}

/// The editor's rows: navigation targets in the web's order (then the
/// server's screens), then every tag, filtered by the search.
pub fn shortcut_rows(console: &mut Console, cx: &gpui::App) -> Vec<ShortcutRow> {
    let query = console
        .state::<Shell>()
        .editor
        .as_ref()
        .map(|editor| editor.filter.read(cx).value().trim().to_lowercase())
        .unwrap_or_default();
    let shell = console.state::<Shell>();
    let mut targets: Vec<String> = NAV_TARGETS.iter().map(|(id, _)| (*id).to_owned()).collect();
    targets.extend(
        shell
            .nav
            .keys()
            .filter(|target| !NAV_TARGETS.iter().any(|(id, _)| id == target))
            .cloned(),
    );
    let mut rows: Vec<ShortcutRow> = targets
        .into_iter()
        .map(|target| ShortcutRow {
            kind: "Navigation",
            label: shortcuts::nav_label(&target),
            combo: shell.nav.get(&target).cloned().unwrap_or_default(),
            skipped: shell.plan.skipped.get(&format!("nav:{target}")).cloned(),
            detail: target.clone(),
            entry: Entry::Nav(target),
        })
        .collect();
    let tags: Vec<(i64, String, String)> = console
        .store
        .tags
        .iter()
        .map(|tag| (tag.info.id, tag.info.name.clone(), tag.info.key.clone()))
        .collect();
    let shell = console.state::<Shell>();
    rows.extend(tags.into_iter().map(|(id, name, key)| ShortcutRow {
        kind: "Tag",
        label: if name.is_empty() { key.clone() } else { name },
        combo: shell.tags.get(&id.to_string()).cloned().unwrap_or_default(),
        skipped: shell.plan.skipped.get(&format!("tag:{id}")).cloned(),
        detail: key,
        entry: Entry::Tag(id),
    }));
    rows.retain(|row| {
        query.is_empty()
            || row.label.to_lowercase().contains(&query)
            || row.detail.to_lowercase().contains(&query)
            || row.combo.to_lowercase().contains(&query)
    });
    rows
}

fn activate_shortcut(
    console: &mut Console,
    row: usize,
    clear: bool,
    _: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(found) = shortcut_rows(console, cx).into_iter().nth(row) else {
        return;
    };
    if clear {
        if !found.combo.is_empty() {
            shortcuts::save(console, &found.entry, None, cx);
        }
    } else {
        shell::record(console, found.entry, cx);
    }
}

/// The server's shortcuts for navigation and tags: Enter records new keys,
/// Space clears.
pub fn shortcuts(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(filter) = console
        .state::<Shell>()
        .editor
        .as_ref()
        .map(|editor| editor.filter.clone())
    else {
        return div().into_any_element();
    };
    let recording = console.state::<Shell>().recording.clone();
    let username = console
        .user
        .as_ref()
        .map_or_else(String::new, |(name, _)| name.clone());
    let rows = shortcut_rows(console, cx);
    let count = rows.len();
    let bound = console.state::<Shell>().plan.bound.len();
    let list = keyboard_list(
        ListKeys {
            id: SHORTCUTS_LIST,
            on_move: None,
            on_activate: activate_shortcut,
        },
        count,
        VISIBLE_ROWS as usize,
        console,
        cx,
    );
    let nav = console.list(SHORTCUTS_LIST, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    let rows_view = uniform_list(
        "shortcut-rows",
        count,
        cx.processor(
            move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = vt_ui::theme::theme(cx).clone();
                range
                    .filter_map(|index| {
                        let row = rows.get(index)?;
                        let active = cursor == Some(index);
                        let waiting = recording.as_ref() == Some(&row.entry);
                        let hover = theme.hover;
                        let keys: AnyElement = if waiting {
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.accent)
                                .child("Press keys…")
                                .into_any_element()
                        } else if row.combo.is_empty() {
                            div().text_color(theme.fg_dim).child("—").into_any_element()
                        } else {
                            kbd(shortcuts::display(&row.combo), cx).into_any_element()
                        };
                        let status: AnyElement = match (&row.skipped, waiting) {
                            (_, true) => div()
                                .text_color(theme.fg_dim)
                                .child("Esc cancels · ⌫ clears")
                                .into_any_element(),
                            (Some(Skipped::WebOnly), _) => div()
                                .text_color(theme.fg_dim)
                                .child(Skipped::WebOnly.reason())
                                .into_any_element(),
                            (Some(skipped), _) => div()
                                .text_color(theme.warning)
                                .child(skipped.reason())
                                .into_any_element(),
                            (None, _) if row.combo.is_empty() => div().into_any_element(),
                            (None, _) => pill("Works here", theme.success).into_any_element(),
                        };
                        Some(
                            div()
                                .id(("shortcut-row", index))
                                .relative()
                                .flex()
                                .items_center()
                                .gap(SPACE_5)
                                .h(px(SHORTCUT_ROW))
                                .w_full()
                                .px(SPACE_5)
                                .rounded(RADIUS_MD)
                                .cursor_pointer()
                                .when(active || waiting, |this| this.bg(theme.selected))
                                .when(!active && !waiting, |this| {
                                    this.hover(move |style| style.bg(hover))
                                })
                                .on_click(cx.listener(move |console, _, window, cx| {
                                    console.list(SHORTCUTS_LIST, cx).select(index);
                                    activate_shortcut(console, index, false, window, cx);
                                }))
                                .child(
                                    div()
                                        .w(px(78.0))
                                        .flex_none()
                                        .text_size(FONT_CAPTION)
                                        .text_color(theme.fg_dim)
                                        .child(row.kind),
                                )
                                .child(
                                    div().flex().flex_col().flex_1().min_w_0().child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(row.label.clone()),
                                    ),
                                )
                                .child(
                                    div()
                                        .w(px(150.0))
                                        .flex_none()
                                        .truncate()
                                        .font_family(MONO)
                                        .text_size(FONT_CAPTION)
                                        .text_color(theme.fg_dim)
                                        .child(row.detail.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(96.0))
                                        .flex_none()
                                        .flex()
                                        .justify_end()
                                        .child(keys),
                                )
                                .child(
                                    div()
                                        .w(px(150.0))
                                        .flex_none()
                                        .flex()
                                        .justify_end()
                                        .truncate()
                                        .text_size(FONT_CAPTION)
                                        .child(status),
                                )
                                .when(focused && active, |this| this.child(focus_ring(&theme))),
                        )
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .px(SPACE_3)
    .h(px(SHORTCUT_ROW * VISIBLE_ROWS));

    let field = filter.clone();
    div()
        .flex()
        .flex_col()
        .on_action(cx.listener(move |_, _: &FocusSearch, window, cx| {
            field.update(cx, |field, _| field.focus(window));
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(SPACE_4)
                .p(SPACE_6)
                .border_b_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .child(vt_ui::icon(Icon::Keyboard, ICON_LG, theme.fg_dim))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Keyboard shortcuts"),
                                )
                                .child(
                                    div()
                                        .text_size(FONT_SMALL)
                                        .text_color(theme.fg_muted)
                                        .child(format!(
                                            "Kept on the server for {username} and shared with the web client"
                                        )),
                                ),
                        ),
                )
                .child(filter),
        )
        .child(list.py(SPACE_3).child(if count == 0 {
            div()
                .h(px(SHORTCUT_ROW * 2.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.fg_dim)
                .child("Nothing matches")
                .into_any_element()
        } else {
            rows_view.into_any_element()
        }))
        .child(footer(
            &[("↵", "Record"), ("Space", "Clear"), ("/", "Find"), ("Esc", "Close")],
            format!("{bound} bound here · {count} shown"),
            cx,
        ))
        .into_any_element()
}
