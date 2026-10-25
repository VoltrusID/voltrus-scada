//! The page registry: every page of the console in one table.
//!
//! A new page is a new module under `views/` plus one [`PageSpec`] row in
//! [`PAGES`] and one [`Page`] variant. The row gives everything the shell
//! needs: the navbar tab (or the More menu), the ⌘ digit, the View menu
//! item, the palette entry, the help overlay line, `--page <id>`, who may
//! open it, how it draws, and where its REST replies go. See
//! `native/docs/architecture.md`.

use gpui::{AnyElement, Context, Entity, Pixels, Window, px};
use voltrus_scada_native::runtime::{CallKey, Reply};
use vt_ui::Icon;
use vt_ui::text_input::TextInput;

use super::{Console, views};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Overview,
    Tags,
    Alarms,
    Trends,
    Performance,
    DataManager,
    Dashboard,
    TagTree,
    Commands,
    Shift,
    Audit,
    Mimic,
}

/// The least a user's role must be to see a page or run a command; the
/// server's own grouping (`auth_middleware.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Any signed-in user.
    Any,
    /// Admin, demo or integration: the server's `demo_protected` routes.
    Operator,
    /// Strictly admin: users, roles, API keys, imports, tag writes.
    Admin,
}

/// Where the navbar shows a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Its own tab.
    Tab,
    /// An item of the More menu at the end of the tabs.
    More,
}

/// Draws a page. Runs inside the page's cached view: it redraws when the
/// console changes or when the page asks for an animation frame.
pub type Render = fn(&mut Console, &mut Window, &mut Context<Console>) -> AnyElement;
/// Runs when the page comes on screen: focus its list, fetch once.
pub type OnShow = fn(&mut Console, &mut Window, &mut Context<Console>);
/// Handles the answer to a generic REST call whose key scope is the page id.
pub type OnReply = fn(&mut Console, CallKey, Reply, &mut Context<Console>);
/// The page's search field, if it has one.
pub type Search = fn(&Console) -> Option<Entity<TextInput>>;

pub struct PageSpec {
    pub page: Page,
    /// `--page <id>`, the scope of the page's [`CallKey`]s.
    pub id: &'static str,
    pub title: &'static str,
    pub icon: Icon,
    /// ⌘1–⌘9; each digit at most once.
    pub digit: Option<u8>,
    pub role: Role,
    pub placement: Placement,
    pub render: Render,
    /// A count on the tab, e.g. alarms to acknowledge; hidden at 0.
    pub badge: Option<fn(&Console) -> usize>,
    pub on_show: Option<OnShow>,
    pub on_reply: Option<OnReply>,
    /// The page's search field, focused by `/`.
    pub search: Option<Search>,
    /// The width of a panel along the page's right edge that the toasts
    /// keep clear of.
    pub side_panel: fn(&Console) -> Pixels,
}

fn no_panel(_: &Console) -> Pixels {
    px(0.0)
}

