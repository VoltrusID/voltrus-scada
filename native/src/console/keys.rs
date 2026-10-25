//! Actions, key bindings, menus and the command table they all come from.
//!
//! One table, [`COMMANDS`], lists every console command with its keys, its
//! key context, a label and whether the ⌘K palette offers it. The key
//! bindings, the View menu, the palette and the help overlay (`?`) are all
//! generated from it and from the page registry ([`super::pages::PAGES`]),
//! so a new command is one row here (plus its handler on the root view).
//!
//! Key contexts:
//! - [`CONSOLE`]: the window's root; global commands.
//! - [`NOT_TYPING`]: the root, unless a text field has focus (`/`, `?`).
//! - [`LIST`]: a keyboard list (rows with a cursor) has focus.
//! - [`MENU`]: a popup list inside an overlay (the palette, the More menu).

use std::rc::Rc;

use gpui::{
    Action, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, Menu, MenuItem, OsAction,
    actions,
};
use vt_ui::Icon;

use super::pages::{PAGES, Page, Role};

actions!(
    voltrus_console,
    [
        Quit,
        SignOut,
        AcknowledgeAll,
        ToggleAppearance,
        TogglePalette,
        ToggleHelp,
        TogglePerfHud,
        ShowMore,
        Back,
        Forward,
        FocusSearch,
        FocusNext,
        FocusPrevious,
        Dismiss,
        ListUp,
        ListDown,
        ListPageUp,
        ListPageDown,
        ListFirst,
        ListLast,
        ListActivate,
        ListToggle,
        StartStress,
        StopStress,
        ShellOpenSettings,
        ShellToggleNotifications,
        ShellEditShortcuts,
        ShellPickTheme,
        TagsWrite,
        TagsWritePolicy,
        TagsAddNote,
        TagsCompare,
        TagsCustomRange,
        TagsExportCsv,
        TagsExpandDetail,
        OpsPreviousPage,
        OpsNextPage,
        OpsNextTab,
        OpsNextFilter,
        OpsPreviousFilter,
        OpsRefresh,
        OpsCancelCommand,
        OpsSortNext,
        OpsSortReverse,
        OpsNewHandover,
        OpsExportAudit,
        OpsExportData,
        AlarmsAcknowledge,
        AlarmsShelve,
        AlarmsNextFilter,
        AlarmsPreviousFilter,
        AlarmsCycleSort,
        AlarmsToggleMute,
        AlarmsShowList,
        AlarmsShowAnalytics,
        AlarmsShowAudit,
        DataNewSource,
        DataDuplicateSource,
        DataDeleteSource,
        DataMoveSourceUp,
        DataMoveSourceDown,
        DataSortNext,
        DataReverseSort,
    ]
);

/// Show a page (⌘1–⌘9, the View menu, the palette).
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = voltrus_console, no_json)]
pub struct ShowPage(pub Page);

pub const CONSOLE: &str = "Console";
pub const NOT_TYPING: &str = "Console && !TextInput";
pub const LIST: &str = "List";
pub const MENU: &str = "Menu";
/// The operations pages' keys (Commands, Shift handover, Audit log): their
/// page roots carry `OpsPaged`, `OpsTabbed`, … and the keys never fire
/// while a text field has focus.
pub const OPS_PAGED: &str = "OpsPaged && !TextInput";
pub const OPS_TABBED: &str = "OpsTabbed && !TextInput";
pub const OPS_FILTERED: &str = "OpsFiltered && !TextInput";
pub const OPS_REFRESHABLE: &str = "OpsRefreshable && !TextInput";
pub const OPS_COMMANDS: &str = "OpsCommands && !TextInput";
pub const OPS_SHIFT: &str = "OpsShift && !TextInput";
pub const OPS_AUDIT: &str = "OpsAudit && !TextInput";
/// The Alarms page's lists (and its detail sheet): a = acknowledge, s =
/// shelve, ⇧A = acknowledge all, ←/→ = filter, o = sort.
pub const ALARM_LIST: &str = "AlarmList";
/// The Data Manager's sources table has focus.
pub const DATA_SOURCES: &str = "DataSources";

