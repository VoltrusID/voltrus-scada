//! The Data Manager page (`--page data`, the More menu, the palette): a
//! sidebar of sections grouped like the web client's `DataManager.tsx`
//! (Data, Protocols, Insights, Admin) and the chosen section beside it.
//!
//! A section is one module plus one [`SectionSpec`] row in [`SECTIONS`].
//! The sidebar lists only the rows the user's role allows, and a group only
//! when it has one, so a section that is not built is simply not listed.
//! A section keeps its state in its module (`console.state::<S>()`) and
//! names its REST calls `data.<section id>.<what>`; this page routes their
//! answers to the section's `on_reply`. See `native/docs/architecture.md`,
//! "Add a Data Manager section".
//!
//! Keys: j/k or ↑↓ in the sidebar switch sections, Enter goes into the
//! section, Esc in the section comes back to the sidebar (and Esc there
//! stays put).

pub(crate) mod analytics;
pub(crate) mod insights_kit;
pub(crate) mod oee;
pub(crate) mod recipes;
pub(crate) mod source_form;
pub(crate) mod source_model;
pub(crate) mod sources;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, FontWeight, Window, div, px};
use voltrus_scada_native::runtime::{CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::empty_state;
use vt_ui::text_input::TextInput;
use vt_ui::theme::*;

use super::{ListKeys, focus_ring, keyboard_list};
use crate::console::Console;
use crate::console::keys::Dismiss;
use crate::console::pages::{OnReply, OnShow, Render, Role, Search};

/// The sidebar's groups, in the web client's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionGroup {
    Data,
    Protocols,
    Insights,
    Admin,
}

impl SectionGroup {
    pub const ALL: [Self; 4] = [Self::Data, Self::Protocols, Self::Insights, Self::Admin];

    pub fn title(self) -> &'static str {
        match self {
            Self::Data => "Data",
            Self::Protocols => "Protocols",
            Self::Insights => "Insights",
            Self::Admin => "Admin",
        }
    }
}

/// The section came on screen (the page opened, or the sidebar moved to
/// it): fetch what it does not have yet.
pub type OnOpen = fn(&mut Console, &mut Context<Console>);

pub struct SectionSpec {
    /// The section's [`CallKey`] scope after `data.`: `data.sources.save`.
    pub id: &'static str,
    pub title: &'static str,
    pub icon: Icon,
    pub group: SectionGroup,
    /// Who sees it: the web client's `admin` items are [`Role::Operator`]
    /// (the server's `demo_protected` routes), `strictAdmin` ones
    /// [`Role::Admin`].
    pub role: Role,
    /// Draws the section beside the sidebar, header included.
    pub render: Render,
    pub on_open: Option<OnOpen>,
    /// Put focus in the section (its table), for Enter in the sidebar and
    /// the page's own focus; without it the sidebar keeps focus.
    pub focus: Option<OnShow>,
    /// Answers to the section's `data.<id>.*` calls.
    pub on_reply: Option<OnReply>,
    /// The section's search field, focused by `/`.
    pub search: Option<Search>,
}

/// Every built section, in sidebar order within its group.
pub static SECTIONS: &[SectionSpec] = &[
    SectionSpec {
        id: "sources",
        title: "Sources",
        icon: Icon::Database,
        group: SectionGroup::Data,
        role: Role::Operator,
        render: sources::render,
        on_open: Some(sources::on_open),
        focus: Some(sources::focus),
        on_reply: Some(sources::on_reply),
        search: Some(sources::search),
    },
    SectionSpec {
        id: oee::ID,
        title: "OEE",
        icon: Icon::Gauge,
        group: SectionGroup::Insights,
        role: Role::Any,
        render: oee::render,
        on_open: Some(oee::on_open),
        focus: Some(oee::focus),
        on_reply: Some(oee::on_reply),
        search: None,
    },
    SectionSpec {
        id: analytics::ID,
        title: "Analytics",
        icon: Icon::ChartLine,
        group: SectionGroup::Insights,
        role: Role::Any,
        render: analytics::render,
        on_open: Some(analytics::on_open),
        focus: Some(analytics::focus),
        on_reply: Some(analytics::on_reply),
        search: None,
    },
    SectionSpec {
        id: recipes::ID,
        title: "Recipes",
        icon: Icon::Layers,
        group: SectionGroup::Insights,
        role: Role::Operator,
        render: recipes::render,
        on_open: Some(recipes::on_open),
        focus: Some(recipes::focus),
        on_reply: Some(recipes::on_reply),
        search: Some(recipes::search),
    },
];

