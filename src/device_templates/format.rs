//! The `voltrus-device-template/1` file format: types, validation and search.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// The `format` value every template file carries. A change that older servers
/// cannot read bumps the number.
pub const TEMPLATE_FORMAT: &str = "voltrus-device-template/1";
/// The `format` value of a catalog `index.json`.
pub const CATALOG_FORMAT: &str = "voltrus-template-catalog/1";
pub const DEFAULT_LICENSE: &str = "CC-BY-4.0";

pub const SOURCE_BUILTIN: &str = "builtin";
pub const SOURCE_LOCAL: &str = "local";
pub const SOURCE_CATALOG_PREFIX: &str = "catalog:";

/// Slugs that would shadow a fixed route under `/api/v1/device-templates/`.
const RESERVED_IDS: [&str; 2] = ["catalog", "from-device"];

pub const MODBUS_TYPES: [&str; 5] = ["int16", "uint16", "int32", "float32", "float64"];
pub const MODBUS_AREAS: [&str; 4] = ["holding", "input", "coil", "discrete"];
pub const MODBUS_ENDIANNESS: [&str; 3] = ["big", "little", "swapped"];
pub const BACNET_TYPES: [&str; 9] = [
    "analogInput",
    "analogOutput",
    "analogValue",
    "binaryInput",
    "binaryOutput",
    "binaryValue",
    "multiStateInput",
    "multiStateOutput",
    "multiStateValue",
];

pub const DEFAULT_MODBUS_PORT: u16 = 502;
pub const DEFAULT_BACNET_PORT: u16 = 47808;
pub const DEFAULT_INTERVAL_MS: u64 = 1000;
pub const DEFAULT_COV_LIFETIME: u32 = 3600;
pub const MAX_INTERVAL_MS: u64 = 3_600_000;
pub const MAX_BACNET_INSTANCE: u32 = 4_194_303;

const MAX_ITEMS: usize = 5000;
const MAX_TEXT: usize = 2000;
const MAX_QUIRKS: usize = 50;
const MAX_QUIRK_LEN: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Modbus,
    Bacnet,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Modbus => "modbus",
            Protocol::Bacnet => "bacnet",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "modbus" => Some(Protocol::Modbus),
            "bacnet" => Some(Protocol::Bacnet),
            _ => None,
        }
    }
}

fn default_license() -> String {
    DEFAULT_LICENSE.to_string()
}

fn default_register_type() -> String {
    "float32".to_string()
}

fn default_register_area() -> String {
    "holding".to_string()
}

fn default_scale() -> f64 {
    1.0
}

fn default_property() -> String {
    "presentValue".to_string()
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// A Modbus register of a template. `offset` is the 0-based offset within
/// `register_area`, as on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TemplateRegister {
    pub name: String,
    pub offset: u16,
    #[serde(default = "default_register_type")]
    pub register_type: String,
    #[serde(default = "default_register_area")]
    pub register_area: String,
    #[serde(default = "default_scale")]
    pub scale: f64,
    /// Engineering unit, informational.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The register accepts writes on the device, informational.
    #[serde(default, skip_serializing_if = "is_false")]
    pub writable: bool,
}

impl TemplateRegister {
    /// Number of 16-bit registers the value spans.
    pub fn width(&self) -> u32 {
        match self.register_type.as_str() {
            "int16" | "uint16" => 1,
            "float64" => 4,
            _ => 2,
        }
    }
}

/// A BACnet object of a template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TemplateObject {
    pub object_type: String,
    pub object_instance: u32,
    #[serde(default = "default_property")]
    pub property: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cov_increment: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub writable: bool,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// A device template: metadata plus the device defaults and its registers
/// (Modbus) or objects (BACnet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DeviceTemplate {
    pub format: String,
    pub template_id: String,
    pub manufacturer: String,
    pub model: String,
    pub protocol: Protocol,
    #[serde(default)]
    pub description: String,
    pub version: String,
    /// Checked against a real deployment, see `verified_notes`.
    #[serde(default)]
    pub verified: bool,
    /// Field-verified deployment notes: firmware, site, what was checked.
    #[serde(default)]
    pub verified_notes: String,
    /// Word order, offset base, scaling oddities.
    #[serde(default)]
    pub quirks: Vec<String>,
    /// `builtin`, `local` or `catalog:<url>`. Set by the server on import, not read from the file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default = "default_license")]
    pub license: String,

    // Device defaults, both protocols.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    // Modbus only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_id: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endianness: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registers: Vec<TemplateRegister>,
    // BACnet only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cov_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cov_lifetime: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<TemplateObject>,
}

