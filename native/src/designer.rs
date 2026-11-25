//! The Designer's document rules shared with the web Designer
//! (`client/src/designer/`), with no UI. The document is the same
//! [`mimic::Layout`] the mimic viewer draws and the editor saves; the
//! Designer adds:
//!
//! - [`expr`]: binding expressions, a port of `expr.ts` (same grammar,
//!   values and errors);
//! - [`bindings`]: property bindings in `config.bindings` (tag, expression,
//!   state map), a port of `bindings.ts`;
//! - the Designer's edits as [`Edit`]s, so the editor's [`History`]
//!   (undo, redo, coalescing) runs them: [`set_label`], [`bind_primary`]
//!   (clears the stored name, as `bindPrimaryPatch` does), [`set_binding`];
//! - [`rows`]: the `PUT` bodies as `saveLayoutToAPI` writes them (the name
//!   is the label, else the stored name, else the primary tag's name, else
//!   the widget's title), so a screen saved here and one saved by the web
//!   Designer after the same edits are the same rows.
//!
//! See `docs/native-designer.md`.
//!
//! [`History`]: crate::mimic::edit::History

pub mod arrange;
pub mod bindings;
pub mod collab;
pub mod expr;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::api::{Method, ServerError, VoltrusApi};
use crate::mimic::edit::{self, Edit, FieldKind};
use crate::mimic::{
    self, ConnectionRow, EquipmentRow, Items, Layout, Widget, connections_path, equipment_path,
};
use crate::screens::layout_id;

pub use bindings::{Binding, PropType};

/// The label: `config.label` as typed (an empty label is kept, as the web
/// keeps it).
pub fn set_label(widget: &Widget, label: &str) -> Edit {
    Edit::Config {
        id: widget.id.clone(),
        key: "label".into(),
        value: Json::from(label),
    }
}

/// Bind the widget's primary value to a tag, or unbind it: one step that
/// also drops the stored name, so the saved name follows the tag.
pub fn bind_primary(widget: &Widget, tag: Option<i64>) -> Edit {
    Edit::Many(vec![
        Edit::Bind {
            id: widget.id.clone(),
            tags: tag.into_iter().collect(),
        },
        Edit::Name {
            id: widget.id.clone(),
            name: String::new(),
        },
    ])
}

/// Bind one property (or unbind it with `None`).
pub fn set_binding(widget: &Widget, prop: &str, binding: Option<&Binding>) -> Edit {
    Edit::Config {
        id: widget.id.clone(),
        key: bindings::BINDINGS_KEY.into(),
        value: bindings::with_binding(&widget.config, prop, binding),
    }
}

/// The name `saveLayoutToAPI` writes for a widget.
pub fn saved_name(widget: &Widget, tag_name: &dyn Fn(i64) -> Option<String>) -> String {
    widget
        .text("label")
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .or_else(|| (!widget.name.is_empty()).then(|| widget.name.clone()))
        .or_else(|| widget.tag().and_then(tag_name).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| edit::title(widget))
}

/// The layout with every widget's name as the save writes it.
pub fn named(layout: &Layout, tag_name: &dyn Fn(i64) -> Option<String>) -> Layout {
    let mut named = layout.clone();
    for widget in &mut named.widgets {
        widget.name = saved_name(widget, tag_name);
    }
    named
}

/// The rows to `PUT` (`scada-equipment`, `scada-connections`), as the web
/// Designer saves them.
pub fn rows(
    layout: &Layout,
    tag_name: &dyn Fn(i64) -> Option<String>,
) -> (Vec<EquipmentRow>, Vec<ConnectionRow>) {
    mimic::to_rows(&named(layout, tag_name))
}

// ── Loading and saving (`GET`/`PUT scada-layout`) ──────────────────────────

/// `GET` and `PUT` path of a screen's whole layout with its revision.
pub fn layout_path(screen: &str) -> String {
    format!("/api/v1/screens/{}/scada-layout", layout_id(screen))
}

/// `GET scada-layout?published=1`: what operators see (the published copy
/// of the screen, or its draft when it was never published).
pub fn published_layout_path(screen: &str) -> String {
    format!("{}?published=1", layout_path(screen))
}

/// `GET scada-layout`: the rows and the layout's revision, read together.
/// A server with version history also says which rows they are (`view`:
/// `draft` or `published`) and, for the published ones, whether the draft
/// differs from them.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LayoutAnswer {
    pub equipment: Vec<EquipmentRow>,
    pub connections: Vec<ConnectionRow>,
    pub revision: String,
    pub view: String,
    pub draft_differs: bool,
}

