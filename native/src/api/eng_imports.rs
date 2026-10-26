//! Data Manager › Import: the web client's `DataImportWizard.tsx`
//! (`ImportColumnMapper.tsx`, `queries/import.ts`) and
//! `NodeRedImportWizard.tsx` (`queries/import_nodered.ts`).
//!
//! - `POST /api/v1/import/csv?dry_run=&mode=&delimiter=&column_mapping=`
//!   and `POST /api/v1/import/excel?dry_run=&mode=&sheet=&column_mapping=`
//!   (`src/api/csv_import.rs`, `excel_import.rs`): admin only, a multipart
//!   `file` of at most 50 MB. Each row becomes one telemetry reading
//!   (`device_id`, `timestamp`, metric = the tag name, value); nothing else
//!   is created. A dry run counts without writing.
//! - `POST /api/v1/import/node-red` (`src/api/import_nodered.rs`):
//!   operator, a multipart `file` (`flows.json`) within the API's 1 MB body
//!   limit. The server parses and converts it and answers what it would
//!   create; it writes nothing.
//!
//! reqwest is built without its `multipart` feature, so the form body is
//! built here by hand ([`multipart`]). The CSV header and first rows are
//! read locally ([`parse_csv`]) for the column mapping, and the server's
//! column auto-detection is mirrored ([`auto_detect`]) so the mapping
//! starts where the server would. A flows.json is previewed locally
//! ([`parse_flows`]) with the server's node categories before upload.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use super::{Method, ServerError, VoltrusApi};

/// The import routes' body limit (`DefaultBodyLimit::max(50 MB)`).
pub const DATA_LIMIT: u64 = 50 * 1024 * 1024;
/// The API's body limit, which the Node-RED route keeps (1 MB).
pub const FLOWS_LIMIT: u64 = 1024 * 1024;
/// An upload of tens of megabytes outlives the client's 10 s default.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// How many data rows the column mapping previews.
pub const PREVIEW_ROWS: usize = 12;

// ── Files ────────────────────────────────────────────────────────────────

/// What a data file is, by its extension (the web's `detectFileType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Csv,
    Excel,
}

impl FileKind {
    /// `.csv`, or an Excel workbook (`.xlsx`, `.xls`, `.xlsm`, `.xlsb`).
    pub fn of(name: &str) -> Option<Self> {
        let name = name.to_ascii_lowercase();
        if [".xlsx", ".xls", ".xlsm", ".xlsb"]
            .iter()
            .any(|ext| name.ends_with(ext))
        {
            Some(Self::Excel)
        } else if name.ends_with(".csv") {
            Some(Self::Csv)
        } else {
            None
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Excel => "Excel",
        }
    }

    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Csv => "/api/v1/import/csv",
            Self::Excel => "/api/v1/import/excel",
        }
    }

    /// The part's `Content-Type`, as a browser sends it for the file.
    pub fn content_type(self, name: &str) -> &'static str {
        let name = name.to_ascii_lowercase();
        match self {
            Self::Csv => "text/csv",
            Self::Excel if name.ends_with(".xls") => "application/vnd.ms-excel",
            Self::Excel if name.ends_with(".xlsm") => {
                "application/vnd.ms-excel.sheet.macroEnabled.12"
            }
            Self::Excel if name.ends_with(".xlsb") => {
                "application/vnd.ms-excel.sheet.binary.macroEnabled.12"
            }
            Self::Excel => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        }
    }
}

/// Whether `name` is a flows file (the web accepts `.json`).
pub fn is_flows_file(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".json")
}

/// `name`'s size against `limit`, before reading it: `None` when it fits,
/// else why not.
pub fn too_large(size: u64, limit: u64) -> Option<String> {
    (size > limit).then(|| {
        format!(
            "{} is over the server's {} limit",
            size_label(size),
            size_label(limit)
        )
    })
}

/// `812 B`, `14.2 KB`, `3.1 MB`.
pub fn size_label(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", value / 1024.0)
    } else {
        format!("{:.1} MB", value / (1024.0 * 1024.0))
    }
}

/// Read `path` for an upload within `limit`: its file name and bytes.
pub fn read_upload(path: &Path, limit: u64) -> Result<(String, Vec<u8>)> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| anyhow!("{} is not a file", path.display()))?;
    let size = std::fs::metadata(path)
        .map_err(|error| anyhow!("could not read {name}: {error}"))?
        .len();
    if let Some(problem) = too_large(size, limit) {
        return Err(anyhow!("{name}: {problem}"));
    }
    let bytes = std::fs::read(path).map_err(|error| anyhow!("could not read {name}: {error}"))?;
    Ok((name, bytes))
}

// ── Multipart ────────────────────────────────────────────────────────────

/// A `multipart/form-data` body with one file part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multipart {
    pub boundary: String,
    pub body: Vec<u8>,
}

