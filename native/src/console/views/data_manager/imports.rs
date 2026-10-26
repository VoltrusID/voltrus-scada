//! Data Manager › Import: the web client's data import wizard
//! (`DataImportWizard.tsx`, `ImportColumnMapper.tsx`) and Node-RED import
//! wizard (`NodeRedImportWizard.tsx`), switched with W or the header's
//! control.
//!
//! - **Data file** (admin; others see why not): choose a CSV or Excel file
//!   (O, a click, or drop it on the section), map its columns to the tag,
//!   value, timestamp and device fields (a CSV's header and first rows are
//!   read here; an Excel sheet's columns come from the server's dry run),
//!   see the dry run, import, read the report.
//! - **Node-RED** (operator): choose a flows.json, preview its nodes here
//!   with the server's categories, upload it, see what the server converts
//!   it to, read the report.
//!
//! Keys: ↵ next step, Esc a step back (from the first step, back to the
//! sidebar), O choose a file, D the CSV delimiter, W the other wizard; in
//! the mapping, ↑↓ choose a field, ←→ or Space its column, ⌫ unmap it.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, Context, Entity, ExternalPaths, FocusHandle, FontWeight, Hsla,
    KeyDownEvent, PathPromptOptions, SharedString, Subscription, Window, div, px, uniform_list,
};
use voltrus_scada_native::api::eng_imports::{
    Back, ColumnMapping, CsvPreview, DATA_LIMIT, DataStep, Delimiter, FLOWS_LIMIT, Field, FileKind,
    FlowStep, FlowsPreview, ImportOutcome, ImportRequest, ImportResult, Mode, NodeCategory,
    NodeRedImport, PREVIEW_ROWS, auto_detect, column_letters, is_flows_file, parse_csv_bytes,
    parse_flows, read_upload, size_label, too_large,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, Segment, kbd, pill, segmented};
use vt_ui::text_input::{InputEvent, TextInput};
use vt_ui::theme::*;

use crate::console::Console;
use crate::console::keys::{Dismiss, EngImportsDelimiter, EngImportsOpen, EngImportsSwitch};
use crate::console::pages::Role;
use crate::console::views::ops_kit;
use crate::console::views::{
    ListKeys, card, focus_ring, keyboard_list, page_header, rows_on_screen, tail_row, tail_rows,
};

/// A wizard step's footer: the back label, the next label, whether next
/// is enabled, and the key hints.
type StepBar = (
    Option<&'static str>,
    Option<&'static str>,
    bool,
    Vec<(&'static str, &'static str)>,
);

/// The section's id (its calls are `data.imports.*`).
const ID: &str = "imports";
/// The one keyboard list of whichever step shows rows.
const LIST: &str = "data.imports.rows";
/// The key context of the section's root (`keys::ENG_IMPORTS` binds in it).
const CONTEXT: &str = "EngImports";
const ROW_HEIGHT_PX: f32 = 30.0;
const FIELD_ROW: f32 = 52.0;
/// Columns offered for an Excel sheet whose header the server did not send.
const EXCEL_COLUMNS: usize = 8;
const PREVIEW_COLUMN: f32 = 140.0;

/// Request numbers, unique for the console's life: a reply or a file read
/// started before the section's state was dropped (a sign-out) never
/// matches the numbers of a later session.
static SEQ: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn next_seq() -> i64 {
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

/// Whether `role` may import CSV and Excel files (the server's
/// `admin_only_middleware`).
fn may_import_data(role: Option<Role>) -> bool {
    role.is_some_and(|role| role >= Role::Admin)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Wizard {
    #[default]
    Data,
    NodeRed,
}

/// A file the operator chose.
#[derive(Debug, Clone)]
struct Chosen {
    path: PathBuf,
    name: String,
    size: u64,
}

#[derive(Default)]
struct DataWizard {
    step: DataStep,
    file: Option<(Chosen, FileKind)>,
    delimiter: Delimiter,
    /// A CSV's header and first rows.
    preview: Option<CsvPreview>,
    /// The mapping's column names: a CSV's header, an Excel sheet's
    /// columns when the server named them, else letters.
    columns: Vec<String>,
    /// The columns are spreadsheet letters, not header names.
    lettered: bool,
    detected: ColumnMapping,
    mapping: ColumnMapping,
    /// An Excel workbook's sheets, from the server.
    sheets: Vec<String>,
    /// Why the mapping starts as it does.
    note: Option<String>,
    dry: Option<ImportResult>,
    /// The bytes the latest detect or dry run sent: the import sends these
    /// same bytes, never a re-read of a file that may have changed since.
    upload: Arc<OnceLock<Vec<u8>>>,
    report: Option<ImportResult>,
    busy: bool,
    error: Option<String>,
    /// Numbers each request; answers to an older one are dropped.
    seq: i64,
}

impl DataWizard {
    /// Start over with no file, keeping the delimiter.
    fn reset(&mut self) {
        *self = Self {
            delimiter: self.delimiter,
            seq: next_seq(),
            ..Self::default()
        };
    }
}

#[derive(Default)]
struct FlowWizard {
    step: FlowStep,
    file: Option<Chosen>,
    /// The file as read for the local preview, uploaded as it is.
    bytes: Option<Vec<u8>>,
    local: Option<FlowsPreview>,
    result: Option<NodeRedImport>,
    /// The server's conversion, one list row each.
    rows: Vec<PreviewRow>,
    busy: bool,
    error: Option<String>,
    seq: i64,
}

impl FlowWizard {
    fn reset(&mut self) {
        *self = Self {
            seq: next_seq(),
            ..Self::default()
        };
    }
}

#[derive(Default)]
struct State {
    wizard: Wizard,
    focus: Option<FocusHandle>,
    window: Option<AnyWindowHandle>,
    sheet: Option<Entity<TextInput>>,
    _sheet: Option<Subscription>,
    data: DataWizard,
    flows: FlowWizard,
    /// The open panel being shown; its answer is dropped if another
    /// started since, or the state was dropped (a sign-out).
    prompt: i64,
}

/// How a row's last cell reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Plain,
    Warn,
    Danger,
}

/// A row of the Node-RED preview and report lists.
#[derive(Debug, Clone, PartialEq)]
enum PreviewRow {
    Heading(SharedString),
    Columns(&'static [&'static str]),
    Cells(Vec<SharedString>, Tone),
    Warning(SharedString),
}

/// The server's conversion as list rows, in the web's order: warnings,
/// Modbus devices, MQTT sources, widgets, function nodes, skipped nodes.
fn preview_rows(result: &NodeRedImport) -> Vec<PreviewRow> {
    let mut rows = warning_rows(&result.warnings);
    let details = &result.details;
    if !details.modbus_devices.is_empty() {
        rows.push(heading("Modbus devices", details.modbus_devices.len()));
        rows.push(PreviewRow::Columns(&[
            "Name", "IP", "Port", "Unit ID", "Tags",
        ]));
        rows.extend(details.modbus_devices.iter().map(|device| {
            PreviewRow::Cells(
                vec![
                    device.name.clone().into(),
                    device.ip.clone().into(),
                    device.port.to_string().into(),
                    device.unit_id.to_string().into(),
                    device.tags.len().to_string().into(),
                ],
                Tone::Plain,
            )
        }));
    }
    if !details.mqtt_sources.is_empty() {
        rows.push(heading("MQTT sources", details.mqtt_sources.len()));
        rows.push(PreviewRow::Columns(&["Name", "Topic", "QoS", "Direction"]));
        rows.extend(details.mqtt_sources.iter().map(|source| {
            PreviewRow::Cells(
                vec![
                    source.name.clone().into(),
                    source.topic.clone().into(),
                    source.qos.to_string().into(),
                    source.direction.clone().into(),
                ],
                Tone::Plain,
            )
        }));
    }
    if !details.widgets.is_empty() {
        rows.push(heading("Widgets", details.widgets.len()));
        rows.push(PreviewRow::Columns(&["Type", "Name", "Label"]));
        rows.extend(details.widgets.iter().map(|widget| {
            PreviewRow::Cells(
                vec![
                    widget.widget_type.clone().into(),
                    widget.name.clone().into(),
                    widget
                        .label
                        .clone()
                        .filter(|label| !label.is_empty())
                        .unwrap_or_else(|| "-".to_owned())
                        .into(),
                ],
                Tone::Plain,
            )
        }));
    }
    if !details.function_nodes.is_empty() {
        rows.push(heading("Function nodes", details.function_nodes.len()));
        rows.push(PreviewRow::Columns(&["Name", "Warning"]));
        rows.extend(details.function_nodes.iter().map(|node| {
            PreviewRow::Cells(
                vec![node.name.clone().into(), node.warning.clone().into()],
                Tone::Warn,
            )
        }));
    }
    if !details.skipped_nodes.is_empty() {
        rows.push(heading("Skipped nodes", details.skipped_nodes.len()));
        rows.push(PreviewRow::Columns(&["Type", "Name", "Reason"]));
        rows.extend(details.skipped_nodes.iter().map(|node| {
            PreviewRow::Cells(
                vec![
                    node.node_type.clone().into(),
                    node.name
                        .clone()
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "-".to_owned())
                        .into(),
                    node.reason.clone().into(),
                ],
                Tone::Danger,
            )
        }));
    }
    rows
}

fn heading(title: &str, count: usize) -> PreviewRow {
    PreviewRow::Heading(format!("{title} · {count}").into())
}

/// How many rows the warnings take: a heading, then one each.
fn warning_count(warnings: &[String]) -> usize {
    if warnings.is_empty() {
        0
    } else {
        warnings.len() + 1
    }
}

