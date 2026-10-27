//! Semantic checks on a project document: duplicate keys, dangling references,
//! secrets that cannot be resolved.

use std::collections::HashSet;

use serde_yaml_neo::{Mapping, Value};

use super::export::{self, Snapshot, entity_key};
use super::model::{Issue, ProjectDoc, Severity};
use super::norm::{display, get_str};
use super::secrets;
use super::spec::{self, Kind, Section};

/// Entities that already exist in the target database. A merge import may
/// refer to them from the document; a replace import may not (it deletes
/// whatever the document does not list).
#[derive(Default)]
pub struct Known {
    pub tag_refs: HashSet<String>,
    pub tag_keys: HashSet<String>,
    pub scripts: HashSet<String>,
    pub sim_profiles: HashSet<String>,
    pub devices: HashSet<String>,
}

fn collect_devices(snap_or_doc: &dyn Fn(&str) -> Vec<Mapping>) -> HashSet<String> {
    let mut out = HashSet::new();
    for (section, field) in [
        ("modbus_devices", "id"),
        ("opcua_devices", "name"),
        ("s7_devices", "name"),
        ("enip_devices", "name"),
        ("dnp3_devices", "name"),
        ("bacnet_devices", "name"),
        ("voltrus_plc_devices", "name"),
        ("screens", "id"),
    ] {
        for e in snap_or_doc(section) {
            out.insert(get_str(&e, field));
        }
    }
    out
}

impl Known {
    pub fn from_snapshot(snap: &Snapshot) -> Self {
        let rows = |section: &str| -> Vec<Mapping> {
            snap.get(section)
                .map(|r| r.iter().map(|r| r.entity.clone()).collect())
                .unwrap_or_default()
        };
        Known {
            tag_refs: snap
                .get("tags")
                .map(|r| r.iter().map(|r| r.key.clone()).collect())
                .unwrap_or_default(),
            tag_keys: rows("tags").iter().map(|e| get_str(e, "key")).collect(),
            scripts: rows("scripts").iter().map(|e| get_str(e, "name")).collect(),
            sim_profiles: rows("sim_profiles")
                .iter()
                .map(|e| get_str(e, "name"))
                .collect(),
            devices: collect_devices(&rows),
        }
    }
}

struct Ctx<'a> {
    issues: Vec<Issue>,
    tag_refs: HashSet<String>,
    scripts: HashSet<String>,
    sim_profiles: HashSet<String>,
    devices: HashSet<String>,
    tag_keys: HashSet<String>,
    _doc: &'a ProjectDoc,
}

impl Ctx<'_> {
    fn error(&mut self, at: String, message: String) {
        self.issues.push(Issue {
            severity: Severity::Error,
            at,
            message,
        });
    }
    fn warn(&mut self, at: String, message: String) {
        self.issues.push(Issue {
            severity: Severity::Warning,
            at,
            message,
        });
    }
}

fn seq<'a>(m: &'a Mapping, name: &str) -> Vec<&'a Mapping> {
    match m.get(name) {
        Some(Value::Sequence(items)) => items.iter().filter_map(Value::as_mapping).collect(),
        _ => Vec::new(),
    }
}

/// Report duplicates among `keys`; `at` prefixes the message.
fn duplicates(ctx: &mut Ctx<'_>, at: &str, what: &str, keys: Vec<String>) {
    let mut seen = HashSet::new();
    let mut reported = HashSet::new();
    for k in keys {
        if !seen.insert(k.clone()) && reported.insert(k.clone()) {
            ctx.error(at.to_string(), format!("duplicate {what} '{k}'"));
        }
    }
}