impl Multipart {
    /// The request's `Content-Type` header.
    pub fn content_type(&self) -> String {
        format!("multipart/form-data; boundary={}", self.boundary)
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// A file name inside a quoted header parameter, escaped as browsers do
/// (`"` → `%22`, CR → `%0D`, LF → `%0A`).
fn quoted(value: &str) -> String {
    value
        .replace('"', "%22")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// The form body for one file part `field` with `boundary`.
pub fn multipart_with(
    boundary: &str,
    field: &str,
    file_name: &str,
    content_type: &str,
    bytes: &[u8],
) -> Multipart {
    let head = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {content_type}\r\n\r\n",
        quoted(field),
        quoted(file_name),
    );
    let tail = format!("\r\n--{boundary}--\r\n");
    let mut body = Vec::with_capacity(head.len() + bytes.len() + tail.len());
    body.extend_from_slice(head.as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(tail.as_bytes());
    Multipart {
        boundary: boundary.to_owned(),
        body,
    }
}

/// The form body for one file part `field`, with a boundary the file does
/// not contain.
pub fn multipart(field: &str, file_name: &str, content_type: &str, bytes: &[u8]) -> Multipart {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let mut attempt: u32 = 0;
    loop {
        let boundary = format!("----VoltrusNative{seed:x}{attempt:x}");
        if !contains(bytes, boundary.as_bytes()) {
            return multipart_with(&boundary, field, file_name, content_type, bytes);
        }
        attempt += 1;
    }
}

// ── Query strings ────────────────────────────────────────────────────────

/// Percent-encode a query value (everything but RFC 3986 unreserved).
pub fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The delimiters the server understands (`,`, `;`, and `\t` spelt as a
/// backslash and a `t`); anything else reads as a comma.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Delimiter {
    #[default]
    Comma,
    Semicolon,
    Tab,
}

impl Delimiter {
    pub const ALL: [Self; 3] = [Self::Comma, Self::Semicolon, Self::Tab];

    pub fn byte(self) -> u8 {
        match self {
            Self::Comma => b',',
            Self::Semicolon => b';',
            Self::Tab => b'\t',
        }
    }

    /// The `delimiter=` value.
    pub fn param(self) -> &'static str {
        match self {
            Self::Comma => ",",
            Self::Semicolon => ";",
            Self::Tab => "\\t",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Comma => "Comma",
            Self::Semicolon => "Semicolon",
            Self::Tab => "Tab",
        }
    }

    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Comma => Self::Semicolon,
            Self::Semicolon => Self::Tab,
            Self::Tab => Self::Comma,
        }
    }
}

/// `mode=`: the server accepts both words but answers 501 for `replace`
/// ("replace mode is not yet supported; use merge").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Merge,
    Replace,
}

impl Mode {
    pub const ALL: [Self; 2] = [Self::Merge, Self::Replace];

    pub fn param(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Replace => "replace",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge (update existing, add new)",
            Self::Replace => "Replace (clear existing first)",
        }
    }

    /// Whether the server carries it out.
    pub fn supported(self) -> bool {
        self == Self::Merge
    }
}

/// One import or dry run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    pub kind: FileKind,
    pub dry_run: bool,
    pub mode: Mode,
    /// CSV only.
    pub delimiter: Delimiter,
    /// Excel only; blank = the first sheet.
    pub sheet: String,
    /// `None`: the server auto-detects the columns.
    pub mapping: Option<ColumnMapping>,
}

impl ImportRequest {
    /// The path with its query string, as the web client builds it.
    pub fn path(&self) -> String {
        let mut params = Vec::new();
        if self.dry_run {
            params.push("dry_run=true".to_owned());
        }
        params.push(format!("mode={}", self.mode.param()));
        match self.kind {
            FileKind::Csv => params.push(format!(
                "delimiter={}",
                percent_encode(self.delimiter.param())
            )),
            FileKind::Excel => {
                let sheet = self.sheet.trim();
                if !sheet.is_empty() {
                    params.push(format!("sheet={}", percent_encode(sheet)));
                }
            }
        }
        if let Some(mapping) = self.mapping.as_ref().filter(|mapping| !mapping.is_empty()) {
            params.push(format!(
                "column_mapping={}",
                percent_encode(&mapping.to_json())
            ));
        }
        format!("{}?{}", self.kind.endpoint(), params.join("&"))
    }
}

// ── Column mapping ───────────────────────────────────────────────────────

/// A Voltrus field a file column maps to (`ImportColumnMapper.tsx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    TagName,
    Value,
    Timestamp,
    DeviceId,
}

impl Field {
    pub const ALL: [Self; 4] = [Self::TagName, Self::Value, Self::Timestamp, Self::DeviceId];

    /// The `column_mapping` key.
    pub fn key(self) -> &'static str {
        match self {
            Self::TagName => "tag_name",
            Self::Value => "value",
            Self::Timestamp => "timestamp",
            Self::DeviceId => "device_id",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TagName => "Tag Name",
            Self::Value => "Value",
            Self::Timestamp => "Timestamp",
            Self::DeviceId => "Device ID",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::TagName => "Unique tag identifier (required)",
            Self::Value => "Numeric value; rows without one are skipped",
            Self::Timestamp => "ISO 8601 or Unix seconds; blank = now",
            Self::DeviceId => "Device or data source identifier; blank = default",
        }
    }

    pub fn required(self) -> bool {
        self == Self::TagName
    }

    fn index(self) -> usize {
        match self {
            Self::TagName => 0,
            Self::Value => 1,
            Self::Timestamp => 2,
            Self::DeviceId => 3,
        }
    }

    fn of_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|field| field.key() == key)
    }
}

/// Which file column (0-based) each field reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColumnMapping([Option<usize>; 4]);

/// Why a mapping cannot be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingProblem {
    MissingTag,
    /// Two fields read the same column.
    Conflict(Field, Field),
}

impl MappingProblem {
    pub fn message(self) -> String {
        match self {
            Self::MissingTag => {
                "Tag Name column must be mapped before proceeding with import.".to_owned()
            }
            Self::Conflict(field, other) => format!(
                "{} and {} read the same column; each column maps to one field.",
                field.label(),
                other.label()
            ),
        }
    }
}

impl ColumnMapping {
    pub fn get(&self, field: Field) -> Option<usize> {
        self.0[field.index()]
    }

