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
    /// The number compared with (not finite: the case never matches, and
    /// the editor says the case needs a number).
    #[serde(default = "nan", deserialize_with = "number_or_nan")]
    pub v: f64,
    /// What the case gives; absent: the property keeps its fixed value.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub out: Option<Json>,
}

/// The comparison operators of a map case, in the editor's order.
pub const OPS: [&str; 5] = ["==", ">=", ">", "<=", "<"];

fn nan() -> f64 {
    f64::NAN
}

/// A case's number: anything but a number (`null` for the web's `NaN`)
/// is no number.
fn number_or_nan<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    Ok(Json::deserialize(deserializer)?
        .as_f64()
        .unwrap_or(f64::NAN))
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

/// A tag's base project ref: `device/key`. A tag that shares it with
/// others may have a numbered ref: see [`refs_by_id`].
pub fn tag_ref(device_id: &str, key: &str) -> String {
    format!("{device_id}/{key}")
}

/// The ref of every tag, numbered as the project exporter does
/// (`refsById`): tags in id order, the first with a given `device/key`
/// gets the plain ref and the next ones `device/key#2`, `#3`… So a ref
/// names the same tag in the console, the browser, the project file and
/// on the server.
pub fn refs_by_id<'a>(
    tags: impl IntoIterator<Item = (i64, &'a str, &'a str)>,
) -> HashMap<i64, String> {
    let mut sorted: Vec<(i64, &str, &str)> = tags.into_iter().collect();
    sorted.sort_by_key(|(id, ..)| *id);
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut out = HashMap::new();
    for (id, device, key) in sorted {
        let base = tag_ref(device, key);
        let n = counts.entry(base.clone()).or_insert(0);
        *n += 1;
        let reference = if *n == 1 { base } else { format!("{base}#{n}") };
        out.insert(id, reference);
    }
    out
}

/// Refs to tag ids (`refIndex`), numbered as [`refs_by_id`].
pub fn ref_index<'a>(
    tags: impl IntoIterator<Item = (i64, &'a str, &'a str)>,
) -> HashMap<String, i64> {
    refs_by_id(tags)
        .into_iter()
        .map(|(id, reference)| (reference, id))
        .collect()
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
        Binding::Expr { expr } | Binding::Map { expr, .. } => {
            let mut errors: Vec<String> = match expr::parse(expr) {
                Err(error) => return vec![error.message],
                Ok(node) => expr::refs(&node)
                    .into_iter()
                    .filter(|reference| !known(reference))
                    .map(|reference| format!("Unknown tag {{{reference}}}"))
                    .collect(),
            };
            if let Binding::Map { cases, .. } = binding {
                for (i, case) in cases.iter().enumerate() {
                    if !case.v.is_finite() {
                        errors.push(format!("Case {} needs a number", i + 1));
                    }
                }
            }
            errors
        }
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

/// A value as a property of `kind` takes it (`coerce`); `None` for a
/// number property that gets no finite number (it keeps its fixed value).
pub fn coerce(value: &Json, kind: Option<PropType>) -> Option<Json> {
    let as_expr = Value::from_json(value);
    Some(match kind {
        Some(PropType::Boolean) => match value {
            Json::Bool(_) => value.clone(),
            _ => Json::Bool(as_expr.truthy()),
        },
        Some(PropType::Number) => {
            let x = match value {
                Json::Null => f64::NAN,
                Json::String(s) if s.is_empty() => f64::NAN,
                _ => as_expr.num(),
            };
            if !x.is_finite() {
                return None;
            }
            Value::Num(x).to_json()
        }
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
    })
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
        Binding::Tag { tag } => coerce(&Value::Num(read(tag)?).to_json(), kind),
        Binding::Expr { expr } => {
            let node = cache.compile(expr).as_ref().ok()?;
            match expr::safe_evaluate(node, read) {
                Value::Null => None,
                value => coerce(&value.to_json(), kind),
            }
        }
        Binding::Map {
            expr,
            cases,
            otherwise,
        } => {
            let node = cache.compile(expr).as_ref().ok()?;
            let x = match expr::safe_evaluate(node, read) {
                Value::Bool(b) => f64::from(u8::from(b)),
                Value::Num(x) => x,
                _ => f64::NAN,
            };
            let hit = (!x.is_nan())
                .then(|| {
                    cases
                        .iter()
                        .find(|case| case.v.is_finite() && case.matches(x))
                })
                .flatten();
            // A hit case without a result keeps the fixed value (the web's
            // `hit ? hit.out : b.else`).
            let out = match hit {
                Some(case) => case.out.as_ref(),
                None => otherwise.as_ref(),
            }?;
            coerce(out, kind)
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
        // Tag-picker fields hold tag ids, not values: never bound (the
        // editor offers no binding for them).
        if prop == BINDINGS_KEY || kind == Some(PropType::Tag) {
            continue;
        }
        if let Some(value) = evaluate_binding(&binding, read, kind, cache) {
            out.insert(prop, value);
        }
    }
    Some(out)
}

/// Whether a widget is drawn at runtime (`shownAtRuntime`): its bound
/// `visible` when that has a value, else the fixed one.
pub fn shown_at_runtime(
    config: &Map<String, Json>,
    read: &dyn Fn(&str) -> Option<f64>,
    cache: &mut Cache,
) -> bool {
    if let Some(binding) = binding_of(config, "visible")
        && let Some(value) = evaluate_binding(&binding, read, Some(PropType::Boolean), cache)
    {
        return value != Json::Bool(false);
    }
    config.get("visible") != Some(&Json::Bool(false))
}

/// One line describing a binding, for the property grid.
pub fn describe(binding: &Binding) -> String {
    match binding {
        Binding::Tag { tag } => expr::escape_ref(tag),
        Binding::Expr { expr } => expr.clone(),
        Binding::Map { expr, cases, .. } => format!(
            "map {expr} ({} case{})",
            cases.len(),
            if cases.len() == 1 { "" } else { "s" }
        ),
    }
}
