//! The session's screens: the server's list, the screen on show, and each
//! screen's tags and card layouts. The card dashboard draws it
//! (`views/dashboard.rs`); the mimic viewer builds on the same model.
//!
//! # Using it from a page
//!
//! - Read it with `console.state_ref::<Screens>()` (`None` before anything
//!   asked for it); [`Screens::active`] is the screen on show,
//!   [`OVERVIEW`] for the overview, which has every tag.
//! - Call [`sync`] when the page shows or draws: it fetches what is missing
//!   for the active screen (the list, its tags, its layouts) once per
//!   session, and does nothing when everything is there.
//! - [`select`] shows another screen; [`reload`] forgets everything and
//!   fetches again.
//! - [`Screens::tag_ids`] gives a screen's tags in the screen's order,
//!   only those the store knows.
//! - Changes go through [`create`], [`rename`], [`delete`], [`add_tag`],
//!   [`remove_tag`] and [`save_layouts`] (operator routes: check
//!   [`rights`]). They apply at once and are fetched again after the
//!   server answers; a refusal becomes a toast.
//!
//! Replies come back under the `dashboard.*` keys whatever page asked, and
//! the dashboard page's `on_reply` hands them to [`on_reply`].
//! [`Screens::generation`] counts the changes, for caches.

use std::collections::{HashMap, HashSet};

use gpui::Context;
use serde::de::IgnoredAny;
use voltrus_scada_native::api::{DataSource, Method};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use voltrus_scada_native::screens::{
    OVERVIEW, Screen, ScreenLayouts, layout_id, layouts_path, tags_path,
};

use super::Console;
use super::pages::Role;
use super::store::Store;

/// What one call in flight was for.
enum Ask {
    List,
    Tags(String),
    Layouts(String),
    Create(String),
    Rename(String),
    Delete(String),
    AddTag(String),
    RemoveTag(String),
    Save(String),
}

pub struct Screens {
    /// The server's screens, in its order.
    pub list: Vec<Screen>,
    /// Whether the list has arrived.
    pub listed: bool,
    /// The screen on show: [`OVERVIEW`] or a screen's id.
    pub active: String,
    /// Each screen's tags (data source ids) in the screen's order; the
    /// overview's are the store's.
    pub tags: HashMap<String, Vec<i64>>,
    /// Each screen's saved layouts, the overview's under [`OVERVIEW`].
    pub layouts: HashMap<String, ScreenLayouts>,
    /// Why the last fetch failed, until one succeeds.
    pub error: Option<String>,
    /// Counts every change to the list, tags or layouts.
    pub generation: u64,
    /// Calls in flight, by their key's argument.
    asks: HashMap<i64, Ask>,
    /// What is being fetched, so a redraw does not ask twice.
    fetching: HashSet<String>,
    next_ask: i64,
    /// The server and user the model belongs to.
    session: Option<(String, String)>,
}

impl Default for Screens {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            listed: false,
            active: OVERVIEW.to_owned(),
            tags: HashMap::new(),
            layouts: HashMap::new(),
            error: None,
            generation: 0,
            asks: HashMap::new(),
            fetching: HashSet::new(),
            next_ask: 0,
            session: None,
        }
    }
}

impl Screens {
    /// The screen on show, `None` for the overview.
    pub fn active_screen(&self) -> Option<&Screen> {
        self.list.iter().find(|screen| screen.id == self.active)
    }

    /// A screen's name; the overview is "Overview".
    pub fn name(&self, id: &str) -> String {
        if id == OVERVIEW {
            return "Overview".to_owned();
        }
        self.list
            .iter()
            .find(|screen| screen.id == id)
            .map_or_else(|| id.to_owned(), |screen| screen.name.clone())
    }

    /// The ids of the screens as the tabs show them: the overview, then
    /// the server's.
    pub fn ids(&self) -> Vec<String> {
        std::iter::once(OVERVIEW.to_owned())
            .chain(self.list.iter().map(|screen| screen.id.clone()))
            .collect()
    }

    /// Whether screen `id`'s tags and layouts have arrived.
    pub fn ready(&self, id: &str) -> bool {
        (id == OVERVIEW || self.tags.contains_key(id)) && self.layouts.contains_key(id)
    }

