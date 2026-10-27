//! Shared device template library.
//!
//! A template is a Modbus or BACnet device description (defaults plus registers
//! or objects) with metadata about the hardware it fits; see
//! `docs/device-templates.md`. Templates come from three places: built-ins
//! generated from the simulator profiles, files saved or imported locally, and
//! a public catalog.

pub mod builtin;
pub mod catalog;
pub mod format;

use std::path::Path;

pub use format::*;

/// `voltrus template builtins <dir>`: write each built-in template as `<template_id>.json`.
pub fn write_builtins(dir: &Path) -> anyhow::Result<usize> {
    use anyhow::Context;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let all = builtin::builtins();
    for t in all {
        // A catalog file does not name its own source: whoever imports it sets that.
        let mut t = t.clone();
        t.source.clear();
        let mut body = serde_json::to_string_pretty(&t)?;
        body.push('\n');
        let path = dir.join(format!("{}.json", t.template_id));
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(all.len())
}

/// `voltrus template catalog <dir>`: write `<dir>/index.json` from the template files in `dir`.
pub fn write_catalog_index(dir: &Path) -> anyhow::Result<usize> {
    use anyhow::Context;
    let index = catalog::build_index(dir)?;
    let mut body = serde_json::to_string_pretty(&index)?;
    body.push('\n');
    let path = dir.join("index.json");
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(index.templates.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `docs/device-templates-catalog-seed/` is what `voltrus template builtins` and
    /// `voltrus template catalog` write; a change to a built-in must regenerate it.
    #[test]
    fn catalog_seed_matches_the_builtins() {
        let seed = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/device-templates-catalog-seed");
        let dir = tempfile::tempdir().unwrap();
        write_builtins(dir.path()).unwrap();
        write_catalog_index(dir.path()).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names.len(), builtin::builtins().len() + 1);
        for name in names {
            let fresh = std::fs::read(dir.path().join(&name)).unwrap();
            let committed = std::fs::read(seed.join(&name))
                .unwrap_or_else(|_| panic!("{} missing from the seed", name.to_string_lossy()));
            assert!(
                fresh == committed,
                "{} is out of date; run `voltrus template builtins` and `voltrus template catalog` on the seed directory",
                name.to_string_lossy()
            );
        }
        let index = catalog::build_index(&seed).unwrap();
        index.validate().unwrap();
    }

    /// The template examples in `docs/device-templates.md` are valid templates.
    #[test]
    fn documented_examples_validate() {
        let doc = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/device-templates.md"),
        )
        .unwrap();
        let mut checked = 0;
        for block in doc.split("```json\n").skip(1) {
            let json = block.split("```").next().unwrap();
            let value: serde_json::Value = serde_json::from_str(json).unwrap();
            if value["format"] == TEMPLATE_FORMAT {
                let mut t: DeviceTemplate = serde_json::from_value(value).unwrap();
                t.normalize_and_validate()
                    .unwrap_or_else(|e| panic!("{}: {e:?}", t.template_id));
                checked += 1;
            } else if value["format"] == CATALOG_FORMAT {
                let index: catalog::CatalogIndex = serde_json::from_value(value).unwrap();
                index.validate().unwrap();
                checked += 1;
            }
        }
        assert_eq!(checked, 3);
    }
}
