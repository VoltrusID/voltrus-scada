//! Modbus devices linked to a device template, on import.
//!
//! A linked device follows its template (docs/device-templates.md, "Linked
//! devices"), so an import must not leave it with other registers:
//!
//! * Merge: a device written without `template_id`, `template_version` and
//!   `template_source` (a file from before links) keeps the link it has.
//!   Replace writes the file as it is, so there the device is unlinked.
//! * A linked device whose registers differ from its template's gets the
//!   template's registers, with a warning naming it.
//! * Merge: a linked device the file leaves out whose registers differ from
//!   its template (the file changes the template) is updated to them too.
//!
//! The template is the one in the file, else the stored one, else the
//! built-in. The import's plan lists these devices as updated, so their
//! pollers restart with the rest.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_yaml_neo::{Mapping, Value};

use super::export::{Snapshot, entity_key};
use super::model::{Mode, ProjectDoc};
use super::norm::{self, get_str, int, text};
use super::spec::{self, Ty};
use super::templates;
use crate::device_templates::link::TemplateLink;
use crate::device_templates::{Protocol, TemplateRegister, builtin};

const DEVICES: &str = "modbus_devices";
const LINK_FIELDS: [&str; 3] = ["template_id", "template_version", "template_source"];

fn link_of(e: &Mapping) -> Option<TemplateLink> {
    let template_id = get_str(e, "template_id");
    (!template_id.is_empty()).then(|| TemplateLink {
        template_id,
        version: get_str(e, "template_version"),
        source: get_str(e, "template_source"),
    })
}

/// Register maps of the templates the import can see, file first.
fn template_registers(
    doc: &ProjectDoc,
    snap: &Snapshot,
) -> HashMap<TemplateLink, Vec<TemplateRegister>> {
    let mut out = HashMap::new();
    let builtins = builtin::builtins().iter().cloned();
    let stored = snap
        .get(templates::SECTION)
        .into_iter()
        .flatten()
        .filter_map(|r| templates::to_template(&r.entity).ok());
    let in_file = doc
        .entities(templates::SECTION)
        .iter()
        .filter_map(|e| templates::to_template(e).ok());
    for mut t in builtins.chain(stored).chain(in_file) {
        if t.protocol != Protocol::Modbus || t.normalize_and_validate().is_err() {
            continue;
        }
        let link = TemplateLink {
            template_id: t.template_id.clone(),
            version: t.version.clone(),
            source: t.source.clone(),
        };
        out.insert(link, t.registers);
    }
    out
}

fn register_fields() -> &'static [spec::Field] {
    spec::section(DEVICES)
        .and_then(|s| s.fields.iter().find(|f| f.name == "registers"))
        .and_then(|f| match f.ty {
            Ty::List(sub) => Some(sub),
            _ => None,
        })
        .expect("modbus_devices.registers")
}

/// The template's registers as a device stores them after the template is
/// applied (0-based offsets, in template order), in canonical form.
fn as_device_registers(regs: &[TemplateRegister]) -> Vec<Value> {
    let fields = register_fields();
    regs.iter()
        .enumerate()
        .map(|(i, r)| {
            let mut raw = Mapping::new();
            raw.insert(text("name"), text(r.name.clone()));
            raw.insert(text("address"), int(r.offset.into()));
            raw.insert(text("register_type"), text(r.register_type.clone()));
            raw.insert(text("register_area"), text(r.register_area.clone()));
            raw.insert(text("scale"), Value::from(r.scale));
            raw.insert(text("sort_order"), int(i as i64));
            raw.insert(text("zero_based"), Value::Bool(true));
            Value::Mapping(norm::normalize(fields, &raw, "", &mut Vec::new()))
        })
        .collect()
}

/// One register reduced to what a template sets: name, wire offset, type,
/// area and scale.
fn register_key(m: &Mapping) -> (String, u16, String, String, String) {
    let fields = register_fields();
    let field = |name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .expect("register field")
    };
    let address = norm::get(m, field("address")).as_i64().unwrap_or(-1);
    let zero_based = norm::get(m, field("zero_based")).as_bool().unwrap_or(false);
    let offset = u16::try_from(address)
        .map(|a| crate::models::protocol_address(a, zero_based))
        .unwrap_or(u16::MAX);
    (
        get_str(m, "name"),
        offset,
        norm::display(&norm::get(m, field("register_type"))),
        norm::display(&norm::get(m, field("register_area"))),
        norm::display(&norm::get(m, field("scale"))),
    )
}

fn registers_of(e: &Mapping) -> Vec<Value> {
    match e.get("registers") {
        Some(Value::Sequence(s)) => s.clone(),
        _ => Vec::new(),
    }
}

fn same_registers(a: &[Value], b: &[Value]) -> bool {
    let keys = |v: &[Value]| {
        let mut k: Vec<_> = v
            .iter()
            .filter_map(Value::as_mapping)
            .map(register_key)
            .collect();
        k.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
        k
    };
    a.len() == b.len() && keys(a) == keys(b)
}

fn set_registers(e: &mut Mapping, regs: Vec<Value>) {
    if regs.is_empty() {
        e.remove("registers");
    } else {
        e.insert(text("registers"), Value::Sequence(regs));
    }
}

/// The document to import, with linked Modbus devices made to follow their
/// templates. Notes on what changed go to `warnings`.
pub fn follow_templates(
    doc: &ProjectDoc,
    snap: &Snapshot,
    mode: Mode,
    warnings: &mut Vec<String>,
) -> ProjectDoc {
    let mut doc = doc.clone();
    let templates = template_registers(&doc, snap);
    let stored: BTreeMap<String, &Mapping> = snap
        .get(DEVICES)
        .into_iter()
        .flatten()
        .map(|r| (r.key.clone(), &r.entity))
        .collect();
    let sec = spec::section(DEVICES).expect("modbus_devices");

    let mut in_file = HashSet::new();
    let devices = doc.sections.entry(DEVICES.to_string()).or_default();
    for e in devices.iter_mut() {
        let id = entity_key(sec, e);
        in_file.insert(id.clone());
        if mode == Mode::Merge
            && LINK_FIELDS.iter().all(|f| !e.contains_key(*f))
            && let Some(old) = stored.get(&id)
            && link_of(old).is_some()
        {
            for f in LINK_FIELDS {
                if let Some(v) = old.get(f) {
                    e.insert(text(f), v.clone());
                }
            }
        }
        let Some(link) = link_of(e) else { continue };
        let Some(regs) = templates.get(&link) else {
            continue;
        };
        let wanted = as_device_registers(regs);
        if !same_registers(&registers_of(e), &wanted) {
            warnings.push(format!(
                "modbus_devices[{id}]: follows template {link}, so its registers are the template's, not the ones in the file (unlink the device to keep its own)"
            ));
            set_registers(e, wanted);
        }
    }

    if mode == Mode::Merge {
        for (id, old) in &stored {
            if in_file.contains(id) {
                continue;
            }
            let Some(link) = link_of(old) else { continue };
            let Some(regs) = templates.get(&link) else {
                continue;
            };
            let wanted = as_device_registers(regs);
            if !same_registers(&registers_of(old), &wanted) {
                warnings.push(format!(
                    "modbus_devices[{id}]: follows template {link}, whose registers it does not have; it is updated to them"
                ));
                let mut e = (*old).clone();
                set_registers(&mut e, wanted);
                devices.push(e);
            }
        }
    }
    doc
}