/// Row `row` of the warnings: the heading, then one each.
fn warning_row(warnings: &[String], row: usize) -> Option<PreviewRow> {
    match row {
        _ if warnings.is_empty() => None,
        0 => Some(heading("Warnings", warnings.len())),
        row => warnings
            .get(row - 1)
            .map(|warning| PreviewRow::Warning(warning.clone().into())),
    }
}

fn warning_rows(warnings: &[String]) -> Vec<PreviewRow> {
    (0..warning_count(warnings))
        .filter_map(|row| warning_row(warnings, row))
        .collect()
}

/// The mapping's columns for an Excel sheet the server auto-detected:
/// letters, as many as the detected columns reach and at least a few.
fn excel_columns(detected: &ColumnMapping) -> Vec<String> {
    let reach = Field::ALL
        .into_iter()
        .filter_map(|field| detected.get(field))
        .max()
        .map_or(0, |column| column + 1);
    (0..reach.max(EXCEL_COLUMNS)).map(column_letters).collect()
}

/// `2 · value` (a CSV header), `C` (an Excel letter), or "not mapped".
fn column_label(columns: &[String], lettered: bool, column: Option<usize>) -> String {
    match column {
        None => "— not mapped —".to_owned(),
        Some(column) => match columns.get(column) {
            Some(name) if lettered => name.clone(),
            Some(name) => format!("{column} · {name}"),
            None => format!("column {column} (not in the file)"),
        },
    }
}

// ── Hooks ────────────────────────────────────────────────────────────────

/// Nothing to fetch: the wizards start from a file.
pub fn on_open(_: &mut Console, _: &mut Context<Console>) {}

pub fn search(_: &Console) -> Option<Entity<TextInput>> {
    None
}

fn ensure_focus(console: &mut Console, cx: &mut Context<Console>) -> FocusHandle {
    if let Some(focus) = console.state::<State>().focus.clone() {
        return focus;
    }
    let focus = cx.focus_handle();
    console.state::<State>().focus = Some(focus.clone());
    focus
}

/// How many list rows the step on screen shows.
fn rows(console: &Console) -> usize {
    let Some(state) = console.state_ref::<State>() else {
        return 0;
    };
    match state.wizard {
        Wizard::Data if !may_import_data(console.role()) => 0,
        Wizard::Data => match state.data.step {
            DataStep::Mapping => Field::ALL.len(),
            DataStep::DryRun => state.data.dry.as_ref().map_or(0, |dry| dry.errors.len()),
            DataStep::Report => state
                .data
                .report
                .as_ref()
                .map_or(0, |report| report.errors.len()),
            DataStep::File | DataStep::Importing => 0,
        },
        Wizard::NodeRed => match state.flows.step {
            FlowStep::File => state.flows.local.as_ref().map_or(0, FlowsPreview::total),
            FlowStep::Preview => state.flows.rows.len(),
            FlowStep::Report => state
                .flows
                .result
                .as_ref()
                .map_or(0, |result| warning_count(&result.warnings)),
            FlowStep::Uploading => 0,
        },
    }
}

/// Focus the step's list when it shows rows, else the section itself (so
/// ↵ and Esc reach the wizard).
pub fn focus(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let root = ensure_focus(console, cx);
    console.state::<State>().window = Some(window.window_handle());
    if rows(console) > 0 {
        console.focus_list(LIST, window, cx);
    } else {
        window.focus(&root);
    }
    cx.notify();
}

/// After a step change from code without a window (a reply, a file
/// read): focus again, if focus is still in the section.
fn refocus(console: &mut Console, cx: &mut Context<Console>) {
    let state = console.state::<State>();
    let (Some(handle), Some(root)) = (state.window, state.focus.clone()) else {
        return;
    };
    let entity = cx.entity();
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            entity.update(cx, |console, cx| {
                let inside = root.is_focused(window) || root.contains_focused(window, cx);
                if inside && super::current_id(console) == Some(ID) {
                    focus(console, window, cx);
                }
            });
        });
    });
}

fn ensure_sheet(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    if console.state::<State>().sheet.is_some() {
        return;
    }
    let field = ops_kit::text_field("Sheet1", true, cx);
    let subscription = ops_kit::on_field(&field, on_sheet, window, cx);
    let state = console.state::<State>();
    state.sheet = Some(field);
    state._sheet = Some(subscription);
}

fn on_sheet(
    console: &mut Console,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    match event {
        InputEvent::Submitted => next(console, cx),
        InputEvent::Cancelled => focus(console, window, cx),
        InputEvent::Changed | InputEvent::Blurred => {}
    }
}

fn sheet_value(console: &Console, cx: &gpui::App) -> String {
    console
        .state_ref::<State>()
        .and_then(|state| state.sheet.as_ref())
        .map(|field| ops_kit::value(field, cx))
        .unwrap_or_default()
}

fn set_sheet(console: &mut Console, sheet: &str, cx: &mut Context<Console>) {
    if let Some(field) = console.state::<State>().sheet.clone() {
        let sheet = sheet.to_owned();
        field.update(cx, |field, cx| field.set_text(sheet, cx));
    }
}

// ── Actions ──────────────────────────────────────────────────────────────

fn switch(console: &mut Console, wizard: Wizard, cx: &mut Context<Console>) {
    console.state::<State>().wizard = wizard;
    console.list(LIST, cx).cursor = None;
    refocus(console, cx);
    cx.notify();
}

/// O, a click on the drop zone: the system's open panel.
fn choose_file(console: &mut Console, cx: &mut Context<Console>) {
    let wizard = console.state::<State>().wizard;
    if wizard == Wizard::Data && !may_import_data(console.role()) {
        return;
    }
    if busy(console) {
        return;
    }
    let token = next_seq();
    console.state::<State>().prompt = token;
    let prompt = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(match wizard {
            Wizard::Data => "Choose".into(),
            Wizard::NodeRed => "Preview".into(),
        }),
    });
    cx.spawn(async move |this, cx| {
        let chosen = prompt.await;
        let _ = this.update(cx, |console, cx| {
            // Signed out (the state was dropped) or another panel since.
            if console.state::<State>().prompt != token {
                return;
            }
            match chosen {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        picked(console, wizard, path, cx);
                    }
                }
                Ok(Err(error)) => {
                    console.show_toast(format!("No open panel: {error}"), true, cx);
                }
                _ => {}
            }
        });
    })
    .detach();
}

/// The open panel's answer: for the wizard that asked, not whichever
/// shows now (the operator may have switched while the panel was open).
fn picked(console: &mut Console, asked: Wizard, path: PathBuf, cx: &mut Context<Console>) {
    if console.state::<State>().wizard != asked {
        let name = match asked {
            Wizard::Data => "data",
            Wizard::NodeRed => "Node-RED",
        };
        console.show_toast(
            format!("The file was chosen for the {name} import: open it there again"),
            true,
            cx,
        );
        return;
    }
    accept_path(console, path, cx);
}

/// A request of either wizard is in flight: a new file would drop its
/// answer (a report lost, an import that can run again).
fn busy(console: &mut Console) -> bool {
    let state = console.state::<State>();
    state.data.busy || state.flows.busy
}

