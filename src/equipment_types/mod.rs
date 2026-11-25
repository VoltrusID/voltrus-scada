//! Equipment types and instances (docs/equipment-types.md).
//!
//! A *type* (Pump, Valve, Tank, ...) says once which tags one piece of
//! equipment has (by key pattern, and optionally by register offset from a
//! base address), their limits, alarms and write policy, the faceplate, the
//! mimic symbol and how the symbol binds. An *instance* is one line:
//! `{name: P-101A, type: pump, device: wtp}`. Resolving an instance against
//! the tags and devices that exist ([`resolve::resolve`]) gives the tag of
//! every role, the registers and tags to create, and the patch that binds a
//! SCADA symbol to it ([`resolve::widget_patch`]).
//!
//! The same model serves the REST routes, the MCP tools and the project
//! format, where instances are expanded at plan time
//! (`project_yaml/equipment.rs`).

pub mod library;
pub mod resolve;
pub mod symbols;

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::project_yaml::spec::{FACEPLATE_ROLE_KINDS, REGISTER_AREAS, REGISTER_TYPES};

/// `config` keys of a SCADA symbol that equipment binding owns; a type's
/// `symbol_config` may not set them.
pub const OWNED_CONFIG_KEYS: &[&str] = &[
    "equipment",
    "symbol",
    "faceplate",
    "faceplateMap",
    "label",
    "runSensorId",
    "faultSensorId",
    "manualSensorId",
    "interlockSensorId",
];

/// Type ids that name a route under `/api/v1/equipment-types/`.
pub const RESERVED_TYPE_IDS: &[&str] = &["library"];

/// An instance's prefix as uniqueness compares it: in lower case, with what a
/// flow id cannot hold turned into `-` (the form [`resolve::flow_id`] uses),
/// so two prefixes that give the same flow ids are the same prefix.
pub fn prefix_identity(prefix: &str) -> String {
    let id = resolve::flow_id(prefix, "");
    id.strip_suffix('-').unwrap_or(&id).to_string()
}

/// The symbol `config` key a [`TypeBindings`] entry fills.
pub const BINDING_KEYS: &[(&str, &str)] = &[
    ("fault", "faultSensorId"),
    ("run", "runSensorId"),
    ("manual", "manualSensorId"),
    ("interlock", "interlockSensorId"),
];

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_empty_str(s: &str) -> bool {
    s.is_empty()
}
fn default_register_type() -> String {
    "float32".into()
}
fn is_float32(s: &str) -> bool {
    s == "float32"
}
fn default_area() -> String {
    "holding".into()
}
fn is_holding(s: &str) -> bool {
    s == "holding"
}
fn default_scale() -> f64 {
    1.0
}
fn is_one(f: &f64) -> bool {
    *f == 1.0
}
fn default_key() -> String {
    "{prefix}_{role}".into()
}

/// Alarm limits a type (or an override) sets on a tag. Only the fields given
/// are applied; the tag keeps the others. Same meaning as a tag's `alarm` in
/// the project format.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TagAlarm {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_warn: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_crit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadband: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_delay_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_delay_secs: Option<i64>,
}

impl TagAlarm {
    /// `self` with every field `over` sets replaced.
    pub fn merged(&self, over: &TagAlarm) -> TagAlarm {
        TagAlarm {
            warn: over.warn.or(self.warn),
            crit: over.crit.or(self.crit),
            low_warn: over.low_warn.or(self.low_warn),
            low_crit: over.low_crit.or(self.low_crit),
            deadband: over.deadband.or(self.deadband),
            on_delay_secs: over.on_delay_secs.or(self.on_delay_secs),
            off_delay_secs: over.off_delay_secs.or(self.off_delay_secs),
        }
    }
    pub fn is_empty(&self) -> bool {
        *self == TagAlarm::default()
    }
}

/// Write policy a type (or an override) sets on a tag; only the fields given
/// are applied.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TagWrite {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rate: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_reason: Option<bool>,
}

impl TagWrite {
    pub fn merged(&self, over: &TagWrite) -> TagWrite {
        TagWrite {
            writable: over.writable.or(self.writable),
            min: over.min.or(self.min),
            max: over.max.or(self.max),
            max_rate: over.max_rate.or(self.max_rate),
            requires_reason: over.requires_reason.or(self.requires_reason),
        }
    }
    pub fn is_empty(&self) -> bool {
        *self == TagWrite::default()
    }
}

