//! Equipment types and instances in a project, and their expansion at plan
//! time (docs/equipment-types.md, docs/project-format.md).
//!
//! `equipment_types` and `equipment_instances` are ordinary sections (stored
//! in their own tables, JSON put in canonical form by the hooks below). Before
//! an import is validated and diffed, [`expand`] turns every instance into the
//! entities it implies, so the plan lists them like any other change:
//!
//! * the tags of its roles: a tag the file lists is used as written; a stored
//!   tag the file leaves out (merge) gets the type's attributes; a missing one
//!   is created;
//! * the registers a base address and the type's offsets call for, added to
//!   the Modbus device (taken from the database when the file leaves it out);
//! * the type's flows (`<prefix>-<id>`), unless the file lists them;
//! * every SCADA widget whose `config.equipment` names the instance, bound to
//!   its tags (in the file's screens and, on merge, the stored ones).
//!
//! On merge, an instance the file leaves out is expanded too when the file
//! changes its type: that is how a changed type reaches all its instances.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_yaml_neo::{Mapping, Value};

use super::export::{Ctx, Snapshot, entity_key};
use super::model::{Issue, Mode, ProjectDoc, Severity};
use super::norm::{self, get_str, int, text};
use super::spec;
use crate::equipment_types::resolve::{self, ModbusDevice, RoleRes, Status, World};
use crate::equipment_types::{EquipmentInstance, EquipmentType};

pub const TYPES: &str = "equipment_types";
pub const INSTANCES: &str = "equipment_instances";

// ── Entities ──

fn to_json(e: &Mapping) -> Result<serde_json::Value> {
    norm::yaml_to_json(&Value::Mapping(e.clone()))
}

fn from_json(sec: &str, j: &serde_json::Value) -> Result<Mapping> {
    let fields = spec::section(sec).context("section")?.fields;
    let Value::Mapping(raw) = norm::json_to_yaml(j)? else {
        anyhow::bail!("not a mapping");
    };
    let mut errs = Vec::new();
    let out = norm::normalize(fields, &raw, "", &mut errs);
    if errs.is_empty() {
        Ok(out)
    } else {
        anyhow::bail!("{}", errs.join("; "))
    }
}

pub fn parse_type(e: &Mapping) -> Result<EquipmentType> {
    serde_json::from_value(to_json(e)?).context("not an equipment type")
}

pub fn parse_instance(e: &Mapping) -> Result<EquipmentInstance> {
    serde_json::from_value(to_json(e)?).context("not an equipment instance")
}

/// A type as a project entity, in canonical form.
pub fn type_entity(t: &EquipmentType) -> Result<Mapping> {
    from_json(TYPES, &serde_json::to_value(t)?)
}

/// An instance as a project entity, in canonical form.
pub fn instance_entity(i: &EquipmentInstance) -> Result<Mapping> {
    from_json(INSTANCES, &serde_json::to_value(i)?)
}

/// Put a type or instance entity in the form export writes (defaults of the
/// nested JSON filled or dropped as serde writes them), so a hand-written
/// entity and the stored one compare equal. Invalid entities stay as they
/// are; validation reports them.
pub fn canonicalize(section: &str, e: &mut Mapping) {
    let canonical = match section {
        TYPES => parse_type(e).and_then(|t| type_entity(&t)),
        INSTANCES => parse_instance(e).and_then(|i| instance_entity(&i)),
        _ => return,
    };
    if let Ok(c) = canonical {
        *e = c;
    }
}

pub fn export_type(raw: &mut Mapping, _ctx: &mut Ctx) {
    canonicalize_raw(TYPES, raw);
}

pub fn export_instance(raw: &mut Mapping, _ctx: &mut Ctx) {
    canonicalize_raw(INSTANCES, raw);
}

fn canonicalize_raw(section: &str, raw: &mut Mapping) {
    let mut errs = Vec::new();
    let fields = spec::section(section).expect("section").fields;
    let mut e = norm::normalize(fields, raw, "", &mut errs);
    if errs.is_empty() {
        canonicalize(section, &mut e);
        *raw = e;
    }
}

pub fn import_type(entity: &mut Mapping, _ctx: &Ctx) -> Result<()> {
    let t = parse_type(entity)?;
    *entity = type_entity(&t)?;
    Ok(())
}

pub fn import_instance(entity: &mut Mapping, _ctx: &Ctx) -> Result<()> {
    let i = parse_instance(entity)?;
    *entity = instance_entity(&i)?;
    Ok(())
}

// ── Expansion ──

/// What one tag of an instance becomes.
#[derive(Debug, Clone, Serialize)]
pub struct TagChange {
    pub key: String,
    #[serde(rename = "ref")]
    pub reference: String,
    /// `create`, `update` (the type's attributes change it), `unchanged`, or
    /// `bind` (used as it is: the file lists it, or a fallback role).
    pub action: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    /// Every attribute the change sets, with its value before and after
    /// (`write.max`, `alarm.crit`, `unit`, ...; `before` is null for a new tag).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diff: Vec<FieldDiff>,
}