/// A screen as one reader sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct View {
    pub layout: Layout,
    /// The draft's revision (a save is based on it); `None` from a server
    /// without revisions.
    pub revision: Option<String>,
    /// The rows are the published copy (not the draft an editor changes).
    pub published: bool,
    /// The published copy is shown and the draft differs from it.
    pub draft_differs: bool,
}

/// `PUT scada-layout`: equipment and connections in one transaction,
/// refused (409) when the screen is no longer at `base_revision`.
#[derive(Debug, Serialize)]
pub struct LayoutSave<'a> {
    pub equipment: &'a [EquipmentRow],
    pub connections: &'a [ConnectionRow],
    pub base_revision: Option<&'a str>,
}

/// `PUT` of the two older endpoints (`scada-equipment`,
/// `scada-connections`), which take a `base_revision` too.
#[derive(Debug, Serialize)]
struct ItemsSave<T> {
    items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_revision: Option<String>,
}

/// How a save went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// Saved; the layout's new revision (`None`: the server named none,
    /// so keep the one the edits were based on, never drop the check).
    Done { revision: Option<String> },
    /// The screen changed elsewhere since the revision the edits started
    /// from; nothing was written. Its current revision, to overwrite with.
    Conflict { revision: String, message: String },
    /// A server without `scada-layout` took the equipment but not the
    /// connections (the two `PUT`s it needs): what went wrong.
    Half(String),
}

/// Whether a failed request means the server predates the endpoint: 404
/// or 405, or a success that is not JSON at all (a server from before it
/// answers its web page). Anything else (refused, signed out, a server
/// error, a timeout, JSON of the wrong shape) is an error to show.
pub fn older_server(error: &anyhow::Error) -> bool {
    if let Some(server) = error.downcast_ref::<ServerError>() {
        return matches!(
            server.status,
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
        );
    }
    error
        .downcast_ref::<serde_json::Error>()
        .is_some_and(serde_json::Error::is_syntax)
}

/// The revision in a save's or a load's answer, when it names one.
fn revision_in(answer: &Json) -> Option<String> {
    answer
        .get("revision")
        .and_then(Json::as_str)
        .filter(|revision| !revision.is_empty())
        .map(str::to_owned)
}

/// A screen's layout and its revision. A server from before the Designer
/// has no `scada-layout` (404, 405, or its web page instead of JSON):
/// then the two lists, with no revision. Any other failure is the error.
pub async fn load(api: &VoltrusApi, screen: &str) -> anyhow::Result<(Layout, Option<String>)> {
    let view = load_view(api, screen, false).await?;
    Ok((view.layout, view.revision))
}

/// A screen's layout as the editor (`published: false`, the draft) or the
/// run mode (`published: true`, what operators see) reads it. A server
/// from before version history ignores `?published=1` and answers the only
/// rows it has; one from before `scada-layout` gets the two lists.
pub async fn load_view(api: &VoltrusApi, screen: &str, published: bool) -> anyhow::Result<View> {
    let path = if published {
        published_layout_path(screen)
    } else {
        layout_path(screen)
    };
    match api.get_json::<Json>(&path).await {
        Ok(answer) => {
            let revision = revision_in(&answer).ok_or_else(|| {
                anyhow::anyhow!("the server's scada-layout answer has no revision")
            })?;
            let answer: LayoutAnswer = serde_json::from_value(answer)?;
            let layout = mimic::parse(&answer.equipment, &answer.connections);
            let shown_published = answer.view == "published";
            return Ok(View {
                layout,
                revision: Some(revision),
                published: shown_published,
                draft_differs: shown_published && answer.draft_differs,
            });
        }
        Err(error) if older_server(&error) => {}
        Err(error) => return Err(error),
    }
    let equipment: Items<EquipmentRow> = api.get_json(&equipment_path(screen)).await?;
    let connections: Items<ConnectionRow> = api.get_json(&connections_path(screen)).await?;
    Ok(View {
        layout: mimic::parse(&equipment.items, &connections.items),
        ..View::default()
    })
}

/// The layout's current revision (`GET scada-layout`), for a save whose
/// answer named none.
async fn current_revision(api: &VoltrusApi, screen: &str) -> Option<String> {
    let answer = api.get_json::<Json>(&layout_path(screen)).await.ok()?;
    revision_in(&answer)
}