    pub fn set(&mut self, field: Field, column: Option<usize>) {
        self.0[field.index()] = column;
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Option::is_none)
    }

    /// The server's `column_mapping` (keys it does not know are dropped).
    pub fn from_server(map: &HashMap<String, usize>) -> Self {
        let mut mapping = Self::default();
        for (key, column) in map {
            if let Some(field) = Field::of_key(key) {
                mapping.set(field, Some(*column));
            }
        }
        mapping
    }

    /// `{"tag_name":0,"value":2}`: the mapped fields, in field order.
    pub fn to_json(&self) -> String {
        let parts: Vec<String> = Field::ALL
            .into_iter()
            .filter_map(|field| {
                self.get(field)
                    .map(|column| format!("\"{}\":{column}", field.key()))
            })
            .collect();
        format!("{{{}}}", parts.join(","))
    }

    /// The other field that reads `field`'s column, if any (the web's
    /// "Already used by …").
    pub fn conflict(&self, field: Field) -> Option<Field> {
        let column = self.get(field)?;
        Field::ALL
            .into_iter()
            .find(|other| *other != field && self.get(*other) == Some(column))
    }

    /// Whether the mapping can be sent: a tag column, no column twice.
    pub fn check(&self) -> Result<(), MappingProblem> {
        if self.get(Field::TagName).is_none() {
            return Err(MappingProblem::MissingTag);
        }
        for field in Field::ALL {
            if let Some(other) = self.conflict(field) {
                return Err(MappingProblem::Conflict(field, other));
            }
        }
        Ok(())
    }

    /// Step `field` to the next (or previous) of: not mapped, column 0 …
    /// `columns - 1`, wrapping.
    pub fn cycle(&mut self, field: Field, columns: usize, forward: bool) {
        if columns == 0 {
            self.set(field, None);
            return;
        }
        let next = match (self.get(field), forward) {
            (None, true) => Some(0),
            (None, false) => Some(columns - 1),
            (Some(column), true) if column + 1 < columns => Some(column + 1),
            (Some(_), true) => None,
            (Some(0), false) => None,
            (Some(column), false) => Some(column.min(columns) - 1),
        };
        self.set(field, next);
    }
}

/// The server's `auto_detect_columns`: a header names a field after
/// trimming, lower-casing and turning `_`/`-` into spaces; a later column
/// wins over an earlier one.
pub fn auto_detect(headers: &[String]) -> ColumnMapping {
    let mut mapping = ColumnMapping::default();
    for (column, header) in headers.iter().enumerate() {
        let header = header.trim().to_lowercase().replace(['_', '-'], " ");
        let field = match header.as_str() {
            "name" | "tag" | "tag name" | "sensor" | "sensor name" | "key" | "data source"
            | "tagname" | "point" | "point name" => Field::TagName,
            "value" | "val" | "reading" | "measurement" | "data" | "pv" => Field::Value,
            "timestamp" | "time" | "ts" | "datetime" | "date" | "date time" => Field::Timestamp,
            // `device_name` is in the server's list too, but the `_` has
            // become a space by then, so it never matches; mirrored.
            "device" | "device id" | "deviceid" | "device_name" | "plc" => Field::DeviceId,
            _ => continue,
        };
        mapping.set(field, Some(column));
    }
    mapping
}

/// `A`, `B`, … `Z`, `AA`: a spreadsheet column's letters.
pub fn column_letters(mut column: usize) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push(b'A' + u8::try_from(column % 26).unwrap_or(0));
        if column < 26 {
            break;
        }
        column = column / 26 - 1;
    }
    letters.reverse();
    String::from_utf8(letters).unwrap_or_default()
}

// ── CSV ──────────────────────────────────────────────────────────────────

/// A CSV file's header, its first rows and how many data rows it has.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CsvPreview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Data rows (after the header), as the server counts `total_rows`.
    pub total_rows: usize,
}

/// Records of `text` split as the server's `csv` reader does: RFC 4180
/// quoting (`""` inside quotes is a quote), `\n`, `\r\n` or `\r` line ends,
/// blank lines skipped, rows of any length, a leading BOM dropped.
struct Records<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    delimiter: char,
}

impl Iterator for Records<'_> {
    type Item = Vec<String>;

    fn next(&mut self) -> Option<Vec<String>> {
        loop {
            self.chars.peek()?;
            let mut fields = Vec::new();
            let mut field = String::new();
            let mut quoted = false;
            let mut started = false;
            while let Some(c) = self.chars.next() {
                if quoted {
                    if c == '"' {
                        if self.chars.peek() == Some(&'"') {
                            self.chars.next();
                            field.push('"');
                        } else {
                            quoted = false;
                        }
                    } else {
                        field.push(c);
                    }
                    continue;
                }
                match c {
                    '"' if field.is_empty() => {
                        quoted = true;
                        started = true;
                    }
                    '\n' | '\r' => {
                        if c == '\r' && self.chars.peek() == Some(&'\n') {
                            self.chars.next();
                        }
                        break;
                    }
                    c if c == self.delimiter => {
                        fields.push(std::mem::take(&mut field));
                        started = true;
                    }
                    c => {
                        field.push(c);
                        started = true;
                    }
                }
            }
            if !started && fields.is_empty() && field.is_empty() {
                // A blank line.
                continue;
            }
            fields.push(field);
            return Some(fields);
        }
    }
}

/// Parse `text` with `delimiter`: the header, the first `keep` rows and the
/// number of data rows.
pub fn parse_csv(text: &str, delimiter: u8, keep: usize) -> CsvPreview {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut records = Records {
        chars: text.chars().peekable(),
        delimiter: char::from(delimiter),
    };
    let Some(headers) = records.next() else {
        return CsvPreview::default();
    };
    let mut preview = CsvPreview {
        headers,
        ..CsvPreview::default()
    };
    for record in records {
        if preview.rows.len() < keep {
            preview.rows.push(record);
        }
        preview.total_rows += 1;
    }
    preview
}

/// Parse a CSV file's bytes as the server reads them (lossy UTF-8).
pub fn parse_csv_bytes(bytes: &[u8], delimiter: u8, keep: usize) -> CsvPreview {
    parse_csv(&String::from_utf8_lossy(bytes), delimiter, keep)
}

// ── Answers ──────────────────────────────────────────────────────────────

/// One row the server skipped, or a batch it failed to write.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RowError {
    /// The file's line (the header is line 1); none for a batch failure.
    pub row: Option<usize>,
    pub tag: Option<String>,
    pub error: String,
}

impl RowError {
    /// `Row 3: Invalid value: 'abc' (pump_1)`, as the web lists them.
    pub fn line(&self) -> String {
        let row = self
            .row
            .map_or_else(|| "?".to_owned(), |row| row.to_string());
        match &self.tag {
            Some(tag) => format!("Row {row}: {} ({tag})", self.error),
            None => format!("Row {row}: {}", self.error),
        }
    }
}