/// One attribute of a tag, before and after.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FieldDiff {
    pub field: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
}

impl TagChange {
    /// The write-policy part of the diff (`write.*`).
    pub fn policy_diff(&self) -> impl Iterator<Item = &FieldDiff> {
        self.diff.iter().filter(|d| d.field.starts_with("write."))
    }
}

/// Top-level attributes, and those of `alarm` and `write` one level down,
/// that differ between two tag entities.
fn tag_diff(before: Option<&Mapping>, after: &Mapping) -> Vec<FieldDiff> {
    fn flat(e: Option<&Mapping>) -> BTreeMap<String, serde_json::Value> {
        let mut out = BTreeMap::new();
        let Some(e) = e else { return out };
        for (k, v) in e {
            let k = norm::display(k);
            match v {
                Value::Mapping(m) if k == "alarm" || k == "write" => {
                    for (sk, sv) in m {
                        out.insert(
                            format!("{k}.{}", norm::display(sk)),
                            norm::yaml_to_json(sv).unwrap_or(serde_json::Value::Null),
                        );
                    }
                }
                _ => {
                    out.insert(k, norm::yaml_to_json(v).unwrap_or(serde_json::Value::Null));
                }
            }
        }
        out
    }
    let (a, b) = (flat(before), flat(Some(after)));
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .filter_map(|k| {
            let (x, y) = (a.get(k), b.get(k));
            (x != y).then(|| FieldDiff {
                field: k.clone(),
                before: x.cloned().unwrap_or(serde_json::Value::Null),
                after: y.cloned().unwrap_or(serde_json::Value::Null),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct WidgetRef {
    pub screen_id: String,
    pub equipment_id: String,
}

/// What expanding one instance generates.
#[derive(Debug, Clone, Serialize)]
pub struct Expanded {
    pub instance: String,
    #[serde(rename = "type")]
    pub type_id: String,
    pub tags: Vec<TagChange>,
    /// `device/register@address` of every register added.
    pub registers_added: Vec<String>,
    /// `device/register@old->new` of every register of the instance moved to
    /// a new address (its base or the type's offset changed).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub registers_moved: Vec<String>,
    /// Flow ids created or updated.
    pub flows: Vec<String>,
    /// Of `flows`, those that exist and are overwritten.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flows_updated: Vec<String>,
    /// Flows the instance made before that it no longer has (its prefix or
    /// its type's flows changed); deleted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flows_removed: Vec<String>,
    /// Every SCADA widget bound to the instance.
    pub widgets: Vec<WidgetRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip)]
    pub roles: Vec<RoleRes>,
}

impl Expanded {
    pub fn render(&self) -> String {
        let mut parts = Vec::new();
        for t in &self.tags {
            let sym = match t.action {
                "create" => "+",
                "update" => "~",
                _ => "=",
            };
            if t.fields.is_empty() {
                parts.push(format!("{sym}tag {}", t.reference));
            } else {
                parts.push(format!(
                    "{sym}tag {} ({})",
                    t.reference,
                    t.fields.join(", ")
                ));
            }
        }
        for r in &self.registers_added {
            parts.push(format!("+register {r}"));
        }
        for r in &self.registers_moved {
            parts.push(format!("~register {r}"));
        }
        for f in &self.flows {
            parts.push(format!("flow {f}"));
        }
        for f in &self.flows_removed {
            parts.push(format!("-flow {f}"));
        }
        if !self.widgets.is_empty() {
            parts.push(format!("{} widget(s)", self.widgets.len()));
        }
        format!(
            "= expand {} ({}): {}",
            self.instance,
            self.type_id,
            parts.join(", ")
        )
    }
}

const DEVICE_SECTIONS: &[(&str, &str)] = &[
    ("modbus_devices", "id"),
    ("opcua_devices", "name"),
    ("s7_devices", "name"),
    ("enip_devices", "name"),
    ("dnp3_devices", "name"),
    ("bacnet_devices", "name"),
    ("voltrus_plc_devices", "name"),
];

fn modbus_of(e: &Mapping) -> ModbusDevice {
    let registers = match e.get("registers") {
        Some(Value::Sequence(s)) => s
            .iter()
            .filter_map(Value::as_mapping)
            .map(|r| get_str(r, "name"))
            .collect(),
        _ => HashSet::new(),
    };
    ModbusDevice {
        registers,
        linked: e.contains_key("template_id"),
    }
}

fn tag_ident(e: &Mapping) -> (String, String) {
    (get_str(e, "device"), get_str(e, "key"))
}

/// The world an import sees: the file, and on merge what is stored.
pub fn world_of(doc: &ProjectDoc, snap: &Snapshot, mode: Mode) -> World {
    let mut w = World::default();
    let stored = |section: &str| -> Vec<Mapping> {
        if mode == Mode::Merge {
            snap.get(section)
                .map(|rows| rows.iter().map(|r| r.entity.clone()).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    };
    for e in doc.entities("tags").iter().cloned().chain(stored("tags")) {
        let (device, key) = tag_ident(&e);
        w.devices.insert(device.clone());
        let reference = format!("{device}/{key}");
        w.tags
            .entry((device, key))
            .or_insert(serde_json::Value::String(reference));
    }
    for (section, field) in DEVICE_SECTIONS {
        let in_doc = doc.entities(section).to_vec();
        let doc_ids: HashSet<String> = in_doc.iter().map(|e| get_str(e, field)).collect();
        for e in in_doc.into_iter().chain(
            stored(section)
                .into_iter()
                .filter(|e| !doc_ids.contains(&get_str(e, field))),
        ) {
            let id = get_str(&e, field);
            w.devices.insert(id.clone());
            if *section == "modbus_devices" {
                w.modbus.insert(id, modbus_of(&e));
            }
        }
    }
    w
}

/// Tag fields a type sets, applied to a tag entity. Returns the entity in
/// canonical form.
fn apply_attrs(e: &Mapping, r: &RoleRes) -> Mapping {
    let mut e = e.clone();
    let a = &r.attrs;
    if let Some(n) = &a.name {
        e.insert(text("name"), text(n.clone()));
    }
    if let Some(u) = &a.unit {
        e.insert(text("unit"), text(u.clone()));
    }
    if let Some(v) = a.min {
        e.insert(text("min"), norm::real(v));
    }
    if let Some(v) = a.max {
        e.insert(text("max"), norm::real(v));
    }
    if let Some(v) = a.decimals {
        e.insert(text("decimals"), int(v));
    }
    let sub = |e: &Mapping, name: &str| -> Mapping {
        e.get(name)
            .and_then(Value::as_mapping)
            .cloned()
            .unwrap_or_default()
    };
    if !a.alarm.is_empty() {
        let mut m = sub(&e, "alarm");
        for (k, v) in [
            ("warn", a.alarm.warn),
            ("crit", a.alarm.crit),
            ("low_warn", a.alarm.low_warn),
            ("low_crit", a.alarm.low_crit),
            ("deadband", a.alarm.deadband),
        ] {
            if let Some(v) = v {
                m.insert(text(k), norm::real(v));
            }
        }
        for (k, v) in [
            ("on_delay_secs", a.alarm.on_delay_secs),
            ("off_delay_secs", a.alarm.off_delay_secs),
        ] {
            if let Some(v) = v {
                m.insert(text(k), int(v));
            }
        }
        e.insert(text("alarm"), Value::Mapping(m));
    }
    if !a.write.is_empty() {
        let mut m = sub(&e, "write");
        if let Some(v) = a.write.writable {
            m.insert(text("writable"), Value::Bool(v));
        }
        if let Some(v) = a.write.min {
            m.insert(text("min"), norm::real(v));
        }
        if let Some(v) = a.write.max {
            m.insert(text("max"), norm::real(v));
        }
        if let Some(v) = a.write.max_rate {
            m.insert(text("max_rate"), int(v));
        }
        if let Some(v) = a.write.requires_reason {
            m.insert(text("requires_reason"), Value::Bool(v));
        }
        // A writable tag without write limits is limited to its own range:
        // a type never opens a tag to any value.
        let range = (
            e.get("min").and_then(Value::as_f64),
            e.get("max").and_then(Value::as_f64),
        );
        if m.get("writable").and_then(Value::as_bool) == Some(true)
            && let (Some(lo), Some(hi)) = range
            && lo < hi
        {
            for (limit, v) in [("min", lo), ("max", hi)] {
                if !m.contains_key(limit) {
                    m.insert(text(limit), norm::real(v));
                }
            }
        }
        e.insert(text("write"), Value::Mapping(m));
    }
    let mut errs = Vec::new();
    norm::normalize(spec::TAG_FIELDS, &e, "", &mut errs)
}

/// A new tag for a role, before the type's attributes.
fn new_tag(device: &str, r: &RoleRes, modbus: bool) -> Mapping {
    let digital = matches!(r.kind.as_str(), "state" | "fault" | "command");
    let mut e = Mapping::new();
    e.insert(text("key"), text(r.key.clone()));
    e.insert(text("device"), text(device));
    e.insert(text("name"), text(r.default_name.clone()));
    e.insert(text("min"), norm::real(0.0));
    e.insert(text("max"), norm::real(if digital { 1.0 } else { 100.0 }));
    if digital {
        e.insert(text("decimals"), int(0));
    }
    if modbus {
        e.insert(text("source_type"), text("modbus"));
    }
    e
}

fn register_fields() -> &'static [spec::Field] {
    match spec::section("modbus_devices")
        .and_then(|s| s.fields.iter().find(|f| f.name == "registers"))
        .map(|f| f.ty)
    {
        Some(spec::Ty::List(sub)) => sub,
        _ => &[],
    }
}

fn register_entity(reg: &resolve::NewRegister, sort_order: i64) -> Value {
    let fields = register_fields();
    let mut m = Mapping::new();
    m.insert(text("name"), text(reg.name.clone()));
    m.insert(text("address"), int(reg.address));
    m.insert(text("register_type"), text(reg.register_type.clone()));
    m.insert(text("register_area"), text(reg.register_area.clone()));
    m.insert(text("scale"), norm::real(reg.scale));
    m.insert(text("sort_order"), int(sort_order));
    m.insert(text("zero_based"), Value::Bool(true));
    Value::Mapping(norm::normalize(fields, &m, "", &mut Vec::new()))
}

/// Entities of one section: the file's, plus stored ones pulled in (merge)
/// to be changed. Only pulled entities that end up different are added to
/// the document.
struct Pool {
    section: &'static str,
    doc: Vec<Mapping>,
    /// key → (stored entity, working copy)
    pulled: BTreeMap<String, (Mapping, Mapping)>,
    stored: HashMap<String, Mapping>,
}

impl Pool {
    fn new(section: &'static str, doc: &ProjectDoc, snap: &Snapshot, mode: Mode) -> Self {
        let stored = if mode == Mode::Merge {
            snap.get(section)
                .map(|rows| {
                    rows.iter()
                        .map(|r| (r.key.clone(), r.entity.clone()))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        Pool {
            section,
            doc: doc.entities(section).to_vec(),
            pulled: BTreeMap::new(),
            stored,
        }
    }

    fn sec(&self) -> &'static spec::Section {
        spec::section(self.section).expect("section")
    }

    fn in_doc(&self, key: &str) -> Option<usize> {
        let sec = self.sec();
        self.doc.iter().position(|e| entity_key(sec, e) == key)
    }

    /// The entity of `key` to change: the file's, or a stored one pulled in.
    fn get_mut(&mut self, key: &str) -> Option<&mut Mapping> {
        if let Some(i) = self.in_doc(key) {
            return Some(&mut self.doc[i]);
        }
        if !self.pulled.contains_key(key) {
            let e = self.stored.get(key)?.clone();
            self.pulled.insert(key.to_string(), (e.clone(), e));
        }
        self.pulled.get_mut(key).map(|(_, w)| w)
    }

    fn finish(self, doc: &mut ProjectDoc) {
        let mut out = self.doc;
        for (_, (orig, working)) in self.pulled {
            if orig != working {
                out.push(working);
            }
        }
        doc.sections.insert(self.section.to_string(), out);
    }
}

fn error(issues: &mut Vec<Issue>, at: String, message: String) {
    issues.push(Issue {
        severity: Severity::Error,
        at,
        message,
    });
}

/// The 0-based wire address of a register entity.
fn wire_address(m: &Mapping) -> i64 {
    let address = m.get("address").and_then(Value::as_i64).unwrap_or(0);
    let zero_based = m
        .get("zero_based")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match u16::try_from(address) {
        Ok(a) => i64::from(crate::models::protocol_address(a, zero_based)),
        Err(_) => address,
    }
}

fn register_area_of(m: &Mapping) -> String {
    match get_str(m, "register_area") {
        a if a.is_empty() => "holding".into(),
        a => a,
    }
}

fn register_type_of(m: &Mapping) -> String {
    match get_str(m, "register_type") {
        t if t.is_empty() => "float32".into(),
        t => t,
    }
}

/// The first register of `regs` other than `name` that shares a 16-bit
/// register (or a bit) with `address..address+width` in `area`.
fn overlapping(regs: &[Value], name: &str, address: i64, width: i64, area: &str) -> Option<String> {
    regs.iter().filter_map(Value::as_mapping).find_map(|m| {
        let other = get_str(m, "name");
        if other == name || register_area_of(m) != area {
            return None;
        }
        let a = wire_address(m);
        let w = resolve::register_width(&register_type_of(m), area);
        (address < a + w && a < address + width).then(|| {
            format!(
                "'{other}' ({} {a}{})",
                register_type_of(m),
                if w > 1 {
                    format!("-{}", a + w - 1)
                } else {
                    String::new()
                }
            )
        })
    })
}

fn span(address: i64, width: i64) -> String {
    if width > 1 {
        format!("{address}-{}", address + width - 1)
    } else {
        address.to_string()
    }
}

/// Expand the document's equipment instances (see the module docs).
/// Returns the document to import, what each instance generated, and
/// problems (errors stop the import).
pub fn expand(
    doc: &ProjectDoc,
    snap: &Snapshot,
    mode: Mode,
) -> (ProjectDoc, Vec<Expanded>, Vec<Issue>) {
    let mut issues = Vec::new();
    let merge = mode == Mode::Merge;
    let stored_rows = |section: &str| -> Vec<Mapping> {
        if merge {
            snap.get(section)
                .map(|rows| rows.iter().map(|r| r.entity.clone()).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    };

    // Types: the file's over the stored ones.
    let stored_types: HashMap<String, EquipmentType> = stored_rows(TYPES)
        .iter()
        .filter_map(|e| parse_type(e).ok())
        .map(|t| (t.id.clone(), t))
        .collect();
    let mut types = stored_types.clone();
    let mut changed_types = HashSet::new();
    for e in doc.entities(TYPES) {
        if let Ok(t) = parse_type(e) {
            if stored_types.get(&t.id) != Some(&t) {
                changed_types.insert(t.id.clone());
            }
            types.insert(t.id.clone(), t);
        }
    }

    // Instances: the file's, plus stored instances of a type the file changes.
    let stored_instances: BTreeMap<String, EquipmentInstance> = stored_rows(INSTANCES)
        .iter()
        .filter_map(|e| parse_instance(e).ok())
        .map(|i| (i.name.clone(), i))
        .collect();
    let mut instances: BTreeMap<String, EquipmentInstance> = BTreeMap::new();
    for e in doc.entities(INSTANCES) {
        if let Ok(i) = parse_instance(e) {
            instances.insert(i.name.clone(), i);
        }
    }
    let from_doc: HashSet<String> = instances.keys().cloned().collect();
    for (name, i) in &stored_instances {
        if !instances.contains_key(name) && changed_types.contains(&i.type_id) {
            instances.insert(name.clone(), i.clone());
        }
    }
    // Every instance once the import is done.
    let mut after: BTreeMap<String, EquipmentInstance> = stored_instances.clone();
    for name in &from_doc {
        after.insert(name.clone(), instances[name].clone());
    }
    let all_instance_names: HashSet<String> = after.keys().cloned().collect();
    if instances.is_empty() {
        return (doc.clone(), Vec::new(), issues);
    }

    // Names and prefixes are unique without case (flow ids are lower case):
    // `P101A` next to `P-101A` would share every tag and flow.
    let mut blocked: HashSet<String> = HashSet::new();
    for name in &from_doc {
        let inst = &after[name];
        let mine = crate::equipment_types::prefix_identity(&inst.prefix());
        for (other, o) in &after {
            if other == name {
                continue;
            }
            let at = format!("{INSTANCES}[{name}]");
            if other.to_lowercase() == name.to_lowercase() {
                error(
                    &mut issues,
                    at.clone(),
                    format!(
                        "name '{name}' differs from equipment instance '{other}' only in case; instance names are compared without case"
                    ),
                );
                blocked.insert(name.clone());
            }
            if crate::equipment_types::prefix_identity(&o.prefix()) == mine {
                error(
                    &mut issues,
                    at,
                    format!(
                        "prefix '{}' is the prefix of equipment instance '{other}' ('{}') too; prefixes are compared without case, like the flow ids made from them. Give this instance its own prefix",
                        inst.prefix(),
                        o.prefix()
                    ),
                );
                blocked.insert(name.clone());
            }
        }
    }

    let world = world_of(doc, snap, mode);
    let mut tags = Pool::new("tags", doc, snap, mode);
    let mut devices = Pool::new("modbus_devices", doc, snap, mode);
    let mut flows = Pool::new("flows", doc, snap, mode);
    let mut screens = Pool::new("screens", doc, snap, mode);
    let doc_tags: HashSet<(String, String)> = doc.entities("tags").iter().map(tag_ident).collect();
    let doc_flows: HashSet<String> = doc
        .entities("flows")
        .iter()
        .map(|e| get_str(e, "id"))
        .collect();
    let stored_tag_key: HashMap<(String, String), String> = if merge {
        snap.get("tags")
            .map(|rows| {
                rows.iter()
                    .map(|r| (tag_ident(&r.entity), r.key.clone()))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let mut out: Vec<Expanded> = Vec::new();
    let mut patches: HashMap<String, resolve::WidgetPatch> = HashMap::new();
    let mut new_tags: Vec<Mapping> = Vec::new();

    // Tags the instances that are not expanded keep: an expanded instance
    // may not take one of them.
    let mut owner: HashMap<(String, String), String> = HashMap::new();
    for (name, o) in &after {
        if instances.contains_key(name) {
            continue;
        }
        let Some(t) = types.get(&o.type_id) else {
            continue;
        };
        for r in resolve::resolve(t, o, &world).roles {
            if r.owns_tag() {
                owner.insert((o.device.clone(), r.key.clone()), name.clone());
            }
        }
    }

    for (name, inst) in &instances {
        if blocked.contains(name) {
            continue;
        }
        let at = format!("{INSTANCES}[{name}]");
        let listed = from_doc.contains(name);
        // A stored instance pulled in by its type's change that cannot be
        // expanded any more (its device was deleted, ...) is skipped with a
        // warning instead of failing the change for every other instance.
        let problem = |issues: &mut Vec<Issue>, message: String| {
            if listed {
                error(issues, at.clone(), message);
            } else {
                issues.push(Issue {
                    severity: Severity::Warning,
                    at: at.clone(),
                    message: format!("{message}; this instance is skipped (not updated)"),
                });
            }
        };
        let Some(t) = types.get(&inst.type_id) else {
            problem(
                &mut issues,
                format!("unknown equipment type '{}'", inst.type_id),
            );
            continue;
        };
        let problems = inst.problems(Some(t));
        let mut res = resolve::resolve(t, inst, &world);
        if !problems.is_empty() || !res.problems.is_empty() {
            let all: Vec<String> = problems.into_iter().chain(res.problems.drain(..)).collect();
            if listed {
                for p in all {
                    error(&mut issues, at.clone(), p);
                }
            } else {
                problem(&mut issues, all.join("; "));
            }
            continue;
        }
        // What the instance was before this import: its registers and flows
        // are its own to move and replace.
        let before = stored_instances.get(name);
        let before_type = before.and_then(|b| stored_types.get(&b.type_id));
        let expected_before: HashMap<String, i64> = match (before, before_type) {
            (Some(b), Some(bt)) if b.device == inst.device => resolve::expected_registers(bt, b),
            _ => HashMap::new(),
        };
        let own_flows: HashSet<String> = match (before, before_type) {
            (Some(b), Some(bt)) => bt
                .flows
                .iter()
                .filter_map(|f| f.get("id").and_then(serde_json::Value::as_str))
                .map(|id| resolve::flow_id(&b.prefix(), id))
                .collect(),
            _ => HashSet::new(),
        };
        // Tags made by this import are named by ref like the others.
        for r in &mut res.roles {
            if r.resolved() && r.tag_id.is_none() {
                r.tag_id = Some(serde_json::Value::String(r.reference.clone()));
            }
        }
        let mut exp = Expanded {
            instance: name.clone(),
            type_id: t.id.clone(),
            tags: Vec::new(),
            registers_added: Vec::new(),
            registers_moved: Vec::new(),
            flows: Vec::new(),
            flows_updated: Vec::new(),
            flows_removed: Vec::new(),
            widgets: Vec::new(),
            notes: res.notes.clone(),
            roles: Vec::new(),
        };
        let device = inst.device.clone();
        for r in &res.roles {
            if !r.resolved() {
                continue;
            }
            let ident = (device.clone(), r.key.clone());
            if r.status == Status::Fallback {
                exp.tags.push(TagChange {
                    key: r.key.clone(),
                    reference: r.reference.clone(),
                    action: "bind",
                    fields: Vec::new(),
                    diff: Vec::new(),
                });
                continue;
            }
            if let Some(other) = owner.insert(ident.clone(), name.clone())
                && &other != name
            {
                error(
                    &mut issues,
                    at.clone(),
                    format!(
                        "role '{}' gives tag '{}', which equipment instance '{other}' has too",
                        r.role, r.reference
                    ),
                );
                continue;
            }
            if let Some(reg) = &r.register {
                place_register(
                    &mut devices,
                    &device,
                    reg,
                    &expected_before,
                    &mut exp,
                    &mut issues,
                    &at,
                );
            }
            let fields_of = |a: &Mapping, b: &Mapping| -> Vec<String> {
                let mut keys: BTreeSet<String> = BTreeSet::new();
                for k in a.keys().chain(b.keys()) {
                    let k = norm::display(k);
                    if a.get(k.as_str()) != b.get(k.as_str()) {
                        keys.insert(k);
                    }
                }
                keys.into_iter().collect()
            };
            if doc_tags.contains(&ident) {
                exp.tags.push(TagChange {
                    key: r.key.clone(),
                    reference: r.reference.clone(),
                    action: "bind",
                    fields: Vec::new(),
                    diff: Vec::new(),
                });
            } else if let Some(key) = stored_tag_key.get(&ident) {
                let Some(entity) = tags.get_mut(key) else {
                    continue;
                };
                let before = entity.clone();
                let after = apply_attrs(&before, r);
                let fields = fields_of(&before, &after);
                let diff = tag_diff(Some(&before), &after);
                *entity = after;
                exp.tags.push(TagChange {
                    key: r.key.clone(),
                    reference: r.reference.clone(),
                    action: if fields.is_empty() {
                        "unchanged"
                    } else {
                        "update"
                    },
                    fields,
                    diff,
                });
            } else {
                let modbus = world.modbus.contains_key(&device);
                let made = apply_attrs(&new_tag(&device, r, modbus), r);
                let diff = tag_diff(None, &made);
                new_tags.push(made);
                exp.tags.push(TagChange {
                    key: r.key.clone(),
                    reference: r.reference.clone(),
                    action: "create",
                    fields: Vec::new(),
                    diff,
                });
            }
        }
        let mut wanted_ids: HashSet<String> = HashSet::new();
        for f in &res.flows {
            let id = f
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            wanted_ids.insert(id.clone());
            if doc_flows.contains(&id) {
                continue;
            }
            // A flow of that id that this instance did not make (a user's
            // flow, or another instance's) is not overwritten.
            if flows.stored.contains_key(&id) && !own_flows.contains(&id) {
                error(
                    &mut issues,
                    at.clone(),
                    format!(
                        "flow '{id}' exists and was not made by this instance, so its copy of the type's flow cannot have that id; rename that flow or give the instance another prefix"
                    ),
                );
                continue;
            }
            let wanted = match norm::json_to_yaml(f) {
                Ok(Value::Mapping(m)) => {
                    let mut errs = Vec::new();
                    let e = norm::normalize(
                        spec::section("flows").expect("flows").fields,
                        &m,
                        "",
                        &mut errs,
                    );
                    if !errs.is_empty() {
                        error(
                            &mut issues,
                            at.clone(),
                            format!("flow '{id}': {}", errs.join("; ")),
                        );
                        continue;
                    }
                    e
                }
                _ => continue,
            };
            match flows.get_mut(&id) {
                Some(existing) => {
                    if *existing != wanted {
                        *existing = wanted;
                        exp.flows_updated.push(id.clone());
                        exp.flows.push(id);
                    }
                }
                None => {
                    flows.doc.push(wanted);
                    exp.flows.push(id);
                }
            }
        }
        let mut removed: Vec<String> = own_flows
            .iter()
            .filter(|id| {
                !wanted_ids.contains(*id)
                    && !doc_flows.contains(*id)
                    && flows.stored.contains_key(*id)
            })
            .cloned()
            .collect();
        removed.sort();
        exp.flows_removed = removed;
        let mut patch = resolve::widget_patch(t, inst, &res);
        patch.prev_defaults = before_type.map(|bt| bt.symbol_config.clone());
        patches.insert(name.clone(), patch);
        exp.roles = res.roles;
        out.push(exp);
    }
    for t in new_tags {
        tags.doc.push(t);
    }

    // Widgets bound to the expanded instances, in the file's screens and the
    // stored ones.
    let mut screen_ids: Vec<String> = screens
        .doc
        .iter()
        .map(|e| get_str(e, "id"))
        .chain(screens.stored.keys().cloned())
        .collect();
    screen_ids.sort();
    screen_ids.dedup();
    let mut bound: HashMap<String, Vec<WidgetRef>> = HashMap::new();
    for sid in screen_ids {
        let names = widget_instances(&screens, &sid);
        for n in &names {
            if !all_instance_names.contains(n) && !instances.contains_key(n) {
                issues.push(Issue {
                    severity: Severity::Warning,
                    at: format!("screens[{sid}]"),
                    message: format!(
                        "a SCADA widget names equipment instance '{n}', which does not exist; it keeps its bindings"
                    ),
                });
            }
        }
        if !names.iter().any(|n| patches.contains_key(n)) {
            continue;
        }
        let Some(screen) = screens.get_mut(&sid) else {
            continue;
        };
        materialize_screen(screen, &sid, &patches, &mut bound);
    }
    for e in &mut out {
        if let Some(mut w) = bound.remove(&e.instance) {
            w.sort();
            e.widgets = w;
        }
    }

    let mut doc = doc.clone();
    tags.finish(&mut doc);
    devices.finish(&mut doc);
    flows.finish(&mut doc);
    screens.finish(&mut doc);
    (doc, out, issues)
}

/// Add the register a role needs to its Modbus device, or check the one
/// that is there: a register is never put over another one (by address,
/// with the width of its type), and an existing register at another address
/// than base + offset is moved only when it is the instance's own (where its
/// previous base and offset put it); otherwise that is an error.
fn place_register(
    devices: &mut Pool,
    device: &str,
    reg: &resolve::NewRegister,
    expected_before: &HashMap<String, i64>,
    exp: &mut Expanded,
    issues: &mut Vec<Issue>,
    at: &str,
) {
    let Some(dev) = devices.get_mut(device) else {
        if !reg.exists {
            error(
                issues,
                at.to_string(),
                format!(
                    "device '{device}' is not in the project, so register '{}' cannot be added",
                    reg.name
                ),
            );
        }
        return;
    };
    let linked = dev.contains_key("template_id");
    let mut regs = match dev.get("registers") {
        Some(Value::Sequence(s)) => s.clone(),
        _ => Vec::new(),
    };
    let width = resolve::register_width(&reg.register_type, &reg.register_area);
    let pos = regs.iter().position(|v| {
        v.as_mapping()
            .is_some_and(|m| get_str(m, "name") == reg.name)
    });
    match pos {
        Some(i) => {
            let Some(m) = regs[i].as_mapping().cloned() else {
                return;
            };
            let current = wire_address(&m);
            if current == reg.address {
                return;
            }
            let ours = expected_before.get(&reg.name) == Some(&current);
            if linked || !ours {
                error(
                    issues,
                    at.to_string(),
                    format!(
                        "register '{}' on device '{device}' is at address {current}, but the instance's base and the type's offset put it at {}; {}",
                        reg.name,
                        reg.address,
                        if linked {
                            "the device follows a device template, so it cannot be moved (change base to match, or leave base out)"
                        } else {
                            "set base so they agree, or leave base out to bind the register by its name"
                        }
                    ),
                );
                return;
            }
            let area = register_area_of(&m);
            let w = resolve::register_width(&register_type_of(&m), &area);
            if let Some(other) = overlapping(&regs, &reg.name, reg.address, w, &area) {
                error(
                    issues,
                    at.to_string(),
                    format!(
                        "register '{}' cannot move to {area} {} on device '{device}': register {other} is there",
                        reg.name,
                        span(reg.address, w)
                    ),
                );
                return;
            }
            let mut moved = m;
            moved.insert(text("address"), int(reg.address));
            moved.insert(text("zero_based"), Value::Bool(true));
            let mut errs = Vec::new();
            regs[i] = Value::Mapping(norm::normalize(register_fields(), &moved, "", &mut errs));
            exp.registers_moved
                .push(format!("{device}/{}@{current}->{}", reg.name, reg.address));
        }
        None => {
            if linked {
                // resolve() refused already.
                return;
            }
            if let Some(other) =
                overlapping(&regs, &reg.name, reg.address, width, &reg.register_area)
            {
                error(
                    issues,
                    at.to_string(),
                    format!(
                        "register '{}' ({} at {} {}) overlaps register {other} on device '{device}'",
                        reg.name,
                        reg.register_type,
                        reg.register_area,
                        span(reg.address, width)
                    ),
                );
                return;
            }
            let n = regs.len() as i64;
            regs.push(register_entity(reg, n));
            exp.registers_added
                .push(format!("{device}/{}@{}", reg.name, reg.address));
        }
    }
    dev.insert(text("registers"), Value::Sequence(regs));
}

fn widgets_of(screen: &Mapping) -> Vec<Mapping> {
    screen
        .get("scada")
        .and_then(Value::as_mapping)
        .and_then(|s| s.get("equipment"))
        .and_then(Value::as_sequence)
        .map(|s| s.iter().filter_map(Value::as_mapping).cloned().collect())
        .unwrap_or_default()
}

fn equipment_of(widget: &Mapping) -> Option<String> {
    widget
        .get("config")
        .and_then(Value::as_mapping)
        .and_then(|c| c.get("equipment"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn widget_instances(pool: &Pool, sid: &str) -> Vec<String> {
    let e = match pool.in_doc(sid) {
        Some(i) => Some(&pool.doc[i]),
        None => pool.stored.get(sid),
    };
    e.map(|e| widgets_of(e).iter().filter_map(equipment_of).collect())
        .unwrap_or_default()
}

/// Bind the widgets of one screen entity to their instances (refs).
fn materialize_screen(
    screen: &mut Mapping,
    sid: &str,
    patches: &HashMap<String, resolve::WidgetPatch>,
    bound: &mut HashMap<String, Vec<WidgetRef>>,
) {
    let Some(Value::Mapping(scada)) = screen.get_mut("scada") else {
        return;
    };
    let Some(Value::Sequence(items)) = scada.get_mut("equipment") else {
        return;
    };
    let fields = spec::SCADA_EQUIPMENT;
    for item in items.iter_mut() {
        let Value::Mapping(w) = item else { continue };
        let Some(name) = equipment_of(w) else {
            continue;
        };
        let Some(patch) = patches.get(&name) else {
            continue;
        };
        bound.entry(name).or_default().push(WidgetRef {
            screen_id: sid.to_string(),
            equipment_id: get_str(w, "equipment_id"),
        });
        let mut kind = get_str(w, "equipment_type");
        let mut sensors: Vec<serde_json::Value> = w
            .get("sensors")
            .and_then(|v| norm::yaml_to_json(v).ok())
            .and_then(|j| j.as_array().cloned())
            .unwrap_or_default();
        let mut config = w
            .get("config")
            .and_then(|v| norm::yaml_to_json(v).ok())
            .and_then(|j| j.as_object().cloned())
            .unwrap_or_default();
        if !resolve::apply_patch(patch, &mut kind, &mut sensors, &mut config) {
            continue;
        }
        w.insert(text("equipment_type"), text(kind));
        if let Ok(v) = norm::json_to_yaml(&serde_json::Value::Array(sensors)) {
            w.insert(text("sensors"), v);
        }
        if let Ok(v) = norm::json_to_yaml(&serde_json::Value::Object(config)) {
            w.insert(text("config"), norm::canon(v));
        }
        let mut errs = Vec::new();
        *w = norm::normalize(fields, w, "", &mut errs);
    }
}
