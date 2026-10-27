//! Reading and writing project documents: single YAML text, one file, or a
//! directory.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_yaml_neo::{Mapping, Value};

use super::export::entity_key;
use super::model::{FORMAT, InvalidProject, Issue, ProjectDoc, Severity};
use super::norm::{self, display};
use super::spec::{self, Kind, Layout, Section};

const MANIFEST_FILE: &str = "project.yaml";

/// Serialise the single-file form.
pub fn to_yaml(doc: &ProjectDoc) -> String {
    serde_yaml_neo::to_string(&doc.to_value()).unwrap_or_default()
}

/// Parse the single-file form.
pub fn parse_project_yaml(src: &str) -> Result<ProjectDoc> {
    let value: Value = serde_yaml_neo::from_str(src).context("not valid YAML")?;
    match value {
        Value::Mapping(m) => doc_from_mapping(m),
        _ => bail!("a project file must be a YAML mapping"),
    }
}

fn invalid(at: impl Into<String>, message: impl Into<String>) -> anyhow::Error {
    InvalidProject(vec![Issue {
        severity: Severity::Error,
        at: at.into(),
        message: message.into(),
    }])
    .into()
}

/// Build a document from the top-level mapping (manifest keys plus sections).
fn doc_from_mapping(top: Mapping) -> Result<ProjectDoc> {
    let mut doc = ProjectDoc::default();
    let mut issues: Vec<Issue> = Vec::new();
    let mut format_seen = false;

    for (k, v) in &top {
        let Some(name) = k.as_str() else {
            issues.push(Issue {
                severity: Severity::Error,
                at: "(top level)".into(),
                message: format!("non-text key {}", display(k)),
            });
            continue;
        };
        match name {
            "format" => {
                format_seen = true;
                let f = display(v);
                if f != FORMAT {
                    return Err(invalid(
                        "format",
                        format!("unsupported format '{f}' (this server reads '{FORMAT}')"),
                    ));
                }
                doc.format = f;
            }
            "server_version" => doc.server_version = display(v),
            "generated_at" => doc.generated_at = display(v),
            section => {
                let Some(sec) = spec::section(section) else {
                    issues.push(Issue {
                        severity: Severity::Error,
                        at: section.to_string(),
                        message: "unknown top-level key".into(),
                    });
                    continue;
                };
                let entities = parse_section(sec, v, &mut issues);
                doc.sections.insert(sec.name.to_string(), entities);
            }
        }
    }
    if !format_seen {
        return Err(invalid(
            "format",
            format!("missing 'format' (expected '{FORMAT}')"),
        ));
    }
    if issues.iter().any(|i| i.severity == Severity::Error) {
        return Err(InvalidProject(issues).into());
    }
    for sec in spec::SECTIONS {
        let items = doc.sections.entry(sec.name.to_string()).or_default();
        items.sort_by_cached_key(|e| entity_key(sec, e));
    }
    Ok(doc)
}

fn parse_section(sec: &Section, v: &Value, issues: &mut Vec<Issue>) -> Vec<Mapping> {
    let items: Vec<&Value> = match (sec.layout, v) {
        (Layout::Single, Value::Mapping(_)) => vec![v],
        (_, Value::Sequence(s)) => s.iter().collect(),
        (_, Value::Null) => Vec::new(),
        (Layout::Single, _) => {
            issues.push(Issue {
                severity: Severity::Error,
                at: sec.name.into(),
                message: "expected a mapping".into(),
            });
            return Vec::new();
        }
        _ => {
            issues.push(Issue {
                severity: Severity::Error,
                at: sec.name.into(),
                message: "expected a list".into(),
            });
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        let Value::Mapping(m) = item else {
            issues.push(Issue {
                severity: Severity::Error,
                at: format!("{}[{i}]", sec.name),
                message: "expected a mapping".into(),
            });
            continue;
        };
        let mut errs = Vec::new();
        let mut entity = norm::normalize(sec.fields, m, "", &mut errs);
        canonicalize(sec, &mut entity);
        if errs.is_empty() {
            out.push(entity);
        } else {
            let at = format!("{}[{i}]", sec.name);
            for e in errs {
                issues.push(Issue {
                    severity: Severity::Error,
                    at: at.clone(),
                    message: e,
                });
            }
        }
    }
    out
}

/// Drop redundant spellings so equal entities compare equal.
fn canonicalize(sec: &Section, e: &mut Mapping) {
    if let Kind::Tags = sec.kind
        && e.get("ref").and_then(Value::as_str) == Some(&super::export::tag_base_ref(e))
    {
        e.remove("ref");
    }
}

// ── Files ──

fn is_yaml(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()), Some("yaml" | "yml"))
}

/// Whether a path names the single-file form.
pub fn is_single_file_path(p: &Path) -> bool {
    p.is_file() || (!p.exists() && is_yaml(p))
}

fn yaml_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            yaml_files(&p, out)?;
        } else if is_yaml(&p) {
            out.push(p);
        }
    }
    Ok(())
}

fn read_yaml(p: &Path) -> Result<Value> {
    let src = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
    serde_yaml_neo::from_str(&src).with_context(|| format!("{} is not valid YAML", p.display()))
}