/// The answer of `POST /api/v1/import/csv` and `/import/excel` (the latter
/// adds `sheet` and `available_sheets`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportResult {
    pub total_rows: usize,
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<RowError>,
    pub dry_run: bool,
    pub mode: String,
    pub column_mapping: HashMap<String, usize>,
    pub sheet: Option<String>,
    pub available_sheets: Vec<String>,
}

/// An import the server refused (`{"error": …}` with a 4xx/5xx), with the
/// extra fields some refusals carry: the file's columns when no tag column
/// was found, and the workbook's sheets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ImportRefusal {
    #[serde(skip)]
    pub status: u16,
    pub error: String,
    pub detected_columns: Vec<String>,
    pub expected: Vec<String>,
    pub sheet: Option<String>,
    pub available_sheets: Vec<String>,
}

impl ImportRefusal {
    /// A refusal from a status and body (not JSON: the status and body).
    pub fn parse(status: reqwest::StatusCode, body: &str) -> Self {
        let mut refusal: Self = serde_json::from_str(body).unwrap_or_default();
        refusal.status = status.as_u16();
        if refusal.error.is_empty() {
            refusal.error = ServerError::new(status, body).message;
        }
        refusal
    }
}

/// An import's answer: carried out (or counted), or refused.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportOutcome {
    Done(ImportResult),
    Refused(ImportRefusal),
}

/// `parse` of the Node-RED import's answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeRedParse {
    pub total_nodes: usize,
    pub convertible: usize,
    pub manual_review: usize,
    pub ui_decoration: usize,
    pub unsupported: usize,
}

/// `conversion`: how many of each Voltrus entity the flows map to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeRedConversion {
    pub modbus_devices: usize,
    pub mqtt_sources: usize,
    pub widgets: usize,
    pub flow_nodes: usize,
    pub function_nodes: usize,
    pub skipped_nodes: usize,
}

/// Mirrors `import_nodered::converter::ConvertedTag`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertedTag {
    pub name: String,
    pub address: u16,
    pub data_type: String,
    pub register_area: String,
    pub scale: f64,
}

/// Mirrors `ConvertedModbusDevice`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertedModbusDevice {
    pub device_id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub unit_id: u8,
    pub interval_ms: u64,
    pub tags: Vec<ConvertedTag>,
}

/// Mirrors `ConvertedMqttSource`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertedMqttSource {
    pub topic: String,
    pub qos: u8,
    pub name: String,
    /// `in` or `out`.
    pub direction: String,
    pub source_node_id: String,
}

/// Mirrors `ConvertedWidget`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertedWidget {
    pub widget_type: String,
    pub name: String,
    pub label: Option<String>,
    pub min_val: Option<f64>,
    pub max_val: Option<f64>,
    pub source_node_id: String,
    pub format: Option<String>,
}

/// Mirrors `ConvertedFlowNode`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertedFlowNode {
    pub node_type: String,
    pub name: String,
    pub config: serde_json::Value,
    pub source_node_id: String,
}

/// Mirrors `ConvertedFunctionNode`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertedFunctionNode {
    pub node_id: String,
    pub name: String,
    pub source_code: Option<String>,
    pub warning: String,
}

/// Mirrors `SkippedNode`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkippedNode {
    pub node_id: String,
    pub node_type: String,
    pub name: Option<String>,
    pub reason: String,
}

/// `details`: every converted entity.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeRedDetails {
    pub modbus_devices: Vec<ConvertedModbusDevice>,
    pub mqtt_sources: Vec<ConvertedMqttSource>,
    pub widgets: Vec<ConvertedWidget>,
    pub flow_nodes: Vec<ConvertedFlowNode>,
    pub function_nodes: Vec<ConvertedFunctionNode>,
    pub skipped_nodes: Vec<SkippedNode>,
}

/// `license`: the Modbus devices now and those the flows would add.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeRedLicense {
    pub current_devices: u64,
    pub new_devices: u64,
    /// `null` today: no limit.
    pub limit: Option<u64>,
}

impl NodeRedLicense {
    /// Whether the new devices would pass the limit (the web's
    /// `licenseOk`, negated).
    pub fn exceeded(&self) -> bool {
        self.limit
            .is_some_and(|limit| limit > 0 && self.current_devices + self.new_devices > limit)
    }
}

/// The answer of `POST /api/v1/import/node-red`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeRedImport {
    pub parse: NodeRedParse,
    pub conversion: NodeRedConversion,
    pub details: NodeRedDetails,
    pub warnings: Vec<String>,
    pub license: NodeRedLicense,
}

// ── Local flows preview ──────────────────────────────────────────────────

/// The server's `NodeCategory` of a Node-RED node type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCategory {
    Convertible,
    ManualReview,
    UiDecoration,
    Unsupported,
}

impl NodeCategory {
    pub const ALL: [Self; 4] = [
        Self::Convertible,
        Self::ManualReview,
        Self::UiDecoration,
        Self::Unsupported,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Convertible => "Convertible",
            Self::ManualReview => "Manual review",
            Self::UiDecoration => "Decoration",
            Self::Unsupported => "Unsupported",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Convertible => 0,
            Self::ManualReview => 1,
            Self::UiDecoration => 2,
            Self::Unsupported => 3,
        }
    }

    /// `parser::NodeRedNode::category`, type for type.
    pub fn of(node_type: &str) -> Self {
        match node_type {
            "modbus-read" | "modbus-write" | "modbus-response" | "modbus-flex-getter"
            | "modbus-flex-write" | "modbus-client" | "modbus-server" | "opcua-client"
            | "opcua-item" | "opcua-browser" | "s7-client" | "s7-in" | "s7-out" | "enip-read"
            | "enip-write" | "bacnet-client" => Self::Convertible,
            "mqtt in" | "mqtt out" | "mqtt-broker" => Self::Convertible,
            "switch" | "change" | "delay" | "trigger" | "link in" | "link out" | "link-call"
            | "complete" | "catch" | "status" | "exec" | "template" => Self::Convertible,
            "ui_gauge" | "ui_chart" | "ui_text" | "ui_button" | "ui_slider" | "ui_dropdown"
            | "ui_switch" | "ui_led" | "ui_notification" | "ui_toast" | "ui_table" | "ui_group"
            | "ui_tab" | "ui_spacer" => Self::Convertible,
            "function" | "python" | "java" => Self::ManualReview,
            "inject" | "debug" => Self::UiDecoration,
            "comment" | "tab" | "subflow" | "subflow:" => Self::UiDecoration,
            _ => Self::Unsupported,
        }
    }
}