/// A chosen or dropped file: the wizard on screen starts over with it.
fn accept_path(console: &mut Console, path: PathBuf, cx: &mut Context<Console>) {
    if busy(console) {
        console.show_toast("Wait for the current import step to finish", true, cx);
        return;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size = std::fs::metadata(&path).map(|meta| meta.len());
    let wizard = console.state::<State>().wizard;
    match wizard {
        Wizard::Data => {
            if !may_import_data(console.role()) {
                console.show_toast("CSV and Excel imports are admin-only", true, cx);
                return;
            }
            set_sheet(console, "", cx);
            let data = &mut console.state::<State>().data;
            data.reset();
            let Some(kind) = FileKind::of(&name) else {
                data.error =
                    Some("Choose a CSV or Excel file (.csv, .xlsx, .xls, .xlsm, .xlsb).".into());
                cx.notify();
                return;
            };
            match size {
                Err(error) => data.error = Some(format!("Could not read {name}: {error}")),
                Ok(size) => match too_large(size, DATA_LIMIT) {
                    Some(problem) => data.error = Some(format!("{name}: {problem}")),
                    None => data.file = Some((Chosen { path, name, size }, kind)),
                },
            }
        }
        Wizard::NodeRed => {
            let flows = &mut console.state::<State>().flows;
            flows.reset();
            if !is_flows_file(&name) {
                flows.error = Some("Choose a Node-RED flows file (.json).".into());
                cx.notify();
                return;
            }
            match size {
                Err(error) => flows.error = Some(format!("Could not read {name}: {error}")),
                Ok(size) => match too_large(size, FLOWS_LIMIT) {
                    Some(problem) => flows.error = Some(format!("{name}: {problem}")),
                    None => {
                        flows.file = Some(Chosen {
                            path: path.clone(),
                            name,
                            size,
                        });
                        flows.busy = true;
                        read_flows(console, path, cx);
                    }
                },
            }
        }
    }
    refocus(console, cx);
    cx.notify();
}

/// Read and preview a flows.json off the UI thread.
fn read_flows(console: &mut Console, path: PathBuf, cx: &mut Context<Console>) {
    let seq = console.state::<State>().flows.seq;
    cx.spawn(async move |this, cx| {
        let read = cx
            .background_executor()
            .spawn(async move {
                let (_, bytes) =
                    read_upload(&path, FLOWS_LIMIT).map_err(|error| error.to_string())?;
                let preview = parse_flows(&String::from_utf8_lossy(&bytes));
                Ok::<_, String>((bytes, preview))
            })
            .await;
        let _ = this.update(cx, |console, cx| {
            let flows = &mut console.state::<State>().flows;
            if flows.seq != seq {
                return;
            }
            flows.busy = false;
            match read {
                Ok((bytes, Ok(preview))) => {
                    flows.bytes = Some(bytes);
                    flows.local = Some(preview);
                }
                Ok((_, Err(error))) | Err(error) => flows.error = Some(error),
            }
            refocus(console, cx);
            cx.notify();
        });
    })
    .detach();
}

/// Read a CSV's header and first rows off the UI thread, then map.
fn read_csv(console: &mut Console, cx: &mut Context<Console>) {
    let data = &mut console.state::<State>().data;
    let Some((chosen, _)) = &data.file else {
        return;
    };
    let path = chosen.path.clone();
    let delimiter = data.delimiter.byte();
    data.seq = next_seq();
    data.busy = true;
    data.error = None;
    let seq = data.seq;
    cx.spawn(async move |this, cx| {
        let parsed = cx
            .background_executor()
            .spawn(async move {
                read_upload(&path, DATA_LIMIT)
                    .map(|(_, bytes)| parse_csv_bytes(&bytes, delimiter, PREVIEW_ROWS))
                    .map_err(|error| error.to_string())
            })
            .await;
        let _ = this.update(cx, |console, cx| {
            let data = &mut console.state::<State>().data;
            if data.seq != seq {
                return;
            }
            data.busy = false;
            match parsed {
                Ok(preview)
                    if preview
                        .headers
                        .iter()
                        .all(|header| header.trim().is_empty()) =>
                {
                    data.error = Some("The file has no header row.".into());
                }
                Ok(preview) => {
                    data.detected = auto_detect(&preview.headers);
                    data.mapping = data.detected;
                    data.columns = preview.headers.clone();
                    data.lettered = false;
                    data.note = data.detected.get(Field::TagName).is_none().then(|| {
                        "No column is named like a tag (name, tag, sensor, key…): map one."
                            .to_owned()
                    });
                    data.preview = Some(preview);
                    data.step = DataStep::Mapping;
                }
                Err(error) => data.error = Some(error),
            }
            console.list(LIST, cx).cursor = Some(0);
            refocus(console, cx);
            cx.notify();
        });
    })
    .detach();
}

/// Send the chosen file to the import route: `name` is the call
/// (`detect`: an Excel dry run with the server's detection; `dry`: a dry
/// run with the mapping; `run`: the import).
fn send_data(console: &mut Console, name: &'static str, cx: &mut Context<Console>) {
    let sheet = sheet_value(console, cx);
    let data = &mut console.state::<State>().data;
    let Some((chosen, kind)) = &data.file else {
        return;
    };
    let request = ImportRequest {
        kind: *kind,
        dry_run: name != "data.imports.run",
        mode: Mode::Merge,
        delimiter: data.delimiter,
        sheet,
        mapping: (name != "data.imports.detect").then_some(data.mapping),
    };
    let path = chosen.path.clone();
    let file_name = chosen.name.clone();
    // Detect and dry run read the file and keep its bytes; the import
    // sends exactly those.
    let run = name == "data.imports.run";
    if !run {
        data.upload = Arc::default();
    }
    let upload = data.upload.clone();
    data.seq = next_seq();
    data.busy = true;
    data.error = None;
    let seq = data.seq;
    console.call(
        CallKey::new(name).with(seq),
        Call::new(move |api| async move {
            let bytes = upload_bytes(&upload, &path, run)?;
            api.import_data(&request, &file_name, bytes).await
        }),
    );
}

/// The bytes to send: a detect or dry run reads `path` and keeps them in
/// `upload`; the import takes the kept ones, so it imports exactly what
/// the dry run checked.
fn upload_bytes<'a>(
    upload: &'a OnceLock<Vec<u8>>,
    path: &Path,
    run: bool,
) -> anyhow::Result<&'a [u8]> {
    if run {
        return upload
            .get()
            .map(Vec::as_slice)
            .ok_or_else(|| anyhow::anyhow!("Run the dry run again before importing"));
    }
    let (_, bytes) = read_upload(path, DATA_LIMIT)?;
    Ok(upload.get_or_init(|| bytes))
}

/// ↵, the Next button.
fn next(console: &mut Console, cx: &mut Context<Console>) {
    let wizard = console.state::<State>().wizard;
    match wizard {
        Wizard::Data => next_data(console, cx),
        Wizard::NodeRed => next_flows(console, cx),
    }
    cx.notify();
}

fn next_data(console: &mut Console, cx: &mut Context<Console>) {
    if !may_import_data(console.role()) {
        return;
    }
    let data = &mut console.state::<State>().data;
    if data.busy {
        return;
    }
    match data.step {
        DataStep::File => match data.file.as_ref().map(|(_, kind)| *kind) {
            None => choose_file(console, cx),
            Some(FileKind::Csv) => read_csv(console, cx),
            Some(FileKind::Excel) => send_data(console, "data.imports.detect", cx),
        },
        DataStep::Mapping => match data.mapping.check() {
            Err(problem) => data.error = Some(problem.message()),
            Ok(()) => send_data(console, "data.imports.dry", cx),
        },
        DataStep::DryRun => {
            data.step = DataStep::Importing;
            send_data(console, "data.imports.run", cx);
            refocus(console, cx);
        }
        DataStep::Importing => {}
        DataStep::Report => {
            data.reset();
            set_sheet(console, "", cx);
            refocus(console, cx);
        }
    }
}

fn next_flows(console: &mut Console, cx: &mut Context<Console>) {
    let flows = &mut console.state::<State>().flows;
    if flows.busy {
        return;
    }
    match flows.step {
        FlowStep::File => {
            let (Some(chosen), Some(bytes)) = (&flows.file, &flows.bytes) else {
                if flows.file.is_none() {
                    choose_file(console, cx);
                }
                return;
            };
            if flows.local.is_none() {
                return;
            }
            let (name, bytes) = (chosen.name.clone(), bytes.clone());
            flows.seq = next_seq();
            flows.busy = true;
            flows.error = None;
            flows.step = FlowStep::Uploading;
            let seq = flows.seq;
            console.call(
                CallKey::new("data.imports.nodered").with(seq),
                Call::new(move |api| async move { api.import_node_red(&name, &bytes).await }),
            );
            refocus(console, cx);
        }
        FlowStep::Uploading => {}
        FlowStep::Preview => {
            flows.step = FlowStep::Report;
            console.list(LIST, cx).cursor = None;
            refocus(console, cx);
        }
        FlowStep::Report => {
            flows.reset();
            refocus(console, cx);
        }
    }
}

/// Esc: a step back. False when there is none (the page then goes back to
/// the sidebar).
fn go_back(console: &mut Console, cx: &mut Context<Console>) -> bool {
    let wizard = console.state::<State>().wizard;
    let admin = may_import_data(console.role());
    let handled = match wizard {
        Wizard::Data if !admin => false,
        Wizard::Data => {
            let data = &mut console.state::<State>().data;
            match data.step.back() {
                Back::Leave => false,
                Back::Stay => true,
                Back::To(step) => {
                    if data.step == DataStep::Report {
                        data.reset();
                        set_sheet(console, "", cx);
                    } else {
                        data.step = step;
                        data.busy = false;
                        data.error = None;
                        data.seq = next_seq();
                    }
                    true
                }
            }
        }
        Wizard::NodeRed => {
            let flows = &mut console.state::<State>().flows;
            match flows.step.back() {
                Back::Leave => false,
                Back::Stay => true,
                Back::To(step) => {
                    if step == FlowStep::File {
                        flows.result = None;
                        flows.rows.clear();
                        flows.error = None;
                    }
                    flows.step = step;
                    true
                }
            }
        }
    };
    if handled {
        console.list(LIST, cx).cursor = None;
        refocus(console, cx);
        cx.notify();
    }
    handled
}

fn mapping_cursor(console: &mut Console, cx: &mut Context<Console>) -> Option<Field> {
    let state = console.state_ref::<State>()?;
    if state.wizard != Wizard::Data || state.data.step != DataStep::Mapping {
        return None;
    }
    let row = console.list(LIST, cx).cursor?;
    Field::ALL.get(row).copied()
}

/// ←→, Space, the arrows beside a field: its next or previous column.
fn cycle(console: &mut Console, field: Field, forward: bool, cx: &mut Context<Console>) {
    let data = &mut console.state::<State>().data;
    let columns = data.columns.len();
    data.mapping.cycle(field, columns, forward);
    data.error = None;
    cx.notify();
}

fn assign(console: &mut Console, field: Field, column: Option<usize>, cx: &mut Context<Console>) {
    let data = &mut console.state::<State>().data;
    data.mapping.set(field, column);
    data.error = None;
    cx.notify();
}

/// Enter (a row's) or Space in the list: only while the list itself has
/// focus, not a button inside it.
fn activate(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !console.list(LIST, cx).focus.is_focused(window) {
        return;
    }
    if toggle {
        if let Some(field) = mapping_cursor(console, cx).filter(|_| Field::ALL.get(row).is_some()) {
            cycle(console, field, true, cx);
        }
        return;
    }
    next(console, cx);
}

/// ↵ on the section itself (not on a focused button or field); ←→ and ⌫
/// on the mapping list itself.
fn on_key(
    console: &mut Console,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let keystroke = &event.keystroke;
    if keystroke.modifiers.modified() {
        return;
    }
    let on_root = console
        .state_ref::<State>()
        .and_then(|state| state.focus.as_ref())
        .is_some_and(|focus| focus.is_focused(window));
    let on_list = console.list(LIST, cx).focus.is_focused(window);
    match keystroke.key.as_str() {
        "enter" if on_root => {
            next(console, cx);
            cx.stop_propagation();
        }
        "left" | "right" if on_list => {
            if let Some(field) = mapping_cursor(console, cx) {
                cycle(console, field, keystroke.key == "right", cx);
                cx.stop_propagation();
            }
        }
        "backspace" | "delete" if on_list => {
            if let Some(field) = mapping_cursor(console, cx) {
                assign(console, field, None, cx);
                cx.stop_propagation();
            }
        }
        _ => {}
    }
}