/// How the help overlay groups the commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Pages,
    Navigation,
    Lists,
    Alarms,
    View,
    Session,
    Data,
}

impl Group {
    pub const ALL: [Self; 7] = [
        Self::Pages,
        Self::Navigation,
        Self::Lists,
        Self::Alarms,
        Self::View,
        Self::Session,
        Self::Data,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Pages => "Pages",
            Self::Navigation => "Navigation",
            Self::Lists => "Lists",
            Self::Alarms => "Alarms",
            Self::View => "View",
            Self::Session => "Session",
            Self::Data => "Data Manager",
        }
    }
}

/// One console command.
pub struct CommandSpec {
    pub label: &'static str,
    pub group: Group,
    pub icon: Icon,
    /// GPUI keystrokes (`secondary` is ⌘ on macOS and Ctrl elsewhere); the
    /// first is the one menus and hints show.
    pub keys: &'static [&'static str],
    pub context: &'static str,
    pub action: fn() -> Box<dyn Action>,
    /// Offered in the ⌘K palette.
    pub palette: bool,
    /// Who the palette offers it to.
    pub role: Role,
}

macro_rules! command {
    ($label:expr, $group:ident, $icon:ident, [$($key:expr),*], $context:expr, $action:expr, $palette:expr) => {
        CommandSpec {
            label: $label,
            group: Group::$group,
            icon: Icon::$icon,
            keys: &[$($key),*],
            context: $context,
            action: || -> Box<dyn Action> { Box::new($action) },
            palette: $palette,
            role: Role::Any,
        }
    };
}

