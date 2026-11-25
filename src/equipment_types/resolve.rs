//! Resolving an instance against what exists, and binding symbols to it.
//!
//! Pure functions over a [`World`] (tags by `device/key`, Modbus devices with
//! their register names, other device ids). The REST routes build the world
//! from the database with integer tag ids; the project import builds it from
//! the document and the database with refs (`device/key`) as tag handles. The
//! widget patch is the same either way, so a symbol bound in the Designer and
//! one bound by an import carry the same config.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;
use serde_json::{Map, Value};

use super::{BINDING_KEYS, EquipmentInstance, EquipmentType, TagAlarm, TagWrite, Vars, expand};

/// A Modbus device as resolution needs it.
#[derive(Debug, Clone, Default)]
pub struct ModbusDevice {
    pub registers: HashSet<String>,
    /// Follows a device template: its registers cannot be added to.
    pub linked: bool,
}

/// The tags and devices an instance is resolved against.
#[derive(Debug, Clone, Default)]
pub struct World {
    /// `(device, key)` → handle of the tag (a database id or a ref).
    pub tags: HashMap<(String, String), Value>,
    pub modbus: HashMap<String, ModbusDevice>,
    /// Ids of all devices of every protocol (Modbus included).
    pub devices: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The tag exists and is used.
    Bound,
    /// The tag (and, with a base, its register) is made by instantiating.
    Created,
    /// The role uses the tag of its `fallback` role.
    Fallback,
    /// No tag: no tag of that key, and nothing to make one from.
    Missing,
    /// The instance leaves the role out (`overrides.<role>.skip`).
    Skipped,
}

/// A register instantiation adds to a Modbus device.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewRegister {
    pub name: String,
    pub address: i64,
    pub register_type: String,
    pub register_area: String,
    pub scale: f64,
    /// A register of that name is on the device already (its address is
    /// checked against `address` by the expansion).
    #[serde(skip)]
    pub exists: bool,
}

/// 16-bit registers a value of `register_type` takes in `area` (coils and
/// discrete inputs are one bit each).
pub fn register_width(register_type: &str, area: &str) -> i64 {
    if matches!(area, "coil" | "discrete") {
        return 1;
    }
    match register_type {
        "int16" | "uint16" => 1,
        "float64" => 4,
        _ => 2,
    }
}

/// `register name → base + offset` of the roles of `inst` (of type `t`) that
/// have an offset, when the instance has a base. What the instance's registers
/// are expected to be at, so the expansion can tell its own register (moved
/// with the base) from someone else's.
pub fn expected_registers(t: &EquipmentType, inst: &EquipmentInstance) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    let Some(base) = inst.base else {
        return out;
    };
    let prefix = inst.prefix();
    for tag in &t.tags {
        let Some(offset) = tag.offset else { continue };
        let over = inst.overrides.get(&tag.role).cloned().unwrap_or_default();
        if over.skip {
            continue;
        }
        let label = if tag.label.is_empty() {
            tag.role.as_str()
        } else {
            tag.label.as_str()
        };
        let vars = Vars {
            prefix: &prefix,
            name: &inst.name,
            device: &inst.device,
            role: &tag.role,
            label,
        };
        let pattern = over.key.unwrap_or_else(|| tag.key.clone());
        if let Ok(key) = expand(&pattern, &vars, &|_| None) {
            out.insert(key, base + offset);
        }
    }
    out
}

/// What the type and the instance's override set on a tag.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagAttrs {
    pub name: Option<String>,
    pub unit: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub decimals: Option<i64>,
    pub alarm: TagAlarm,
    pub write: TagWrite,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleRes {
    pub role: String,
    pub kind: String,
    pub label: String,
    /// Tag key (for a fallback, the fallback role's).
    pub key: String,
    #[serde(rename = "ref")]
    pub reference: String,
    /// The tag's handle: its id from the database (`null` until it is made),
    /// or its ref in a project.
    pub tag_id: Option<Value>,
    pub status: Status,
    #[serde(skip)]
    pub register: Option<NewRegister>,
    #[serde(skip)]
    pub attrs: TagAttrs,
    /// Default name of a tag made for this role.
    #[serde(skip)]
    pub default_name: String,
}

