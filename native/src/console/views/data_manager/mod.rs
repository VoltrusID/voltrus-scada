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

pub(crate) mod admin_kit;
pub(crate) mod api_keys;
pub(crate) mod bacnet;
pub(crate) mod device_form;
pub(crate) mod dnp3;
pub(crate) mod edr;
pub(crate) mod enip;
pub(crate) mod flows;
pub(crate) mod imports;
pub(crate) mod modbus;
pub(crate) mod mqtt;
pub(crate) mod notification_rules;
pub(crate) mod opcua;
pub(crate) mod roles;
pub(crate) mod s7;
pub(crate) mod scripts;
pub(crate) mod snapshots;
pub(crate) mod source_form;
pub(crate) mod source_model;
pub(crate) mod sources;
pub(crate) mod system;
pub(crate) mod users;

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
    /// The web client's engineering tools (its System panel, the header's
    /// tool menu, `/flows` and `/scripts`).
    Engineering,
}

impl SectionGroup {
    pub const ALL: [Self; 5] = [
        Self::Data,
        Self::Protocols,
        Self::Insights,
        Self::Admin,
        Self::Engineering,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Data => "Data",
            Self::Protocols => "Protocols",
            Self::Insights => "Insights",
            Self::Admin => "Admin",
            Self::Engineering => "Engineering",
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
    // Protocols: Modbus reads are open to every signed-in user (writes need
    // an operator); the other protocols' handlers all check for admin.
    SectionSpec {
        id: "modbus",
        title: "Modbus",
        icon: Icon::Cable,
        group: SectionGroup::Protocols,
        role: Role::Any,
        render: modbus::render,
        on_open: Some(modbus::on_open),
        focus: Some(modbus::focus),
        on_reply: Some(modbus::on_reply),
        search: Some(modbus::search),
    },
    SectionSpec {
        id: "opcua",
        title: "OPC UA",
        icon: Icon::Network,
        group: SectionGroup::Protocols,
        role: Role::Admin,
        render: opcua::render,
        on_open: Some(opcua::on_open),
        focus: Some(opcua::focus),
        on_reply: Some(opcua::on_reply),
        search: Some(opcua::search),
    },
    SectionSpec {
        id: "s7",
        title: "Siemens S7",
        icon: Icon::Cpu,
        group: SectionGroup::Protocols,
        role: Role::Admin,
        render: s7::render,
        on_open: Some(s7::on_open),
        focus: Some(s7::focus),
        on_reply: Some(s7::on_reply),
        search: Some(s7::search),
    },
    SectionSpec {
        id: "enip",
        title: "EtherNet/IP",
        icon: Icon::Cable,
        group: SectionGroup::Protocols,
        role: Role::Admin,
        render: enip::render,
        on_open: Some(enip::on_open),
        focus: Some(enip::focus),
        on_reply: Some(enip::on_reply),
        search: Some(enip::search),
    },
    SectionSpec {
        id: "dnp3",
        title: "DNP3",
        icon: Icon::RadioTower,
        group: SectionGroup::Protocols,
        role: Role::Admin,
        render: dnp3::render,
        on_open: Some(dnp3::on_open),
        focus: Some(dnp3::focus),
        on_reply: Some(dnp3::on_reply),
        search: Some(dnp3::search),
    },
    SectionSpec {
        id: "bacnet",
        title: "BACnet",
        icon: Icon::Factory,
        group: SectionGroup::Protocols,
        role: Role::Admin,
        render: bacnet::render,
        on_open: Some(bacnet::on_open),
        focus: Some(bacnet::focus),
        on_reply: Some(bacnet::on_reply),
        search: Some(bacnet::search),
    },
    SectionSpec {
        id: "users",
        title: "Users",
        icon: Icon::CircleDot,
        group: SectionGroup::Admin,
        role: Role::Admin,
        render: users::render,
        on_open: Some(users::on_open),
        focus: Some(users::focus),
        on_reply: Some(users::on_reply),
        search: Some(users::search),
    },
    SectionSpec {
        id: "roles",
        title: "Roles",
        icon: Icon::Layers,
        group: SectionGroup::Admin,
        role: Role::Admin,
        render: roles::render,
        on_open: Some(roles::on_open),
        focus: Some(roles::focus),
        on_reply: Some(roles::on_reply),
        search: Some(roles::search),
    },
    SectionSpec {
        id: "notifications",
        title: "Notifications",
        icon: Icon::Send,
        group: SectionGroup::Admin,
        role: Role::Operator,
        render: notification_rules::render,
        on_open: Some(notification_rules::on_open),
        focus: Some(notification_rules::focus),
        on_reply: Some(notification_rules::on_reply),
        search: Some(notification_rules::search),
    },
    SectionSpec {
        id: "api-keys",
        title: "API keys",
        icon: Icon::Lock,
        group: SectionGroup::Admin,
        role: Role::Admin,
        render: api_keys::render,
        on_open: Some(api_keys::on_open),
        focus: Some(api_keys::focus),
        on_reply: Some(api_keys::on_reply),
        search: Some(api_keys::search),
    },
    SectionSpec {
        id: "system",
        title: "System",
        icon: Icon::Server,
        group: SectionGroup::Engineering,
        role: Role::Any,
        render: system::render,
        on_open: Some(system::on_open),
        focus: Some(system::focus),
        on_reply: Some(system::on_reply),
        search: Some(system::search),
    },
    SectionSpec {
        id: "snapshots",
        title: "Config snapshots",
        icon: Icon::History,
        group: SectionGroup::Engineering,
        role: Role::Operator,
        render: snapshots::render,
        on_open: Some(snapshots::on_open),
        focus: Some(snapshots::focus),
        on_reply: Some(snapshots::on_reply),
        search: Some(snapshots::search),
    },
    SectionSpec {
        id: "mqtt",
        title: "MQTT broker",
        icon: Icon::RadioTower,
        group: SectionGroup::Engineering,
        role: Role::Any,
        render: mqtt::render,
        on_open: Some(mqtt::on_open),
        focus: Some(mqtt::focus),
        on_reply: Some(mqtt::on_reply),
        search: Some(mqtt::search),
    },
    SectionSpec {
        id: "edr",
        title: "EDR compatibility",
        icon: Icon::Lock,
        group: SectionGroup::Engineering,
        role: Role::Any,
        render: edr::render,
        on_open: Some(edr::on_open),
        focus: Some(edr::focus),
        on_reply: Some(edr::on_reply),
        search: Some(edr::search),
    },
    SectionSpec {
        id: "imports",
        title: "Import",
        icon: Icon::Upload,
        group: SectionGroup::Engineering,
        role: Role::Operator,
        render: imports::render,
        on_open: Some(imports::on_open),
        focus: Some(imports::focus),
        on_reply: Some(imports::on_reply),
        search: Some(imports::search),
    },
    SectionSpec {
        id: "flows",
        title: "Flows",
        icon: Icon::Waypoints,
        group: SectionGroup::Engineering,
        role: Role::Operator,
        render: flows::render,
        on_open: Some(flows::on_open),
        focus: Some(flows::focus),
        on_reply: Some(flows::on_reply),
        search: Some(flows::search),
    },
    SectionSpec {
        id: "scripts",
        title: "Scripts",
        icon: Icon::Terminal,
        group: SectionGroup::Engineering,
        role: Role::Operator,
        render: scripts::render,
        on_open: Some(scripts::on_open),
        focus: Some(scripts::focus),
        on_reply: Some(scripts::on_reply),
        search: Some(scripts::search),
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

/// Whether `role` sees any section: the page is offered (More menu,
/// palette, `--page`) only then, never as an empty page.
pub fn has_sections(role: Option<Role>) -> bool {
    !visible(SECTIONS, role).is_empty()
}

/// The section on screen.
fn current(console: &Console) -> Option<&'static SectionSpec> {
    let sections = visible(SECTIONS, console.role());
    let chosen = console.state_ref::<State>().and_then(|state| state.section);
    chosen
        .and_then(|id| sections.iter().copied().find(|spec| spec.id == id))
        .or_else(|| sections.first().copied())
}

/// Whether section `id` is on screen (the Data Manager shown, the section
/// chosen), for a section's own timers.
pub(crate) fn section_shown(console: &Console, id: &str) -> bool {
    console.page == crate::console::pages::Page::DataManager
        && current(console).is_some_and(|spec| spec.id == id)
}

/// The id of the section on screen (whether or not the page is), for
/// sections that poll: fetch only while it is the one shown.
pub(crate) fn current_id(console: &Console) -> Option<&'static str> {
    current(console).map(|spec| spec.id)
}

/// `--section <id>`: show that section when the page opens (if the role
/// allows it; otherwise the first the user may open).
pub(crate) fn launch(console: &mut Console, id: &str) {
    if let Some(spec) = SECTIONS.iter().find(|spec| spec.id == id) {
        console.state::<State>().section = Some(spec.id);
    }
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
            "No sections for your role",
            "Ask an administrator for the operator role to manage data.",
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
    fn the_built_sections_are_listed_once_and_follow_their_role() {
        let any: Vec<_> = visible(SECTIONS, Some(Role::Any))
            .iter()
            .map(|spec| spec.id)
            .collect();
        for id in ["system", "mqtt", "edr"] {
            assert!(any.contains(&id), "{id} is open to every user");
        }
        assert!(!any.contains(&"sources") && !any.contains(&"flows"));
        let operator: Vec<_> = visible(SECTIONS, Some(Role::Operator))
            .iter()
            .map(|spec| spec.id)
            .collect();
        for id in ["sources", "snapshots", "imports", "flows", "scripts"] {
            assert!(operator.contains(&id), "{id} is for operators");
        }
        // The Engineering sections come last, in this order.
        let engineering: Vec<_> = operator
            .iter()
            .copied()
            .filter(|id| {
                SECTIONS
                    .iter()
                    .any(|spec| spec.id == *id && spec.group == SectionGroup::Engineering)
            })
            .collect();
        assert_eq!(
            engineering,
            [
                "system",
                "snapshots",
                "mqtt",
                "edr",
                "imports",
                "flows",
                "scripts"
            ]
        );
        assert_eq!(operator.last(), Some(&"scripts"));
        assert!(operator.contains(&"sources") && operator.contains(&"notifications"));
        assert!(
            !operator
                .iter()
                .any(|id| ["users", "roles", "api-keys"].contains(id)),
            "users, roles and API keys are strictly admin"
        );
        let admin: Vec<_> = visible(SECTIONS, Some(Role::Admin))
            .iter()
            .map(|spec| spec.id)
            .collect();
        let tail: Vec<_> = admin
            .iter()
            .copied()
            .filter(|id| ["users", "roles", "notifications", "api-keys"].contains(id))
            .collect();
        assert_eq!(
            tail,
            ["users", "roles", "notifications", "api-keys"],
            "the web's Admin order"
        );
        // Protocols: Modbus reads are open to all; the other protocols'
        // handlers answer only admins.
        assert!(any.contains(&"modbus"));
        for admin_only in ["opcua", "s7", "enip", "dnp3", "bacnet"] {
            assert!(!operator.contains(&admin_only), "{admin_only}");
            assert!(admin.contains(&admin_only), "{admin_only}");
        }
        let protocols: Vec<_> = admin
            .iter()
            .copied()
            .filter(|id| {
                SECTIONS
                    .iter()
                    .any(|spec| spec.id == *id && spec.group == SectionGroup::Protocols)
            })
            .collect();
        assert_eq!(
            protocols,
            ["modbus", "opcua", "s7", "enip", "dnp3", "bacnet"]
        );
        let mut all: Vec<_> = SECTIONS.iter().map(|spec| spec.id).collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), SECTIONS.len(), "ids are unique");
        assert!(SECTIONS.iter().all(|spec| !spec.id.contains('.')));
    }

    #[test]
    fn the_page_is_offered_only_with_a_section_to_show() {
        assert!(!has_sections(None));
        for role in [Role::Any, Role::Operator, Role::Admin] {
            assert!(has_sections(Some(role)), "{role:?} sees a section");
        }
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