/// Check a document. `known` lists entities already in the target database
/// (merge imports); `check_env` adds a warning for every `${ENV}` reference
/// whose variable is unset.
pub fn validate_doc(doc: &ProjectDoc, known: Option<&Known>, check_env: bool) -> Vec<Issue> {
    let rows = |section: &str| -> Vec<Mapping> { doc.entities(section).to_vec() };
    let mut ctx = Ctx {
        issues: Vec::new(),
        tag_refs: doc
            .entities("tags")
            .iter()
            .map(|e| entity_key(spec::section("tags").expect("tags"), e))
            .collect(),
        scripts: doc
            .entities("scripts")
            .iter()
            .map(|e| get_str(e, "name"))
            .collect(),
        sim_profiles: doc
            .entities("sim_profiles")
            .iter()
            .map(|e| get_str(e, "name"))
            .collect(),
        devices: collect_devices(&rows),
        tag_keys: doc
            .entities("tags")
            .iter()
            .map(|e| get_str(e, "key"))
            .collect(),
        _doc: doc,
    };
    if let Some(k) = known {
        ctx.tag_refs.extend(k.tag_refs.iter().cloned());
        ctx.tag_keys.extend(k.tag_keys.iter().cloned());
        ctx.scripts.extend(k.scripts.iter().cloned());
        ctx.sim_profiles.extend(k.sim_profiles.iter().cloned());
        ctx.devices.extend(k.devices.iter().cloned());
    }

    for sec in spec::SECTIONS {
        let entities = doc.entities(sec.name);
        if sec.unique {
            let keys: Vec<String> = entities.iter().map(|e| entity_key(sec, e)).collect();
            duplicates(&mut ctx, sec.name, sec.label, keys);
        }
        for e in entities {
            let at = format!("{}[{}]", sec.name, entity_key(sec, e));
            check_children(&mut ctx, sec, e, &at);
            match sec.kind {
                Kind::Tags => check_tag(&mut ctx, e, &at),
                Kind::Screens => check_screen(&mut ctx, e, &at),
                Kind::Plain(_) => check_plain(&mut ctx, sec, e, &at),
                Kind::Settings => {}
            }
            // Unset variables are only a problem for entities that do not exist
            // yet, which the import reports; here they are a heads-up.
            let pending = if check_env {
                secrets::pending_references(sec.name, e)
            } else {
                Vec::new()
            };
            for var in pending {
                if std::env::var(&var).map(|v| v.is_empty()).unwrap_or(true) {
                    ctx.warn(
                        at.clone(),
                        format!("environment variable {var} is not set (needed when this entity is created)"),
                    );
                }
            }
        }
    }
    ctx.issues
}

