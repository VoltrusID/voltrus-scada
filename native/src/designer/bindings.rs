//! Property bindings: a port of the web Designer's
//! `client/src/designer/bindings.ts`. Any widget property can follow a tag,
//! an expression or a state-to-value map instead of holding a fixed value.
//! Bindings live in the widget's `config.bindings`, keyed by property name,
//! so they travel with the screen through the REST rows, MCP and the
//! project file unchanged:
//!
//! ```json
//! "bindings": {
//!   "label":   { "kind": "expr", "expr": "\"P-101A \" + text({wtp/intake_flow}, 0)" },
//!   "visible": { "kind": "tag",  "tag": "wtp/p101a_run" },
//!   "color":   { "kind": "map",  "expr": "{wtp/p101b_fault}",
//!                "cases": [{ "op": ">=", "v": 1, "out": "#f04646" }], "else": "#8b939e" }
//! }
//! ```
//!
//! Tags are named by project ref (`device/key`). The widget's primary
//! value tag stays in `sensor_bindings`.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value as Json};

use super::expr::{self, Cache, Value};

/// The config key the bindings live under.
pub const BINDINGS_KEY: &str = "bindings";

/// One property's binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Binding {
    Tag {
        tag: String,
    },
    Expr {
        expr: String,
    },
    Map {
        expr: String,
        cases: Vec<MapCase>,
        /// What no case matches gives; absent: the fixed value stays.
        #[serde(
            default,
            rename = "else",
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        otherwise: Option<Json>,
    },
}

/// A JSON value that is there, `null` included.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Json>, D::Error> {
    Json::deserialize(deserializer).map(Some)
}

/// One case of a map binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapCase {
    /// `==`, `>=`, `<=`, `>` or `<`.
    pub op: String,
    pub v: f64,
    pub out: Json,
}

impl MapCase {
    #[allow(clippy::float_cmp)]
    fn matches(&self, x: f64) -> bool {
        match self.op.as_str() {
            "==" => x == self.v,
            ">=" => x >= self.v,
            "<=" => x <= self.v,
            ">" => x > self.v,
            _ => x < self.v,
        }
    }
}

/// A property's type, which a binding's result is coerced to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropType {
    String,
    Number,
    Boolean,
    Color,
    Select,
    Tag,
}

/// Properties every widget has besides its own schema: key, label, type,
/// default.
pub const COMMON_PROPS: [(&str, &str, PropType, bool); 2] = [
    ("visible", "Visible", PropType::Boolean, true),
    ("opacity", "Opacity", PropType::Number, false),
];

/// A tag's project ref: `device/key`.
pub fn tag_ref(device_id: &str, key: &str) -> String {
    format!("{device_id}/{key}")
}

/// Refs to tag ids; the first tag of a duplicated ref wins.
pub fn ref_index<'a>(
    tags: impl IntoIterator<Item = (i64, &'a str, &'a str)>,
) -> HashMap<String, i64> {
    let mut index = HashMap::new();
    for (id, device, key) in tags {
        index.entry(tag_ref(device, key)).or_insert(id);
    }
    index
}

/// The bindings in a config, by property; entries that are not a binding
/// the editor knows are left out (and kept in the config as they are).
pub fn bindings_of(config: &Map<String, Json>) -> Vec<(String, Binding)> {
    let Some(Json::Object(map)) = config.get(BINDINGS_KEY) else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(prop, raw)| {
            serde_json::from_value(raw.clone())
                .ok()
                .map(|binding| (prop.clone(), binding))
        })
        .collect()
}

/// One property's binding.
pub fn binding_of(config: &Map<String, Json>, prop: &str) -> Option<Binding> {
    let raw = config.get(BINDINGS_KEY)?.get(prop)?;
    serde_json::from_value(raw.clone()).ok()
}

/// Whether anything is bound.
pub fn has_bindings(config: &Map<String, Json>) -> bool {
    matches!(config.get(BINDINGS_KEY), Some(Json::Object(map)) if !map.is_empty())
}