/// The sidebar's keyboard list.
pub(crate) const NAV: &str = "data.sections";
const SIDEBAR: f32 = 208.0;
const NAV_ROW: f32 = 30.0;

/// The width of the sidebar, for sections that size columns to the window.
pub(crate) fn sidebar_width() -> gpui::Pixels {
    px(SIDEBAR)
}

#[derive(Default)]
struct State {
    /// The chosen section's id; the first visible one until a choice.
    section: Option<&'static str>,
}

/// The sections `role` may open, grouped in sidebar order.
pub fn visible(sections: &'static [SectionSpec], role: Option<Role>) -> Vec<&'static SectionSpec> {
    let Some(role) = role else {
        return Vec::new();
    };
    SectionGroup::ALL
        .iter()
        .flat_map(|group| {
            sections
                .iter()
                .filter(move |spec| spec.group == *group && role >= spec.role)
        })
        .collect()
}

/// The section on screen.
fn current(console: &Console) -> Option<&'static SectionSpec> {
    let sections = visible(SECTIONS, console.role());
    let chosen = console.state_ref::<State>().and_then(|state| state.section);
    chosen
        .and_then(|id| sections.iter().copied().find(|spec| spec.id == id))
        .or_else(|| sections.first().copied())
}

/// The section a reply belongs to: `data.sources.save` → `sources`.
pub fn section_of(name: &str) -> Option<&str> {
    let mut parts = name.split('.');
    (parts.next() == Some("data"))
        .then(|| parts.next())
        .flatten()
}

/// Show section `id` (and fetch what it lacks); focus stays where it is.
pub(crate) fn choose(console: &mut Console, id: &'static str, cx: &mut Context<Console>) {
    console.state::<State>().section = Some(id);
    if let Some(on_open) = current(console).and_then(|spec| spec.on_open) {
        on_open(console, cx);
    }
    cx.notify();
}

/// Whether section `id` is on screen: the Data Manager shown with it
/// chosen. A section's own timers (a status poll) run only while it is.
pub(crate) fn section_shown(console: &Console, id: &str) -> bool {
    console.page == crate::console::Page::DataManager
        && current(console).is_some_and(|spec| spec.id == id)
}

/// The page's `on_show`: open the section and focus inside it.
pub fn on_show(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(spec) = current(console) else {
        window.focus(&console.focus);
        return;
    };
    if let Some(on_open) = spec.on_open {
        on_open(console, cx);
    }
    match spec.focus {
        Some(focus) => focus(console, window, cx),
        None => focus_sidebar(console, window, cx),
    }
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let handler = section_of(key.name)
        .and_then(|id| SECTIONS.iter().find(|spec| spec.id == id))
        .and_then(|spec| spec.on_reply);
    match handler {
        Some(on_reply) => on_reply(console, key, reply, cx),
        None => {
            if let Some(error) = reply.error() {
                console.show_toast(format!("{} failed: {error}", key.name), true, cx);
            }
        }
    }
}

pub fn search(console: &Console) -> Option<Entity<TextInput>> {
    current(console)
        .and_then(|spec| spec.search)
        .and_then(|search| search(console))
}

fn focus_sidebar(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let at = current(console).and_then(|spec| {
        visible(SECTIONS, console.role())
            .iter()
            .position(|other| other.id == spec.id)
    });
    let nav = console.list(NAV, cx);
    nav.cursor = at.or(nav.cursor);
    window.focus(&nav.focus);
    cx.notify();
}