/// Every page, in navbar order.
pub static PAGES: &[PageSpec] = &[
    PageSpec {
        page: Page::Overview,
        id: "overview",
        title: "Overview",
        icon: Icon::Gauge,
        digit: Some(1),
        role: Role::Any,
        placement: Placement::Tab,
        render: views::overview::render,
        badge: None,
        on_show: Some(views::overview::on_show),
        on_reply: None,
        search: None,
        side_panel: no_panel,
    },
    PageSpec {
        page: Page::Tags,
        id: "tags",
        title: "Tags",
        icon: Icon::Table,
        digit: Some(2),
        role: Role::Any,
        placement: Placement::Tab,
        render: views::tags::render,
        badge: None,
        on_show: Some(views::tags::on_show),
        on_reply: Some(views::tags::on_reply),
        search: Some(views::tags::search),
        side_panel: views::tags::side_panel,
    },
    PageSpec {
        page: Page::Alarms,
        id: "alarms",
        title: "Alarms",
        icon: Icon::CircleAlert,
        digit: Some(3),
        role: Role::Any,
        placement: Placement::Tab,
        render: views::alarms::render,
        badge: Some(Console::unacknowledged),
        on_show: Some(views::alarms::on_show),
        on_reply: Some(views::alarms::on_reply),
        search: Some(views::alarms::search),
        side_panel: no_panel,
    },
    PageSpec {
        page: Page::Trends,
        id: "trends",
        title: "Trends",
        icon: Icon::ChartLine,
        digit: Some(4),
        role: Role::Any,
        placement: Placement::Tab,
        render: views::trends::render,
        badge: None,
        on_show: Some(views::trends::on_show),
        on_reply: None,
        search: Some(views::trends::search),
        side_panel: views::trends::side_panel,
    },
    PageSpec {
        page: Page::Performance,
        id: "performance",
        title: "Performance",
        icon: Icon::Activity,
        digit: Some(9),
        role: Role::Any,
        placement: Placement::More,
        render: views::performance::render,
        badge: None,
        on_show: None,
        on_reply: None,
        search: None,
        side_panel: no_panel,
    },
    PageSpec {
        page: Page::DataManager,
        id: "data",
        title: "Data Manager",
        icon: Icon::Database,
        digit: None,
        role: Role::Operator,
        placement: Placement::More,
        render: views::data_manager::render,
        badge: None,
        on_show: Some(views::data_manager::on_show),
        on_reply: Some(views::data_manager::on_reply),
        search: Some(views::data_manager::search),
        side_panel: no_panel,
    },
    PageSpec {
        page: Page::TagTree,
        id: "tagtree",
        title: "Tag tree",
        icon: Icon::Network,
        digit: Some(5),
        role: Role::Any,
        placement: Placement::Tab,
        render: views::tag_tree::render,
        badge: None,
        on_show: Some(views::tag_tree::on_show),
        on_reply: Some(views::tag_tree::on_reply),
        search: Some(views::tag_tree::search),
        side_panel: views::tag_tree::side_panel,
    },
    PageSpec {
        page: Page::Commands,
        id: "commands",
        title: "Commands",
        icon: Icon::Send,
        digit: None,
        role: Role::Any,
        placement: Placement::More,
        render: views::commands::render,
        badge: None,
        on_show: Some(views::commands::on_show),
        on_reply: Some(views::commands::on_reply),
        search: None,
        side_panel: views::commands::side_panel,
    },
    PageSpec {
        page: Page::Shift,
        id: "shift",
        title: "Shift handover",
        icon: Icon::Clock,
        digit: None,
        role: Role::Operator,
        placement: Placement::More,
        render: views::shift::render,
        badge: None,
        on_show: Some(views::shift::on_show),
        on_reply: Some(views::shift::on_reply),
        search: Some(views::shift::search),
        side_panel: no_panel,
    },
    PageSpec {
        page: Page::Audit,
        id: "audit",
        title: "Audit log",
        icon: Icon::ScrollText,
        digit: None,
        role: Role::Operator,
        placement: Placement::More,
        render: views::audit::render,
        badge: None,
        on_show: Some(views::audit::on_show),
        on_reply: Some(views::audit::on_reply),
        search: Some(views::audit::search),
        side_panel: views::audit::side_panel,
    },
    PageSpec {
        page: Page::Dashboard,
        id: "dashboard",
        title: "Dashboard",
        icon: Icon::Columns,
        digit: Some(6),
        role: Role::Any,
        placement: Placement::Tab,
        render: views::dashboard::render,
        badge: None,
        on_show: Some(views::dashboard::on_show),
        on_reply: Some(views::dashboard::on_reply),
        search: None,
        side_panel: no_panel,
    },
    PageSpec {
        page: Page::Mimic,
        id: "mimic",
        title: "Mimic",
        icon: Icon::Factory,
        digit: Some(7),
        role: Role::Any,
        placement: Placement::Tab,
        render: views::mimic::render,
        badge: None,
        on_show: Some(views::mimic::on_show),
        on_reply: Some(views::mimic::on_reply),
        search: None,
        side_panel: views::mimic::side_panel,
    },
];

/// The row of `page`. Every variant has one (a unit test checks).
pub fn spec(page: Page) -> &'static PageSpec {
    PAGES
        .iter()
        .find(|spec| spec.page == page)
        .unwrap_or(&PAGES[0])
}

/// The row whose id is `id`, e.g. from `--page` or a [`CallKey`] scope.
pub fn by_id(id: &str) -> Option<&'static PageSpec> {
    PAGES.iter().find(|spec| spec.id == id)
}

impl Page {
    pub fn parse(id: &str) -> Option<Self> {
        by_id(id).map(|spec| spec.page)
    }

    pub fn title(self) -> &'static str {
        spec(self).title
    }
}

/// The page ids, for usage text: `overview|tags|…`.
pub fn ids() -> String {
    PAGES
        .iter()
        .map(|spec| spec.id)
        .collect::<Vec<_>>()
        .join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_one_row_with_a_unique_id_and_digit() {
        for page in [
            Page::Overview,
            Page::Tags,
            Page::Alarms,
            Page::Trends,
            Page::Performance,
            Page::DataManager,
            Page::Dashboard,
            Page::TagTree,
            Page::Commands,
            Page::Shift,
            Page::Audit,
            Page::Mimic,
        ] {
            assert_eq!(PAGES.iter().filter(|spec| spec.page == page).count(), 1);
            assert_eq!(Page::parse(spec(page).id), Some(page));
        }
        let mut all: Vec<_> = PAGES.iter().map(|spec| spec.id).collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), PAGES.len(), "ids are unique");
        let mut digits: Vec<u8> = PAGES.iter().filter_map(|spec| spec.digit).collect();
        assert!(digits.iter().all(|digit| (1..=9).contains(digit)));
        digits.sort_unstable();
        let count = digits.len();
        digits.dedup();
        assert_eq!(digits.len(), count, "each ⌘ digit once");
        assert_eq!(Page::parse("nowhere"), None);
        assert_eq!(
            ids(),
            "overview|tags|alarms|trends|performance|data|tagtree|commands|shift|audit|dashboard|mimic"
        );
    }

    #[test]
    fn roles_order_from_any_to_admin() {
        assert!(Role::Any < Role::Operator && Role::Operator < Role::Admin);
    }
}