fn cycle_delimiter(console: &mut Console, cx: &mut Context<Console>) {
    let data = &mut console.state::<State>().data;
    if data.step == DataStep::File {
        data.delimiter = data.delimiter.next();
        cx.notify();
    }
}

// ── Replies ──────────────────────────────────────────────────────────────

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        "data.imports.detect" | "data.imports.dry" | "data.imports.run" => {
            data_reply(console, key, reply, cx);
        }
        "data.imports.nodered" => flows_reply(console, key, reply, cx),
        _ => {
            if let Some(error) = reply.error() {
                console.show_toast(format!("{} failed: {error}", key.name), true, cx);
            }
        }
    }
    cx.notify();
}

fn data_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let data = &mut console.state::<State>().data;
    if key.arg != data.seq {
        return;
    }
    data.busy = false;
    let outcome = match reply.take::<ImportOutcome>() {
        Ok(outcome) => outcome,
        Err(error) => {
            data.error = Some(error);
            if data.step == DataStep::Importing {
                data.step = DataStep::DryRun;
            }
            refocus(console, cx);
            return;
        }
    };
    let mut sheet = None;
    match (key.name, outcome) {
        ("data.imports.detect", ImportOutcome::Done(result)) => {
            data.detected = ColumnMapping::from_server(&result.column_mapping);
            data.mapping = data.detected;
            data.columns = excel_columns(&data.detected);
            data.lettered = true;
            data.sheets = result.available_sheets;
            data.note = Some(
                "The server does not send an Excel sheet's header names: columns are by letter."
                    .into(),
            );
            data.preview = None;
            data.step = DataStep::Mapping;
            sheet = result.sheet;
        }
        ("data.imports.detect", ImportOutcome::Refused(refusal))
            if !refusal.detected_columns.is_empty() =>
        {
            data.detected = ColumnMapping::default();
            data.mapping = data.detected;
            data.columns = refusal.detected_columns;
            data.lettered = false;
            data.sheets = refusal.available_sheets;
            data.note = Some(refusal.error);
            data.preview = None;
            data.step = DataStep::Mapping;
            sheet = refusal.sheet;
        }
        (_, ImportOutcome::Refused(refusal)) => {
            if !refusal.available_sheets.is_empty() {
                data.sheets = refusal.available_sheets;
            }
            data.error = Some(refusal.error);
            if data.step == DataStep::Importing {
                data.step = DataStep::DryRun;
            }
        }
        ("data.imports.dry", ImportOutcome::Done(result)) => {
            data.dry = Some(result);
            data.step = DataStep::DryRun;
        }
        (_, ImportOutcome::Done(result)) => {
            let message = format!("Imported {} of {} rows", result.imported, result.total_rows);
            let failed = result.imported == 0 && result.total_rows > 0;
            data.report = Some(result);
            data.step = DataStep::Report;
            console.show_toast(message, failed, cx);
        }
    }
    if let Some(sheet) = sheet {
        set_sheet(console, &sheet, cx);
    }
    console.list(LIST, cx).cursor = None;
    refocus(console, cx);
}

fn flows_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    let flows = &mut console.state::<State>().flows;
    if key.arg != flows.seq {
        return;
    }
    flows.busy = false;
    match reply.take::<NodeRedImport>() {
        Ok(result) => {
            flows.rows = preview_rows(&result);
            flows.result = Some(result);
            flows.step = FlowStep::Preview;
        }
        Err(error) => {
            flows.error = Some(error);
            flows.step = FlowStep::File;
        }
    }
    console.list(LIST, cx).cursor = None;
    refocus(console, cx);
}

// ── Drawing ──────────────────────────────────────────────────────────────

/// The list's cursor and focus, for rows.
#[derive(Clone, Copy)]
struct Look {
    cursor: Option<usize>,
    focused: bool,
}

fn steps_bar(labels: &[&'static str], active: usize, theme: &Theme) -> AnyElement {
    let mut bar = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .px(SPACE_9)
        .py(SPACE_5)
        .border_b_1()
        .border_color(theme.border);
    for (index, label) in labels.iter().enumerate() {
        let done = index < active;
        let current = index == active;
        let color = if done {
            theme.success
        } else if current {
            theme.accent
        } else {
            theme.fg_dim
        };
        if index > 0 {
            bar = bar.child(vt_ui::icon(Icon::ChevronRight, ICON_SM, theme.fg_dim));
        }
        bar = bar.child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_3)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(CONTROL_SM)
                        .rounded(RADIUS_XL)
                        .border_1()
                        .border_color(color)
                        .when(current, |this| this.bg(theme.accent_soft))
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(color)
                        .map(|this| {
                            if done {
                                this.child(vt_ui::icon(Icon::Check, ICON_SM, color))
                            } else {
                                this.child((index + 1).to_string())
                            }
                        }),
                )
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .font_weight(if current {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if current || done {
                            theme.fg
                        } else {
                            theme.fg_dim
                        })
                        .child(*label),
                ),
        );
    }
    bar.into_any_element()
}

fn stat(label: &'static str, value: usize, color: Hsla, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(SPACE_2)
        .flex_1()
        .min_w_0()
        .p(SPACE_6)
        .rounded(RADIUS_LG)
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_DISPLAY)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(value.to_string()),
        )
        .child(
            div()
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .child(label.to_uppercase()),
        )
        .into_any_element()
}

fn line(text: impl Into<SharedString>, icon: Icon, color: Hsla) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_3)
        .text_size(FONT_BODY)
        .text_color(color)
        .child(vt_ui::icon(icon, ICON_MD, color))
        .child(div().flex_1().min_w_0().child(text.into()))
        .into_any_element()
}

fn error_line(error: Option<&String>, theme: &Theme) -> Option<AnyElement> {
    error.map(|error| line(error.clone(), Icon::CircleAlert, theme.danger))
}

fn row_shell(
    row: usize,
    look: Look,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> gpui::Stateful<gpui::Div> {
    let hover = theme.hover;
    div()
        .id(("data-imports-row", row))
        .relative()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(ROW_HEIGHT_PX))
        .px(SPACE_9)
        .text_size(FONT_BODY)
        .hover(move |style| style.bg(hover))
        .on_click(cx.listener(move |console, _, _, cx| {
            console.list(LIST, cx).select(row);
            cx.notify();
        }))
        .when(look.focused && look.cursor == Some(row), |this| {
            this.child(focus_ring(theme))
        })
}

/// The rows of the step on screen, virtualised.
fn rows_list(
    len: usize,
    look: Look,
    console: &mut Console,
    cx: &mut Context<Console>,
) -> AnyElement {
    let scroll = console.list(LIST, cx).scroll.clone();
    uniform_list(
        "data-imports-rows",
        len + tail_rows(len, ROW_HEIGHT_PX),
        cx.processor(
            move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                let theme = theme(cx).clone();
                range
                    .map(|row| {
                        if row >= len {
                            return tail_row(ROW_HEIGHT_PX);
                        }
                        list_row(console, row, look, &theme, cx)
                    })
                    .collect::<Vec<_>>()
            },
        ),
    )
    .track_scroll(scroll)
    .size_full()
    .into_any_element()
}

fn list_row(
    console: &Console,
    row: usize,
    look: Look,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let Some(state) = console.state_ref::<State>() else {
        return tail_row(ROW_HEIGHT_PX);
    };
    let shell = row_shell(row, look, theme, cx);
    match state.wizard {
        Wizard::Data => {
            let result = match state.data.step {
                DataStep::DryRun => state.data.dry.as_ref(),
                _ => state.data.report.as_ref(),
            };
            match result.and_then(|result| result.errors.get(row)) {
                Some(error) => shell
                    .text_color(theme.danger)
                    .child(vt_ui::icon(Icon::CircleX, ICON_SM, theme.danger))
                    .child(div().flex_1().min_w_0().truncate().child(error.line()))
                    .into_any_element(),
                None => tail_row(ROW_HEIGHT_PX),
            }
        }
        Wizard::NodeRed => match state.flows.step {
            FlowStep::File => match state
                .flows
                .local
                .as_ref()
                .and_then(|local| local.nodes.get(row))
            {
                Some(node) => {
                    let category = node.category();
                    shell
                        .child(
                            div()
                                .w(px(180.0))
                                .flex_none()
                                .truncate()
                                .font_family(MONO)
                                .text_color(theme.fg_muted)
                                .child(node.node_type.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(node.title().to_owned()),
                        )
                        .child(pill(category.label(), category_color(category, theme)))
                        .into_any_element()
                }
                None => tail_row(ROW_HEIGHT_PX),
            },
            FlowStep::Preview => match state.flows.rows.get(row) {
                Some(preview) => preview_row(shell, preview, theme),
                None => tail_row(ROW_HEIGHT_PX),
            },
            FlowStep::Report => match state
                .flows
                .result
                .as_ref()
                .and_then(|result| warning_row(&result.warnings, row))
            {
                Some(preview) => preview_row(shell, &preview, theme),
                None => tail_row(ROW_HEIGHT_PX),
            },
            FlowStep::Uploading => tail_row(ROW_HEIGHT_PX),
        },
    }
}

fn category_color(category: NodeCategory, theme: &Theme) -> Hsla {
    match category {
        NodeCategory::Convertible => theme.success,
        NodeCategory::ManualReview => theme.warning,
        NodeCategory::UiDecoration => theme.fg_dim,
        NodeCategory::Unsupported => theme.danger,
    }
}

fn preview_row(shell: gpui::Stateful<gpui::Div>, row: &PreviewRow, theme: &Theme) -> AnyElement {
    match row {
        PreviewRow::Heading(title) => shell
            .pt(SPACE_4)
            .text_size(FONT_CAPTION)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.fg_muted)
            .child(title.to_uppercase())
            .into_any_element(),
        PreviewRow::Columns(columns) => shell
            .bg(theme.surface)
            .text_size(FONT_CAPTION)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.fg_dim)
            .children(columns.iter().map(|column| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(column.to_uppercase())
            }))
            .into_any_element(),
        PreviewRow::Cells(cells, tone) => {
            let last = cells.len().saturating_sub(1);
            shell
                .children(cells.iter().enumerate().map(|(index, cell)| {
                    let color = match (index == last, tone) {
                        (true, Tone::Warn) => theme.warning,
                        (true, Tone::Danger) => theme.danger,
                        _ => theme.fg,
                    };
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO)
                        .text_color(color)
                        .child(cell.clone())
                }))
                .into_any_element()
        }
        PreviewRow::Warning(text) => shell
            .text_color(theme.warning)
            .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.warning))
            .child(div().flex_1().min_w_0().truncate().child(text.clone()))
            .into_any_element(),
    }
}

