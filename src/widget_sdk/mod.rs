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
        fs::create_dir_all(&widget_dir)?;

        let reader = std::io::Cursor::new(zip_data);
        let mut archive = zip::ZipArchive::new(reader)?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_string();

            if name.contains("..") {
                continue;
            }
            if name.ends_with(".exe") || name.ends_with(".sh") || name.ends_with(".bat") {
                continue;
            }

            let out_path = widget_dir.join(&name);
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