impl DeviceTemplate {
    pub fn is_builtin(&self) -> bool {
        self.source == SOURCE_BUILTIN
    }

    /// Registers (Modbus) or objects (BACnet).
    pub fn item_count(&self) -> usize {
        match self.protocol {
            Protocol::Modbus => self.registers.len(),
            Protocol::Bacnet => self.objects.len(),
        }
    }

    /// Trim text, fill protocol defaults and check the template. Returns every
    /// problem found, not just the first.
    pub fn normalize_and_validate(&mut self) -> Result<(), Vec<String>> {
        let mut errs: Vec<String> = Vec::new();

        self.template_id = self.template_id.trim().to_string();
        self.manufacturer = self.manufacturer.trim().to_string();
        self.model = self.model.trim().to_string();
        self.description = self.description.trim().to_string();
        self.version = self.version.trim().to_string();
        self.verified_notes = self.verified_notes.trim().to_string();
        self.license = self.license.trim().to_string();
        if self.license.is_empty() {
            self.license = default_license();
        }
        self.quirks = self
            .quirks
            .iter()
            .map(|q| q.trim().to_string())
            .filter(|q| !q.is_empty())
            .collect();

        if self.format != TEMPLATE_FORMAT {
            errs.push(format!(
                "format must be '{TEMPLATE_FORMAT}', got '{}'",
                self.format
            ));
        }
        if let Err(e) = validate_slug(&self.template_id) {
            errs.push(format!("template_id: {e}"));
        }
        if self.manufacturer.is_empty() || self.manufacturer.chars().count() > 120 {
            errs.push("manufacturer is required (at most 120 characters)".into());
        }
        if self.model.is_empty() || self.model.chars().count() > 120 {
            errs.push("model is required (at most 120 characters)".into());
        }
        if self.description.chars().count() > MAX_TEXT {
            errs.push(format!("description is longer than {MAX_TEXT} characters"));
        }
        if self.verified_notes.chars().count() > MAX_TEXT {
            errs.push(format!(
                "verified_notes is longer than {MAX_TEXT} characters"
            ));
        }
        if self.license.chars().count() > 64 {
            errs.push("license is longer than 64 characters".into());
        }
        if SemVer::parse(&self.version).is_none() {
            errs.push(format!(
                "version '{}' is not a semantic version (MAJOR.MINOR.PATCH)",
                self.version
            ));
        }
        if self.quirks.len() > MAX_QUIRKS {
            errs.push(format!("more than {MAX_QUIRKS} quirks"));
        }
        if self
            .quirks
            .iter()
            .any(|q| q.chars().count() > MAX_QUIRK_LEN)
        {
            errs.push(format!("a quirk is longer than {MAX_QUIRK_LEN} characters"));
        }

        match self.protocol {
            Protocol::Modbus => self.validate_modbus(&mut errs),
            Protocol::Bacnet => self.validate_bacnet(&mut errs),
        }

        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    fn validate_modbus(&mut self, errs: &mut Vec<String>) {
        if !self.objects.is_empty() {
            errs.push("objects do not apply to a modbus template".into());
        }
        if self.cov_enabled.is_some() || self.cov_lifetime.is_some() {
            errs.push("cov_enabled and cov_lifetime do not apply to a modbus template".into());
        }
        let port = *self.port.get_or_insert(DEFAULT_MODBUS_PORT);
        if port == 0 {
            errs.push("port must be 1-65535".into());
        }
        self.unit_id.get_or_insert(1);
        let endianness = self.endianness.get_or_insert_with(|| "big".to_string());
        if !MODBUS_ENDIANNESS.contains(&endianness.as_str()) {
            errs.push(format!(
                "endianness '{endianness}' is not one of {}",
                MODBUS_ENDIANNESS.join(", ")
            ));
        }
        check_interval(self.interval_ms.get_or_insert(DEFAULT_INTERVAL_MS), errs);

        if self.registers.is_empty() {
            errs.push("a modbus template needs at least one register".into());
        }
        if self.registers.len() > MAX_ITEMS {
            errs.push(format!("more than {MAX_ITEMS} registers"));
            return;
        }
        let mut seen = std::collections::HashSet::new();
        for (i, r) in self.registers.iter_mut().enumerate() {
            r.name = r.name.trim().to_string();
            r.unit = r.unit.trim().to_string();
            r.description = r.description.trim().to_string();
            let label = if r.name.is_empty() {
                format!("register {}", i + 1)
            } else {
                format!("register '{}'", r.name)
            };
            if r.name.is_empty() || r.name.chars().count() > 128 {
                errs.push(format!(
                    "{label}: name is required (at most 128 characters)"
                ));
            } else if !seen.insert(r.name.clone()) {
                errs.push(format!("{label}: name is used twice"));
            }
            if !MODBUS_TYPES.contains(&r.register_type.as_str()) {
                errs.push(format!(
                    "{label}: register_type '{}' is not one of {}",
                    r.register_type,
                    MODBUS_TYPES.join(", ")
                ));
            }
            if !MODBUS_AREAS.contains(&r.register_area.as_str()) {
                errs.push(format!(
                    "{label}: register_area '{}' is not one of {}",
                    r.register_area,
                    MODBUS_AREAS.join(", ")
                ));
            }
            if !r.scale.is_finite() {
                errs.push(format!("{label}: scale is not a finite number"));
            }
            if r.offset as u32 + r.width() > 65536 {
                errs.push(format!(
                    "{label}: offset {} and its {} register(s) run past 65535",
                    r.offset,
                    r.width()
                ));
            }
        }
    }

    fn validate_bacnet(&mut self, errs: &mut Vec<String>) {
        if !self.registers.is_empty() {
            errs.push("registers do not apply to a bacnet template".into());
        }
        if self.unit_id.is_some() || self.endianness.is_some() {
            errs.push("unit_id and endianness do not apply to a bacnet template".into());
        }
        let port = *self.port.get_or_insert(DEFAULT_BACNET_PORT);
        if port == 0 {
            errs.push("port must be 1-65535".into());
        }
        check_interval(self.interval_ms.get_or_insert(DEFAULT_INTERVAL_MS), errs);
        self.cov_enabled.get_or_insert(false);
        self.cov_lifetime.get_or_insert(DEFAULT_COV_LIFETIME);

        if self.objects.is_empty() {
            errs.push("a bacnet template needs at least one object".into());
        }
        if self.objects.len() > MAX_ITEMS {
            errs.push(format!("more than {MAX_ITEMS} objects"));
            return;
        }
        let mut seen = std::collections::HashSet::new();
        for (i, o) in self.objects.iter_mut().enumerate() {
            o.name = o.name.trim().to_string();
            o.display_name = o.display_name.trim().to_string();
            o.unit = o.unit.trim().to_string();
            o.description = o.description.trim().to_string();
            o.property = o.property.trim().to_string();
            if o.property.is_empty() {
                o.property = default_property();
            }
            let label = if o.name.is_empty() {
                format!("object {}", i + 1)
            } else {
                format!("object '{}'", o.name)
            };
            if o.name.is_empty() || o.name.chars().count() > 128 {
                errs.push(format!(
                    "{label}: name is required (at most 128 characters)"
                ));
            }
            if !BACNET_TYPES.contains(&o.object_type.as_str()) {
                errs.push(format!(
                    "{label}: object_type '{}' is not one of {}",
                    o.object_type,
                    BACNET_TYPES.join(", ")
                ));
            }
            if o.object_instance > MAX_BACNET_INSTANCE {
                errs.push(format!(
                    "{label}: object_instance {} is not 0-{MAX_BACNET_INSTANCE}",
                    o.object_instance
                ));
            }
            if !o.cov_increment.is_finite() || o.cov_increment < 0.0 {
                errs.push(format!("{label}: cov_increment must be 0 or more"));
            }
            if !seen.insert((o.object_type.clone(), o.object_instance, o.property.clone())) {
                errs.push(format!(
                    "{label}: {} {} {} is listed twice",
                    o.object_type, o.object_instance, o.property
                ));
            }
        }
    }
}

fn check_interval(interval: &mut u64, errs: &mut Vec<String>) {
    if *interval == 0 || *interval > MAX_INTERVAL_MS {
        errs.push(format!("interval_ms must be 1-{MAX_INTERVAL_MS}"));
    }
}

/// A lowercase slug: `a-z`, `0-9` and single `-` between them, 1-64 characters.
pub fn validate_slug(s: &str) -> Result<(), String> {
    let ok = !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(format!(
            "'{s}' is not a slug (lowercase letters, digits and single '-', at most 64 characters)"
        ));
    }
    if RESERVED_IDS.contains(&s) {
        return Err(format!("'{s}' is reserved"));
    }
    Ok(())
}

