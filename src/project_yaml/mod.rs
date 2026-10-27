//! Declarative project format `voltrus-project/1`.
//!
//! A project is the whole configuration of a Voltrus server as YAML: devices of
//! every protocol with their registers/nodes/tags/points, tags with alarm and
//! write policies, screens with layouts and mimics, flows, scripts, recipes,
//! notification rules, integrations, simulation profiles, equipment templates,
//! local and catalog device templates (built-ins are not exported), shifts,
//! custom metrics, assets and a whitelist of settings. Everything is
//! keyed by natural keys (never database ids), secrets are `${ENV}` references,
//! and runtime data (telemetry, alarms, audit, users, API keys, sessions,
//! licence) is out of scope. See `docs/project-format.md`.
//!
//! Entry points:
//! * [`export_project_yaml`] / [`to_yaml`] / [`parse_project_yaml`]
//! * [`plan_import`] (dry run) and [`apply_import`] (one transaction)
//! * [`read_project`] / [`write_project`] for the file and directory forms
//! * [`validate_doc`] for offline checks

pub mod export;
pub mod import;
pub mod io;
pub mod model;
pub mod norm;
pub mod secrets;
pub mod spec;
pub mod templates;
pub mod validate;

#[cfg(test)]
mod tests;

pub use export::export_project_yaml;
pub use import::{
    Env, apply_import, apply_import_with_env, plan_import, plan_import_with_env, process_env,
};
pub use io::{
    is_single_file_path, parse_project_yaml, read_project, to_yaml, write_project,
    write_project_dir, write_project_file,
};
pub use model::{
    Action, Change, Counts, FORMAT, ImportPlan, InvalidProject, Issue, Mode, ProjectDoc, Severity,
};
pub use validate::validate_doc;