/// One tag of a type, named by its role (`run`, `fault`, `level`, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TypeTag {
    pub role: String,
    #[serde(default, skip_serializing_if = "is_empty_str")]
    pub label: String,
    /// `state`, `fault`, `analog`, `command` or `setpoint` (faceplate role kinds).
    pub kind: String,
    /// Tag key pattern; default `{prefix}_{role}`.
    #[serde(default = "default_key")]
    pub key: String,
    /// Tag name pattern; `null` leaves an existing tag's name alone (a new
    /// tag is named `{name} {label}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Register offset from the instance's `base` (Modbus).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
    #[serde(default = "default_register_type", skip_serializing_if = "is_float32")]
    pub register_type: String,
    #[serde(default = "default_area", skip_serializing_if = "is_holding")]
    pub register_area: String,
    #[serde(default = "default_scale", skip_serializing_if = "is_one")]
    pub scale: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alarm: Option<TagAlarm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<TagWrite>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// Role whose tag this role uses when it has none of its own (a start
    /// command written to the run tag).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

/// Which roles drive the symbol: its primary tag and the fault, run (flow,
/// agitator, backwash), manual-mode and interlock indicators.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TypeBindings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interlock: Option<String>,
}

impl TypeBindings {
    /// `(name, role)` of every binding that is set, in a fixed order.
    pub fn entries(&self) -> Vec<(&'static str, &str)> {
        [
            ("primary", &self.primary),
            ("fault", &self.fault),
            ("run", &self.run),
            ("manual", &self.manual),
            ("interlock", &self.interlock),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.as_deref().map(|r| (k, r)))
        .collect()
    }
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries()
            .into_iter()
            .find(|(k, _)| *k == name)
            .map(|(_, r)| r)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EquipmentType {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Palette grouping in the Designer.
    #[serde(default)]
    pub category: String,
    /// Key of the mimic symbol (`symbols/<key>.json` or a built-in key).
    pub symbol: String,
    /// Faceplate definition its symbols open.
    #[serde(default)]
    pub faceplate: Option<String>,
    #[serde(default)]
    pub tags: Vec<TypeTag>,
    #[serde(default)]
    pub bindings: TypeBindings,
    /// `config` keys set on every symbol of its instances.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub symbol_config: Map<String, Value>,
    /// Flow templates created per instance (`<prefix>-<id>`); strings may
    /// use the instance placeholders and `{ref:<role>}`, `{key:<role>}`.
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub flows: Vec<Value>,
}

/// What one instance changes on one role of its type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alarm: Option<TagAlarm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<TagWrite>,
    /// The instance has no tag for this role.
    #[serde(default, skip_serializing_if = "is_false")]
    pub skip: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EquipmentInstance {
    pub name: String,
    #[serde(rename = "type")]
    pub type_id: String,
    pub device: String,
    #[serde(default)]
    pub base: Option<i64>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub description: String,
    /// Symbol instead of the type's.
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub overrides: BTreeMap<String, RoleOverride>,
}

impl EquipmentInstance {
    /// The `{prefix}` of its patterns: the one given, else the name in lower
    /// case with everything but letters and digits dropped (`P-101A` → `p101a`).
    pub fn prefix(&self) -> String {
        match self.prefix.as_deref() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => default_prefix(&self.name),
        }
    }
}

pub fn default_prefix(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

pub fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Values of the placeholders of one instance.
pub struct Vars<'a> {
    pub prefix: &'a str,
    pub name: &'a str,
    pub device: &'a str,
    pub role: &'a str,
    pub label: &'a str,
}

/// Placeholder names a key or name pattern may use.
pub const PLACEHOLDERS: &[&str] = &["prefix", "name", "device", "role", "label"];