/// Moving in the sidebar shows the section at once.
fn move_to(console: &mut Console, row: usize, _: &mut Window, cx: &mut Context<Console>) {
    if let Some(spec) = visible(SECTIONS, console.role()).get(row).copied() {
        choose(console, spec.id, cx);
    }
}

/// Enter or Space in the sidebar goes into the section.
fn enter(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if let Some(spec) = visible(SECTIONS, console.role()).get(row).copied() {
        choose(console, spec.id, cx);
        if let Some(focus) = spec.focus {
            focus(console, window, cx);
        }
    }
}

pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let sections = visible(SECTIONS, console.role());
    let Some(spec) = current(console) else {
        return empty_state(
            Icon::Lock,
            "Nothing to manage",
            "Managing data needs the admin, demo or integration role.",
            Vec::new(),
            cx,
        )
        .into_any_element();
    };
    let chosen = sections.iter().position(|other| other.id == spec.id);

    let sidebar = keyboard_list(
        ListKeys {
            id: NAV,
            on_move: Some(move_to),
            on_activate: enter,
        },
        sections.len(),
        sections.len(),
        console,
        cx,
    );
    let nav = console.list(NAV, cx);
    if nav.cursor.is_none() {
        nav.cursor = chosen;
    }
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));

    let mut groups = Vec::new();
    for group in SectionGroup::ALL {
        let rows: Vec<(usize, &SectionSpec)> = sections
            .iter()
            .enumerate()
            .filter(|(_, spec)| spec.group == group)
            .map(|(row, spec)| (row, *spec))
            .collect();
        if rows.is_empty() {
            continue;
        }
        groups.push(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_1)
                .child(
                    div()
                        .px(SPACE_6)
                        .pt(SPACE_6)
                        .pb(SPACE_2)
                        .text_size(FONT_CAPTION)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.fg_dim)
                        .child(group.title().to_uppercase()),
                )
                .children(rows.into_iter().map(|(row, section)| {
                    let active = section.id == spec.id;
                    let hover = theme.hover;
                    let id = section.id;
                    div()
                        .id(("data-section", row))
                        .relative()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .h(px(NAV_ROW))
                        .mx(SPACE_3)
                        .px(SPACE_4)
                        .rounded(RADIUS_MD)
                        .cursor_pointer()
                        .text_color(if active { theme.fg } else { theme.fg_muted })
                        .when(active, |this| {
                            this.bg(theme.selected).font_weight(FontWeight::MEDIUM)
                        })
                        .when(!active, |this| this.hover(move |style| style.bg(hover)))
                        .on_click(cx.listener(move |console, _, window, cx| {
                            console.list(NAV, cx).select(row);
                            choose(console, id, cx);
                            if let Some(focus) = current(console).and_then(|spec| spec.focus) {
                                focus(console, window, cx);
                            }
                        }))
                        .when(focused && cursor == Some(row), |this| {
                            this.child(focus_ring(&theme))
                        })
                        .child(vt_ui::icon(
                            section.icon,
                            ICON_MD,
                            if active { theme.accent } else { theme.fg_dim },
                        ))
                        .child(div().flex_1().min_w_0().truncate().child(section.title))
                }))
                .into_any_element(),
        );
    }

    let content = (spec.render)(console, window, cx);

    div()
        .flex()
        .size_full()
        .child(
            sidebar
                // Esc in the sidebar stays there: without this the root's
                // Esc would focus the page, that is, go into the section.
                .on_action(cx.listener(|_, _: &Dismiss, _, _| {}))
                .flex()
                .flex_col()
                .flex_none()
                .w(px(SIDEBAR))
                .h_full()
                .bg(theme.chrome)
                .border_r_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .flex_none()
                        .h(px(56.0))
                        .px(SPACE_7)
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .text_size(FONT_TITLE)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Data Manager"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_muted)
                                .child("↑↓ choose · ↵ open"),
                        ),
                )
                .children(groups),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                // Esc inside the section comes back to the sidebar; an open
                // overlay and text fields take their own Esc first.
                .on_action(cx.listener(|console, _: &Dismiss, window, cx| {
                    focus_sidebar(console, window, cx);
                }))
                .child(content),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nothing(_: &mut Console, _: &mut Window, _: &mut Context<Console>) -> AnyElement {
        unreachable!("not drawn in tests")
    }

    const fn row(id: &'static str, group: SectionGroup, role: Role) -> SectionSpec {
        SectionSpec {
            id,
            title: id,
            icon: Icon::Database,
            group,
            role,
            render: nothing,
            on_open: None,
            focus: None,
            on_reply: None,
            search: None,
        }
    }

    static TABLE: &[SectionSpec] = &[
        row("users", SectionGroup::Admin, Role::Admin),
        row("oee", SectionGroup::Insights, Role::Any),
        row("sources", SectionGroup::Data, Role::Operator),
        row("modbus", SectionGroup::Protocols, Role::Operator),
    ];

    fn ids(role: Option<Role>) -> Vec<&'static str> {
        visible(TABLE, role).iter().map(|spec| spec.id).collect()
    }

    #[test]
    fn sections_group_in_web_order_and_follow_the_role() {
        assert!(ids(None).is_empty(), "nothing before signing in");
        assert_eq!(
            ids(Some(Role::Any)),
            ["oee"],
            "a viewer sees only open sections"
        );
        assert_eq!(ids(Some(Role::Operator)), ["sources", "modbus", "oee"]);
        assert_eq!(
            ids(Some(Role::Admin)),
            ["sources", "modbus", "oee", "users"],
            "Data, Protocols, Insights, Admin"
        );
    }

    #[test]
    fn the_built_sections_are_listed_once_for_operators() {
        let ids = |role| -> Vec<&'static str> {
            visible(SECTIONS, Some(role))
                .iter()
                .map(|spec| spec.id)
                .collect()
        };
        let operator = ids(Role::Operator);
        for id in ["sources", "oee", "analytics", "recipes"] {
            assert!(operator.contains(&id), "operators see {id}");
        }
        // OEE and analytics are read by anyone; recipes (the list too) and
        // sources are operator routes.
        let viewer = ids(Role::Any);
        assert!(viewer.contains(&"oee") && viewer.contains(&"analytics"));
        assert!(!viewer.contains(&"recipes") && !viewer.contains(&"sources"));
        let mut all: Vec<_> = SECTIONS.iter().map(|spec| spec.id).collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), SECTIONS.len(), "ids are unique");
        assert!(SECTIONS.iter().all(|spec| !spec.id.contains('.')));
    }

    #[test]
    fn replies_route_by_the_second_part_of_their_key() {
        assert_eq!(section_of("data.sources.save"), Some("sources"));
        assert_eq!(section_of("data.sources"), Some("sources"));
        assert_eq!(section_of("data"), None);
        assert_eq!(section_of("alarms.summary"), None);
    }

    #[gpui::test]
    fn esc_in_the_sidebar_stays_in_the_sidebar(cx: &mut gpui::TestAppContext) {
        use crate::console::pages::Page;
        use crate::console::testing::Harness;

        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, window, cx| console.show(Page::DataManager, window, cx));
        h.draw();
        let in_sidebar = |h: &mut Harness| {
            h.update(|console, window, cx| console.list(NAV, cx).focus.is_focused(window))
        };
        assert!(!in_sidebar(&mut h), "the page opens in the section");
        h.keys("escape");
        h.draw();
        assert!(
            in_sidebar(&mut h),
            "Esc in the section comes back to the sidebar"
        );
        h.keys("escape");
        h.draw();
        assert!(in_sidebar(&mut h), "and Esc there stays put");
    }
}