/// Every command but the page shortcuts, which come from the page registry.
pub static COMMANDS: &[CommandSpec] = &[
    command!(
        "Command palette",
        Navigation,
        Search,
        ["secondary-k"],
        CONSOLE,
        TogglePalette,
        false
    ),
    command!(
        "Keyboard shortcuts",
        Navigation,
        Keyboard,
        ["?"],
        NOT_TYPING,
        ToggleHelp,
        true
    ),
    command!(
        "Back",
        Navigation,
        ChevronLeft,
        ["secondary-["],
        CONSOLE,
        Back,
        true
    ),
    command!(
        "Forward",
        Navigation,
        ChevronRight,
        ["secondary-]"],
        CONSOLE,
        Forward,
        true
    ),
    command!(
        "More pages",
        Navigation,
        Ellipsis,
        [],
        CONSOLE,
        ShowMore,
        true
    ),
    command!(
        "Search this page",
        Navigation,
        Search,
        ["/"],
        NOT_TYPING,
        FocusSearch,
        false
    ),
    command!(
        "Next control",
        Navigation,
        ChevronRight,
        ["tab"],
        CONSOLE,
        FocusNext,
        false
    ),
    command!(
        "Previous control",
        Navigation,
        ChevronLeft,
        ["shift-tab"],
        CONSOLE,
        FocusPrevious,
        false
    ),
    command!(
        "Close or go back",
        Navigation,
        X,
        ["escape"],
        CONSOLE,
        Dismiss,
        false
    ),
    command!(
        "Move down",
        Lists,
        ChevronDown,
        ["down", "j"],
        LIST,
        ListDown,
        false
    ),
    command!(
        "Move up",
        Lists,
        ChevronUp,
        ["up", "k"],
        LIST,
        ListUp,
        false
    ),
    command!(
        "Page down",
        Lists,
        ChevronDown,
        ["pagedown"],
        LIST,
        ListPageDown,
        false
    ),
    command!(
        "Page up",
        Lists,
        ChevronUp,
        ["pageup"],
        LIST,
        ListPageUp,
        false
    ),
    command!(
        "First row",
        Lists,
        ChevronUp,
        ["home"],
        LIST,
        ListFirst,
        false
    ),
    command!(
        "Last row",
        Lists,
        ChevronDown,
        ["end"],
        LIST,
        ListLast,
        false
    ),
    command!(
        "Open the row",
        Lists,
        CircleDot,
        ["enter"],
        LIST,
        ListActivate,
        false
    ),
    command!(
        "Toggle the row",
        Lists,
        Check,
        ["space"],
        LIST,
        ListToggle,
        false
    ),
    // Arrows move a menu's cursor also while its search field has focus
    // (the field takes only left and right); letters type.
    command!(
        "Move down",
        Lists,
        ChevronDown,
        ["down"],
        MENU,
        ListDown,
        false
    ),
    command!("Move up", Lists, ChevronUp, ["up"], MENU, ListUp, false),
    command!(
        "Acknowledge all alarms",
        Alarms,
        Check,
        ["secondary-shift-a"],
        CONSOLE,
        AcknowledgeAll,
        true
    ),
    command!(
        "Light or dark",
        View,
        Sun,
        ["secondary-shift-l"],
        CONSOLE,
        ToggleAppearance,
        true
    ),
    command!(
        "Performance HUD",
        View,
        Activity,
        ["secondary-shift-p"],
        CONSOLE,
        TogglePerfHud,
        true
    ),
    command!(
        "Start the stress test",
        View,
        Zap,
        [],
        CONSOLE,
        StartStress,
        true
    ),
    command!(
        "Stop the stress test",
        View,
        Square,
        [],
        CONSOLE,
        StopStress,
        true
    ),
    command!("Sign out", Session, Unplug, [], CONSOLE, SignOut, true),
    command!("Quit", Session, X, ["secondary-q"], CONSOLE, Quit, true),
    // The operations pages (Commands, Shift handover, Audit log).
    command!(
        "Previous page of results",
        Lists,
        ChevronLeft,
        ["left"],
        OPS_PAGED,
        OpsPreviousPage,
        false
    ),
    command!(
        "Next page of results",
        Lists,
        ChevronRight,
        ["right"],
        OPS_PAGED,
        OpsNextPage,
        false
    ),
    command!(
        "Switch the page's tab",
        Lists,
        Columns,
        ["t"],
        OPS_TABBED,
        OpsNextTab,
        false
    ),
    command!(
        "Next filter",
        Lists,
        Funnel,
        ["]"],
        OPS_FILTERED,
        OpsNextFilter,
        false
    ),
    command!(
        "Previous filter",
        Lists,
        Funnel,
        ["["],
        OPS_FILTERED,
        OpsPreviousFilter,
        false
    ),
    command!(
        "Refresh the list",
        Lists,
        RefreshCw,
        ["r"],
        OPS_REFRESHABLE,
        OpsRefresh,
        false
    ),
    command!(
        "Cancel the command",
        Lists,
        X,
        ["x"],
        OPS_COMMANDS,
        OpsCancelCommand,
        false
    ),
    command!(
        "Sort by the next column",
        Lists,
        ArrowUpDown,
        ["s"],
        OPS_AUDIT,
        OpsSortNext,
        false
    ),
    command!(
        "Reverse the sort",
        Lists,
        ArrowUpDown,
        ["shift-s"],
        OPS_AUDIT,
        OpsSortReverse,
        false
    ),
    CommandSpec {
        label: "New shift handover entry",
        group: Group::Lists,
        icon: Icon::Plus,
        keys: &["n"],
        context: OPS_SHIFT,
        action: || -> Box<dyn Action> { Box::new(OpsNewHandover) },
        palette: true,
        role: Role::Operator,
    },
    CommandSpec {
        label: "Export the audit log as CSV",
        group: Group::Lists,
        icon: Icon::Download,
        keys: &["e"],
        context: OPS_AUDIT,
        action: || -> Box<dyn Action> { Box::new(OpsExportAudit) },
        palette: true,
        role: Role::Operator,
    },
    command!(
        "Export data…",
        View,
        Download,
        [],
        CONSOLE,
        OpsExportData,
        true
    ),
    command!(
        "Settings",
        View,
        Settings,
        ["secondary-,"],
        CONSOLE,
        ShellOpenSettings,
        true
    ),
    command!(
        "Notifications",
        View,
        Inbox,
        ["secondary-alt-n"],
        CONSOLE,
        ShellToggleNotifications,
        true
    ),
    command!(
        "Edit keyboard shortcuts",
        View,
        Keyboard,
        [],
        CONSOLE,
        ShellEditShortcuts,
        true
    ),
    command!(
        "Pick a theme",
        View,
        Palette,
        [],
        CONSOLE,
        ShellPickTheme,
        true
    ),
    command!(
        "Write to the tag…",
        View,
        Send,
        ["secondary-shift-w"],
        CONSOLE,
        TagsWrite,
        true
    ),
    command!(
        "Edit the tag's write policy…",
        View,
        Settings,
        [],
        CONSOLE,
        TagsWritePolicy,
        true
    ),
    command!(
        "Add a note to the tag's chart…",
        View,
        Pencil,
        ["secondary-shift-n"],
        CONSOLE,
        TagsAddNote,
        true
    ),
    command!(
        "Compare the tag with another…",
        View,
        Layers,
        [],
        CONSOLE,
        TagsCompare,
        true
    ),
    command!(
        "Show a custom span on the tag's chart…",
        View,
        Clock,
        [],
        CONSOLE,
        TagsCustomRange,
        true
    ),
    command!(
        "Save the tag's chart as CSV…",
        View,
        Download,
        ["secondary-shift-s"],
        CONSOLE,
        TagsExportCsv,
        true
    ),
    command!(
        "Widen or narrow the tag detail",
        View,
        PanelLeft,
        [],
        CONSOLE,
        TagsExpandDetail,
        true
    ),
    command!(
        "Acknowledge all alarms",
        Alarms,
        Check,
        ["shift-a"],
        ALARM_LIST,
        AcknowledgeAll,
        false
    ),
    command!(
        "Acknowledge the alarm",
        Alarms,
        Check,
        ["a"],
        ALARM_LIST,
        AlarmsAcknowledge,
        false
    ),
    CommandSpec {
        label: "Shelve or unshelve the alarm",
        group: Group::Alarms,
        icon: Icon::Clock,
        keys: &["s"],
        context: ALARM_LIST,
        action: || -> Box<dyn Action> { Box::new(AlarmsShelve) },
        palette: false,
        role: Role::Operator,
    },
    command!(
        "Next alarm filter",
        Alarms,
        ChevronRight,
        ["right"],
        ALARM_LIST,
        AlarmsNextFilter,
        false
    ),
    command!(
        "Previous alarm filter",
        Alarms,
        ChevronLeft,
        ["left"],
        ALARM_LIST,
        AlarmsPreviousFilter,
        false
    ),
    command!(
        "Sort the alarms",
        Alarms,
        ArrowUpDown,
        ["o"],
        ALARM_LIST,
        AlarmsCycleSort,
        false
    ),
    command!(
        "Mute or unmute the alarm sound",
        Alarms,
        Radio,
        ["secondary-shift-m"],
        CONSOLE,
        AlarmsToggleMute,
        true
    ),
    command!(
        "Alarm list",
        Alarms,
        List,
        [],
        CONSOLE,
        AlarmsShowList,
        true
    ),
    CommandSpec {
        label: "Alarm analytics",
        group: Group::Alarms,
        icon: Icon::ChartLine,
        keys: &[],
        context: CONSOLE,
        action: || -> Box<dyn Action> { Box::new(AlarmsShowAnalytics) },
        palette: true,
        role: Role::Operator,
    },
    CommandSpec {
        label: "Alarm audit trail",
        group: Group::Alarms,
        icon: Icon::ScrollText,
        keys: &[],
        context: CONSOLE,
        action: || -> Box<dyn Action> { Box::new(AlarmsShowAudit) },
        palette: true,
        role: Role::Operator,
    },
    CommandSpec {
        label: "New data source",
        group: Group::Data,
        icon: Icon::Plus,
        keys: &["n"],
        context: DATA_SOURCES,
        action: || -> Box<dyn Action> { Box::new(DataNewSource) },
        palette: true,
        role: Role::Operator,
    },
    command!(
        "Duplicate the source",
        Data,
        Copy,
        ["d"],
        DATA_SOURCES,
        DataDuplicateSource,
        false
    ),
    command!(
        "Delete the source",
        Data,
        Trash,
        ["backspace", "delete"],
        DATA_SOURCES,
        DataDeleteSource,
        false
    ),
    command!(
        "Move the source up",
        Data,
        ChevronUp,
        ["alt-up"],
        DATA_SOURCES,
        DataMoveSourceUp,
        false
    ),
    command!(
        "Move the source down",
        Data,
        ChevronDown,
        ["alt-down"],
        DATA_SOURCES,
        DataMoveSourceDown,
        false
    ),
    command!(
        "Sort by the next column",
        Data,
        ArrowUpDown,
        ["s"],
        DATA_SOURCES,
        DataSortNext,
        false
    ),
    command!(
        "Reverse the sort",
        Data,
        ArrowUpDown,
        ["shift-s"],
        DATA_SOURCES,
        DataReverseSort,
        false
    ),
];

