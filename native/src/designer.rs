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

/// `GET scada-layout`: the rows and the layout's revision, read together.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LayoutAnswer {
    pub equipment: Vec<EquipmentRow>,
    pub connections: Vec<ConnectionRow>,
    pub revision: String,
}

/// `PUT scada-layout`: equipment and connections in one transaction,
/// refused (409) when the screen is no longer at `base_revision`.
#[derive(Debug, Serialize)]
pub struct LayoutSave<'a> {
    pub equipment: &'a [EquipmentRow],
    pub connections: &'a [ConnectionRow],
    pub base_revision: Option<&'a str>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SavedAnswer {
    revision: String,
}

/// How a save went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// Saved; the layout's new revision (`None` from a server without
    /// revisions).
    Done { revision: Option<String> },
    /// The screen changed elsewhere since the revision the edits started
    /// from; nothing was written. Its current revision, to overwrite with.
    Conflict { revision: String, message: String },
    /// A server without `scada-layout` took the equipment but not the
    /// connections (the two `PUT`s it needs): what went wrong.
    Half(String),
}

/// A screen's layout and its revision. A server from before the Designer
/// has no `scada-layout` (it may answer its web page instead): then the
/// two lists, with no revision.
pub async fn load(api: &VoltrusApi, screen: &str) -> anyhow::Result<(Layout, Option<String>)> {
    if let Ok(answer) = api.get_json::<LayoutAnswer>(&layout_path(screen)).await
        && !answer.revision.is_empty()
    {
        let layout = mimic::parse(&answer.equipment, &answer.connections);
        return Ok((layout, Some(answer.revision)));
    }
    let equipment: Items<EquipmentRow> = api.get_json(&equipment_path(screen)).await?;
    let connections: Items<ConnectionRow> = api.get_json(&connections_path(screen)).await?;
    Ok((mimic::parse(&equipment.items, &connections.items), None))
}

/// Save rows as the web Designer does: one `PUT scada-layout` based on
/// `base` (the revision loaded, or the one a conflict named to overwrite
/// it). With no revision (an older server) the two `PUT`s.
pub async fn save(
    api: &VoltrusApi,
    screen: &str,
    equipment: Vec<EquipmentRow>,
    connections: Vec<ConnectionRow>,
    base: Option<String>,
) -> anyhow::Result<Saved> {
    let Some(base) = base else {
        api.send_json::<_, serde::de::IgnoredAny>(
            Method::PUT,
            &equipment_path(screen),
            &Items { items: equipment },
        )
        .await?;
        // The equipment is in by now: a failure here leaves the server
        // half-saved, which the answer says.
        let lines = api
            .send_json::<_, serde::de::IgnoredAny>(
                Method::PUT,
                &connections_path(screen),
                &Items { items: connections },
            )
            .await;
        return Ok(match lines {
            Ok(_) => Saved::Done { revision: None },
            Err(error) => Saved::Half(error.to_string()),
        });
    };
    let body = LayoutSave {
        equipment: &equipment,
        connections: &connections,
        base_revision: Some(&base),
    };
    match api
        .send_json::<_, SavedAnswer>(Method::PUT, &layout_path(screen), &body)
        .await
    {
        Ok(answer) => Ok(Saved::Done {
            revision: Some(answer.revision).filter(|r| !r.is_empty()),
        }),
        Err(error) => match conflict(&error) {
            Some(saved) => Ok(saved),
            None => Err(error),
        },
    }
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