/// One node of a flows.json.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FlowNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default)]
    pub name: Option<String>,
}

impl FlowNode {
    pub fn category(&self) -> NodeCategory {
        NodeCategory::of(&self.node_type)
    }

    /// Its name, else its id (as the server names it).
    pub fn title(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.id)
    }
}

/// A flows.json read locally: its nodes and how many fall in each category.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowsPreview {
    pub nodes: Vec<FlowNode>,
    counts: [usize; 4],
}

impl FlowsPreview {
    pub fn count(&self, category: NodeCategory) -> usize {
        self.counts[category.index()]
    }

    pub fn total(&self) -> usize {
        self.nodes.len()
    }
}

#[derive(Deserialize)]
struct FlowsObject {
    #[serde(default)]
    nodes: Vec<FlowNode>,
}

/// `parser::parse_flows`: an array of nodes, or `{"nodes": […]}`.
pub fn parse_flows(text: &str) -> Result<FlowsPreview, String> {
    let nodes: Vec<FlowNode> = if text.trim().starts_with('[') {
        serde_json::from_str(text)
    } else {
        serde_json::from_str::<FlowsObject>(text).map(|flows| flows.nodes)
    }
    .map_err(|error| format!("Failed to parse flows.json: {error}"))?;
    let mut counts = [0; 4];
    for node in &nodes {
        counts[node.category().index()] += 1;
    }
    Ok(FlowsPreview { nodes, counts })
}

// ── Wizard steps ─────────────────────────────────────────────────────────

/// Where Esc goes from a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Back<S> {
    To(S),
    /// Nothing before it: Esc leaves the section (to the sidebar).
    Leave,
    /// A request is running: Esc waits.
    Stay,
}

/// The data import wizard: choose the file and options, map the columns,
/// see the dry run, import, read the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataStep {
    #[default]
    File,
    Mapping,
    DryRun,
    Importing,
    Report,
}

impl DataStep {
    pub const ALL: [Self; 5] = [
        Self::File,
        Self::Mapping,
        Self::DryRun,
        Self::Importing,
        Self::Report,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Mapping => "Mapping",
            Self::DryRun => "Dry run",
            Self::Importing => "Import",
            Self::Report => "Report",
        }
    }

    /// Esc: a step back (the report starts over).
    pub fn back(self) -> Back<Self> {
        match self {
            Self::File => Back::Leave,
            Self::Mapping => Back::To(Self::File),
            Self::DryRun => Back::To(Self::Mapping),
            Self::Importing => Back::Stay,
            Self::Report => Back::To(Self::File),
        }
    }
}

/// The Node-RED wizard: choose flows.json (previewed locally), upload,
/// see the server's conversion, read the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlowStep {
    #[default]
    File,
    Uploading,
    Preview,
    Report,
}

impl FlowStep {
    pub const ALL: [Self; 4] = [Self::File, Self::Uploading, Self::Preview, Self::Report];

    pub fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Uploading => "Upload",
            Self::Preview => "Preview",
            Self::Report => "Report",
        }
    }

    pub fn back(self) -> Back<Self> {
        match self {
            Self::File => Back::Leave,
            Self::Uploading => Back::Stay,
            Self::Preview => Back::To(Self::File),
            Self::Report => Back::To(Self::Preview),
        }
    }
}

// ── Calls ────────────────────────────────────────────────────────────────

impl VoltrusApi {
    /// `POST path` with `form`, the raw answer.
    async fn post_form(&self, path: &str, form: Multipart) -> Result<reqwest::Response> {
        let content_type = form.content_type();
        Ok(self
            .req(Method::POST, path)?
            .timeout(UPLOAD_TIMEOUT)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(form.body)
            .send()
            .await?)
    }

    /// `POST /api/v1/import/csv` or `/import/excel` with `bytes` as the
    /// `file` part named `file_name`: the counts, or the server's refusal.
    pub async fn import_data(
        &self,
        request: &ImportRequest,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<ImportOutcome> {
        if let Some(problem) = too_large(bytes.len() as u64, DATA_LIMIT) {
            return Err(anyhow!("{file_name}: {problem}"));
        }
        let form = multipart(
            "file",
            file_name,
            request.kind.content_type(file_name),
            bytes,
        );
        let resp = self.post_form(&request.path(), form).await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Ok(ImportOutcome::Refused(ImportRefusal::parse(status, &body)));
        }
        let body = if body.trim().is_empty() {
            "null"
        } else {
            &body
        };
        Ok(ImportOutcome::Done(serde_json::from_str(body)?))
    }