fn check_children(ctx: &mut Ctx<'_>, sec: &Section, e: &Mapping, at: &str) {
    let Kind::Plain(p) = &sec.kind else { return };
    for child in p.children {
        let rows = seq(e, child.yaml);
        let keys: Vec<String> = rows
            .iter()
            .map(|r| {
                child
                    .key
                    .iter()
                    .map(|k| r.get(*k).map(display).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();
        duplicates(ctx, &format!("{at}.{}", child.yaml), "entry", keys);
    }
}

fn check_plain(ctx: &mut Ctx<'_>, sec: &Section, e: &Mapping, at: &str) {
    match sec.name {
        "flows" => {
            for (i, node) in seq(e, "nodes").into_iter().enumerate() {
                if let Some(r) = node.get("tag").and_then(Value::as_str)
                    && !ctx.tag_refs.contains(r)
                {
                    ctx.error(format!("{at}.nodes[{i}]"), format!("unknown tag '{r}'"));
                }
                if let Some(s) = node.get("script").and_then(Value::as_str)
                    && !ctx.scripts.contains(s)
                {
                    ctx.error(format!("{at}.nodes[{i}]"), format!("unknown script '{s}'"));
                }
            }
        }
        "recipes" => {
            let device = get_str(e, "device_id");
            if !ctx.devices.contains(&device) {
                ctx.warn(
                    at.to_string(),
                    format!("device '{device}' is not defined in the project"),
                );
            }
        }
        "notifications" => {
            let key = get_str(e, "source_key");
            if !ctx.tag_keys.contains(&key) {
                ctx.warn(
                    at.to_string(),
                    format!("no tag has the key '{key}'; the rule will never fire"),
                );
            }
        }
        super::templates::SECTION => {
            for message in super::templates::check(e) {
                ctx.error(at.to_string(), message);
            }
        }
        _ => {}
    }
}

fn check_tag(ctx: &mut Ctx<'_>, e: &Mapping, at: &str) {
    let base = export::tag_base_ref(e);
    if let Some(r) = e.get("ref").and_then(Value::as_str) {
        let ok = r == base
            || r.strip_prefix(&base)
                .and_then(|rest| rest.strip_prefix('#'))
                .is_some_and(|n| n.parse::<u32>().is_ok_and(|n| n >= 2));
        if !ok {
            ctx.error(
                at.to_string(),
                format!("ref '{r}' must be '{base}' or '{base}#N' (N >= 2)"),
            );
        }
    }
    let device = get_str(e, "device");
    if device != "default" && !ctx.devices.contains(&device) {
        ctx.warn(
            at.to_string(),
            format!("device '{device}' is not defined in the project"),
        );
    }
}

fn check_screen(ctx: &mut Ctx<'_>, e: &Mapping, at: &str) {
    if let Some(p) = e.get("sim_profile").and_then(Value::as_str)
        && !ctx.sim_profiles.contains(p)
    {
        ctx.error(at.to_string(), format!("unknown simulation profile '{p}'"));
    }
    if e.get("name").is_none() && (e.get("sim_profile").is_some() || e.get("sim_config").is_some())
    {
        ctx.error(
            at.to_string(),
            "a screen without a name has no device row, so it cannot carry simulation settings"
                .into(),
        );
    }

    let members: Vec<String> = seq(e, "tags").iter().map(|m| get_str(m, "ref")).collect();
    for (i, r) in members.iter().enumerate() {
        if !ctx.tag_refs.contains(r) {
            ctx.error(format!("{at}.tags[{i}]"), format!("unknown tag '{r}'"));
        }
    }
    duplicates(ctx, &format!("{at}.tags"), "tag", members);

    if let Some(Value::Mapping(layouts)) = e.get("layouts") {
        for (bp, items) in layouts {
            let Value::Sequence(items) = items else {
                ctx.error(
                    format!("{at}.layouts.{}", display(bp)),
                    "a layout must be a list".into(),
                );
                continue;
            };
            for (i, item) in items.iter().enumerate() {
                if let Some(r) = item
                    .as_mapping()
                    .and_then(|m| m.get("tag"))
                    .and_then(Value::as_str)
                    && !ctx.tag_refs.contains(r)
                {
                    ctx.error(
                        format!("{at}.layouts.{}[{i}]", display(bp)),
                        format!("unknown tag '{r}'"),
                    );
                }
            }
        }
    }

    let canvas: Vec<String> = seq(e, "canvas").iter().map(|m| get_str(m, "tag")).collect();
    for (i, r) in canvas.iter().enumerate() {
        if !ctx.tag_refs.contains(r) {
            ctx.error(format!("{at}.canvas[{i}]"), format!("unknown tag '{r}'"));
        }
    }
    duplicates(ctx, &format!("{at}.canvas"), "tag", canvas);

    let twin = e
        .get("twin")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    duplicates(
        ctx,
        &format!("{at}.twin.layouts"),
        "equipment",
        seq(&twin, "layouts")
            .iter()
            .map(|m| get_str(m, "equipment_id"))
            .collect(),
    );
    duplicates(
        ctx,
        &format!("{at}.twin.connections"),
        "connection",
        seq(&twin, "connections")
            .iter()
            .map(|m| format!("{}->{}", get_str(m, "from_id"), get_str(m, "to_id")))
            .collect(),
    );

    let scada = e
        .get("scada")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    let equipment = seq(&scada, "equipment");
    for (i, eq) in equipment.iter().enumerate() {
        if let Some(Value::Sequence(refs)) = eq.get("sensors") {
            for r in refs {
                let r = display(r);
                if !ctx.tag_refs.contains(&r) {
                    ctx.error(
                        format!("{at}.scada.equipment[{i}].sensors"),
                        format!("unknown tag '{r}'"),
                    );
                }
            }
        }
    }
    duplicates(
        ctx,
        &format!("{at}.scada.equipment"),
        "equipment",
        equipment
            .iter()
            .map(|m| get_str(m, "equipment_id"))
            .collect(),
    );
    duplicates(
        ctx,
        &format!("{at}.scada.connections"),
        "connection",
        seq(&scada, "connections")
            .iter()
            .map(|m| get_str(m, "connection_id"))
            .collect(),
    );
}