/// The drop zone: a click opens the panel; a file dragged over lights it
/// (the drop itself lands on the section).
fn drop_zone(
    title: &'static str,
    detail: &'static str,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let (accent, soft, hover) = (theme.accent, theme.accent_soft, theme.hover);
    div()
        .id("data-imports-drop")
        .flex()
        .flex_col()
        .items_center()
        .gap(SPACE_3)
        .p(SPACE_10)
        .rounded(RADIUS_LG)
        .border_2()
        .border_dashed()
        .border_color(theme.border_strong)
        .bg(theme.surface)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(accent).bg(soft))
        .on_click(cx.listener(|console, _, _, cx| choose_file(console, cx)))
        .child(vt_ui::icon(Icon::Upload, ICON_LG, theme.fg_dim))
        .child(
            div()
                .text_size(FONT_EMPHASIS)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(detail),
        )
        .into_any_element()
}

fn file_line(chosen: &Chosen, kind: &'static str, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_4)
        .px(SPACE_6)
        .py(SPACE_4)
        .rounded(RADIUS_MD)
        .bg(theme.surface)
        .border_1()
        .border_color(theme.border)
        .child(vt_ui::icon(Icon::File, ICON_MD, theme.fg_muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(chosen.name.clone()),
        )
        .child(pill(kind, theme.accent))
        .child(
            div()
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(size_label(chosen.size)),
        )
        .into_any_element()
}

/// A padded column for a step's form content.
fn form(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(SPACE_7)
        .px(SPACE_9)
        .py(SPACE_7)
        .max_w(px(760.0))
        .text_color(theme.fg)
}

fn data_file_step(console: &mut Console, theme: &Theme, cx: &mut Context<Console>) -> AnyElement {
    let sheet_field = console.state::<State>().sheet.clone();
    let data = &console.state::<State>().data;
    let kind = data.file.as_ref().map(|(_, kind)| *kind);
    let file = data
        .file
        .as_ref()
        .map(|(chosen, kind)| file_line(chosen, kind.label(), theme));
    let delimiter = data.delimiter;
    let busy = data.busy.then(|| {
        line(
            match kind {
                Some(FileKind::Excel) => "Asking the server for the sheet's columns…",
                _ => "Reading the file…",
            },
            Icon::Clock,
            theme.fg_muted,
        )
    });
    let error = error_line(data.error.as_ref(), theme);
    let mode = div()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .child(segmented(
            "data-imports-mode",
            Mode::ALL
                .into_iter()
                .filter(|mode| mode.supported())
                .map(|mode| Segment::new(mode.label(), true, |_, _, _| {}))
                .collect(),
            cx,
        ))
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_dim)
                .child(format!(
                    "{}: not supported by the server yet (it answers 501)",
                    Mode::Replace.label()
                )),
        );
    let option = match kind {
        Some(FileKind::Excel) => Some(
            ops_kit::labelled(
                "Sheet (blank = first)",
                div().w(px(240.0)).children(sheet_field),
                theme,
            )
            .into_any_element(),
        ),
        Some(FileKind::Csv) => Some(
            ops_kit::labelled(
                "Delimiter  (D)",
                segmented(
                    "data-imports-delimiter",
                    Delimiter::ALL
                        .into_iter()
                        .map(|choice| {
                            Segment::new(
                                choice.label(),
                                choice == delimiter,
                                cx.listener(move |console, _, _, cx| {
                                    console.state::<State>().data.delimiter = choice;
                                    cx.notify();
                                }),
                            )
                        })
                        .collect(),
                    cx,
                ),
                theme,
            )
            .into_any_element(),
        ),
        None => None,
    };
    form(theme)
        .child(drop_zone(
            "Drop a CSV or Excel file here, or press O to browse",
            "Supports .csv, .xlsx, .xls, .xlsm and .xlsb files up to 50 MB",
            theme,
            cx,
        ))
        .children(file)
        .child(ops_kit::labelled("Mode", mode, theme))
        .children(option)
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(
                    "Each row becomes one reading in the history: the tag name is the metric, \
                     the device defaults to \"default\", the time to now. Nothing else is created.",
                ),
        )
        .children(busy)
        .children(error)
        .into_any_element()
}