/// A binding from table data; panics on a bad keystroke or context, which
/// the unit tests rule out for every row.
fn bind(keys: &str, action: Box<dyn Action>, context: &str) -> KeyBinding {
    let predicate = KeyBindingContextPredicate::parse(context).expect("a valid key context");
    KeyBinding::load(
        keys,
        action,
        Some(Rc::new(predicate)),
        false,
        None,
        &DummyKeyboardMapper,
    )
    .expect("a valid keystroke")
}

/// The ⌘ digit binding of a page.
pub fn page_keys(digit: u8) -> String {
    format!("secondary-{digit}")
}

pub fn key_bindings() -> Vec<KeyBinding> {
    let mut bindings: Vec<KeyBinding> = COMMANDS
        .iter()
        .flat_map(|command| {
            command
                .keys
                .iter()
                .map(|keys| bind(keys, (command.action)(), command.context))
        })
        .collect();
    for spec in PAGES {
        if let Some(digit) = spec.digit {
            bindings.push(bind(
                &page_keys(digit),
                Box::new(ShowPage(spec.page)),
                CONSOLE,
            ));
        }
    }
    bindings
}

pub fn menus() -> Vec<Menu> {
    let mut view: Vec<MenuItem> = PAGES
        .iter()
        .map(|spec| MenuItem::action(spec.title, ShowPage(spec.page)))
        .collect();
    view.push(MenuItem::separator());
    view.extend(
        COMMANDS
            .iter()
            .filter(|command| command.palette && command.group != Group::Session)
            .map(|command| MenuItem::Action {
                name: command.label.into(),
                action: (command.action)(),
                os_action: None,
            }),
    );
    vec![
        Menu {
            name: "Voltrus SCADA".into(),
            items: vec![
                MenuItem::action("Sign Out", SignOut),
                MenuItem::separator(),
                MenuItem::action("Quit Voltrus SCADA", Quit),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Cut", vt_ui::text_input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", vt_ui::text_input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", vt_ui::text_input::Paste, OsAction::Paste),
                MenuItem::os_action(
                    "Select All",
                    vt_ui::text_input::SelectAll,
                    OsAction::SelectAll,
                ),
            ],
        },
        Menu {
            name: "View".into(),
            items: view,
        },
    ]
}

