//! Devices linked to a template: instead of keeping the copy of the register
//! map made when they were created, a linked Modbus device follows its
//! template. Saving the template rewrites the registers of every linked
//! device; unlinking keeps the current registers as the device's own copy.

use serde::{Deserialize, Serialize};

use super::TemplateRegister;

/// Which stored template a device follows: the same triple that identifies a
/// template everywhere (`template_id`, `version`, `source`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateLink {
    pub template_id: String,
    pub version: String,
    pub source: String,
}

impl std::fmt::Display for TemplateLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} ({})", self.template_id, self.version, self.source)
    }
}

/// One register as a device stores it, reduced to what a template sets.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RegisterRow {
    pub name: String,
    /// 0-based offset within `register_area`.
    pub offset: u16,
    pub register_type: String,
    pub register_area: String,
    pub scale: f64,
}

impl From<&TemplateRegister> for RegisterRow {
    fn from(r: &TemplateRegister) -> Self {
        RegisterRow {
            name: r.name.clone(),
            offset: r.offset,
            register_type: r.register_type.clone(),
            register_area: r.register_area.clone(),
            scale: r.scale,
        }
    }
}

/// What applying a template's registers would change on one device, by
/// register name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, utoipa::ToSchema)]
pub struct RegisterDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Same name, different offset, type, area or scale.
    pub changed: Vec<String>,
    /// Keys of the data sources (tags) on `removed` registers: they stop
    /// updating once the template is applied. A renamed register shows up as
    /// removed and added, so its tags are listed here too.
    pub affected_data_sources: Vec<String>,
}

impl RegisterDiff {
    /// No register changes (`affected_data_sources` follows from `removed`).
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

pub fn diff(current: &[RegisterRow], template: &[TemplateRegister]) -> RegisterDiff {
    let mut d = RegisterDiff::default();
    for t in template {
        match current.iter().find(|c| c.name == t.name) {
            None => d.added.push(t.name.clone()),
            Some(c) if *c != RegisterRow::from(t) => d.changed.push(t.name.clone()),
            Some(_) => {}
        }
    }
    for c in current {
        if !template.iter().any(|t| t.name == c.name) {
            d.removed.push(c.name.clone());
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn treg(name: &str, offset: u16, scale: f64) -> TemplateRegister {
        serde_json::from_value(serde_json::json!({"name": name, "offset": offset, "scale": scale}))
            .unwrap()
    }

    fn row(name: &str, offset: u16, scale: f64) -> RegisterRow {
        RegisterRow {
            name: name.into(),
            offset,
            register_type: "float32".into(),
            register_area: "holding".into(),
            scale,
        }
    }

    #[test]
    fn diff_names_added_removed_and_changed_registers() {
        let current = [row("v", 0, 1.0), row("i", 2, 1.0), row("old", 8, 1.0)];
        let template = [treg("v", 0, 1.0), treg("i", 2, 0.1), treg("p", 4, 1.0)];
        let d = diff(&current, &template);
        assert_eq!(d.added, vec!["p"]);
        assert_eq!(d.removed, vec!["old"]);
        assert_eq!(d.changed, vec!["i"]);
        assert!(diff(&[row("v", 0, 1.0)], &[treg("v", 0, 1.0)]).is_empty());
    }
}