/// The `bindings` object with one property bound (or unbound with `None`);
/// `Null` when nothing is left, which removes the key.
pub fn with_binding(config: &Map<String, Json>, prop: &str, binding: Option<&Binding>) -> Json {
    let mut next = match config.get(BINDINGS_KEY) {
        Some(Json::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    match binding {
        Some(binding) => {
            next.insert(
                prop.to_owned(),
                serde_json::to_value(binding).unwrap_or(Json::Null),
            );
        }
        None => {
            next.remove(prop);
        }
    }
    if next.is_empty() {
        Json::Null
    } else {
        Json::Object(next)
    }
}

/// Problems in a binding, for the property editor. Empty means fine.
pub fn binding_errors(binding: &Binding, known: &dyn Fn(&str) -> bool) -> Vec<String> {
    match binding {
        Binding::Tag { tag } if tag.is_empty() => vec!["Pick a tag".into()],
        Binding::Tag { tag } if !known(tag) => vec![format!("Unknown tag {tag}")],
        Binding::Tag { .. } => Vec::new(),
        Binding::Expr { expr } | Binding::Map { expr, .. } => match expr::parse(expr) {
            Err(error) => vec![error.message],
            Ok(node) => expr::refs(&node)
                .into_iter()
                .filter(|reference| !known(reference))
                .map(|reference| format!("Unknown tag {{{reference}}}"))
                .collect(),
        },
    }
}

/// Every tag ref a config's bindings read.
pub fn binding_refs(config: &Map<String, Json>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (_, binding) in bindings_of(config) {
        match binding {
            Binding::Tag { tag } => {
                if !tag.is_empty() {
                    out.insert(tag);
                }
            }
            Binding::Expr { expr } | Binding::Map { expr, .. } => {
                if let Ok(node) = expr::parse(&expr) {
                    out.extend(expr::refs(&node));
                }
            }
        }
    }
    out
}

/// A value as a property of `kind` takes it (`coerce`).
pub fn coerce(value: &Json, kind: Option<PropType>) -> Json {
    let as_expr = Value::from_json(value);
    match kind {
        Some(PropType::Boolean) => match value {
            Json::Bool(_) => value.clone(),
            _ => Json::Bool(as_expr.truthy()),
        },
        Some(PropType::Number) => match value {
            Json::Number(_) | Json::Null => value.clone(),
            _ => Value::Num(as_expr.num()).to_json(),
        },
        Some(PropType::String) => match value {
            Json::Null => Json::String(String::new()),
            Json::Number(n) => {
                let x = n.as_f64().unwrap_or(f64::NAN);
                Json::String(expr::js_number((x * 1000.0 + 0.5).floor() / 1000.0))
            }
            Json::String(_) => value.clone(),
            Json::Bool(b) => Json::String(b.to_string()),
            other => Json::String(other.to_string()),
        },
        _ => value.clone(),
    }
}

/// The value a binding gives now; `None` when it cannot be evaluated (the
/// property keeps its fixed value).
pub fn evaluate_binding(
    binding: &Binding,
    read: &dyn Fn(&str) -> Option<f64>,
    kind: Option<PropType>,
    cache: &mut Cache,
) -> Option<Json> {
    match binding {
        Binding::Tag { tag } => read(tag).map(|v| coerce(&Value::Num(v).to_json(), kind)),
        Binding::Expr { expr } => {
            let node = cache.compile(expr).as_ref().ok()?;
            match expr::evaluate(node, read) {
                Value::Null => None,
                value => Some(coerce(&value.to_json(), kind)),
            }
        }
        Binding::Map {
            expr,
            cases,
            otherwise,
        } => {
            let node = cache.compile(expr).as_ref().ok()?;
            let x = match expr::evaluate(node, read) {
                Value::Bool(b) => f64::from(u8::from(b)),
                Value::Num(x) => x,
                _ => f64::NAN,
            };
            let hit = (!x.is_nan())
                .then(|| cases.iter().find(|case| case.matches(x)))
                .flatten();
            let out = hit.map(|case| &case.out).or(otherwise.as_ref())?;
            Some(coerce(out, kind))
        }
    }
}

/// The default type of a common property.
fn common_type(prop: &str) -> Option<PropType> {
    COMMON_PROPS
        .iter()
        .find(|(key, ..)| *key == prop)
        .map(|(_, _, kind, _)| *kind)
}

/// The config a widget is drawn with: every bound property replaced by its
/// current value. `types` gives the property types (from the widget's
/// schema). `None` when nothing is bound (draw the config as it is).
pub fn apply_bindings(
    config: &Map<String, Json>,
    read: &dyn Fn(&str) -> Option<f64>,
    types: &dyn Fn(&str) -> Option<PropType>,
    cache: &mut Cache,
) -> Option<Map<String, Json>> {
    let bindings = bindings_of(config);
    if bindings.is_empty() {
        return None;
    }
    let mut out = config.clone();
    for (prop, binding) in bindings {
        let kind = types(&prop).or_else(|| common_type(&prop));
        if let Some(value) = evaluate_binding(&binding, read, kind, cache) {
            out.insert(prop, value);
        }
    }
    Some(out)
}

/// One line describing a binding, for the property grid.
pub fn describe(binding: &Binding) -> String {
    match binding {
        Binding::Tag { tag } => format!("{{{tag}}}"),
        Binding::Expr { expr } => expr.clone(),
        Binding::Map { expr, cases, .. } => format!(
            "map {expr} ({} case{})",
            cases.len(),
            if cases.len() == 1 { "" } else { "s" }
        ),
    }
}
