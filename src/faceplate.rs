//! Faceplate definitions: one popup for an equipment kind (pump, motor, valve,
//! analog measurement) with named roles (`run`, `fault`, `cmd`, `speed`, ...).
//! A SCADA symbol opens it with its own tag per role, stored in the symbol's
//! config as `{"faceplate": "<id>", "faceplateMap": {"run": <tag id>, ...}}`,
//! so one definition serves any number of instances. See docs/faceplates.md.

use serde::{Deserialize, Serialize};

use crate::project_yaml::spec::{FACEPLATE_KINDS, FACEPLATE_ROLE_KINDS};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FaceplateRole {
    /// Name the instances map a tag to, e.g. `run`.
    pub role: String,
    /// Shown in the popup; default: the role name.
    #[serde(default)]
    pub label: String,
    /// `state` (on/off feedback), `fault` (on means tripped), `analog` (value with
    /// its alarm limits), `command` (start/stop or open/close, writes 1 or 0),
    /// `setpoint` (a value the operator types).
    pub kind: String,
    /// An instance must map this role.
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Faceplate {
    pub id: String,
    pub name: String,
    /// `pump`, `motor`, `valve` or `analog`; picks the command words
    /// (START/STOP or OPEN/CLOSE).
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    pub roles: Vec<FaceplateRole>,
}

fn default_kind() -> String {
    "analog".to_string()
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

impl Faceplate {
    /// Every problem, each naming its field.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !is_slug(&self.id) {
            out.push(format!(
                "id '{}' must be 1-64 lowercase letters, digits, '_' or '-'",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            out.push("name must not be empty".to_string());
        }
        if !FACEPLATE_KINDS.contains(&self.kind.as_str()) {
            out.push(format!(
                "kind '{}' is not one of {}",
                self.kind,
                FACEPLATE_KINDS.join(", ")
            ));
        }
        if self.roles.is_empty() {
            out.push("roles must list at least one role".to_string());
        }
        let mut seen = std::collections::HashSet::new();
        for (i, r) in self.roles.iter().enumerate() {
            if !is_slug(&r.role) {
                out.push(format!(
                    "roles[{i}].role '{}' must be 1-64 lowercase letters, digits, '_' or '-'",
                    r.role
                ));
            }
            if !seen.insert(r.role.as_str()) {
                out.push(format!("roles[{i}].role '{}' is listed twice", r.role));
            }
            if !FACEPLATE_ROLE_KINDS.contains(&r.kind.as_str()) {
                out.push(format!(
                    "roles[{i}].kind '{}' is not one of {}",
                    r.kind,
                    FACEPLATE_ROLE_KINDS.join(", ")
                ));
            }
        }
        out
    }

    /// Required roles an instance mapping leaves out.
    pub fn missing_roles<'a>(&'a self, mapped: &[&str]) -> Vec<&'a str> {
        self.roles
            .iter()
            .filter(|r| r.required && !mapped.contains(&r.role.as_str()))
            .map(|r| r.role.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pump() -> Faceplate {
        serde_json::from_value(serde_json::json!({
            "id": "pump_std", "name": "Pump", "kind": "pump",
            "roles": [
                {"role": "run", "kind": "state", "required": true},
                {"role": "fault", "kind": "fault"},
                {"role": "cmd", "kind": "command", "label": "Start / stop"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn a_valid_definition_has_no_problems() {
        assert!(pump().problems().is_empty());
        assert_eq!(pump().missing_roles(&["fault"]), vec!["run"]);
        assert!(pump().missing_roles(&["run"]).is_empty());
    }

    #[test]
    fn problems_name_the_field() {
        let mut f = pump();
        f.id = "Pump 1".into();
        f.kind = "fan".into();
        f.roles[1].role = "run".into();
        f.roles[2].kind = "button".into();
        let p = f.problems().join("\n");
        assert!(p.contains("id 'Pump 1'"), "{p}");
        assert!(
            p.contains("kind 'fan' is not one of pump, motor, valve, analog"),
            "{p}"
        );
        assert!(p.contains("roles[1].role 'run' is listed twice"), "{p}");
        assert!(
            p.contains("roles[2].kind 'button' is not one of state, fault"),
            "{p}"
        );
    }

    #[test]
    fn unknown_fields_are_refused() {
        let e = serde_json::from_value::<Faceplate>(serde_json::json!({
            "id": "a", "name": "A", "roles": [], "colour": "red"
        }))
        .unwrap_err()
        .to_string();
        assert!(e.contains("unknown field `colour`"), "{e}");
    }
}