fn admin_only(theme: &Theme) -> AnyElement {
    form(theme)
        .child(
            card(theme)
                .p(SPACE_8)
                .gap(SPACE_4)
                .child(line(
                    "CSV and Excel imports are admin-only",
                    Icon::Lock,
                    theme.warning,
                ))
                .child(div().text_color(theme.fg_muted).child(
                    "The server's /api/v1/import routes accept administrators only. Ask an \
                     administrator to import the file, or convert Node-RED flows here (W).",
                )),
        )
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
fn mapping_step(
    console: &mut Console,
    window: &mut Window,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate,
        },
        Field::ALL.len(),
        Field::ALL.len(),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let look = Look {
        cursor: nav.cursor,
        focused: nav.focus.is_focused(window),
    };
    let current_sheet = sheet_value(console, cx);
    let data = &console.state::<State>().data;
    let mapping = data.mapping;
    let detected = data.detected;
    let columns = data.columns.clone();
    let lettered = data.lettered;
    let sheets = data.sheets.clone();
    let preview = data.preview.clone();
    let note = data.note.clone();
    let busy = data.busy;
    let error = error_line(data.error.as_ref(), theme);
    let tag_mapped = mapping.get(Field::TagName).is_some();

    let rows = Field::ALL.into_iter().enumerate().map(|(row, field)| {
        let column = mapping.get(field);
        let conflict = mapping.conflict(field);
        let auto = column.is_some() && detected.get(field) == column;
        let border = if conflict.is_some() {
            theme.danger
        } else if auto {
            theme.accent
        } else {
            theme.border
        };
        let status: AnyElement = match (conflict, field.required()) {
            (Some(other), _) => div()
                .text_color(theme.danger)
                .child(format!("Already used by {}", other.label()))
                .into_any_element(),
            (None, true) => pill(
                if tag_mapped { "SET" } else { "MISSING" },
                if tag_mapped {
                    theme.success
                } else {
                    theme.danger
                },
            )
            .into_any_element(),
            (None, false) if auto => div()
                .text_color(theme.accent)
                .child("Auto-detected")
                .into_any_element(),
            (None, false) => div()
                .text_color(theme.fg_dim)
                .child("Optional")
                .into_any_element(),
        };
        let hover = theme.hover;
        div()
            .id(("data-imports-field", row))
            .relative()
            .flex()
            .items_center()
            .gap(SPACE_6)
            .h(px(FIELD_ROW))
            .px(SPACE_7)
            .border_b_1()
            .border_color(theme.border)
            .hover(move |style| style.bg(hover))
            .on_click(cx.listener(move |console, _, _, cx| {
                console.list(LIST, cx).select(row);
                cx.notify();
            }))
            .when(look.focused && look.cursor == Some(row), |this| {
                this.child(focus_ring(theme))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(220.0))
                    .flex_none()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(field.label()))
                    .child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .truncate()
                            .child(field.description()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .flex_1()
                    .min_w_0()
                    .child(
                        IconButton::new(("data-imports-previous", row), Icon::ChevronLeft)
                            .tooltip("Previous column  ←")
                            .on_click(cx.listener(move |console, _, _, cx| {
                                console.list(LIST, cx).select(row);
                                cycle(console, field, false, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .h(CONTROL_MD)
                            .px(SPACE_4)
                            .rounded(RADIUS_MD)
                            .border_1()
                            .border_color(border)
                            .bg(theme.bg)
                            .font_family(MONO)
                            .text_color(if column.is_some() {
                                theme.fg
                            } else {
                                theme.fg_dim
                            })
                            .child(
                                div()
                                    .truncate()
                                    .child(column_label(&columns, lettered, column)),
                            ),
                    )
                    .child(
                        IconButton::new(("data-imports-next", row), Icon::ChevronRight)
                            .tooltip("Next column  →")
                            .on_click(cx.listener(move |console, _, _, cx| {
                                console.list(LIST, cx).select(row);
                                cycle(console, field, true, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .w(px(150.0))
                    .flex_none()
                    .text_size(FONT_SMALL)
                    .child(status),
            )
    });

    let cursor_field = look.cursor.and_then(|row| Field::ALL.get(row).copied());
    let chips = div()
        .flex()
        .flex_wrap()
        .gap(SPACE_2)
        .children(columns.iter().enumerate().map(|(index, name)| {
            let used = Field::ALL
                .into_iter()
                .find(|field| mapping.get(*field) == Some(index));
            let label = if lettered {
                name.clone()
            } else {
                format!("{index}: {name}")
            };
            let hover = theme.hover;
            div()
                .id(("data-imports-column", index))
                .px(SPACE_4)
                .py(SPACE_1)
                .rounded(RADIUS_SM)
                .border_1()
                .border_color(if used.is_some() {
                    theme.accent
                } else {
                    theme.border
                })
                .bg(theme.bg)
                .font_family(MONO)
                .text_size(FONT_SMALL)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .child(match used {
                    Some(field) => format!("{label} → {}", field.key()),
                    None => label,
                })
                .on_click(cx.listener(move |console, _, _, cx| {
                    let field = cursor_field.unwrap_or(Field::TagName);
                    assign(console, field, Some(index), cx);
                }))
        }));

    let sheet_picker = (sheets.len() > 1).then(|| {
        ops_kit::labelled(
            "Sheet",
            segmented(
                "data-imports-sheets",
                sheets
                    .iter()
                    .map(|sheet| {
                        let name = sheet.clone();
                        Segment::new(
                            sheet.clone(),
                            *sheet == current_sheet,
                            cx.listener(move |console, _, _, cx| {
                                set_sheet(console, &name, cx);
                                send_data(console, "data.imports.detect", cx);
                                cx.notify();
                            }),
                        )
                    })
                    .collect(),
                cx,
            ),
            theme,
        )
        .into_any_element()
    });

    let table = preview.map(|preview| preview_table(&preview, &mapping, theme));

    div()
        .id("data-imports-mapping")
        .flex()
        .flex_col()
        .size_full()
        .overflow_y_scroll()
        .child(
            form(theme)
                .max_w(px(960.0))
                .children(note.map(|note| line(note, Icon::Info, theme.fg_muted)))
                .children(sheet_picker)
                .child(card(theme).child(list.flex().flex_col().children(rows)))
                .when(!tag_mapped, |this| {
                    this.child(line(
                        "Tag Name column must be mapped before proceeding with import.",
                        Icon::CircleAlert,
                        theme.danger,
                    ))
                })
                .children(error)
                .when(busy, |this| {
                    this.child(line("Running the dry run…", Icon::Clock, theme.fg_muted))
                })
                .child(ops_kit::labelled(
                    "File columns · click one to map it to the chosen field",
                    chips,
                    theme,
                ))
                .children(table),
        )
        .into_any_element()
}

/// The CSV's header and first rows, mapped columns marked.
fn preview_table(preview: &CsvPreview, mapping: &ColumnMapping, theme: &Theme) -> AnyElement {
    let width = preview
        .rows
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(preview.headers.len()))
        .max()
        .unwrap_or(0);
    let field_of = |column: usize| {
        Field::ALL
            .into_iter()
            .find(|field| mapping.get(*field) == Some(column))
    };
    let cell = |text: String, column: usize, header: bool| {
        let mapped = field_of(column).is_some();
        div()
            .w(px(PREVIEW_COLUMN))
            .flex_none()
            .px(SPACE_4)
            .py(SPACE_2)
            .truncate()
            .font_family(MONO)
            .text_size(FONT_SMALL)
            .when(mapped, |this| this.bg(theme.accent_soft))
            .when(header, |this| this.font_weight(FontWeight::SEMIBOLD))
            .child(text)
    };
    let header = div()
        .flex()
        .border_b_1()
        .border_color(theme.border)
        .children((0..width).map(|column| {
            let name = preview.headers.get(column).cloned().unwrap_or_default();
            let text = match field_of(column) {
                Some(field) => format!("{name} → {}", field.key()),
                None => name,
            };
            cell(text, column, true)
        }));
    let body =
        preview.rows.iter().map(|row| {
            div()
                .flex()
                .border_b_1()
                .border_color(theme.border)
                .children((0..width).map(|column| {
                    cell(row.get(column).cloned().unwrap_or_default(), column, false)
                }))
        });
    ops_kit::labelled(
        "Preview",
        card(theme)
            .child(
                div()
                    .id("data-imports-preview")
                    .overflow_x_scroll()
                    .child(div().flex().flex_col().child(header).children(body)),
            )
            .child(
                div()
                    .px(SPACE_6)
                    .py(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(format!(
                        "First {} of {} rows",
                        preview.rows.len(),
                        preview.total_rows
                    )),
            ),
        theme,
    )
    .into_any_element()
}

fn result_step(
    console: &mut Console,
    window: &mut Window,
    report: bool,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let data = &console.state::<State>().data;
    let result = if report { &data.report } else { &data.dry };
    let Some((total, imported, skipped, len)) = result.as_ref().map(|result| {
        (
            result.total_rows,
            result.imported,
            result.skipped,
            result.errors.len(),
        )
    }) else {
        return div().into_any_element();
    };
    let error = error_line(data.error.as_ref(), theme);
    let busy = data.busy;
    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate,
        },
        len,
        rows_on_screen(window, ROW_HEIGHT_PX),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let look = Look {
        cursor: nav.cursor,
        focused: nav.focus.is_focused(window),
    };
    let stats = div()
        .flex()
        .gap(SPACE_6)
        .child(stat("Total rows", total, theme.fg, theme))
        .child(stat(
            if report { "Imported" } else { "Would import" },
            imported,
            theme.success,
            theme,
        ))
        .child(stat(
            if report { "Skipped" } else { "Would skip" },
            skipped,
            theme.warning,
            theme,
        ));
    let summary = if len == 0 {
        Some(line(
            if report {
                "Import completed successfully with no errors."
            } else {
                "No row would be skipped."
            },
            Icon::CircleCheck,
            theme.success,
        ))
    } else {
        None
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            form(theme)
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(if report {
                            "Import report"
                        } else {
                            "Dry run results"
                        }),
                )
                .child(stats)
                .children(summary)
                .children(error)
                .when(busy, |this| {
                    this.child(line("Importing…", Icon::Clock, theme.fg_muted))
                })
                .when(len > 0, |this| {
                    this.child(
                        div()
                            .text_size(FONT_SMALL)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.danger)
                            .child(format!("Errors ({len})")),
                    )
                }),
        )
        .when(len > 0, |this| {
            this.child(
                list.flex_1()
                    .min_h_0()
                    .child(rows_list(len, look, console, cx)),
            )
        })
        .into_any_element()
}

fn centered(text: &'static str, detail: &'static str, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(SPACE_3)
        .size_full()
        .child(vt_ui::icon(Icon::Clock, ICON_LG, theme.fg_dim))
        .child(
            div()
                .text_size(FONT_EMPHASIS)
                .font_weight(FontWeight::SEMIBOLD)
                .child(text),
        )
        .child(
            div()
                .text_size(FONT_SMALL)
                .text_color(theme.fg_muted)
                .child(detail),
        )
        .into_any_element()
}

fn flows_file_step(
    console: &mut Console,
    window: &mut Window,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let flows = &console.state::<State>().flows;
    let file = flows
        .file
        .as_ref()
        .map(|chosen| file_line(chosen, "flows", theme));
    let busy = flows
        .busy
        .then(|| line("Reading the flows…", Icon::Clock, theme.fg_muted));
    let error = error_line(flows.error.as_ref(), theme);
    let counts = flows.local.as_ref().map(|local| {
        div()
            .flex()
            .gap(SPACE_6)
            .child(stat("Nodes", local.total(), theme.fg, theme))
            .children(NodeCategory::ALL.into_iter().map(|category| {
                stat(
                    category.label(),
                    local.count(category),
                    category_color(category, theme),
                    theme,
                )
            }))
    });
    let len = flows.local.as_ref().map_or(0, FlowsPreview::total);
    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate,
        },
        len,
        rows_on_screen(window, ROW_HEIGHT_PX),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let look = Look {
        cursor: nav.cursor,
        focused: nav.focus.is_focused(window),
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            form(theme)
                .child(drop_zone(
                    "Drop a Node-RED flows.json here, or press O to browse",
                    "Previewed here first; the server then converts it (up to 1 MB) without creating anything",
                    theme,
                    cx,
                ))
                .children(file)
                .children(busy)
                .children(error)
                .children(counts)
                .when(len > 0, |this| {
                    this.child(
                        div()
                            .text_size(FONT_SMALL)
                            .text_color(theme.fg_muted)
                            .child("Nodes in the file · ↵ uploads them for conversion"),
                    )
                }),
        )
        .when(len > 0, |this| {
            this.child(
                list.flex_1()
                    .min_h_0()
                    .child(rows_list(len, look, console, cx)),
            )
        })
        .into_any_element()
}

fn flows_result_step(
    console: &mut Console,
    window: &mut Window,
    report: bool,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let flows = &console.state::<State>().flows;
    let Some(result) = flows.result.as_ref() else {
        return div().into_any_element();
    };
    let (parse, conversion, license) = (
        result.parse.clone(),
        result.conversion.clone(),
        result.license.clone(),
    );
    let file_name = flows
        .file
        .as_ref()
        .map_or_else(|| "-".to_owned(), |chosen| chosen.name.clone());
    let len = if report {
        warning_count(&result.warnings)
    } else {
        flows.rows.len()
    };
    let list = keyboard_list(
        ListKeys {
            id: LIST,
            on_move: None,
            on_activate: activate,
        },
        len,
        rows_on_screen(window, ROW_HEIGHT_PX),
        console,
        cx,
    );
    let nav = console.list(LIST, cx);
    let look = Look {
        cursor: nav.cursor,
        focused: nav.focus.is_focused(window),
    };
    let conversion = &conversion;
    let counts = [
        ("Modbus devices", conversion.modbus_devices),
        ("MQTT sources", conversion.mqtt_sources),
        ("Widgets", conversion.widgets),
        ("Flow nodes", conversion.flow_nodes),
        ("Function nodes", conversion.function_nodes),
        ("Skipped", conversion.skipped_nodes),
    ];
    let count_grid = div()
        .flex()
        .flex_wrap()
        .gap(SPACE_4)
        .children(counts.into_iter().map(|(label, value)| {
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .w(px(200.0))
                .px(SPACE_6)
                .py(SPACE_3)
                .rounded(RADIUS_MD)
                .bg(theme.surface)
                .border_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex_1()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(label),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.accent)
                        .child(value.to_string()),
                )
        }));
    let license = &license;
    let license_line = format!(
        "Devices now {} · new {} · limit {}",
        license.current_devices,
        license.new_devices,
        license
            .limit
            .map_or_else(|| "unlimited".to_owned(), |limit| limit.to_string())
    );
    let parse = &parse;
    let top =
        if report {
            let fact = |label: &'static str, value: String, color: Hsla| {
                div()
                    .flex()
                    .flex_col()
                    .gap(SPACE_1)
                    .flex_1()
                    .min_w_0()
                    .p(SPACE_5)
                    .rounded(RADIUS_MD)
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(FONT_CAPTION)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.fg_dim)
                            .child(label.to_uppercase()),
                    )
                    .child(div().truncate().text_color(color).child(value))
            };
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Import summary report"),
                )
                .child(
                    div()
                        .flex()
                        .gap(SPACE_4)
                        .child(fact("File", file_name, theme.fg))
                        .child(fact("Total nodes", parse.total_nodes.to_string(), theme.fg))
                        .child(fact(
                            "Converted",
                            format!("{} nodes", parse.convertible),
                            theme.success,
                        ))
                        .child(fact(
                            "Skipped",
                            format!("{} nodes", parse.unsupported),
                            theme.danger,
                        )),
                )
                .child(ops_kit::labelled("Conversion results", count_grid, theme))
                .child(ops_kit::labelled(
                    "License",
                    div()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .child(license_line),
                    theme,
                ))
                .into_any_element()
        } else {
            div()
            .flex()
            .flex_col()
            .gap(SPACE_6)
            .child(
                div()
                    .flex()
                    .gap(SPACE_6)
                    .child(stat("Total nodes", parse.total_nodes, theme.fg, theme))
                    .child(stat("Convertible", parse.convertible, theme.success, theme))
                    .child(stat("Manual review", parse.manual_review, theme.warning, theme))
                    .child(stat("Unsupported", parse.unsupported, theme.danger, theme)),
            )
            .when(license.exceeded(), |this| {
                this.child(line(
                    format!("License limit warning: {license_line}"),
                    Icon::CircleAlert,
                    theme.warning,
                ))
            })
            .child(ops_kit::labelled("Conversion preview", count_grid, theme))
            .child(
                div()
                    .text_size(FONT_SMALL)
                    .text_color(theme.fg_muted)
                    .child(
                        "The server converts and reports; it creates no devices, sources or flows. \
                         Configure them from this preview.",
                    ),
            )
            .into_any_element()
        };
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(form(theme).max_w(px(960.0)).child(top))
        .when(len > 0, |this| {
            this.child(
                list.flex_1()
                    .min_h_0()
                    .child(rows_list(len, look, console, cx)),
            )
        })
        .into_any_element()
}