/// A keystroke as the operator reads it: `⇧⌘P`, `⌘[`, `↓`, `Space` on
/// macOS; `Ctrl+Shift+P` elsewhere.
pub fn display(keys: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let mut modifiers = String::new();
    let mut parts: Vec<&str> = keys.split('-').collect();
    let key = parts.pop().unwrap_or("");
    let mut names: Vec<&str> = Vec::new();
    for modifier in parts {
        let (symbol, name) = match modifier {
            "secondary" | "cmd" if mac => ("⌘", "Cmd"),
            "secondary" | "ctrl" => ("⌃", "Ctrl"),
            "cmd" => ("⌘", "Super"),
            "shift" => ("⇧", "Shift"),
            "alt" => ("⌥", "Alt"),
            _ => ("", modifier),
        };
        names.push(name);
        modifiers.push_str(symbol);
    }
    let key = match key {
        "up" => "↑".to_owned(),
        "down" => "↓".to_owned(),
        "left" => "←".to_owned(),
        "right" => "→".to_owned(),
        "enter" => "↵".to_owned(),
        "escape" => "Esc".to_owned(),
        "tab" => "Tab".to_owned(),
        "space" => "Space".to_owned(),
        "pageup" => "PgUp".to_owned(),
        "pagedown" => "PgDn".to_owned(),
        "home" => "Home".to_owned(),
        "end" => "End".to_owned(),
        other => other.to_uppercase(),
    };
    if mac {
        // macOS orders modifiers ⌃⌥⇧⌘.
        let order = ['⌃', '⌥', '⇧', '⌘'];
        let mut symbols: Vec<char> = modifiers.chars().collect();
        symbols.sort_by_key(|symbol| order.iter().position(|o| o == symbol));
        format!("{}{key}", symbols.into_iter().collect::<String>())
    } else {
        names.push(&key);
        names.join("+")
    }
}

