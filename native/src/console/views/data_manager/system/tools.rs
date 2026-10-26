//! System › Tools, after the web panel's `ToolsPanel`: Voltrus PLC and
//! Voltrus HMI opened by deep link (`voltrus-plc://open?scada=<server>`,
//! the desktop app must be installed), and the documentation: the system
//! documentation as HTML, the server's API reference and the web console.

use gpui::prelude::*;
use gpui::{AnyElement, Context, ElementId, FontWeight, SharedString, Window, div, px};
use voltrus_scada_native::api::eng_system::{TOOLS, deep_link};
use vt_ui::Icon;
use vt_ui::theme::*;

use super::{Look, Tab, click_row, open_snapshots, row_shell, rows, set_tab};
use crate::console::Console;
use crate::console::views::ListKeys;
use crate::console::views::ops_kit::notice;

const LIST: &str = "data.system.tools";
const ROW: f32 = 56.0;

/// What a row of the Tools list opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    /// One of [`TOOLS`], by index.
    Tool(usize),
    /// The Documentation tab.
    Docs,
    /// The server's API reference, `/api/docs` (when the server is built
    /// with its `openapi` feature).
    ApiReference,
    /// The web console in the browser.
    WebConsole,
    /// Engineering › Config snapshots.
    Snapshots,
}

const ITEMS: [Item; 6] = [
    Item::Tool(0),
    Item::Tool(1),
    Item::Docs,
    Item::ApiReference,
    Item::WebConsole,
    Item::Snapshots,
];

impl Item {
    fn icon(self) -> Icon {
        match self {
            Self::Tool(0) => Icon::Cpu,
            Self::Tool(_) => Icon::Layers,
            Self::Docs => Icon::ScrollText,
            Self::ApiReference => Icon::File,
            Self::WebConsole => Icon::Network,
            Self::Snapshots => Icon::History,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Tool(index) => TOOLS[index].name,
            Self::Docs => "System documentation",
            Self::ApiReference => "API reference",
            Self::WebConsole => "Web console",
            Self::Snapshots => "Config snapshots",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Self::Tool(index) => TOOLS[index].about,
            Self::Docs => {
                "Topology, tag cross-reference and automation for handover and audits, and its HTML export."
            }
            Self::ApiReference => {
                "The server's REST API, in the browser (on servers built with the API reference)."
            }
            Self::WebConsole => "This server's web client, in the browser.",
            Self::Snapshots => "Snapshots of the configuration, their differences and a restore.",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Tool(_) => "Open the app",
            Self::Docs => "Show",
            Self::ApiReference | Self::WebConsole => "Open in the browser",
            Self::Snapshots => "Go to the section",
        }
    }
}

/// The URL a row opens outside the console, if it does.
fn url(item: Item, server: &str) -> Option<String> {
    let server = server.trim_end_matches('/');
    match item {
        Item::Tool(index) => Some(deep_link(TOOLS[index].scheme, server)),
        Item::ApiReference => Some(format!("{server}/api/docs")),
        Item::WebConsole => Some(server.to_owned()),
        Item::Docs | Item::Snapshots => None,
    }
}

pub(super) fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> bool {
    console.focus_list(LIST, window, cx);
    true
}

fn activate(
    console: &mut Console,
    row: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(item) = ITEMS.get(row).copied() else {
        return;
    };
    match item {
        Item::Docs => set_tab(console, Tab::Docs, window, cx),
        Item::Snapshots => open_snapshots(console, window, cx),
        _ => {
            if console.server.is_empty() {
                console.show_toast("Not signed in to a server", true, cx);
                return;
            }
            if let Some(url) = url(item, &console.server) {
                cx.open_url(&url);
                console.show_toast(format!("Opening {}", item.title()), false, cx);
            }
        }
    }
}

pub(super) fn render(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let server = console.server.clone();
    let operator = console.is_operator();
    let intro = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(SPACE_2)
        .px(SPACE_9)
        .pb(SPACE_6)
        .child(
            div()
                .text_size(FONT_EMPHASIS)
                .font_weight(FontWeight::SEMIBOLD)
                .child("External tools"),
        )
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child("Launch the Voltrus desktop apps with this server as their SCADA context; the app must be installed."),
        );
    let list = rows(
        console,
        window,
        cx,
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate,
        },
        ITEMS.len(),
        ROW,
        notice("No tools", false, &theme),
        move |_, row, look: Look, theme, cx| {
            let item = ITEMS[row];
            let link = url(item, &server);
            let enabled = item != Item::Snapshots || operator;
            row_shell(
                ElementId::NamedInteger("system-tool".into(), u64::try_from(row).unwrap_or(0)),
                row,
                look,
                ROW,
                theme,
            )
            .on_click(click_row(LIST, row, Some(activate), cx))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .rounded(RADIUS_MD)
                    .bg(theme.accent_soft)
                    .child(vt_ui::icon(item.icon(), ICON_LG, theme.accent)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(SPACE_1)
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(item.title()))
                    .child(
                        div()
                            .truncate()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child(item.about()),
                    ),
            )
            .child(
                div()
                    .w(px(320.0))
                    .flex_none()
                    .truncate()
                    .font_family(MONO)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_dim)
                    .child(SharedString::from(link.unwrap_or_default())),
            )
            .child(
                div()
                    .w(px(140.0))
                    .flex_none()
                    .text_size(FONT_SMALL)
                    .text_color(if enabled { theme.accent } else { theme.fg_dim })
                    .child(if enabled {
                        item.action()
                    } else {
                        "Needs the operator role"
                    }),
            )
            .into_any_element()
        },
    );
    div()
        .flex()
        .flex_col()
        .size_full()
        .pt(SPACE_6)
        .child(intro)
        .child(list)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_open_the_tools_and_the_docs() {
        let server = "http://plant.local:3000/";
        assert_eq!(
            url(Item::Tool(0), server).as_deref(),
            Some("voltrus-plc://open?scada=http%3A%2F%2Fplant.local%3A3000")
        );
        assert_eq!(
            url(Item::Tool(1), server).as_deref(),
            Some("voltrus-hmi://open?scada=http%3A%2F%2Fplant.local%3A3000")
        );
        assert_eq!(
            url(Item::ApiReference, server).as_deref(),
            Some("http://plant.local:3000/api/docs")
        );
        assert_eq!(url(Item::Docs, server), None);
        assert_eq!(Item::Tool(1).title(), "Voltrus HMI");
    }
}