/// Expand `{prefix}`, `{name}`, `{device}`, `{role}`, `{label}` and, through
/// `extra`, `{ref:<role>}` / `{key:<role>}`. An unknown placeholder or an
/// unclosed `{` is an error naming it.
pub fn expand(
    pattern: &str,
    vars: &Vars<'_>,
    extra: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else {
            return Err(format!("'{pattern}' has a '{{' without '}}'"));
        };
        let name = &after[..j];
        let value = match name {
            "prefix" => Some(vars.prefix.to_string()),
            "name" => Some(vars.name.to_string()),
            "device" => Some(vars.device.to_string()),
            "role" => Some(vars.role.to_string()),
            "label" => Some(vars.label.to_string()),
            other => extra(other),
        };
        match value {
            Some(v) => out.push_str(&v),
            None => {
                return Err(format!(
                    "'{pattern}' uses an unknown placeholder {{{name}}}"
                ));
            }
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn check_pattern(
    out: &mut Vec<String>,
    at: &str,
    pattern: &str,
    extra: &dyn Fn(&str) -> Option<String>,
) {
    let vars = Vars {
        prefix: "p",
        name: "N",
        device: "d",
        role: "r",
        label: "l",
    };
    if let Err(e) = expand(pattern, &vars, extra) {
        out.push(format!("{at}: {e}"));
    }
}

/// Every string in a JSON value, with its path.
fn strings<'a>(v: &'a Value, path: String, out: &mut Vec<(String, &'a str)>) {
    match v {
        Value::String(s) => out.push((path, s)),
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                strings(x, format!("{path}[{i}]"), out);
            }
        }
        Value::Object(o) => {
            for (k, x) in o {
                strings(x, format!("{path}.{k}"), out);
            }
        }
        _ => {}
    }
}

impl EquipmentType {
    pub fn tag(&self, role: &str) -> Option<&TypeTag> {
        self.tags.iter().find(|t| t.role == role)
    }

    /// Every problem, each naming its field. Does not check that the
    /// faceplate exists (that needs the database; see the callers).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !is_slug(&self.id) {
            out.push(format!(
                "id '{}' must be 1-64 lowercase letters, digits, '_' or '-'",
                self.id
            ));
        }
        if RESERVED_TYPE_IDS.contains(&self.id.as_str()) {
            out.push(format!(
                "id '{}' is reserved (it is a route: /api/v1/equipment-types/{})",
                self.id, self.id
            ));
        }
        if self.name.trim().is_empty() {
            out.push("name must not be empty".into());
        }
        if !symbols::is_known(&self.symbol) {
            out.push(format!("symbol '{}' is not a known symbol", self.symbol));
        }
        if let Some(fp) = &self.faceplate
            && !is_slug(fp)
        {
            out.push(format!("faceplate '{fp}' is not a faceplate id"));
        }
        let roles: HashSet<&str> = self.tags.iter().map(|t| t.role.as_str()).collect();
        let mut seen = HashSet::new();
        for (i, t) in self.tags.iter().enumerate() {
            let at = format!("tags[{i}]");
            if !is_slug(&t.role) {
                out.push(format!(
                    "{at}.role '{}' must be 1-64 lowercase letters, digits, '_' or '-'",
                    t.role
                ));
            }
            if !seen.insert(t.role.as_str()) {
                out.push(format!("{at}.role '{}' is listed twice", t.role));
            }
            if !FACEPLATE_ROLE_KINDS.contains(&t.kind.as_str()) {
                out.push(format!(
                    "{at}.kind '{}' is not one of {}",
                    t.kind,
                    FACEPLATE_ROLE_KINDS.join(", ")
                ));
            }
            if t.key.trim().is_empty() {
                out.push(format!("{at}.key must not be empty"));
            }
            check_pattern(&mut out, &format!("{at}.key"), &t.key, &|_| None);
            if let Some(n) = &t.name {
                check_pattern(&mut out, &format!("{at}.name"), n, &|_| None);
            }
            if let Some(o) = t.offset
                && !(0..=65535).contains(&o)
            {
                out.push(format!("{at}.offset {o} must be 0-65535"));
            }
            if !REGISTER_TYPES.contains(&t.register_type.as_str()) {
                out.push(format!(
                    "{at}.register_type '{}' is not one of {}",
                    t.register_type,
                    REGISTER_TYPES.join(", ")
                ));
            }
            if !REGISTER_AREAS.contains(&t.register_area.as_str()) {
                out.push(format!(
                    "{at}.register_area '{}' is not one of {}",
                    t.register_area,
                    REGISTER_AREAS.join(", ")
                ));
            }
            if !t.scale.is_finite() || t.scale == 0.0 {
                out.push(format!("{at}.scale must be a non-zero number"));
            }
            if let Some(d) = t.decimals
                && !(0..=10).contains(&d)
            {
                out.push(format!("{at}.decimals {d} must be 0-10"));
            }
            if let (Some(lo), Some(hi)) = (t.min, t.max)
                && lo > hi
            {
                out.push(format!("{at}: min {lo} is above max {hi}"));
            }
            if let Some(f) = &t.fallback {
                if f == &t.role {
                    out.push(format!("{at}.fallback names the role itself"));
                } else {
                    match self.tag(f) {
                        None => out.push(format!("{at}.fallback names an unknown role '{f}'")),
                        Some(target) if target.fallback.is_some() => out.push(format!(
                            "{at}.fallback '{f}' has a fallback itself; fallbacks do not chain"
                        )),
                        _ => {}
                    }
                }
            }
        }
        for (name, role) in self.bindings.entries() {
            if !roles.contains(role) {
                out.push(format!("bindings.{name} names an unknown role '{role}'"));
            }
        }
        for k in self.symbol_config.keys() {
            if OWNED_CONFIG_KEYS.contains(&k.as_str()) {
                out.push(format!(
                    "symbol_config.{k} is set from the instance and cannot be fixed by the type"
                ));
            }
        }
        let mut flow_ids = HashSet::new();
        for (i, f) in self.flows.iter().enumerate() {
            let at = format!("flows[{i}]");
            let Some(obj) = f.as_object() else {
                out.push(format!("{at} must be a mapping"));
                continue;
            };
            match obj.get("id").and_then(Value::as_str) {
                Some(id) if is_slug(id) => {
                    if !flow_ids.insert(id.to_string()) {
                        out.push(format!("{at}.id '{id}' is listed twice"));
                    }
                }
                _ => out.push(format!(
                    "{at}.id must be 1-64 lowercase letters, digits, '_' or '-'"
                )),
            }
            for field in ["name", "trigger", "nodes"] {
                if !obj.contains_key(field) {
                    out.push(format!("{at}.{field} is missing"));
                }
            }
            for k in obj.keys() {
                if !["id", "name", "enabled", "trigger", "nodes"].contains(&k.as_str()) {
                    out.push(format!("{at}: unknown field '{k}'"));
                }
            }
            let mut texts = Vec::new();
            strings(f, at.clone(), &mut texts);
            for (path, s) in texts {
                check_pattern(&mut out, &path, s, &|p| {
                    let role = p.strip_prefix("ref:").or_else(|| p.strip_prefix("key:"))?;
                    roles.contains(role).then(|| "x".to_string())
                });
            }
        }
        out
    }
}