/// The bar under the steps: what the keys do, Back and Next.
fn footer(
    hints: Vec<(&'static str, &'static str)>,
    back: Option<&'static str>,
    next_label: Option<&'static str>,
    next_enabled: bool,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .px(SPACE_9)
        .py(SPACE_5)
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(SPACE_2)
                .text_size(FONT_CAPTION)
                .text_color(theme.fg_dim)
                .children(hints.into_iter().flat_map(|(key, what)| {
                    [
                        kbd(key, cx).into_any_element(),
                        div().pr(SPACE_3).child(what).into_any_element(),
                    ]
                })),
        )
        .children(back.map(|label| {
            Button::new("data-imports-back", label)
                .ghost()
                .icon(Icon::ChevronLeft)
                .tooltip("Esc")
                .on_click(cx.listener(|console, _, _, cx| {
                    go_back(console, cx);
                }))
        }))
        .children(next_label.map(|label| {
            Button::new("data-imports-next", label)
                .primary()
                .icon(Icon::ChevronRight)
                .tooltip("↵")
                .disabled(!next_enabled)
                .on_click(cx.listener(|console, _, _, cx| next(console, cx)))
        }))
        .into_any_element()
}

#[allow(clippy::too_many_lines)]
pub fn render(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let root = ensure_focus(console, cx);
    ensure_sheet(console, window, cx);
    console.state::<State>().window = Some(window.window_handle());
    let admin = may_import_data(console.role());
    let wizard = console.state::<State>().wizard;

    let (subtitle, steps, body, foot) = match wizard {
        Wizard::Data => {
            let data = &console.state::<State>().data;
            let step = data.step;
            let active = DataStep::ALL.iter().position(|s| *s == step).unwrap_or(0);
            let labels: Vec<&'static str> = DataStep::ALL.iter().map(|s| s.label()).collect();
            let kind = data.file.as_ref().map(|(_, kind)| *kind);
            let has_file = data.file.is_some();
            let mapping_ok = data.mapping.check().is_ok();
            let busy = data.busy;
            let body = if !admin {
                admin_only(&theme)
            } else {
                match step {
                    DataStep::File => data_file_step(console, &theme, cx),
                    DataStep::Mapping => mapping_step(console, window, &theme, cx),
                    DataStep::DryRun => result_step(console, window, false, &theme, cx),
                    DataStep::Importing => centered(
                        "Importing data…",
                        "Keep this section open until the report shows.",
                        &theme,
                    ),
                    DataStep::Report => result_step(console, window, true, &theme, cx),
                }
            };
            let (back, next_label, enabled, hints): StepBar = match step {
                _ if !admin => (None, None, false, vec![("W", "Node-RED import")]),
                DataStep::File => (
                    None,
                    Some(match (has_file, kind) {
                        (false, _) => "Choose file…",
                        (true, Some(FileKind::Excel)) => "Detect columns",
                        _ => "Map columns",
                    }),
                    !busy,
                    vec![
                        ("↵", "next"),
                        ("O", "choose file"),
                        ("D", "delimiter"),
                        ("W", "Node-RED"),
                        ("Esc", "sidebar"),
                    ],
                ),
                DataStep::Mapping => (
                    Some("Back"),
                    Some("Dry run"),
                    mapping_ok && !busy,
                    vec![
                        ("↑↓", "field"),
                        ("←→", "column"),
                        ("⌫", "unmap"),
                        ("↵", "dry run"),
                        ("Esc", "back"),
                    ],
                ),
                DataStep::DryRun => (
                    Some("Edit mapping"),
                    Some("Import"),
                    !busy,
                    vec![("↵", "import"), ("Esc", "edit mapping")],
                ),
                DataStep::Importing => (None, None, false, Vec::new()),
                DataStep::Report => (
                    None,
                    Some("Import another file"),
                    true,
                    vec![("↵", "another file"), ("Esc", "start over")],
                ),
            };
            (
                if admin {
                    "CSV or Excel readings into the history · admin"
                } else {
                    "CSV or Excel readings into the history · admin only"
                },
                steps_bar(&labels, active, &theme),
                body,
                footer(hints, back, next_label, enabled, &theme, cx),
            )
        }
        Wizard::NodeRed => {
            let flows = &console.state::<State>().flows;
            let step = flows.step;
            let active = FlowStep::ALL.iter().position(|s| *s == step).unwrap_or(0);
            let labels: Vec<&'static str> = FlowStep::ALL.iter().map(|s| s.label()).collect();
            let has_file = flows.file.is_some();
            let ready = flows.local.is_some() && !flows.busy;
            let body = match step {
                FlowStep::File => flows_file_step(console, window, &theme, cx),
                FlowStep::Uploading => centered(
                    "Parsing and converting flows…",
                    "This may take a moment for large flows.",
                    &theme,
                ),
                FlowStep::Preview => flows_result_step(console, window, false, &theme, cx),
                FlowStep::Report => flows_result_step(console, window, true, &theme, cx),
            };
            let (back, next_label, enabled, hints): StepBar = match step {
                FlowStep::File => (
                    None,
                    Some(if has_file {
                        "Upload & convert"
                    } else {
                        "Choose file…"
                    }),
                    !has_file || ready,
                    vec![
                        ("↵", "next"),
                        ("O", "choose file"),
                        ("W", "data import"),
                        ("Esc", "sidebar"),
                    ],
                ),
                FlowStep::Uploading => (None, None, false, Vec::new()),
                FlowStep::Preview => (
                    Some("Import another"),
                    Some("View summary report"),
                    true,
                    vec![("↵", "report"), ("Esc", "another file")],
                ),
                FlowStep::Report => (
                    Some("Back to preview"),
                    Some("Import another file"),
                    true,
                    vec![("↵", "another file"), ("Esc", "preview")],
                ),
            };
            (
                "Convert a Node-RED flows.json · preview of what it maps to",
                steps_bar(&labels, active, &theme),
                body,
                footer(hints, back, next_label, enabled, &theme, cx),
            )
        }
    };

    let switcher = segmented(
        "data-imports-wizard",
        vec![
            Segment::new(
                "Data file",
                wizard == Wizard::Data,
                cx.listener(|console, _, _, cx| switch(console, Wizard::Data, cx)),
            )
            .icon(Icon::Table),
            Segment::new(
                "Node-RED",
                wizard == Wizard::NodeRed,
                cx.listener(|console, _, _, cx| switch(console, Wizard::NodeRed, cx)),
            )
            .icon(Icon::Waypoints),
        ],
        cx,
    );

    div()
        .flex()
        .flex_col()
        .size_full()
        .child(page_header("Import", subtitle, vec![switcher], cx))
        .child(
            div()
                .id("data-imports")
                .key_context(CONTEXT)
                .track_focus(&root)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_key_down(cx.listener(|console, event: &KeyDownEvent, window, cx| {
                    on_key(console, event, window, cx);
                }))
                .on_action(cx.listener(|console, _: &Dismiss, _, cx| {
                    if !go_back(console, cx) {
                        cx.propagate();
                    }
                }))
                .on_action(cx.listener(|console, _: &EngImportsOpen, _, cx| {
                    choose_file(console, cx);
                }))
                .on_action(cx.listener(|console, _: &EngImportsSwitch, _, cx| {
                    let other = match console.state::<State>().wizard {
                        Wizard::Data => Wizard::NodeRed,
                        Wizard::NodeRed => Wizard::Data,
                    };
                    switch(console, other, cx);
                }))
                .on_action(cx.listener(|console, _: &EngImportsDelimiter, _, cx| {
                    cycle_delimiter(console, cx);
                }))
                .on_drop(cx.listener(|console, paths: &ExternalPaths, _, cx| {
                    if let Some(path) = paths.paths().first() {
                        accept_path(console, path.clone(), cx);
                    }
                }))
                .child(steps)
                .child(div().flex().flex_col().flex_1().min_h_0().child(body))
                .child(foot),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use voltrus_scada_native::api::eng_imports::{
        ConvertedFunctionNode, ConvertedMqttSource, NodeRedDetails, SkippedNode,
    };

    use super::*;

    #[test]
    fn only_admins_import_data_files() {
        assert!(may_import_data(Some(Role::Admin)));
        assert!(!may_import_data(Some(Role::Operator)));
        assert!(!may_import_data(Some(Role::Any)));
        assert!(!may_import_data(None));
    }

    #[test]
    fn excel_columns_reach_the_detected_ones() {
        let mut detected = ColumnMapping::default();
        assert_eq!(excel_columns(&detected).len(), EXCEL_COLUMNS);
        detected.set(Field::Value, Some(11));
        let columns = excel_columns(&detected);
        assert_eq!(columns.len(), 12);
        assert_eq!(columns[11], "L");
    }

    #[test]
    fn column_labels() {
        let headers = vec!["name".to_owned(), "PV".to_owned()];
        assert_eq!(column_label(&headers, false, None), "— not mapped —");
        assert_eq!(column_label(&headers, false, Some(0)), "0 · name");
        assert_eq!(column_label(&headers, false, Some(1)), "1 · PV");
        assert_eq!(
            column_label(&headers, false, Some(5)),
            "column 5 (not in the file)"
        );
        let letters = excel_columns(&ColumnMapping::default());
        assert_eq!(column_label(&letters, true, Some(2)), "C");
    }

    #[test]
    fn node_red_preview_rows_follow_the_web_order() {
        let result = NodeRedImport {
            warnings: vec!["Node 'f' (n4) requires manual review".into()],
            details: NodeRedDetails {
                mqtt_sources: vec![ConvertedMqttSource {
                    topic: "sensor/#".into(),
                    qos: 1,
                    name: "Subscribe".into(),
                    direction: "in".into(),
                    source_node_id: "n2".into(),
                }],
                function_nodes: vec![ConvertedFunctionNode {
                    node_id: "n4".into(),
                    name: "f".into(),
                    source_code: None,
                    warning: "must be reimplemented".into(),
                }],
                skipped_nodes: vec![SkippedNode {
                    node_id: "n5".into(),
                    node_type: "rare".into(),
                    name: None,
                    reason: "Unsupported node type: rare".into(),
                }],
                ..NodeRedDetails::default()
            },
            ..NodeRedImport::default()
        };
        let rows = preview_rows(&result);
        assert_eq!(rows.len(), 11);
        assert_eq!(rows[0], PreviewRow::Heading("Warnings · 1".into()));
        assert!(matches!(rows[1], PreviewRow::Warning(_)));
        assert_eq!(rows[2], PreviewRow::Heading("MQTT sources · 1".into()));
        assert_eq!(
            rows[4],
            PreviewRow::Cells(
                vec![
                    "Subscribe".into(),
                    "sensor/#".into(),
                    "1".into(),
                    "in".into()
                ],
                Tone::Plain
            )
        );
        assert!(matches!(rows[7], PreviewRow::Cells(_, Tone::Warn)));
        assert_eq!(
            rows[10],
            PreviewRow::Cells(
                vec![
                    "rare".into(),
                    "-".into(),
                    "Unsupported node type: rare".into()
                ],
                Tone::Danger
            )
        );
        assert!(preview_rows(&NodeRedImport::default()).is_empty());
        assert_eq!(warning_count(&result.warnings), 2);
        assert_eq!(
            warning_row(&result.warnings, 0),
            Some(PreviewRow::Heading("Warnings · 1".into()))
        );
        assert_eq!(warning_row(&result.warnings, 2), None);
        assert_eq!(warning_row(&[], 0), None);
    }
}

#[cfg(test)]
mod harness_tests {
    use super::*;
    use crate::console::testing::Harness;

    fn temp_csv(tag: &str, body: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "voltrus-native-imports-{tag}-{}.csv",
            std::process::id()
        ));
        std::fs::write(&path, body).unwrap();
        path
    }

    /// At the dry run's result with `path` chosen and its bytes kept.
    fn at_dry_run(console: &mut Console, path: &Path) {
        let data = &mut console.state::<State>().data;
        data.file = Some((
            Chosen {
                path: path.to_path_buf(),
                name: "plant.csv".into(),
                size: 3,
            },
            FileKind::Csv,
        ));
        data.step = DataStep::DryRun;
        data.dry = Some(ImportResult::default());
        let _ = data.upload.set(b"a,1".to_vec());
    }

    #[gpui::test]
    fn a_file_dropped_during_the_import_keeps_its_report(cx: &mut gpui::TestAppContext) {
        let first = temp_csv("drop-a", b"a,1");
        let second = temp_csv("drop-b", b"b,2");
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, _, cx| {
            at_dry_run(console, &first);
            next_data(console, cx);
        });
        let seq = h.update(|console, _, _| console.state::<State>().data.seq);
        assert!(
            h.calls()
                .contains(&CallKey::new("data.imports.run").with(seq))
        );
        // A drop (or the open panel's answer) while it runs.
        h.update(|console, _, cx| accept_path(console, second.clone(), cx));
        h.reply(
            CallKey::new("data.imports.run").with(seq),
            Reply::ok(ImportOutcome::Done(ImportResult {
                total_rows: 1,
                imported: 1,
                ..ImportResult::default()
            })),
        );
        h.draw();
        h.update(|console, _, _| {
            let data = &console.state::<State>().data;
            assert_eq!(data.step, DataStep::Report);
            assert_eq!(data.report.as_ref().map(|report| report.imported), Some(1));
            assert_eq!(
                data.file.as_ref().map(|(chosen, _)| chosen.path.clone()),
                Some(first.clone())
            );
        });
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[gpui::test]
    fn a_flows_preview_in_flight_ignores_a_dropped_file(cx: &mut gpui::TestAppContext) {
        let path = temp_csv("busy-flows", b"a,1");
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, _, cx| {
            let state = console.state::<State>();
            state.wizard = Wizard::Data;
            state.flows.busy = true;
            accept_path(console, path.clone(), cx);
            assert!(console.state::<State>().data.file.is_none());
        });
        let _ = std::fs::remove_file(&path);
    }

    #[gpui::test]
    fn a_picked_file_goes_to_the_wizard_that_asked(cx: &mut gpui::TestAppContext) {
        let path = temp_csv("picked", b"tag,value\na,1\n");
        let mut h = Harness::new(cx);
        h.sign_in("admin");
        h.update(|console, _, cx| {
            // Asked from the data wizard, answered after a switch.
            console.state::<State>().wizard = Wizard::NodeRed;
            picked(console, Wizard::Data, path.clone(), cx);
            let state = console.state::<State>();
            assert!(state.flows.file.is_none());
            assert!(state.flows.error.is_none(), "no flows-file complaint");
            assert!(state.data.file.is_none());
            // Answered with the asking wizard on screen.
            state.wizard = Wizard::Data;
            picked(console, Wizard::Data, path.clone(), cx);
            assert!(console.state::<State>().data.file.is_some());
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_import_sends_the_bytes_the_dry_run_checked() {
        let path = temp_csv("kept", b"tag,value\na,1\n");
        let upload = OnceLock::new();
        assert!(
            upload_bytes(&upload, &path, true).is_err(),
            "no dry run yet"
        );
        assert_eq!(
            upload_bytes(&upload, &path, false).unwrap(),
            b"tag,value\na,1\n"
        );
        // Edited on disk after the dry run.
        std::fs::write(&path, b"tag,value\nb,999\n").unwrap();
        assert_eq!(
            upload_bytes(&upload, &path, true).unwrap(),
            b"tag,value\na,1\n"
        );
        let _ = std::fs::remove_file(&path);
    }
}
