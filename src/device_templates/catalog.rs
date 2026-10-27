//! The public template catalog: an `index.json` listing template files by
//! sha256, hosted anywhere over HTTP(S) (by default a public git repository).

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::format::{CATALOG_FORMAT, DeviceTemplate, Protocol, compare_versions, validate_slug};

pub const DEFAULT_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/VoltrusID/voltrus-device-templates/main/index.json";
/// Settings key holding the catalog index URL; `device_templates.catalog_url` in the config file is written to it at startup.
pub const SETTING_CATALOG_URL: &str = "device_templates.catalog_url";

/// An index lists a whole catalog, a template file one device.
const MAX_INDEX_BYTES: usize = 4 * 1024 * 1024;
const MAX_TEMPLATE_BYTES: usize = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Why a catalog operation failed; the API maps `Fetch` to 502 and `Invalid` to 422.
#[derive(Debug)]
pub enum CatalogError {
    /// The catalog could not be reached or answered with an error.
    Fetch(String),
    /// The catalog answered but what it served is not acceptable.
    Invalid(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogError::Fetch(m) | CatalogError::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CatalogError {}

/// One template of a catalog index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CatalogEntry {
    pub template_id: String,
    pub manufacturer: String,
    pub model: String,
    pub protocol: String,
    pub version: String,
    #[serde(default)]
    pub verified: bool,
    /// File path relative to the index URL.
    pub path: String,
    /// Lowercase hex sha256 of the file's bytes.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CatalogIndex {
    pub format: String,
    pub templates: Vec<CatalogEntry>,
}

impl CatalogIndex {
    /// Check the format and every entry.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != CATALOG_FORMAT {
            return Err(format!(
                "catalog format must be '{CATALOG_FORMAT}', got '{}'",
                self.format
            ));
        }
        for e in &self.templates {
            validate_slug(&e.template_id)
                .map_err(|m| format!("catalog entry '{}': {m}", e.template_id))?;
            if Protocol::parse(&e.protocol).is_none() {
                return Err(format!(
                    "catalog entry '{}': protocol '{}' is not modbus or bacnet",
                    e.template_id, e.protocol
                ));
            }
            validate_relative_path(&e.path)
                .map_err(|m| format!("catalog entry '{}': {m}", e.template_id))?;
            if e.sha256.len() != 64 || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!(
                    "catalog entry '{}': sha256 is not 64 hex digits",
                    e.template_id
                ));
            }
        }
        Ok(())
    }

    /// The entry a request names: that version, or the newest when none is given.
    pub fn find(&self, template_id: &str, version: Option<&str>) -> Option<&CatalogEntry> {
        self.templates
            .iter()
            .filter(|e| e.template_id == template_id)
            .filter(|e| version.is_none_or(|v| e.version == v))
            .max_by(|a, b| compare_versions(&a.version, &b.version))
    }
}