impl EquipmentInstance {
    /// Every problem, each naming its field. With the type, overrides and the
    /// type's need of a base are checked too.
    pub fn problems(&self, t: Option<&EquipmentType>) -> Vec<String> {
        let mut out = Vec::new();
        if self.name.trim().is_empty() || self.name.len() > 128 {
            out.push("name must be 1-128 characters".into());
        }
        if self.name.contains('/') || self.name.chars().any(char::is_control) {
            out.push(format!(
                "name '{}' must not contain '/' or control characters",
                self.name
            ));
        }
        if !is_slug(&self.type_id) {
            out.push(format!("type '{}' is not a type id", self.type_id));
        }
        if self.device.trim().is_empty() {
            out.push("device must not be empty".into());
        }
        if let Some(b) = self.base
            && !(0..=65535).contains(&b)
        {
            out.push(format!("base {b} must be 0-65535"));
        }
        if let Some(p) = &self.prefix
            && !p.is_empty()
            && !p
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            out.push(format!(
                "prefix '{p}' may hold only letters, digits, '_', '-' and '.'"
            ));
        }
        if self.prefix().is_empty() {
            out.push(format!(
                "name '{}' gives an empty prefix; set prefix",
                self.name
            ));
        }
        if let Some(s) = &self.symbol
            && !symbols::is_known(s)
        {
            out.push(format!("symbol '{s}' is not a known symbol"));
        }
        if let Some(t) = t {
            for (role, o) in &self.overrides {
                let Some(tag) = t.tag(role) else {
                    out.push(format!(
                        "overrides.{role}: type '{}' has no role '{role}'",
                        t.id
                    ));
                    continue;
                };
                if o.skip && tag.required {
                    out.push(format!(
                        "overrides.{role}.skip: role '{role}' is required by type '{}'",
                        t.id
                    ));
                }
                if let Some(k) = &o.key {
                    if k.trim().is_empty() {
                        out.push(format!("overrides.{role}.key must not be empty"));
                    }
                    check_pattern(&mut out, &format!("overrides.{role}.key"), k, &|_| None);
                }
                if let Some(n) = &o.name {
                    check_pattern(&mut out, &format!("overrides.{role}.name"), n, &|_| None);
                }
            }
            if let Some(b) = self.base {
                for tag in &t.tags {
                    if let Some(o) = tag.offset
                        && b + o > 65535
                    {
                        out.push(format!(
                            "base {b} + offset {o} of role '{}' is beyond register 65535",
                            tag.role
                        ));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