/// Save rows as the web Designer does: one `PUT scada-layout` based on
/// `base` (the revision loaded, or the one a conflict named to overwrite
/// it). With no revision (an older server), or a server without
/// `scada-layout`, the two `PUT`s, which carry `base` too when it is known.
pub async fn save(
    api: &VoltrusApi,
    screen: &str,
    equipment: Vec<EquipmentRow>,
    connections: Vec<ConnectionRow>,
    base: Option<String>,
) -> anyhow::Result<Saved> {
    if let Some(base) = &base {
        let body = LayoutSave {
            equipment: &equipment,
            connections: &connections,
            base_revision: Some(base),
        };
        match api
            .send_json::<_, Json>(Method::PUT, &layout_path(screen), &body)
            .await
        {
            Ok(answer) => {
                let revision = match revision_in(&answer) {
                    Some(revision) => Some(revision),
                    None => current_revision(api, screen).await,
                };
                return Ok(Saved::Done { revision });
            }
            Err(error) if older_server(&error) => {}
            Err(error) => {
                return match conflict(&error) {
                    Some(saved) => Ok(saved),
                    None => Err(error),
                };
            }
        }
    }
    let first = api
        .send_json::<_, Json>(
            Method::PUT,
            &equipment_path(screen),
            &ItemsSave {
                items: equipment,
                base_revision: base.clone(),
            },
        )
        .await;
    let first = match first {
        Ok(answer) => answer,
        Err(error) => {
            return match conflict(&error) {
                Some(saved) => Ok(saved),
                None => Err(error),
            };
        }
    };
    // The second save is based on what the first made (or, from a server
    // that names no revision, on what the first was based on).
    let after_first = revision_in(&first).or(base);
    // The equipment is in by now: a failure here leaves the server
    // half-saved, which the answer says.
    let lines = api
        .send_json::<_, Json>(
            Method::PUT,
            &connections_path(screen),
            &ItemsSave {
                items: connections,
                base_revision: after_first.clone(),
            },
        )
        .await;
    Ok(match lines {
        Ok(answer) => Saved::Done {
            revision: revision_in(&answer).or(after_first),
        },
        Err(error) => Saved::Half(error.to_string()),
    })
}

/// A 409 with the current revision: the conflict.
pub fn conflict(error: &anyhow::Error) -> Option<Saved> {
    let server = error.downcast_ref::<ServerError>()?;
    if server.status != reqwest::StatusCode::CONFLICT {
        return None;
    }
    Some(Saved::Conflict {
        revision: server.revision.clone()?,
        message: server.message.clone(),
    })
}

/// A widget's property types by key, for coercing binding results: its
/// schema's fields, the label, and the common properties.
pub fn prop_types(widget: &Widget) -> HashMap<&'static str, PropType> {
    let mut types: HashMap<&'static str, PropType> = edit::schema(widget)
        .into_iter()
        .map(|field| (field.key, prop_type(field.kind)))
        .collect();
    types.entry("label").or_insert(PropType::String);
    for (key, _, kind, _) in bindings::COMMON_PROPS {
        types.entry(key).or_insert(kind);
    }
    types
}

/// The widget as drawn now: bound properties replaced by their values
/// (`None`: nothing bound, draw it as it is).
pub fn bound(
    widget: &Widget,
    read: &dyn Fn(&str) -> Option<f64>,
    cache: &mut expr::Cache,
) -> Option<Widget> {
    if !bindings::has_bindings(&widget.config) {
        return None;
    }
    let types = prop_types(widget);
    let config = bindings::apply_bindings(
        &widget.config,
        read,
        &|prop| types.get(prop).copied(),
        cache,
    )?;
    Some(Widget {
        config,
        ..widget.clone()
    })
}

/// [`bound`] for every widget a screen draws, frame after frame: each
/// widget's bindings are parsed when its config changes and evaluated
/// again only when a tag they read changes, so a large screen costs a
/// lookup and a comparison per bound widget per frame.
#[derive(Debug, Default)]
pub struct BoundCache {
    widgets: HashMap<String, BoundEntry>,
    frame: u64,
}

#[derive(Debug)]
struct BoundEntry {
    /// The widget as last asked for.
    source: Widget,
    compiled: bindings::Compiled,
    /// The values its refs had (bits; `None`: no value).
    inputs: Vec<Option<u64>>,
    out: Option<Widget>,
    /// The frame it was last asked for.
    seen: u64,
}

impl BoundCache {
    /// Frames a widget not drawn (off screen, deleted) is kept for.
    pub const KEEP_FRAMES: u64 = 600;

    /// A new frame: what has not been asked for in a while goes.
    pub fn frame(&mut self) {
        self.frame += 1;
        if self.frame.is_multiple_of(64) {
            let now = self.frame;
            self.widgets
                .retain(|_, entry| now - entry.seen <= Self::KEEP_FRAMES);
        }
    }

    /// Widgets kept.
    pub fn len(&self) -> usize {
        self.widgets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }

    /// The widget as drawn now (as [`bound`]; `None`: draw it as it is).
    pub fn bound(
        &mut self,
        widget: &Widget,
        read: &dyn Fn(&str) -> Option<f64>,
    ) -> Option<&Widget> {
        if !bindings::has_bindings(&widget.config) {
            self.widgets.remove(&widget.id);
            return None;
        }
        let frame = self.frame;
        match self.widgets.get_mut(&widget.id) {
            Some(entry) if entry.source == *widget => {}
            Some(entry)
                if entry.source.config == widget.config && entry.source.kind == widget.kind =>
            {
                // Moved or resized: same bindings, drawn again.
                entry.source = widget.clone();
                entry.out = None;
            }
            _ => {
                let types = prop_types(widget);
                let compiled =
                    bindings::Compiled::new(&widget.config, &|prop| types.get(prop).copied());
                self.widgets.insert(
                    widget.id.clone(),
                    BoundEntry {
                        source: widget.clone(),
                        compiled,
                        inputs: Vec::new(),
                        out: None,
                        seen: frame,
                    },
                );
            }
        }
        let entry = self.widgets.get_mut(&widget.id)?;
        entry.seen = frame;
        if entry.compiled.is_empty() {
            return None;
        }
        let refs = entry.compiled.refs();
        let mut changed = entry.out.is_none() || entry.inputs.len() != refs.len();
        if changed {
            entry.inputs = refs.iter().map(|r| read(r).map(f64::to_bits)).collect();
        } else {
            for (slot, reference) in entry.inputs.iter_mut().zip(refs) {
                let now = read(reference).map(f64::to_bits);
                if *slot != now {
                    *slot = now;
                    changed = true;
                }
            }
        }
        if changed {
            entry.out = Some(Widget {
                config: entry.compiled.apply(&widget.config, read),
                ..widget.clone()
            });
        }
        entry.out.as_ref()
    }
}

/// Whether a widget is drawn at runtime: `config.visible` is not false
/// (the Designer draws hidden widgets faded).
pub fn visible(widget: &Widget) -> bool {
    widget.flag("visible") != Some(false)
}

/// Whether a widget is locked (`config.locked`): not moved, resized,
/// band-selected or deleted.
pub fn locked(widget: &Widget) -> bool {
    edit::is_locked(widget)
}

/// How opaque a widget is drawn, from its (bound) config: at runtime
/// `config.opacity` clamped to 0–1; in the Designer a hidden widget at
/// 0.3 and the rest at least 0.15, so nothing vanishes while designing.
/// `None` draws it as it is.
pub fn opacity(widget: &Widget, designing: bool) -> Option<f32> {
    let set = widget.number("opacity").filter(|o| o.is_finite());
    #[allow(clippy::cast_possible_truncation)]
    if designing {
        if !visible(widget) {
            return Some(0.3);
        }
        set.map(|o| o.clamp(0.15, 1.0) as f32)
    } else {
        set.map(|o| o.clamp(0.0, 1.0) as f32)
    }
}

/// One property the property editor lists (`propertiesOf`): the widget's
/// schema fields, a label when the schema has none, then the common
/// properties (`visible`, `opacity`).
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: PropType,
    /// The schema field, when the property is one (the label and the
    /// common properties may not be).
    pub field: Option<edit::Field>,
}

impl Prop {
    /// Whether it can be bound: anything but a tag picker (that holds a
    /// tag id; there is nothing to bind).
    pub fn bindable(&self) -> bool {
        self.kind != PropType::Tag
    }
}

fn prop_type(kind: FieldKind) -> PropType {
    match kind {
        FieldKind::Text => PropType::String,
        FieldKind::Number { .. } => PropType::Number,
        FieldKind::Flag => PropType::Boolean,
        FieldKind::Select(_) => PropType::Select,
        FieldKind::Color => PropType::Color,
        FieldKind::Tag => PropType::Tag,
    }
}

/// Everything the property editor lists for a widget.
pub fn properties(widget: &Widget) -> Vec<Prop> {
    let mut out: Vec<Prop> = edit::schema(widget)
        .into_iter()
        .map(|field| Prop {
            key: field.key,
            label: field.label,
            kind: prop_type(field.kind),
            field: Some(field),
        })
        .collect();
    if !out.iter().any(|prop| prop.key == "label") {
        out.insert(
            0,
            Prop {
                key: "label",
                label: "Label",
                kind: PropType::String,
                field: None,
            },
        );
    }
    for (key, label, kind, _) in bindings::COMMON_PROPS {
        if !out.iter().any(|prop| prop.key == key) {
            out.push(Prop {
                key,
                label,
                kind,
                field: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests;