/// Read a project from a file (single-file form) or a directory.
pub fn read_project(path: &Path) -> Result<ProjectDoc> {
    if path.is_file() {
        let src =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        return parse_project_yaml(&src).with_context(|| path.display().to_string());
    }
    if !path.is_dir() {
        bail!("{} does not exist", path.display());
    }
    let manifest = path.join(MANIFEST_FILE);
    if !manifest.is_file() {
        bail!(
            "{} has no {MANIFEST_FILE}; it is not a voltrus project directory",
            path.display()
        );
    }
    let Value::Mapping(mut top) = read_yaml(&manifest)? else {
        bail!("{} must be a YAML mapping", manifest.display());
    };

    let mut warnings = Vec::new();
    let mut known_files: HashSet<PathBuf> = HashSet::new();
    known_files.insert(manifest.clone());
    for sec in spec::SECTIONS {
        let mut items: Vec<Value> = Vec::new();
        let mut single: Option<Value> = None;
        let mut files: Vec<PathBuf> = Vec::new();
        match sec.layout {
            Layout::Dir => {
                let dir = path.join(sec.path);
                if dir.is_dir() {
                    yaml_files(&dir, &mut files)?;
                }
            }
            Layout::File | Layout::Single => {
                for ext in ["yaml", "yml"] {
                    let f = path.join(format!("{}.{ext}", sec.path));
                    if f.is_file() {
                        files.push(f);
                    }
                }
            }
        }
        for f in files {
            known_files.insert(f.clone());
            match read_yaml(&f)? {
                Value::Sequence(s) => items.extend(s),
                Value::Null => {}
                v @ Value::Mapping(_) if matches!(sec.layout, Layout::Single) => single = Some(v),
                v @ Value::Mapping(_) => items.push(v),
                _ => bail!("{} must hold a mapping or a list", f.display()),
            }
        }
        let name = Value::String(sec.name.to_string());
        let combined = match sec.layout {
            Layout::Single => single,
            _ if items.is_empty() => None,
            _ => Some(Value::Sequence(items)),
        };
        let Some(combined) = combined else { continue };
        // project.yaml may carry a section inline as well.
        match (top.get_mut(&name), combined) {
            (Some(Value::Sequence(existing)), Value::Sequence(more)) => existing.extend(more),
            (Some(slot), more) => *slot = more,
            (None, more) => {
                top.insert(name, more);
            }
        }
    }
    let mut all = Vec::new();
    yaml_files(path, &mut all)?;
    for f in all {
        if !known_files.contains(&f) {
            warnings.push(format!(
                "ignored {}: not part of the project layout",
                f.display()
            ));
        }
    }
    let mut doc = doc_from_mapping(top).with_context(|| path.display().to_string())?;
    doc.warnings = warnings;
    Ok(doc)
}

/// Write the single-file form.
pub fn write_project_file(doc: &ProjectDoc, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, to_yaml(doc)).with_context(|| format!("writing {}", path.display()))
}

fn stem(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "_".into()
    } else {
        cleaned
    }
}

fn remove_yaml_files(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_dir() {
            remove_yaml_files(&p)?;
            let _ = std::fs::remove_dir(&p);
        } else if is_yaml(&p) {
            std::fs::remove_file(&p)?;
        }
    }
    Ok(())
}

/// Write the directory form. Files this layout manages are replaced; other
/// files in the directory are left alone.
pub fn write_project_dir(doc: &ProjectDoc, root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let _ = std::fs::remove_file(root.join(MANIFEST_FILE));
    for sec in spec::SECTIONS {
        match sec.layout {
            Layout::Dir => remove_yaml_files(&root.join(sec.path))?,
            _ => {
                for ext in ["yaml", "yml"] {
                    let _ = std::fs::remove_file(root.join(format!("{}.{ext}", sec.path)));
                }
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(root.join("devices")) {
        drop(entries);
        let _ = std::fs::remove_dir(root.join("devices"));
    }

    let write = |p: PathBuf, v: &Value| -> Result<()> {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, serde_yaml_neo::to_string(v)?)
            .with_context(|| format!("writing {}", p.display()))
    };

    write(root.join(MANIFEST_FILE), &Value::Mapping(doc.manifest()))?;
    for sec in spec::SECTIONS {
        let items = doc.entities(sec.name);
        if items.is_empty() {
            continue;
        }
        match sec.layout {
            Layout::Single => write(
                root.join(format!("{}.yaml", sec.path)),
                &Value::Mapping(items[0].clone()),
            )?,
            Layout::File => write(
                root.join(format!("{}.yaml", sec.path)),
                &Value::Sequence(items.iter().cloned().map(Value::Mapping).collect()),
            )?,
            Layout::Dir => {
                // Tags are grouped per device; everything else is one file per entity.
                let mut files: Vec<(String, Vec<Value>)> = Vec::new();
                let mut used: HashSet<String> = HashSet::new();
                for e in items {
                    let group = if matches!(sec.kind, Kind::Tags) {
                        e.get("device").map(display).unwrap_or_default()
                    } else {
                        entity_key(sec, e)
                    };
                    let grouped = matches!(sec.kind, Kind::Tags);
                    if grouped && let Some((_, v)) = files.iter_mut().find(|(g, _)| *g == group) {
                        v.push(Value::Mapping(e.clone()));
                        continue;
                    }
                    files.push((group, vec![Value::Mapping(e.clone())]));
                }
                for (group, values) in files {
                    let base = stem(&group);
                    let mut name = base.clone();
                    let mut n = 1;
                    while !used.insert(name.to_ascii_lowercase()) {
                        n += 1;
                        name = format!("{base}~{n:02}");
                    }
                    let v = if matches!(sec.kind, Kind::Tags) {
                        Value::Sequence(values)
                    } else {
                        values.into_iter().next().unwrap_or(Value::Null)
                    };
                    write(root.join(sec.path).join(format!("{name}.yaml")), &v)?;
                }
            }
        }
    }
    Ok(())
}

/// Write to `target`: a `.yaml`/`.yml` path (or existing file) gets the
/// single-file form, anything else the directory form.
pub fn write_project(doc: &ProjectDoc, target: &Path) -> Result<()> {
    if is_single_file_path(target) {
        write_project_file(doc, target)
    } else {
        write_project_dir(doc, target)
    }
}