/// Lowercase, every run of other characters becomes one `-`.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            dash = true;
        }
    }
    out.truncate(64);
    out.trim_end_matches('-').to_string()
}

/// A semantic version, `MAJOR.MINOR.PATCH` with an optional `-prerelease`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

impl SemVer {
    pub fn parse(s: &str) -> Option<Self> {
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        if let Some(p) = pre
            && (p.is_empty()
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
        {
            return None;
        }
        let mut parts = core.split('.');
        let mut num = || -> Option<u64> {
            let p = parts.next()?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            if p.len() > 1 && p.starts_with('0') {
                return None;
            }
            p.parse().ok()
        };
        let (major, minor, patch) = (num()?, num()?, num()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
            pre: pre.map(str::to_string),
        })
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compare two version strings; one that does not parse sorts below any that does.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    SemVer::parse(a).cmp(&SemVer::parse(b))
}

/// Which source wins a tie between equal versions: your own file over a
/// catalog download over the built-in.
fn source_rank(source: &str) -> u8 {
    if source == SOURCE_LOCAL {
        3
    } else if source.starts_with(SOURCE_CATALOG_PREFIX) {
        2
    } else {
        1
    }
}

/// The template a request names: the newest version, own file before catalog
/// before built-in, narrowed by `version` and `source` when given.
pub fn resolve<'a>(
    all: &'a [DeviceTemplate],
    template_id: &str,
    version: Option<&str>,
    source: Option<&str>,
) -> Option<&'a DeviceTemplate> {
    all.iter()
        .filter(|t| t.template_id == template_id)
        .filter(|t| version.is_none_or(|v| t.version == v))
        .filter(|t| source.is_none_or(|s| t.source == s))
        .max_by(|a, b| {
            compare_versions(&a.version, &b.version)
                .then_with(|| source_rank(&a.source).cmp(&source_rank(&b.source)))
        })
}