    /// Screen `id`'s tags that the store knows, in the screen's order: every
    /// tag for the overview.
    pub fn tag_ids(&self, store: &Store, id: &str) -> Vec<i64> {
        if id == OVERVIEW {
            return store.tags.iter().map(|tag| tag.info.id).collect();
        }
        self.tags.get(id).map_or_else(Vec::new, |ids| {
            ids.iter()
                .copied()
                .filter(|tag| store.position(*tag).is_some())
                .collect()
        })
    }

    fn ask(&mut self, ask: Ask) -> i64 {
        self.next_ask += 1;
        self.asks.insert(self.next_ask, ask);
        self.next_ask
    }

    fn changed(&mut self) {
        self.generation += 1;
    }
}

/// What a user may change on a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rights {
    /// Move, resize and arrange the cards, and save the layout.
    pub arrange: bool,
    /// Add tags to the screen and remove them (not the overview: it has
    /// every tag).
    pub tags: bool,
    /// Create, rename and delete screens.
    pub screens: bool,
}

/// The server's rule: screen and layout changes are operator routes
/// (admin, demo, integration).
pub fn rights(role: Option<Role>, screen: &str) -> Rights {
    let operator = role.is_some_and(|role| role >= Role::Operator);
    Rights {
        arrange: operator,
        tags: operator && screen != OVERVIEW,
        screens: operator,
    }
}

fn call(console: &mut Console, name: &'static str, ask: Ask, call: Call) {
    let ticket = console.state::<Screens>().ask(ask);
    console.call(CallKey::new(name).with(ticket), call);
}

/// Fetch what the active screen still needs: the list, its tags and its
/// layouts. A new session (another server or user) starts afresh; a stress
/// run has only the overview, laid out by default.
pub fn sync(console: &mut Console) {
    let session = console
        .user
        .as_ref()
        .map(|(user, _)| (console.server.clone(), user.clone()));
    let stress = console.stress.is_some();
    let screens = console.state::<Screens>();
    if screens.session != session {
        *screens = Screens {
            session: session.clone(),
            ..Screens::default()
        };
    }
    if session.is_none() {
        return;
    }
    if stress {
        if !screens.listed {
            screens.listed = true;
            screens.active = OVERVIEW.to_owned();
            screens.layouts.entry(OVERVIEW.to_owned()).or_default();
            screens.changed();
        }
        return;
    }
    let active = screens.active.clone();
    let list = !screens.listed && screens.fetching.insert("list".to_owned());
    let tags = active != OVERVIEW
        && !screens.tags.contains_key(&active)
        && screens.fetching.insert(format!("tags {active}"));
    let layouts = !screens.layouts.contains_key(&active)
        && screens.fetching.insert(format!("layouts {active}"));
    if list {
        call(
            console,
            "dashboard.screens",
            Ask::List,
            Call::get::<Vec<Screen>>("/api/v1/screens"),
        );
    }
    if tags {
        call(
            console,
            "dashboard.tags",
            Ask::Tags(active.clone()),
            Call::get::<Vec<DataSource>>(tags_path(&active)),
        );
    }
    if layouts {
        call(
            console,
            "dashboard.layouts",
            Ask::Layouts(active.clone()),
            Call::get::<ScreenLayouts>(layouts_path(&active)),
        );
    }
}

/// Show screen `id` (or the overview), fetching what it needs.
pub fn select(console: &mut Console, id: &str) {
    let screens = console.state::<Screens>();
    if screens.active != id {
        screens.active = id.to_owned();
        screens.changed();
    }
    sync(console);
}

/// Forget the screens and fetch them again, staying on the same one.
pub fn reload(console: &mut Console) {
    let screens = console.state::<Screens>();
    screens.list.clear();
    screens.listed = false;
    screens.tags.clear();
    screens.layouts.clear();
    screens.fetching.clear();
    screens.error = None;
    screens.changed();
    sync(console);
}

/// `POST /screens {name}`; the new screen comes on show.
pub fn create(console: &mut Console, name: String) {
    call(
        console,
        "dashboard.created",
        Ask::Create(name.clone()),
        Call::send::<Screen>(
            Method::POST,
            "/api/v1/screens",
            serde_json::json!({ "name": name }),
        ),
    );
}

/// `PUT /screens/{id} {name}`.
pub fn rename(console: &mut Console, id: &str, name: String) {
    let screens = console.state::<Screens>();
    if let Some(screen) = screens.list.iter_mut().find(|screen| screen.id == id) {
        screen.name.clone_from(&name);
        screens.changed();
    }
    call(
        console,
        "dashboard.renamed",
        Ask::Rename(id.to_owned()),
        Call::send::<IgnoredAny>(
            Method::PUT,
            format!("/api/v1/screens/{id}"),
            serde_json::json!({ "name": name }),
        ),
    );
}