/// The help overlay's lines: each label once with all its keys, grouped.
pub fn help_rows() -> Vec<(Group, &'static str, Vec<String>)> {
    let mut rows: Vec<(Group, &'static str, Vec<String>)> = Vec::new();
    for spec in PAGES {
        if let Some(digit) = spec.digit {
            rows.push((Group::Pages, spec.title, vec![display(&page_keys(digit))]));
        }
    }
    for command in COMMANDS.iter().filter(|command| !command.keys.is_empty()) {
        let keys: Vec<String> = command.keys.iter().map(|keys| display(keys)).collect();
        match rows
            .iter_mut()
            .find(|(group, label, _)| *group == command.group && *label == command.label)
        {
            Some((_, _, existing)) => {
                for key in keys {
                    if !existing.contains(&key) {
                        existing.push(key);
                    }
                }
            }
            None => rows.push((command.group, command.label, keys)),
        }
    }
    rows.sort_by_key(|(group, _, _)| Group::ALL.iter().position(|g| g == group));
    rows
}

/// The first key of the command whose action is `action`, for hints.
pub fn hint(action: &dyn Action) -> Option<String> {
    COMMANDS
        .iter()
        .find(|command| (command.action)().partial_eq(action))
        .and_then(|command| command.keys.first())
        .map(|keys| display(keys))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_binds_and_no_two_rows_share_keys_in_a_context() {
        let bindings = key_bindings();
        let pages = PAGES.iter().filter(|spec| spec.digit.is_some()).count();
        let keys: usize = COMMANDS.iter().map(|command| command.keys.len()).sum();
        assert_eq!(bindings.len(), keys + pages);
        let mut seen = std::collections::HashSet::new();
        for command in COMMANDS {
            for keys in command.keys {
                assert!(
                    seen.insert((command.context, *keys)),
                    "{keys} bound twice in {}",
                    command.context
                );
            }
        }
    }

    #[test]
    fn keys_read_like_the_platform_writes_them() {
        if cfg!(target_os = "macos") {
            assert_eq!(display("secondary-shift-p"), "⇧⌘P");
            assert_eq!(display("secondary-["), "⌘[");
            assert_eq!(display("secondary-1"), "⌘1");
        } else {
            assert_eq!(display("secondary-shift-p"), "Ctrl+Shift+P");
        }
        assert_eq!(display("down"), "↓");
        assert_eq!(display("j"), "J");
        assert_eq!(display("?"), "?");
        assert_eq!(display("pagedown"), "PgDn");
    }

    #[test]
    fn help_lists_each_command_once_with_all_its_keys() {
        let rows = help_rows();
        let down: Vec<_> = rows
            .iter()
            .filter(|(_, label, _)| *label == "Move down")
            .collect();
        assert_eq!(down.len(), 1);
        assert_eq!(down[0].2, ["↓", "J"]);
        assert_eq!(rows[0].0, Group::Pages, "pages first");
        assert!(rows.iter().any(|(_, label, _)| *label == "Performance"));
        assert!(
            rows.windows(2)
                .all(|pair| Group::ALL.iter().position(|g| *g == pair[0].0)
                    <= Group::ALL.iter().position(|g| *g == pair[1].0))
        );
        assert_eq!(
            hint(&TogglePalette).as_deref(),
            Some(display("secondary-k").as_str())
        );
    }
}
