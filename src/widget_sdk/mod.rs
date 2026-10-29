pub mod widget_api;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataBinding {
    pub key: String,
    #[serde(default = "default_binding_type")]
    pub r#type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub label: String,
}

fn default_binding_type() -> String {
    "number".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigField {
    pub key: String,
    pub r#type: String,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WidgetSize {
    #[serde(default = "default_min_width")]
    pub min_width: u32,
    #[serde(default = "default_min_height")]
    pub min_height: u32,
}

fn default_min_width() -> u32 {
    2
}
fn default_min_height() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub min_voltrus_version: Option<String>,
    #[serde(default)]
    pub data_bindings: Vec<DataBinding>,
    #[serde(default)]
    pub config_fields: Vec<ConfigField>,
    #[serde(default)]
    pub size: WidgetSize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WidgetInfo {
    pub manifest: WidgetManifest,
    pub installed: bool,
    pub has_js: bool,
    pub has_css: bool,
}

#[derive(Clone)]
pub struct WidgetRegistry {
    widgets_dir: PathBuf,
    widgets: Arc<RwLock<HashMap<String, WidgetInfo>>>,
}

impl WidgetRegistry {
    pub fn new(widgets_dir: PathBuf) -> Self {
        let reg = Self {
            widgets_dir,
            widgets: Arc::new(RwLock::new(HashMap::new())),
        };
        if let Ok(mut widgets) = reg.widgets.try_write() {
            *widgets = reg.scan_disk();
        }
        reg
    }

    fn scan_disk(&self) -> HashMap<String, WidgetInfo> {
        let mut result = HashMap::new();
        if !self.widgets_dir.exists() {
            return result;
        }

        if let Ok(entries) = fs::read_dir(&self.widgets_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let manifest_path = path.join("manifest.json");
                if let Ok(content) = fs::read_to_string(&manifest_path)
                    && let Ok(manifest) = serde_json::from_str::<WidgetManifest>(&content)
                {
                    let widget_id = manifest.id.clone();
                    let has_js = path.join("widget.js").exists();
                    let has_css = path.join("widget.css").exists();
                    result.insert(
                        widget_id,
                        WidgetInfo {
                            manifest,
                            installed: true,
                            has_js,
                            has_css,
                        },
                    );
                }
            }
        }

        tracing::info!(
            "Loaded {} custom widgets from {:?}",
            result.len(),
            self.widgets_dir
        );
        result
    }

    pub async fn list(&self) -> Vec<WidgetInfo> {
        self.widgets.read().await.values().cloned().collect()
    }

    pub async fn get(&self, id: &str) -> Option<WidgetInfo> {
        self.widgets.read().await.get(id).cloned()
    }

    pub fn asset_path(&self, widget_id: &str, filename: &str) -> Option<PathBuf> {
        let path = self.widgets_dir.join(widget_id).join(filename);
        if path.exists() { Some(path) } else { None }
    }

    pub async fn install(&self, widget_id: &str, zip_data: &[u8]) -> anyhow::Result<()> {
        let widget_dir = self.widgets_dir.join(widget_id);

        let reader = std::io::Cursor::new(zip_data);
        let mut archive = zip::ZipArchive::new(reader)?;
        // Rejects duplicate names, local/central header mismatches and
        // overlapping entries (zip cloaking) before anything touches the disk.
        archive.validate_header_consistency()?;

        fs::create_dir_all(&widget_dir)?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| {
                anyhow::anyhow!(
                    "cannot read widget archive entry #{i}: {e} \
                     (supported compression: stored, deflate, deflate64, bzip2, lzma, xz, ppmd; zstd is not supported)"
                )
            })?;
            let Some(rel_path) = file.enclosed_name() else {
                continue;
            };
            let name = rel_path.to_string_lossy();
            if name.ends_with(".exe") || name.ends_with(".sh") || name.ends_with(".bat") {
                continue;
            }

            let out_path = widget_dir.join(&rel_path);
            if file.is_dir() {
                fs::create_dir_all(&out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut out_file = fs::File::create(&out_path)?;
                std::io::copy(&mut file, &mut out_file)?;
            }
        }

        let manifest_path = widget_dir.join("manifest.json");
        let content = fs::read_to_string(&manifest_path)?;
        let manifest: WidgetManifest = serde_json::from_str(&content)?;

        let js_path = widget_dir.join("widget.js");
        if js_path.exists() {
            let js_content = fs::read_to_string(&js_path)?;
            anyhow::ensure!(!js_content.trim().is_empty(), "widget.js is empty");
        }

        let has_js = js_path.exists();
        let has_css = widget_dir.join("widget.css").exists();
        self.widgets.write().await.insert(
            manifest.id.clone(),
            WidgetInfo {
                manifest,
                installed: true,
                has_js,
                has_css,
            },
        );

        Ok(())
    }

    pub async fn uninstall(&self, widget_id: &str) -> anyhow::Result<()> {
        let widget_dir = self.widgets_dir.join(widget_id);
        if widget_dir.exists() {
            fs::remove_dir_all(&widget_dir)?;
        }
        self.widgets.write().await.remove(widget_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    const MANIFEST: &str = r#"{"id":"demo","name":"Demo"}"#;

    fn build_zip(entries: &[(&str, &[u8])], method: zip::CompressionMethod) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(method);
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn replace_nth(haystack: &mut [u8], needle: &[u8], replacement: &[u8], nth: usize) {
        assert_eq!(needle.len(), replacement.len());
        let pos = haystack
            .windows(needle.len())
            .enumerate()
            .filter(|(_, w)| *w == needle)
            .nth(nth)
            .map(|(i, _)| i)
            .expect("needle present");
        haystack[pos..pos + needle.len()].copy_from_slice(replacement);
    }

    #[tokio::test]
    async fn install_accepts_bzip2_and_xz_archives() {
        for method in [zip::CompressionMethod::Bzip2, zip::CompressionMethod::Xz] {
            let root = tempfile::tempdir().unwrap();
            let registry = WidgetRegistry::new(root.path().join("widgets"));
            let zip = build_zip(
                &[
                    ("manifest.json", MANIFEST.as_bytes()),
                    ("widget.js", b"console.log('hi');"),
                ],
                method,
            );
            registry.install("demo", &zip).await.unwrap();
            let js = fs::read_to_string(root.path().join("widgets/demo/widget.js")).unwrap();
            assert_eq!(js, "console.log('hi');");
        }
    }

    #[tokio::test]
    async fn install_rejects_zstd_archive_with_clear_error() {
        let root = tempfile::tempdir().unwrap();
        let registry = WidgetRegistry::new(root.path().join("widgets"));
        let mut zip = build_zip(
            &[("manifest.json", MANIFEST.as_bytes())],
            zip::CompressionMethod::Stored,
        );
        // Method id 93 (zstd): local header at +8, central directory header at +10.
        let local = zip.windows(4).position(|w| w == b"PK\x03\x04").unwrap();
        zip[local + 8..local + 10].copy_from_slice(&93u16.to_le_bytes());
        let central = zip.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        zip[central + 10..central + 12].copy_from_slice(&93u16.to_le_bytes());

        let err = registry.install("demo", &zip).await.unwrap_err().to_string();
        assert!(err.contains("zstd is not supported"), "unclear error: {err}");
    }

    #[tokio::test]
    async fn install_round_trips_archive() {
        let root = tempfile::tempdir().unwrap();
        let registry = WidgetRegistry::new(root.path().join("widgets"));
        let zip = build_zip(
            &[
                ("manifest.json", MANIFEST.as_bytes()),
                ("widget.js", b"console.log('hi');"),
                ("assets/style.css", b"body{}"),
            ],
            zip::CompressionMethod::Deflated,
        );

        registry.install("demo", &zip).await.unwrap();

        let dir = root.path().join("widgets/demo");
        assert_eq!(
            fs::read_to_string(dir.join("widget.js")).unwrap(),
            "console.log('hi');"
        );
        assert_eq!(
            fs::read_to_string(dir.join("assets/style.css")).unwrap(),
            "body{}"
        );
        let info = registry.get("demo").await.unwrap();
        assert!(info.has_js && !info.has_css);
    }

    #[tokio::test]
    async fn install_skips_path_traversal_and_executables() {
        let root = tempfile::tempdir().unwrap();
        let registry = WidgetRegistry::new(root.path().join("widgets"));
        let zip = build_zip(
            &[
                ("manifest.json", MANIFEST.as_bytes()),
                ("../escape.txt", b"nope"),
                ("/abs_escape.txt", b"nope"),
                ("a/../../escape2.txt", b"nope"),
                ("run.sh", b"#!/bin/sh"),
                ("tool.exe", b"MZ"),
            ],
            zip::CompressionMethod::Stored,
        );

        registry.install("demo", &zip).await.unwrap();

        let dir = root.path().join("widgets/demo");
        assert!(dir.join("manifest.json").exists());
        assert!(!dir.join("run.sh").exists());
        assert!(!dir.join("tool.exe").exists());
        assert!(!root.path().join("widgets/escape.txt").exists());
        assert!(!root.path().join("widgets/escape2.txt").exists());
        assert!(!root.path().join("escape.txt").exists());
        assert!(!root.path().join("abs_escape.txt").exists());
        assert!(!std::path::Path::new("/abs_escape.txt").exists());
    }

    #[tokio::test]
    async fn install_rejects_duplicate_entry_names() {
        let root = tempfile::tempdir().unwrap();
        let registry = WidgetRegistry::new(root.path().join("widgets"));
        let mut zip = build_zip(
            &[("a.txt", b"first"), ("b.txt", b"second")],
            zip::CompressionMethod::Stored,
        );
        // Rename `b.txt` to `a.txt` in both the local and central headers.
        replace_nth(&mut zip, b"b.txt", b"a.txt", 0);
        replace_nth(&mut zip, b"b.txt", b"a.txt", 0);

        assert!(registry.install("demo", &zip).await.is_err());
        assert!(!root.path().join("widgets/demo").exists());
    }

    #[tokio::test]
    async fn install_rejects_local_central_header_mismatch() {
        let root = tempfile::tempdir().unwrap();
        let registry = WidgetRegistry::new(root.path().join("widgets"));
        let mut zip = build_zip(
            &[
                ("manifest.json", MANIFEST.as_bytes()),
                ("widget.js", b"console.log('hi');"),
            ],
            zip::CompressionMethod::Stored,
        );
        // Change only the local header's name (first occurrence); the central
        // directory keeps `widget.js`, so the two parsers disagree.
        replace_nth(&mut zip, b"widget.js", b"widgex.js", 0);

        assert!(registry.install("demo", &zip).await.is_err());
        assert!(!root.path().join("widgets/demo").exists());
    }
}