/// A path under the index: no scheme, no leading `/`, no `..`.
fn validate_relative_path(p: &str) -> Result<(), String> {
    let bad = p.is_empty()
        || p.starts_with('/')
        || p.starts_with('\\')
        || p.contains("://")
        || p.contains('\\')
        || p.split('/').any(|seg| seg == ".." || seg.is_empty());
    if bad {
        return Err(format!("path '{p}' is not a relative file path"));
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A catalog URL must be http(s).
pub fn validate_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("'{url}' is not a URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        s => Err(format!("catalog URL must be http or https, not '{s}'")),
    }
}

/// GET `url` with timeouts and a size cap.
async fn get_bytes(url: &str, cap: usize) -> Result<Vec<u8>, CatalogError> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(concat!("voltrus-scada/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| CatalogError::Fetch(format!("HTTP client: {e}")))?;
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| CatalogError::Fetch(format!("fetching {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CatalogError::Fetch(format!(
            "fetching {url}: HTTP {}",
            resp.status()
        )));
    }
    if resp.content_length().is_some_and(|n| n > cap as u64) {
        return Err(CatalogError::Invalid(format!(
            "{url} is larger than {} KiB",
            cap / 1024
        )));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| CatalogError::Fetch(format!("reading {url}: {e}")))?
    {
        if body.len() + chunk.len() > cap {
            return Err(CatalogError::Invalid(format!(
                "{url} is larger than {} KiB",
                cap / 1024
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Download and check a catalog index.
pub async fn fetch_index(url: &str) -> Result<CatalogIndex, CatalogError> {
    validate_url(url).map_err(CatalogError::Invalid)?;
    let bytes = get_bytes(url, MAX_INDEX_BYTES).await?;
    let index: CatalogIndex = serde_json::from_slice(&bytes)
        .map_err(|e| CatalogError::Invalid(format!("{url} is not a catalog index: {e}")))?;
    index.validate().map_err(CatalogError::Invalid)?;
    Ok(index)
}

/// Download the file of `entry`, check its sha256 against the index and
/// that it is the template the entry names.
pub async fn fetch_template(
    index_url: &str,
    entry: &CatalogEntry,
) -> Result<DeviceTemplate, CatalogError> {
    validate_relative_path(&entry.path).map_err(CatalogError::Invalid)?;
    let base = reqwest::Url::parse(index_url)
        .map_err(|e| CatalogError::Invalid(format!("'{index_url}' is not a URL: {e}")))?;
    let file_url = base
        .join(&entry.path)
        .map_err(|e| CatalogError::Invalid(format!("path '{}': {e}", entry.path)))?;
    let bytes = get_bytes(file_url.as_str(), MAX_TEMPLATE_BYTES).await?;

    let actual = sha256_hex(&bytes);
    if !actual.eq_ignore_ascii_case(&entry.sha256) {
        return Err(CatalogError::Invalid(format!(
            "sha256 mismatch for {}: the index says {}, the file is {actual}",
            entry.path, entry.sha256
        )));
    }
    let mut template: DeviceTemplate = serde_json::from_slice(&bytes).map_err(|e| {
        CatalogError::Invalid(format!("{} is not a device template: {e}", entry.path))
    })?;
    template
        .normalize_and_validate()
        .map_err(|errs| CatalogError::Invalid(format!("{}: {}", entry.path, errs.join("; "))))?;
    if template.template_id != entry.template_id
        || template.version != entry.version
        || template.protocol.as_str() != entry.protocol
    {
        return Err(CatalogError::Invalid(format!(
            "{} holds {} {} ({}), not the {} {} ({}) the index lists",
            entry.path,
            template.template_id,
            template.version,
            template.protocol.as_str(),
            entry.template_id,
            entry.version,
            entry.protocol
        )));
    }
    Ok(template)
}

/// Build an index from the `*.json` template files under `dir` (recursively,
/// except `index.json` at the top). Entries are sorted so a rebuild of
/// unchanged files gives the same bytes.
pub fn build_index(dir: &Path) -> anyhow::Result<CatalogIndex> {
    use anyhow::Context;

    fn walk(
        dir: &Path,
        rel: &str,
        out: &mut Vec<(String, std::path::PathBuf)>,
    ) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let rel_name = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if entry.file_type()?.is_dir() {
                walk(&path, &rel_name, out)?;
            } else if name.ends_with(".json") && rel_name != "index.json" {
                out.push((rel_name, path));
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    files.sort();

    let mut templates: Vec<CatalogEntry> = Vec::new();
    for (rel, path) in files {
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut t: DeviceTemplate = serde_json::from_slice(&bytes)
            .with_context(|| format!("{rel} is not a device template"))?;
        t.normalize_and_validate()
            .map_err(|errs| anyhow::anyhow!("{rel}: {}", errs.join("; ")))?;
        if templates
            .iter()
            .any(|e| e.template_id == t.template_id && e.version == t.version)
        {
            anyhow::bail!("{rel}: {} {} is listed twice", t.template_id, t.version);
        }
        templates.push(CatalogEntry {
            template_id: t.template_id,
            manufacturer: t.manufacturer,
            model: t.model,
            protocol: t.protocol.as_str().to_string(),
            version: t.version,
            verified: t.verified,
            path: rel,
            sha256: sha256_hex(&bytes),
        });
    }
    templates.sort_by(|a, b| {
        a.template_id
            .cmp(&b.template_id)
            .then_with(|| compare_versions(&a.version, &b.version))
    });
    Ok(CatalogIndex {
        format: CATALOG_FORMAT.to_string(),
        templates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, sha: &str) -> CatalogEntry {
        CatalogEntry {
            template_id: "acme-m1".into(),
            manufacturer: "Acme".into(),
            model: "M1".into(),
            protocol: "modbus".into(),
            version: "1.0.0".into(),
            verified: false,
            path: path.into(),
            sha256: sha.into(),
        }
    }

    #[test]
    fn rejects_bad_paths_and_hashes() {
        let sha = "a".repeat(64);
        for p in [
            "",
            "/x.json",
            "../x.json",
            "a//b.json",
            "https://evil/x.json",
            "a\\b.json",
        ] {
            let idx = CatalogIndex {
                format: CATALOG_FORMAT.into(),
                templates: vec![entry(p, &sha)],
            };
            assert!(idx.validate().is_err(), "{p}");
        }
        let idx = CatalogIndex {
            format: CATALOG_FORMAT.into(),
            templates: vec![entry("t/x.json", "xyz")],
        };
        assert!(idx.validate().is_err());
        let idx = CatalogIndex {
            format: "other/1".into(),
            templates: vec![],
        };
        assert!(idx.validate().is_err());
        let ok = CatalogIndex {
            format: CATALOG_FORMAT.into(),
            templates: vec![entry("t/x.json", &sha)],
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn find_takes_the_newest_without_a_version() {
        let sha = "b".repeat(64);
        let mut a = entry("a.json", &sha);
        let mut b = entry("b.json", &sha);
        b.version = "1.10.0".into();
        a.version = "1.9.0".into();
        let idx = CatalogIndex {
            format: CATALOG_FORMAT.into(),
            templates: vec![a, b],
        };
        assert_eq!(idx.find("acme-m1", None).unwrap().path, "b.json");
        assert_eq!(idx.find("acme-m1", Some("1.9.0")).unwrap().path, "a.json");
        assert!(idx.find("acme-m1", Some("2.0.0")).is_none());
    }

    #[test]
    fn url_scheme_is_checked() {
        assert!(validate_url("https://example.com/index.json").is_ok());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("not a url").is_err());
    }

    #[test]
    fn index_is_built_from_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = super::super::builtin::builtins()[0].clone();
        t.source = String::new();
        let body = serde_json::to_vec_pretty(&t).unwrap();
        std::fs::write(dir.path().join("a.json"), &body).unwrap();
        std::fs::write(dir.path().join("index.json"), b"{}").unwrap();
        let idx = build_index(dir.path()).unwrap();
        assert_eq!(idx.templates.len(), 1);
        assert_eq!(idx.templates[0].sha256, sha256_hex(&body));
        assert_eq!(idx.templates[0].path, "a.json");
        idx.validate().unwrap();
        // The same template twice is refused.
        std::fs::write(dir.path().join("b.json"), &body).unwrap();
        assert!(build_index(dir.path()).is_err());
    }
}
