//! Public data types of the project format.

use std::collections::BTreeMap;
use std::str::FromStr;

use serde::{Serialize, Serializer};
use serde_yaml_neo::{Mapping, Value};

use super::spec::{self, Section};

pub const FORMAT: &str = "voltrus-project/1";

/// How an import treats entities the project does not mention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Create and update what the project lists, leave everything else alone.
    Merge,
    /// Make the database configuration equal to the project: entities of the
    /// covered kinds that the project does not list are deleted.
    Replace,
}

impl FromStr for Mode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "merge" => Ok(Mode::Merge),
            "replace" => Ok(Mode::Replace),
            other => anyhow::bail!("unknown import mode '{other}' (expected merge or replace)"),
        }
    }
}

/// A whole project: manifest plus every entity, in canonical form.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectDoc {
    pub format: String,
    pub server_version: String,
    pub generated_at: String,
    /// Section name -> entities (already normalised). The settings singleton
    /// holds zero or one entry.
    pub sections: BTreeMap<String, Vec<Mapping>>,
    /// Notes produced while exporting or parsing; never serialised.
    pub warnings: Vec<String>,
}

impl ProjectDoc {
    pub fn entities(&self, section: &str) -> &[Mapping] {
        self.sections.get(section).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn is_empty(&self) -> bool {
        self.sections.values().all(Vec::is_empty)
    }

    /// The manifest as a mapping (`format`, `server_version`, `generated_at`).
    pub fn manifest(&self) -> Mapping {
        let mut m = Mapping::new();
        m.insert(
            Value::String("format".into()),
            Value::String(self.format.clone()),
        );
        m.insert(
            Value::String("server_version".into()),
            Value::String(self.server_version.clone()),
        );
        m.insert(
            Value::String("generated_at".into()),
            Value::String(self.generated_at.clone()),
        );
        m
    }

    /// Value of one section as written to a file: a sequence, or the single
    /// mapping of a singleton section. `None` when the section is empty.
    pub fn section_value(&self, s: &Section) -> Option<Value> {
        let items = self.entities(s.name);
        if items.is_empty() {
            return None;
        }
        Some(match s.layout {
            spec::Layout::Single => Value::Mapping(items[0].clone()),
            _ => Value::Sequence(items.iter().cloned().map(Value::Mapping).collect()),
        })
    }

    /// The single-file form as one mapping.
    pub fn to_value(&self) -> Value {
        let mut m = self.manifest();
        for s in spec::SECTIONS {
            if let Some(v) = self.section_value(s) {
                m.insert(Value::String(s.name.into()), v);
            }
        }
        Value::Mapping(m)
    }
}

impl Serialize for ProjectDoc {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(ser)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// One validation finding.
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    /// Where: `section[key]` or `section[key].field`.
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tag = match self.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARN",
        };
        write!(f, "{tag}: {}: {}", self.at, self.message)
    }
}

/// Returned (inside `anyhow::Error`, downcast with `downcast_ref`) when a
/// project fails validation, so callers can report every finding.
#[derive(Debug, Clone)]
pub struct InvalidProject(pub Vec<Issue>);

impl std::fmt::Display for InvalidProject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let errors: Vec<String> = self
            .0
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.to_string())
            .collect();
        write!(f, "project is invalid:\n{}", errors.join("\n"))
    }
}

impl std::error::Error for InvalidProject {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub section: String,
    pub key: String,
    pub action: Action,
    /// Top-level fields that differ (updates only). Values are not listed so
    /// plans never leak secrets.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub create: usize,
    pub update: usize,
    pub delete: usize,
    pub unchanged: usize,
}

/// What an import does (dry run) or did (applied).
#[derive(Debug, Clone, Serialize)]
pub struct ImportPlan {
    pub mode: Mode,
    pub dry_run: bool,
    pub totals: Counts,
    pub sections: BTreeMap<String, Counts>,
    pub changes: Vec<Change>,
    pub warnings: Vec<String>,
}

impl ImportPlan {
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }

    /// Changes of one section, e.g. to restart pollers after an import.
    pub fn changes_in<'a>(&'a self, section: &'a str) -> impl Iterator<Item = &'a Change> {
        self.changes.iter().filter(move |c| c.section == section)
    }

    /// Human-readable listing (`+ create`, `~ update`, `- delete`).
    pub fn render(&self) -> String {
        let mut out = String::new();
        for c in &self.changes {
            let (sym, verb) = match c.action {
                Action::Create => ('+', "create"),
                Action::Update => ('~', "update"),
                Action::Delete => ('-', "delete"),
            };
            out.push_str(&format!("{sym} {verb} {}: {}", c.section, c.key));
            if !c.fields.is_empty() {
                out.push_str(&format!(" ({})", c.fields.join(", ")));
            }
            out.push('\n');
        }
        for w in &self.warnings {
            out.push_str(&format!("warning: {w}\n"));
        }
        out.push_str(&format!(
            "{} create, {} update, {} delete, {} unchanged ({} mode{})\n",
            self.totals.create,
            self.totals.update,
            self.totals.delete,
            self.totals.unchanged,
            match self.mode {
                Mode::Merge => "merge",
                Mode::Replace => "replace",
            },
            if self.dry_run { ", dry run" } else { "" }
        ));
        out
    }
}