/// `DELETE /screens/{id}`; the overview comes on show if it was.
pub fn delete(console: &mut Console, id: &str) {
    let screens = console.state::<Screens>();
    screens.list.retain(|screen| screen.id != id);
    screens.tags.remove(id);
    screens.layouts.remove(id);
    if screens.active == id {
        screens.active = OVERVIEW.to_owned();
    }
    screens.changed();
    call(
        console,
        "dashboard.deleted",
        Ask::Delete(id.to_owned()),
        Call::send_empty::<IgnoredAny>(Method::DELETE, format!("/api/v1/screens/{id}")),
    );
}

/// `POST /screens/{screen}/data-sources {data_source_id}`.
pub fn add_tag(console: &mut Console, screen: &str, tag: i64) {
    let screens = console.state::<Screens>();
    let list = screens.tags.entry(screen.to_owned()).or_default();
    if !list.contains(&tag) {
        list.push(tag);
    }
    screens.changed();
    call(
        console,
        "dashboard.tag-added",
        Ask::AddTag(screen.to_owned()),
        Call::send::<IgnoredAny>(
            Method::POST,
            format!("/api/v1/screens/{screen}/data-sources"),
            serde_json::json!({ "data_source_id": tag }),
        ),
    );
}

/// `DELETE /screens/{screen}/data-sources/{tag}`.
pub fn remove_tag(console: &mut Console, screen: &str, tag: i64) {
    let screens = console.state::<Screens>();
    if let Some(list) = screens.tags.get_mut(screen) {
        list.retain(|id| *id != tag);
    }
    screens.changed();
    call(
        console,
        "dashboard.tag-removed",
        Ask::RemoveTag(screen.to_owned()),
        Call::send_empty::<IgnoredAny>(
            Method::DELETE,
            format!("/api/v1/screens/{screen}/data-sources/{tag}"),
        ),
    );
}

/// `PUT /screens/{id}/layouts` with every breakpoint (the server replaces
/// them all).
pub fn save_layouts(console: &mut Console, screen: &str, layouts: ScreenLayouts) {
    let body = serde_json::to_value(&layouts).unwrap_or_default();
    let screens = console.state::<Screens>();
    screens.layouts.insert(screen.to_owned(), layouts);
    screens.changed();
    call(
        console,
        "dashboard.saved",
        Ask::Save(screen.to_owned()),
        Call::send::<IgnoredAny>(Method::PUT, layouts_path(screen), body),
    );
}

/// Fetch screen `id`'s tags again (after a change, or to undo a refused one).
fn refetch_tags(console: &mut Console, id: &str) {
    if id == OVERVIEW {
        return;
    }
    console
        .state::<Screens>()
        .fetching
        .insert(format!("tags {id}"));
    call(
        console,
        "dashboard.tags",
        Ask::Tags(id.to_owned()),
        Call::get::<Vec<DataSource>>(tags_path(id)),
    );
}

fn refetch_list(console: &mut Console) {
    console
        .state::<Screens>()
        .fetching
        .insert("list".to_owned());
    call(
        console,
        "dashboard.screens",
        Ask::List,
        Call::get::<Vec<Screen>>("/api/v1/screens"),
    );
}