/// What `GET /api/v1/device-templates` filters on; every field is optional.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct TemplateFilter {
    /// Case-insensitive substring of the id, manufacturer, model or description.
    pub q: Option<String>,
    /// Case-insensitive match on the manufacturer.
    pub manufacturer: Option<String>,
    pub protocol: Option<String>,
    /// `builtin`, `local` or `catalog` (any catalog), or a full source string.
    pub source: Option<String>,
}

impl TemplateFilter {
    pub fn matches(&self, t: &DeviceTemplate) -> bool {
        if let Some(q) = self.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            let q = q.to_lowercase();
            let hit = [&t.template_id, &t.manufacturer, &t.model, &t.description]
                .iter()
                .any(|f| f.to_lowercase().contains(&q));
            if !hit {
                return false;
            }
        }
        if let Some(m) = self
            .manufacturer
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            && !t.manufacturer.eq_ignore_ascii_case(m)
        {
            return false;
        }
        if let Some(p) = self
            .protocol
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            && !t.protocol.as_str().eq_ignore_ascii_case(p)
        {
            return false;
        }
        if let Some(s) = self
            .source
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let hit = if s == "catalog" {
                t.source.starts_with(SOURCE_CATALOG_PREFIX)
            } else {
                t.source == s
            };
            if !hit {
                return false;
            }
        }
        true
    }
}

/// One row of the template list.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct TemplateSummary {
    pub template_id: String,
    pub manufacturer: String,
    pub model: String,
    pub protocol: Protocol,
    pub description: String,
    pub version: String,
    pub verified: bool,
    pub source: String,
    pub license: String,
    /// Registers (Modbus) or objects (BACnet).
    pub item_count: usize,
    /// Built-in templates cannot be deleted.
    pub read_only: bool,
}