    /// `POST /api/v1/import/node-red` with `bytes` as the `file` part.
    pub async fn import_node_red(&self, file_name: &str, bytes: &[u8]) -> Result<NodeRedImport> {
        if let Some(problem) = too_large(bytes.len() as u64, FLOWS_LIMIT) {
            return Err(anyhow!("{file_name}: {problem}"));
        }
        let form = multipart("file", file_name, "application/json", bytes);
        let resp = self.post_form("/api/v1/import/node-red", form).await?;
        Self::decode(resp).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn file_kinds_follow_the_web() {
        assert_eq!(FileKind::of("plant.CSV"), Some(FileKind::Csv));
        for name in ["a.xlsx", "a.XLS", "a.xlsm", "a.xlsb"] {
            assert_eq!(FileKind::of(name), Some(FileKind::Excel), "{name}");
        }
        assert_eq!(FileKind::of("flows.json"), None);
        assert_eq!(FileKind::Csv.endpoint(), "/api/v1/import/csv");
        assert_eq!(FileKind::Excel.endpoint(), "/api/v1/import/excel");
        assert_eq!(
            FileKind::Excel.content_type("a.xls"),
            "application/vnd.ms-excel"
        );
        assert!(
            FileKind::Excel
                .content_type("a.xlsx")
                .contains("spreadsheetml")
        );
        assert!(is_flows_file("flows.JSON"));
        assert!(!is_flows_file("flows.txt"));
    }

    #[test]
    fn sizes_and_limits() {
        assert_eq!(size_label(812), "812 B");
        assert_eq!(size_label(14_540), "14.2 KB");
        assert_eq!(size_label(3 * 1024 * 1024), "3.0 MB");
        assert!(too_large(DATA_LIMIT, DATA_LIMIT).is_none());
        assert_eq!(
            too_large(FLOWS_LIMIT + 1, FLOWS_LIMIT).as_deref(),
            Some("1.0 MB is over the server's 1.0 MB limit")
        );
    }

    #[test]
    fn multipart_body_is_one_file_part() {
        let form = multipart_with("XyZ", "file", "pl\"ant.csv", "text/csv", b"name,value\n");
        assert_eq!(form.content_type(), "multipart/form-data; boundary=XyZ");
        assert_eq!(
            String::from_utf8(form.body).unwrap(),
            "--XyZ\r\nContent-Disposition: form-data; name=\"file\"; filename=\"pl%22ant.csv\"\r\nContent-Type: text/csv\r\n\r\nname,value\n\r\n--XyZ--\r\n"
        );
    }

    #[test]
    fn multipart_boundary_never_occurs_in_the_file() {
        let form = multipart("file", "a.bin", "application/octet-stream", b"\x00\x01abc");
        assert!(form.boundary.starts_with("----VoltrusNative"));
        let body = &form.body;
        let marker = format!("--{}", form.boundary);
        let starts = body
            .windows(marker.len())
            .filter(|window| *window == marker.as_bytes())
            .count();
        assert_eq!(starts, 2, "the opening and the closing delimiter only");
        assert!(body.ends_with(format!("\r\n--{}--\r\n", form.boundary).as_bytes()));
        assert!(contains(body, b"\x00\x01abc"));
        assert!(!contains(b"abc", b""));
    }

    #[test]
    fn query_strings_match_the_web_client() {
        let mut mapping = ColumnMapping::default();
        mapping.set(Field::TagName, Some(0));
        mapping.set(Field::Value, Some(2));
        let csv = ImportRequest {
            kind: FileKind::Csv,
            dry_run: true,
            mode: Mode::Merge,
            delimiter: Delimiter::Tab,
            sheet: "ignored".into(),
            mapping: Some(mapping),
        };
        assert_eq!(
            csv.path(),
            "/api/v1/import/csv?dry_run=true&mode=merge&delimiter=%5Ct&column_mapping=%7B%22tag_name%22%3A0%2C%22value%22%3A2%7D"
        );
        let excel = ImportRequest {
            kind: FileKind::Excel,
            dry_run: false,
            mode: Mode::Merge,
            delimiter: Delimiter::Semicolon,
            sheet: " Line 2 ".into(),
            mapping: Some(ColumnMapping::default()),
        };
        assert_eq!(
            excel.path(),
            "/api/v1/import/excel?mode=merge&sheet=Line%202"
        );
        let first_sheet = ImportRequest {
            sheet: String::new(),
            mapping: None,
            ..excel
        };
        assert_eq!(first_sheet.path(), "/api/v1/import/excel?mode=merge");
        assert_eq!(percent_encode("a-b_c.d~e;,"), "a-b_c.d~e%3B%2C");
        assert_eq!(Delimiter::Comma.param(), ",");
        assert_eq!(Delimiter::Tab.next(), Delimiter::Comma);
        assert!(Mode::Merge.supported() && !Mode::Replace.supported());
    }

    #[test]
    fn auto_detection_mirrors_the_server() {
        let mapping = auto_detect(&strings(&["Tag-Name", " VALUE ", "ts", "PLC", "notes"]));
        assert_eq!(mapping.get(Field::TagName), Some(0));
        assert_eq!(mapping.get(Field::Value), Some(1));
        assert_eq!(mapping.get(Field::Timestamp), Some(2));
        assert_eq!(mapping.get(Field::DeviceId), Some(3));
        // A later column wins; `device_name` never matches on the server.
        let later = auto_detect(&strings(&["name", "key", "device_name"]));
        assert_eq!(later.get(Field::TagName), Some(1));
        assert_eq!(later.get(Field::DeviceId), None);
        assert!(auto_detect(&strings(&["a", "b"])).is_empty());
    }

    #[test]
    fn mapping_validation_and_json() {
        let mut mapping = ColumnMapping::default();
        assert_eq!(mapping.check(), Err(MappingProblem::MissingTag));
        assert_eq!(mapping.to_json(), "{}");
        mapping.set(Field::TagName, Some(1));
        mapping.set(Field::Value, Some(1));
        assert_eq!(mapping.conflict(Field::Value), Some(Field::TagName));
        assert_eq!(
            mapping.check(),
            Err(MappingProblem::Conflict(Field::TagName, Field::Value))
        );
        mapping.set(Field::Value, Some(0));
        mapping.set(Field::DeviceId, Some(3));
        assert_eq!(mapping.check(), Ok(()));
        assert_eq!(
            mapping.to_json(),
            r#"{"tag_name":1,"value":0,"device_id":3}"#
        );
        let server: HashMap<String, usize> =
            serde_json::from_str(r#"{"tag_name":1,"value":0,"device_id":3,"extra":9}"#).unwrap();
        assert_eq!(ColumnMapping::from_server(&server), mapping);
        assert!(MappingProblem::MissingTag.message().contains("Tag Name"));
    }

    #[test]
    fn mapping_cycles_through_the_columns() {
        let mut mapping = ColumnMapping::default();
        mapping.cycle(Field::Value, 3, true);
        assert_eq!(mapping.get(Field::Value), Some(0));
        mapping.cycle(Field::Value, 3, true);
        mapping.cycle(Field::Value, 3, true);
        assert_eq!(mapping.get(Field::Value), Some(2));
        mapping.cycle(Field::Value, 3, true);
        assert_eq!(mapping.get(Field::Value), None);
        mapping.cycle(Field::Value, 3, false);
        assert_eq!(mapping.get(Field::Value), Some(2));
        mapping.set(Field::Value, Some(9));
        mapping.cycle(Field::Value, 3, false);
        assert_eq!(
            mapping.get(Field::Value),
            Some(2),
            "an out-of-range column steps in"
        );
        mapping.set(Field::Value, Some(0));
        mapping.cycle(Field::Value, 3, false);
        assert_eq!(mapping.get(Field::Value), None);
        mapping.cycle(Field::Value, 0, true);
        assert_eq!(mapping.get(Field::Value), None);
    }

    #[test]
    fn column_letters_count_like_a_spreadsheet() {
        assert_eq!(column_letters(0), "A");
        assert_eq!(column_letters(25), "Z");
        assert_eq!(column_letters(26), "AA");
        assert_eq!(column_letters(27), "AB");
        assert_eq!(column_letters(701), "ZZ");
        assert_eq!(column_letters(702), "AAA");
    }

    #[test]
    fn csv_parsing_follows_rfc_4180() {
        let text = "\u{feff}name,value,note\r\n\"pump, 1\",1.5,\"say \"\"hi\"\"\"\n\nflow,2\r\n\"multi\nline\",3,x\rlast,4,\n";
        let preview = parse_csv(text, b',', 2);
        assert_eq!(preview.headers, strings(&["name", "value", "note"]));
        assert_eq!(preview.total_rows, 4);
        assert_eq!(preview.rows.len(), 2);
        assert_eq!(preview.rows[0], strings(&["pump, 1", "1.5", "say \"hi\""]));
        assert_eq!(preview.rows[1], strings(&["flow", "2"]));
        let all = parse_csv(text, b',', 10);
        assert_eq!(all.rows[2], strings(&["multi\nline", "3", "x"]));
        assert_eq!(all.rows[3], strings(&["last", "4", ""]));
    }

    #[test]
    fn csv_parsing_other_delimiters_and_edges() {
        let preview = parse_csv("tag;value\na;1\n", b';', 5);
        assert_eq!(preview.headers, strings(&["tag", "value"]));
        assert_eq!(preview.rows, vec![strings(&["a", "1"])]);
        let tabs = parse_csv("tag\tvalue\nb\t2", b'\t', 5);
        assert_eq!(tabs.rows, vec![strings(&["b", "2"])]);
        assert_eq!(parse_csv("", b',', 5), CsvPreview::default());
        let header_only = parse_csv("name,value\n", b',', 5);
        assert_eq!(header_only.total_rows, 0);
        let empty_fields = parse_csv("a,b\n,\n", b',', 5);
        assert_eq!(empty_fields.rows, vec![strings(&["", ""])]);
        let quoted_empty = parse_csv("a\n\"\"\n", b',', 5);
        assert_eq!(quoted_empty.rows, vec![strings(&[""])]);
        let lossy = parse_csv_bytes(b"name\n\xffx\n", b',', 5);
        assert_eq!(lossy.rows, vec![strings(&["\u{fffd}x"])]);
    }

    #[test]
    fn decodes_the_csv_import_answer() {
        // `csv_import.rs`: a dry run with one bad row.
        let json = r#"{"total_rows":2,"imported":1,"skipped":1,
            "errors":[{"row":3,"tag":"pump_1","error":"Invalid value: 'abc'"}],
            "dry_run":true,"mode":"merge",
            "column_mapping":{"tag_name":0,"value":1,"timestamp":2,"device_id":3}}"#;
        let result: ImportResult = serde_json::from_str(json).unwrap();
        assert_eq!(
            (result.total_rows, result.imported, result.skipped),
            (2, 1, 1)
        );
        assert!(result.dry_run);
        assert_eq!(result.mode, "merge");
        assert_eq!(result.sheet, None);
        assert_eq!(
            result.errors[0].line(),
            "Row 3: Invalid value: 'abc' (pump_1)"
        );
        assert_eq!(
            ColumnMapping::from_server(&result.column_mapping).get(Field::DeviceId),
            Some(3)
        );
        // A batch failure has no row and no tag.
        let batch: RowError = serde_json::from_str(
            r#"{"error":"Final batch insert failed (3 rows dropped): disk full"}"#,
        )
        .unwrap();
        assert_eq!(
            batch.line(),
            "Row ?: Final batch insert failed (3 rows dropped): disk full"
        );
    }

    #[test]
    fn decodes_the_excel_import_answer() {
        let json = r#"{"total_rows":1,"imported":1,"skipped":0,"errors":[],
            "dry_run":false,"mode":"merge","column_mapping":{"tag_name":0,"value":1},
            "sheet":"Readings","available_sheets":["Readings","Notes"]}"#;
        let result: ImportResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.sheet.as_deref(), Some("Readings"));
        assert_eq!(result.available_sheets, strings(&["Readings", "Notes"]));
        assert!(!result.dry_run);
    }

    #[test]
    fn decodes_the_refusals() {
        let status = reqwest::StatusCode::BAD_REQUEST;
        let no_tag = ImportRefusal::parse(
            status,
            r#"{"error":"Could not auto-detect tag/name column. Ensure your sheet has a column named 'name', 'tag', 'sensor', or 'key'.",
            "detected_columns":["Point","Reading"],
            "expected":["name/tag/sensor/key","value","timestamp (optional)","device (optional)"],
            "sheet":"Sheet1","available_sheets":["Sheet1"]}"#,
        );
        assert_eq!(no_tag.status, 400);
        assert_eq!(no_tag.detected_columns, strings(&["Point", "Reading"]));
        assert_eq!(no_tag.available_sheets, strings(&["Sheet1"]));
        let replace = ImportRefusal::parse(
            reqwest::StatusCode::NOT_IMPLEMENTED,
            r#"{"error":"replace mode is not yet supported; use merge"}"#,
        );
        assert_eq!(replace.status, 501);
        assert_eq!(
            replace.error,
            "replace mode is not yet supported; use merge"
        );
        let admin = ImportRefusal::parse(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"error":"admin access required"}"#,
        );
        assert_eq!(admin.error, "admin access required");
        let plain = ImportRefusal::parse(reqwest::StatusCode::PAYLOAD_TOO_LARGE, "too big");
        assert_eq!(plain.error, "HTTP 413 Payload Too Large: too big");
        assert!(plain.detected_columns.is_empty());
    }

    #[test]
    fn decodes_the_node_red_answer() {
        // `import_nodered.rs` for a tab, an MQTT input, a gauge, a function
        // and an unknown node.
        let json = r#"{
            "parse":{"total_nodes":5,"convertible":2,"manual_review":1,"ui_decoration":1,"unsupported":1},
            "conversion":{"modbus_devices":1,"mqtt_sources":1,"widgets":1,"flow_nodes":1,"function_nodes":1,"skipped_nodes":1},
            "details":{
                "modbus_devices":[{"device_id":"m1","name":"NR Modbus m1","ip":"192.168.1.1","port":502,"unit_id":1,"interval_ms":1000,
                    "tags":[{"name":"Read","address":40001,"data_type":"float32","register_area":"holding","scale":1.0}]}],
                "mqtt_sources":[{"topic":"sensor/#","qos":0,"name":"Subscribe","direction":"in","source_node_id":"n2"}],
                "widgets":[{"widget_type":"gauge","name":"Pressure","label":"PSI","min_val":0.0,"max_val":100.0,"source_node_id":"n3","format":null}],
                "flow_nodes":[{"node_type":"delay","name":"Wait","config":{"delay_ms":5000},"source_node_id":"n6"}],
                "function_nodes":[{"node_id":"n4","name":"Scale","source_code":"return msg;","warning":"function node 'Scale' must be reimplemented as a Voltrus flow script or Lua expression"}],
                "skipped_nodes":[{"node_id":"n5","node_type":"rare-node","name":null,"reason":"Unsupported node type: rare-node"}]
            },
            "warnings":["OPC-UA node 'Boiler' detected — configure via Voltrus OPC-UA device manager for full support"],
            "license":{"current_devices":3,"new_devices":1,"limit":null}
        }"#;
        let result: NodeRedImport = serde_json::from_str(json).unwrap();
        assert_eq!(result.parse.total_nodes, 5);
        assert_eq!(result.conversion.skipped_nodes, 1);
        assert_eq!(result.details.modbus_devices[0].tags[0].address, 40001);
        assert_eq!(result.details.mqtt_sources[0].direction, "in");
        assert_eq!(result.details.widgets[0].label.as_deref(), Some("PSI"));
        assert_eq!(result.details.flow_nodes[0].config["delay_ms"], 5000);
        assert_eq!(result.details.skipped_nodes[0].name, None);
        assert_eq!(result.license.limit, None);
        assert!(!result.license.exceeded());
        let limited = NodeRedLicense {
            current_devices: 9,
            new_devices: 2,
            limit: Some(10),
        };
        assert!(limited.exceeded());
    }

    #[test]
    fn previews_flows_with_the_server_categories() {
        let flows = r#"[
            {"id":"t1","type":"tab","label":"Flow 1"},
            {"id":"n1","type":"modbus-read","name":"Read Voltage","server":"m1","fc":3},
            {"id":"n2","type":"function","name":"","func":"return msg;"},
            {"id":"n3","type":"debug"},
            {"id":"n4","type":"rare-node","name":"Weird"},
            {"id":"n5","type":"subflow:abc"}
        ]"#;
        let preview = parse_flows(flows).unwrap();
        assert_eq!(preview.total(), 6);
        assert_eq!(preview.count(NodeCategory::Convertible), 1);
        assert_eq!(preview.count(NodeCategory::ManualReview), 1);
        assert_eq!(preview.count(NodeCategory::UiDecoration), 2);
        // `subflow:<id>` is not the literal `subflow:` the server matches.
        assert_eq!(preview.count(NodeCategory::Unsupported), 2);
        assert_eq!(preview.nodes[2].title(), "n2");
        assert_eq!(preview.nodes[1].title(), "Read Voltage");
        let object = parse_flows(r#"{"nodes":[{"id":"a","type":"mqtt in"}]}"#).unwrap();
        assert_eq!(object.count(NodeCategory::Convertible), 1);
        assert_eq!(parse_flows(r#"{"rev":"1"}"#).unwrap().total(), 0);
        let broken = parse_flows("not json").unwrap_err();
        assert!(broken.starts_with("Failed to parse flows.json"));
        assert!(
            parse_flows(r#"[{"type":"tab"}]"#).is_err(),
            "a node needs an id"
        );
    }

    #[test]
    fn wizard_steps_go_back_one_at_a_time() {
        assert_eq!(DataStep::File.back(), Back::Leave);
        assert_eq!(DataStep::Mapping.back(), Back::To(DataStep::File));
        assert_eq!(DataStep::DryRun.back(), Back::To(DataStep::Mapping));
        assert_eq!(DataStep::Importing.back(), Back::Stay);
        assert_eq!(DataStep::Report.back(), Back::To(DataStep::File));
        assert_eq!(FlowStep::File.back(), Back::Leave);
        assert_eq!(FlowStep::Uploading.back(), Back::Stay);
        assert_eq!(FlowStep::Preview.back(), Back::To(FlowStep::File));
        assert_eq!(FlowStep::Report.back(), Back::To(FlowStep::Preview));
        assert_eq!(DataStep::ALL.len(), 5);
        assert_eq!(FlowStep::Uploading.label(), "Upload");
    }
}