/// Take the answer to one of this model's calls; `false` when the key is
/// not one of them.
pub fn on_reply(
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) -> bool {
    let Some(ask) = console.state::<Screens>().asks.remove(&key.arg) else {
        return false;
    };
    let failed = reply.error().map(str::to_owned);
    match ask {
        Ask::List => {
            let screens = console.state::<Screens>();
            screens.fetching.remove("list");
            match reply.take::<Vec<Screen>>() {
                Ok(mut list) => {
                    list.sort_by_key(|screen| screen.sort_order);
                    screens.list = list;
                    screens.listed = true;
                    screens.error = None;
                    if screens.active != OVERVIEW
                        && !screens
                            .list
                            .iter()
                            .any(|screen| screen.id == screens.active)
                    {
                        screens.active = OVERVIEW.to_owned();
                    }
                    screens.changed();
                    sync(console);
                }
                Err(error) => screens.error = Some(format!("The screens did not load: {error}")),
            }
        }
        Ask::Tags(id) => {
            let screens = console.state::<Screens>();
            screens.fetching.remove(&format!("tags {id}"));
            match reply.take::<Vec<DataSource>>() {
                Ok(rows) => {
                    screens
                        .tags
                        .insert(id, rows.iter().map(|row| row.id).collect());
                    screens.error = None;
                    screens.changed();
                }
                Err(error) => {
                    screens.error = Some(format!("The screen's tags did not load: {error}"));
                }
            }
        }
        Ask::Layouts(id) => {
            let screens = console.state::<Screens>();
            screens.fetching.remove(&format!("layouts {id}"));
            match reply.take::<ScreenLayouts>() {
                Ok(layouts) => {
                    screens.layouts.entry(id).or_insert(layouts);
                    screens.error = None;
                    screens.changed();
                }
                Err(error) => {
                    // Lay the screen out by default rather than not at all.
                    screens.layouts.entry(id).or_default();
                    screens.error = Some(format!("The screen's layout did not load: {error}"));
                    screens.changed();
                }
            }
        }
        Ask::Create(name) => match reply.take::<Screen>() {
            Ok(screen) => {
                let id = screen.id.clone();
                let screens = console.state::<Screens>();
                if !screens.list.iter().any(|known| known.id == id) {
                    screens.list.push(screen);
                }
                screens.tags.insert(id.clone(), Vec::new());
                screens.changed();
                select(console, &id);
                console.show_toast(format!("Screen “{name}” created"), false, cx);
            }
            Err(error) => {
                console.show_toast(format!("Creating “{name}” failed: {error}"), true, cx);
            }
        },
        Ask::Rename(id) | Ask::Delete(id) => {
            if let Some(error) = failed {
                console.show_toast(format!("Changing screen {id} failed: {error}"), true, cx);
            }
            refetch_list(console);
        }
        Ask::AddTag(id) | Ask::RemoveTag(id) => {
            if let Some(error) = failed {
                console.show_toast(
                    format!("Changing the screen's tags failed: {error}"),
                    true,
                    cx,
                );
            }
            refetch_tags(console, &id);
        }
        Ask::Save(id) => {
            if let Some(error) = failed {
                let name = console.state::<Screens>().name(&id);
                console.show_toast(
                    format!(
                        "Saving the layout of {name} failed ({}): {error}",
                        layout_id(&id)
                    ),
                    true,
                    cx,
                );
            }
        }
    }
    cx.notify();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_operators_change_screens_and_not_the_overviews_tags() {
        let viewer = rights(Some(Role::Any), "plts_1");
        assert_eq!(viewer, Rights::default(), "a viewer only looks");
        assert_eq!(rights(None, "plts_1"), Rights::default());
        let operator = rights(Some(Role::Operator), "plts_1");
        assert!(operator.arrange && operator.tags && operator.screens);
        let overview = rights(Some(Role::Admin), OVERVIEW);
        assert!(overview.arrange && overview.screens);
        assert!(!overview.tags, "the overview has every tag");
    }

    #[test]
    fn the_model_names_and_orders_its_screens() {
        let mut screens = Screens::default();
        assert_eq!(screens.active, OVERVIEW);
        assert!(!screens.ready(OVERVIEW));
        screens.list = vec![
            Screen {
                id: "plts_1".into(),
                name: "PLTS 1".into(),
                ..Screen::default()
            },
            Screen {
                id: "grid".into(),
                name: "Grid".into(),
                ..Screen::default()
            },
        ];
        assert_eq!(screens.ids(), ["all", "plts_1", "grid"]);
        assert_eq!(screens.name(OVERVIEW), "Overview");
        assert_eq!(screens.name("grid"), "Grid");
        assert_eq!(screens.name("gone"), "gone");
        assert!(screens.active_screen().is_none());
        screens.active = "grid".into();
        assert_eq!(
            screens.active_screen().map(|s| s.name.as_str()),
            Some("Grid")
        );
        screens
            .layouts
            .insert("grid".into(), ScreenLayouts::default());
        assert!(!screens.ready("grid"), "its tags have not arrived");
        screens.tags.insert("grid".into(), vec![3, 99]);
        assert!(screens.ready("grid"));
        assert!(
            screens.tag_ids(&Store::default(), "grid").is_empty(),
            "tags the store does not know are left out"
        );
        let first = screens.ask(Ask::List);
        assert_ne!(
            screens.ask(Ask::List),
            first,
            "each call has its own ticket"
        );
    }
}