impl From<&DeviceTemplate> for TemplateSummary {
    fn from(t: &DeviceTemplate) -> Self {
        Self {
            template_id: t.template_id.clone(),
            manufacturer: t.manufacturer.clone(),
            model: t.model.clone(),
            protocol: t.protocol,
            description: t.description.clone(),
            version: t.version.clone(),
            verified: t.verified,
            source: t.source.clone(),
            license: t.license.clone(),
            item_count: t.item_count(),
            read_only: t.is_builtin(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modbus() -> DeviceTemplate {
        serde_json::from_value(serde_json::json!({
            "format": TEMPLATE_FORMAT,
            "template_id": "acme-meter",
            "manufacturer": "Acme",
            "model": "M1",
            "protocol": "modbus",
            "version": "1.0.0",
            "registers": [{"name": "v", "offset": 0}]
        }))
        .unwrap()
    }

    #[test]
    fn defaults_are_filled() {
        let mut t = modbus();
        t.normalize_and_validate().unwrap();
        assert_eq!(t.port, Some(502));
        assert_eq!(t.unit_id, Some(1));
        assert_eq!(t.endianness.as_deref(), Some("big"));
        assert_eq!(t.license, DEFAULT_LICENSE);
        assert_eq!(t.registers[0].register_type, "float32");
        assert_eq!(t.registers[0].register_area, "holding");
    }

    #[test]
    fn collects_every_problem() {
        let mut t = modbus();
        t.template_id = "Bad Id".into();
        t.version = "1.0".into();
        t.registers.push(TemplateRegister {
            name: "v".into(),
            offset: 65535,
            register_type: "float32".into(),
            register_area: "nowhere".into(),
            scale: f64::NAN,
            unit: String::new(),
            description: String::new(),
            writable: false,
        });
        let errs = t.normalize_and_validate().unwrap_err();
        let all = errs.join("\n");
        assert!(all.contains("template_id"), "{all}");
        assert!(all.contains("semantic version"), "{all}");
        assert!(all.contains("used twice"), "{all}");
        assert!(all.contains("register_area"), "{all}");
        assert!(all.contains("scale"), "{all}");
        assert!(all.contains("past 65535"), "{all}");
    }

    #[test]
    fn rejects_other_format_and_wrong_protocol_fields() {
        let mut t = modbus();
        t.format = "voltrus-device-template/2".into();
        t.cov_enabled = Some(true);
        let all = t.normalize_and_validate().unwrap_err().join("\n");
        assert!(all.contains("voltrus-device-template/1"));
        assert!(all.contains("cov_enabled"));
    }

    #[test]
    fn reserved_slugs() {
        assert!(validate_slug("catalog").is_err());
        assert!(validate_slug("from-device").is_err());
        assert!(validate_slug("a--b").is_err());
        assert!(validate_slug("-a").is_err());
        assert!(validate_slug("a-b-1").is_ok());
    }

    #[test]
    fn slugify_collapses_runs() {
        assert_eq!(
            slugify("Endress+Hauser Promag 10W DN50"),
            "endress-hauser-promag-10w-dn50"
        );
        assert_eq!(slugify("  --Trane / Tracer SC+ "), "trane-tracer-sc");
    }

    #[test]
    fn semver_order() {
        assert_eq!(compare_versions("1.10.0", "1.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0-rc1", "1.0.0"), Ordering::Less);
        assert!(SemVer::parse("01.0.0").is_none());
        assert!(SemVer::parse("1.0").is_none());
        assert!(SemVer::parse("1.0.0-").is_none());
        assert_eq!(compare_versions("x", "0.0.1"), Ordering::Less);
    }

    #[test]
    fn resolve_prefers_newest_then_local() {
        let mut a = modbus();
        a.source = SOURCE_BUILTIN.into();
        a.version = "1.0.0".into();
        let mut b = a.clone();
        b.source = SOURCE_LOCAL.into();
        let mut c = a.clone();
        c.source = "catalog:https://x/index.json".into();
        c.version = "1.2.0".into();
        let all = vec![a, b, c];
        assert_eq!(
            resolve(&all, "acme-meter", None, None).unwrap().version,
            "1.2.0"
        );
        assert_eq!(
            resolve(&all, "acme-meter", Some("1.0.0"), None)
                .unwrap()
                .source,
            SOURCE_LOCAL
        );
        assert_eq!(
            resolve(&all, "acme-meter", Some("1.0.0"), Some("builtin"))
                .unwrap()
                .source,
            SOURCE_BUILTIN
        );
        assert!(resolve(&all, "nope", None, None).is_none());
    }

    #[test]
    fn filter_matches() {
        let mut t = modbus();
        t.normalize_and_validate().unwrap();
        t.source = SOURCE_LOCAL.into();
        let f = |q: Option<&str>, m: Option<&str>, p: Option<&str>, s: Option<&str>| {
            TemplateFilter {
                q: q.map(Into::into),
                manufacturer: m.map(Into::into),
                protocol: p.map(Into::into),
                source: s.map(Into::into),
            }
            .matches(&t)
        };
        assert!(f(None, None, None, None));
        assert!(f(Some("ACME"), None, None, None));
        assert!(f(Some("m1"), Some("acme"), Some("modbus"), Some("local")));
        assert!(!f(Some("zzz"), None, None, None));
        assert!(!f(None, Some("other"), None, None));
        assert!(!f(None, None, Some("bacnet"), None));
        assert!(!f(None, None, None, Some("catalog")));
    }
}
