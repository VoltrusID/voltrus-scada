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

pub mod bindings;
pub mod expr;

use std::collections::HashMap;

use serde_json::Value as Json;

use crate::mimic::edit::{self, Edit, FieldKind};
use crate::mimic::{self, ConnectionRow, EquipmentRow, Layout, Widget};

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

/// A widget's property types by key, for coercing binding results: its
/// schema's fields, the label, and the common properties.
pub fn prop_types(widget: &Widget) -> HashMap<&'static str, PropType> {
    let mut types: HashMap<&'static str, PropType> = edit::schema(widget)
        .into_iter()
        .map(|field| {
            let kind = match field.kind {
                FieldKind::Text => PropType::String,
                FieldKind::Number { .. } => PropType::Number,
                FieldKind::Flag => PropType::Boolean,
                FieldKind::Select(_) => PropType::Select,
                FieldKind::Color => PropType::Color,
                FieldKind::Tag => PropType::Tag,
            };
            (field.key, kind)
        })
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
    widget.get("locked") == Some(&Json::Bool(true))
}

#[cfg(test)]
mod tests;
