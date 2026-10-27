//! Device templates in a project.
//!
//! The database keeps a template as one JSON `body` next to a few indexed
//! columns. In a project the template's own fields sit directly on the entity,
//! so a template reads the same as a `voltrus-device-template/1` file: this
//! module flattens `body` on export, rebuilds it on import and checks a
//! hand-written entity with the same rules the template routes apply.
//!
//! Built-in templates ship with the server and are never part of a project;
//! `local` and `catalog:<url>` templates are.

use anyhow::{Context, Result, bail};
use serde_yaml_neo::{Mapping, Value};

use super::export::Ctx;
use super::norm::{self, text};
use super::spec;
use crate::device_templates::{
    DeviceTemplate, SOURCE_BUILTIN, SOURCE_CATALOG_PREFIX, SOURCE_LOCAL, TEMPLATE_FORMAT,
};

/// Name of the section in a project.
pub const SECTION: &str = "device_templates";

/// Read the entity back as a template (without validating it).
pub fn to_template(entity: &Mapping) -> Result<DeviceTemplate> {
    let mut json = norm::yaml_to_json(&Value::Mapping(entity.clone()))?;
    let obj = json.as_object_mut().context("template is not a mapping")?;
    obj.remove("body");
    obj.insert("format".into(), TEMPLATE_FORMAT.into());
    serde_json::from_value(json).context("not a device template")
}

/// The entity fields of a template, in canonical form.
fn to_entity(t: &DeviceTemplate) -> Result<Mapping> {
    let fields = spec::section(SECTION)
        .context("device_templates section")?
        .fields;
    let Value::Mapping(raw) = norm::json_to_yaml(&serde_json::to_value(t)?)? else {
        bail!("template is not a mapping");
    };
    flatten(raw, fields)
}

fn flatten(mut raw: Mapping, fields: &[spec::Field]) -> Result<Mapping> {
    raw.remove("format");
    let mut errs = Vec::new();
    let entity = norm::normalize(fields, &raw, "", &mut errs);
    if errs.is_empty() {
        Ok(entity)
    } else {
        bail!("{}", errs.join("; "))
    }
}

/// A hand-written entity in the form export writes: normalised like a template
/// the routes would store. When it is not a valid template the entity is left
/// as written and [`check`] reports why.
pub fn canonicalize(e: &mut Mapping) {
    if e.contains_key("body") {
        return;
    }
    let Ok(mut t) = to_template(e) else { return };
    if t.normalize_and_validate().is_err() {
        return;
    }
    if let Ok(canonical) = to_entity(&t) {
        *e = canonical;
    }
}

/// Problems with one template entity, as messages.
pub fn check(e: &Mapping) -> Vec<String> {
    let mut out = Vec::new();
    if e.contains_key("body") {
        out.push(
            "`body` is how the server stores a template; write the template's fields directly"
                .into(),
        );
        return out;
    }
    let source = norm::get_str(e, "source");
    let from_catalog = source
        .strip_prefix(SOURCE_CATALOG_PREFIX)
        .is_some_and(|url| !url.is_empty());
    if source == SOURCE_BUILTIN {
        out.push("built-in templates ship with the server and are not part of a project".into());
    } else if source != SOURCE_LOCAL && !from_catalog {
        out.push(format!(
            "source '{source}' must be '{SOURCE_LOCAL}' or '{SOURCE_CATALOG_PREFIX}<url>'"
        ));
    }
    match to_template(e) {
        Ok(mut t) => {
            if let Err(errs) = t.normalize_and_validate() {
                out.extend(errs);
            }
        }
        Err(err) => out.push(format!("{err:#}")),
    }
    out
}

/// Export: expand the stored `body` into the entity's own fields.
pub fn export(raw: &mut Mapping, ctx: &mut Ctx) {
    let Some(body) = raw.remove("body") else {
        return;
    };
    let Value::Mapping(body) = body else {
        ctx.warnings.push(format!(
            "device template '{}' has a body that is not a JSON object; exported without its registers",
            norm::get_str(raw, "template_id")
        ));
        return;
    };
    for (k, v) in body {
        if k.as_str() == Some("format") || raw.contains_key(&k) {
            continue;
        }
        raw.insert(k, v);
    }
}

/// Import: build the `body` column from the entity's fields.
pub fn import(entity: &mut Mapping, _ctx: &Ctx) -> Result<()> {
    let mut t = to_template(entity)?;
    t.normalize_and_validate()
        .map_err(|errs| anyhow::anyhow!("invalid device template: {}", errs.join("; ")))?;
    entity.insert(
        text("body"),
        norm::json_to_yaml(&serde_json::to_value(&t)?)?,
    );
    Ok(())
}