impl RoleRes {
    /// The role has a tag (its own or its fallback's).
    pub fn resolved(&self) -> bool {
        matches!(
            self.status,
            Status::Bound | Status::Created | Status::Fallback
        )
    }
    /// The role's own tag (not a fallback), which the type's attributes apply to.
    pub fn owns_tag(&self) -> bool {
        matches!(self.status, Status::Bound | Status::Created)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub prefix: String,
    pub symbol: String,
    pub faceplate: Option<String>,
    pub roles: Vec<RoleRes>,
    /// Errors: the instance cannot be made as it is.
    #[serde(skip)]
    pub problems: Vec<String>,
    /// Flows the instance gets, with their ids, already expanded.
    #[serde(skip)]
    pub flows: Vec<Value>,
    /// Notes (flows left out because a role they name has no tag).
    #[serde(skip)]
    pub notes: Vec<String>,
}

impl Resolution {
    pub fn role(&self, role: &str) -> Option<&RoleRes> {
        self.roles.iter().find(|r| r.role == role)
    }
}

/// Resolve `inst` (of type `t`) against `world`.
pub fn resolve(t: &EquipmentType, inst: &EquipmentInstance, world: &World) -> Resolution {
    let prefix = inst.prefix();
    let mut problems = Vec::new();
    let device = inst.device.as_str();
    let modbus = world.modbus.get(device);
    if !world.devices.contains(device) && modbus.is_none() {
        problems.push(format!("device '{device}' does not exist"));
    }
    let uses_offsets = t.tags.iter().any(|t| t.offset.is_some());
    if inst.base.is_some() && uses_offsets && modbus.is_none() && world.devices.contains(device) {
        problems.push(format!(
            "base needs a Modbus device; '{device}' is not one (leave base out to bind existing tags by key)"
        ));
    }

    let mut roles: Vec<RoleRes> = Vec::with_capacity(t.tags.len());
    for tag in &t.tags {
        let label = if tag.label.is_empty() {
            tag.role.clone()
        } else {
            tag.label.clone()
        };
        let vars = Vars {
            prefix: &prefix,
            name: &inst.name,
            device,
            role: &tag.role,
            label: &label,
        };
        let over = inst.overrides.get(&tag.role).cloned().unwrap_or_default();
        let key_pattern = over.key.clone().unwrap_or_else(|| tag.key.clone());
        let key = expand(&key_pattern, &vars, &|_| None).unwrap_or_else(|e| {
            problems.push(format!("role '{}': {e}", tag.role));
            String::new()
        });
        let name_pattern = over.name.clone().or_else(|| tag.name.clone());
        let name = name_pattern.and_then(|p| match expand(&p, &vars, &|_| None) {
            Ok(n) => Some(n),
            Err(e) => {
                problems.push(format!("role '{}': {e}", tag.role));
                None
            }
        });
        let attrs = TagAttrs {
            name,
            unit: over.unit.clone().or_else(|| tag.unit.clone()),
            min: over.min.or(tag.min),
            max: over.max.or(tag.max),
            decimals: over.decimals.or(tag.decimals),
            alarm: tag
                .alarm
                .clone()
                .unwrap_or_default()
                .merged(&over.alarm.clone().unwrap_or_default()),
            write: tag
                .write
                .clone()
                .unwrap_or_default()
                .merged(&over.write.clone().unwrap_or_default()),
        };
        let mut r = RoleRes {
            role: tag.role.clone(),
            kind: tag.kind.clone(),
            label: label.clone(),
            reference: format!("{device}/{key}"),
            key: key.clone(),
            tag_id: None,
            status: Status::Missing,
            register: None,
            attrs,
            default_name: format!("{} {}", inst.name, label),
        };
        if over.skip {
            r.status = Status::Skipped;
            roles.push(r);
            continue;
        }
        if key.is_empty() {
            roles.push(r);
            continue;
        }
        if let (Some(base), Some(offset), Some(m)) = (inst.base, tag.offset, modbus) {
            let exists = m.registers.contains(&key);
            if !exists && m.linked {
                problems.push(format!(
                    "role '{}': device '{device}' follows a device template, so register '{key}' cannot be added (unlink it, or leave base out)",
                    tag.role
                ));
            } else {
                r.register = Some(NewRegister {
                    name: key.clone(),
                    address: base + offset,
                    register_type: tag.register_type.clone(),
                    register_area: tag.register_area.clone(),
                    scale: tag.scale,
                    exists,
                });
            }
        }
        if let Some(id) = world.tags.get(&(device.to_string(), key.clone())) {
            r.status = Status::Bound;
            r.tag_id = Some(id.clone());
        } else if r.register.is_some() || modbus.is_some_and(|m| m.registers.contains(&key)) {
            r.status = Status::Created;
        }
        roles.push(r);
    }

    // Fallbacks, once every role's own tag is known.
    for i in 0..roles.len() {
        if roles[i].status != Status::Missing {
            continue;
        }
        let Some(fb) = t.tags[i].fallback.as_deref() else {
            continue;
        };
        if let Some(target) = roles.iter().find(|r| r.role == fb && r.owns_tag()).cloned() {
            let r = &mut roles[i];
            r.status = Status::Fallback;
            r.key = target.key;
            r.reference = target.reference;
            r.tag_id = target.tag_id;
        }
    }
    let mut notes = Vec::new();
    for (r, tag) in roles.iter().zip(&t.tags) {
        // A command without a tag of its own writes to the feedback it falls
        // back to: allowed (that is what the WTP showcase does), but worth
        // knowing.
        if r.status == Status::Fallback
            && r.kind == "command"
            && r.attrs.write.writable == Some(true)
            && let Some(target) = tag.fallback.as_deref().and_then(|f| t.tag(f))
            && target.kind == "state"
        {
            notes.push(format!(
                "role '{}' (command) has no tag of its own and falls back to '{}' ({}), the {} feedback: commands are written to the feedback tag. Give it a tag of its own (overrides.{}.key) or skip it",
                r.role, target.role, r.reference, target.role, r.role
            ));
        }
        if r.status == Status::Missing && tag.required {
            problems.push(format!(
                "required role '{}' has no tag: no tag '{}' on device '{device}'{}",
                r.role,
                r.key,
                if tag.offset.is_some() && inst.base.is_none() {
                    " (give the instance a base to create it)"
                } else {
                    ""
                }
            ));
        }
    }

    let mut flows = Vec::new();
    for f in &t.flows {
        let vars = Vars {
            prefix: &prefix,
            name: &inst.name,
            device,
            role: "",
            label: "",
        };
        let mut missing = None;
        let lookup = |p: &str| -> Option<String> {
            let (kind, role) = p.split_once(':')?;
            let r = roles.iter().find(|r| r.role == role)?;
            if !r.resolved() {
                return None;
            }
            match kind {
                "ref" => Some(r.reference.clone()),
                "key" => Some(r.key.clone()),
                _ => None,
            }
        };
        let expanded = map_strings(f, &mut |s| match expand(s, &vars, &lookup) {
            Ok(v) => v,
            Err(e) => {
                missing.get_or_insert(e);
                s.to_string()
            }
        });
        let id = f.get("id").and_then(Value::as_str).unwrap_or_default();
        if let Some(e) = missing {
            notes.push(format!(
                "flow '{id}' of type '{}' is left out: {e} (a role it names has no tag)",
                t.id
            ));
            continue;
        }
        let mut obj = expanded.as_object().cloned().unwrap_or_default();
        obj.insert("id".into(), Value::String(flow_id(&prefix, id)));
        flows.push(Value::Object(obj));
    }

    Resolution {
        prefix,
        symbol: inst.symbol.clone().unwrap_or_else(|| t.symbol.clone()),
        faceplate: t.faceplate.clone(),
        roles,
        problems,
        flows,
        notes,
    }
}

/// Id of an instance's copy of a type's flow.
pub fn flow_id(prefix: &str, id: &str) -> String {
    let p: String = prefix
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{p}-{id}")
}

fn map_strings(v: &Value, f: &mut dyn FnMut(&str) -> String) -> Value {
    match v {
        Value::String(s) => Value::String(f(s)),
        Value::Array(a) => Value::Array(a.iter().map(|x| map_strings(x, f)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), map_strings(x, f)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// How a SCADA symbol is bound to an instance: the primary tag, the config
/// keys set, the keys set only as defaults and the config keys removed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WidgetPatch {
    pub equipment_type: &'static str,
    pub sensor_id: Option<Value>,
    pub config: Map<String, Value>,
    /// The type's `symbol_config`: set when the widget has no value of its
    /// own for the key (a widget's own value is kept).
    pub defaults: Map<String, Value>,
    pub remove: Vec<String>,
    /// The `symbol_config` the instance had before (a type being changed):
    /// a widget value equal to the old default follows the new one, and a
    /// default the type drops is removed when the widget still has it.
    #[serde(skip)]
    pub prev_defaults: Option<Map<String, Value>>,
}

/// The binding of a symbol to `inst`. `config.label` and `defaults` are only
/// defaults: set when the symbol has no value (see [`apply_patch`]).
pub fn widget_patch(t: &EquipmentType, inst: &EquipmentInstance, res: &Resolution) -> WidgetPatch {
    let handle = |role: Option<&str>| -> Option<Value> {
        let r = res.role(role?)?;
        if r.resolved() { r.tag_id.clone() } else { None }
    };
    let mut config = Map::new();
    let mut remove = Vec::new();
    config.insert("equipment".into(), Value::String(inst.name.clone()));
    config.insert("symbol".into(), Value::String(res.symbol.clone()));
    match &t.faceplate {
        Some(fp) => {
            config.insert("faceplate".into(), Value::String(fp.clone()));
        }
        None => remove.push("faceplate".to_string()),
    }
    let mut map: BTreeMap<String, Value> = BTreeMap::new();
    for r in &res.roles {
        if r.resolved()
            && let Some(id) = &r.tag_id
        {
            map.insert(r.role.clone(), id.clone());
        }
    }
    config.insert(
        "faceplateMap".into(),
        Value::Object(map.into_iter().collect()),
    );
    for (binding, key) in BINDING_KEYS {
        match handle(t.bindings.get(binding)) {
            Some(id) => {
                config.insert((*key).to_string(), id);
            }
            None => remove.push((*key).to_string()),
        }
    }
    config.insert("label".into(), Value::String(inst.name.clone()));
    WidgetPatch {
        equipment_type: "symbol",
        sensor_id: handle(t.bindings.primary.as_deref()),
        config,
        defaults: t.symbol_config.clone(),
        remove,
        prev_defaults: None,
    }
}

/// Tag handles a bound widget's config holds for its instance (the role map
/// and the indicator tags).
fn instance_handles(config: &Map<String, Value>) -> Vec<Value> {
    let mut out: Vec<Value> = config
        .get("faceplateMap")
        .and_then(Value::as_object)
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default();
    for (_, key) in BINDING_KEYS {
        if let Some(v) = config.get(*key) {
            out.push(v.clone());
        }
    }
    out
}

fn is_unset(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        _ => false,
    }
}

/// Apply a patch to a widget row. Returns whether anything changed.
///
/// `sensors[0]` becomes the primary tag; the widget's other tags stay, except
/// those that were (or are now) the instance's own. The type's defaults and
/// the label are set only where the widget has no value of its own.
pub fn apply_patch(
    p: &WidgetPatch,
    equipment_type: &mut String,
    sensors: &mut Vec<Value>,
    config: &mut Map<String, Value>,
) -> bool {
    let before = (equipment_type.clone(), sensors.clone(), config.clone());
    *equipment_type = p.equipment_type.to_string();
    let mut owned = instance_handles(config);
    owned.extend(instance_handles(&p.config));
    let extras: Vec<Value> = sensors
        .iter()
        .filter(|v| p.sensor_id.as_ref() != Some(*v) && !owned.contains(v))
        .cloned()
        .collect();
    let mut next: Vec<Value> = p.sensor_id.iter().cloned().collect();
    for v in extras {
        if !next.contains(&v) {
            next.push(v);
        }
    }
    *sensors = next;
    for (k, v) in &p.config {
        if k == "label" {
            if is_unset(config.get("label")) {
                config.insert(k.clone(), v.clone());
            }
        } else {
            config.insert(k.clone(), v.clone());
        }
    }
    let prev = p.prev_defaults.as_ref();
    for (k, v) in &p.defaults {
        let follow = match config.get(k) {
            None | Some(Value::Null) => true,
            Some(cur) => prev.and_then(|m| m.get(k)) == Some(cur),
        };
        if follow {
            config.insert(k.clone(), v.clone());
        }
    }
    if let Some(prev) = prev {
        for (k, v) in prev {
            if !p.defaults.contains_key(k) && config.get(k) == Some(v) {
                config.remove(k);
            }
        }
    }
    for k in &p.remove {
        config.remove(k);
    }
    before != (equipment_type.clone(), sensors.clone(), config.clone())
}
