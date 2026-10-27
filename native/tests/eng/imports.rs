//! Import against the live plant: the CSV and Excel import routes (dry
//! runs, the server's column detection, an explicit mapping, a sheet, the
//! refusals) and the Node-RED import.
//!
//! A CSV or Excel import writes telemetry readings, and the server has no
//! route that deletes readings, so every import that is carried out (not
//! a dry run) here holds only rows the server skips: the non-dry path runs
//! and nothing is written. The test checks the readings of its tag before
//! and after. The Node-RED import only converts and reports (it creates
//! nothing); the test checks that the flows, Modbus devices and data
//! sources are as they were.

use serde_json::Value;
use voltrus_scada_native::api::VoltrusApi;
use voltrus_scada_native::api::eng_imports::{
    ColumnMapping, Delimiter, Field, FileKind, ImportOutcome, ImportRefusal, ImportRequest,
    ImportResult, Mode, NodeCategory, auto_detect, column_letters, parse_csv, parse_flows,
};

const TAG: &str = "native_e2e_import";
const DEVICE: &str = "native-e2e-import";

fn request(kind: FileKind, dry_run: bool, mapping: Option<ColumnMapping>) -> ImportRequest {
    ImportRequest {
        kind,
        dry_run,
        mode: Mode::Merge,
        delimiter: Delimiter::Comma,
        sheet: String::new(),
        mapping,
    }
}

fn done(outcome: ImportOutcome, what: &str) -> ImportResult {
    match outcome {
        ImportOutcome::Done(result) => result,
        ImportOutcome::Refused(refusal) => {
            panic!("{what}: refused ({}): {}", refusal.status, refusal.error)
        }
    }
}

fn refused(outcome: ImportOutcome, what: &str) -> ImportRefusal {
    match outcome {
        ImportOutcome::Refused(refusal) => refusal,
        ImportOutcome::Done(result) => panic!("{what}: carried out: {result:?}"),
    }
}

/// The test tag's readings in the history.
async fn readings(api: &VoltrusApi) -> i64 {
    let answer: Value = api
        .get_json(&format!(
            "/api/v1/data/export?device={DEVICE}&metric={TAG}&limit=1"
        ))
        .await
        .expect("GET /data/export");
    answer["meta"]["total"].as_i64().expect("meta.total")
}

fn mapping(pairs: &[(Field, usize)]) -> ColumnMapping {
    let mut mapping = ColumnMapping::default();
    for (field, column) in pairs {
        mapping.set(*field, Some(*column));
    }
    mapping
}

pub(crate) async fn round_trip(api: &VoltrusApi) {
    let before = readings(api).await;
    csv(api).await;
    excel(api).await;
    assert_eq!(
        readings(api).await,
        before,
        "the imports left the history as it was"
    );
    println!("imports: {TAG} still has {before} readings");
    node_red(api).await;
}

async fn csv(api: &VoltrusApi) {
    // The server's detection, as the section's local parse predicts it.
    let text = format!(
        "name,value,timestamp,device\n{TAG},1.5,1704067200,{DEVICE}\n{TAG},abc,,{DEVICE}\n"
    );
    let local = parse_csv(&text, b',', 10);
    let detected = auto_detect(&local.headers);
    let dry = done(
        api.import_data(
            &request(FileKind::Csv, true, None),
            "readings.csv",
            text.as_bytes(),
        )
        .await
        .expect("POST /import/csv (dry run)"),
        "CSV dry run",
    );
    assert!(dry.dry_run);
    assert_eq!(dry.mode, "merge");
    assert_eq!(
        (dry.total_rows, dry.imported, dry.skipped),
        (local.total_rows, 1, 1)
    );
    assert_eq!(dry.errors[0].row, Some(3));
    assert_eq!(dry.errors[0].tag.as_deref(), Some(TAG));
    assert_eq!(ColumnMapping::from_server(&dry.column_mapping), detected);
    println!(
        "imports: CSV dry run {} rows, would import {}, {}",
        dry.total_rows,
        dry.imported,
        dry.errors[0].line()
    );

    // An explicit mapping and another delimiter.
    let text = format!("Point Label;Reading X;Where\n{TAG};2.5;{DEVICE}\n");
    let explicit = mapping(&[(Field::TagName, 0), (Field::Value, 1), (Field::DeviceId, 2)]);
    let mut semicolons = request(FileKind::Csv, true, Some(explicit));
    semicolons.delimiter = Delimiter::Semicolon;
    let mapped = done(
        api.import_data(&semicolons, "points.csv", text.as_bytes())
            .await
            .expect("POST /import/csv (mapped dry run)"),
        "CSV mapped dry run",
    );
    assert_eq!((mapped.total_rows, mapped.imported), (1, 1));
    assert_eq!(ColumnMapping::from_server(&mapped.column_mapping), explicit);
    println!(
        "imports: CSV mapped dry run (;) would import {}",
        mapped.imported
    );

    // The import itself, with only rows the server skips.
    let text = format!("name,value\n{TAG},not-a-number\n");
    let run = done(
        api.import_data(
            &request(FileKind::Csv, false, None),
            "readings.csv",
            text.as_bytes(),
        )
        .await
        .expect("POST /import/csv"),
        "CSV import",
    );
    assert!(!run.dry_run);
    assert_eq!((run.total_rows, run.imported, run.skipped), (1, 0, 1));
    println!(
        "imports: CSV import ran, {} skipped, nothing written",
        run.skipped
    );

    // Refusals.
    let mut replace = request(FileKind::Csv, true, None);
    replace.mode = Mode::Replace;
    let refusal = refused(
        api.import_data(&replace, "readings.csv", b"name,value\n")
            .await
            .expect("POST /import/csv (replace)"),
        "replace mode",
    );
    assert_eq!(refusal.status, 400);
    println!("imports: replace refused: {}", refusal.error);

    let refusal = refused(
        api.import_data(
            &request(FileKind::Csv, true, None),
            "odd.csv",
            b"alpha,beta\nx,1\n",
        )
        .await
        .expect("POST /import/csv (no tag column)"),
        "no tag column",
    );
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.detected_columns, vec!["alpha", "beta"]);
    println!(
        "imports: no tag column refused, columns {:?}",
        refusal.detected_columns
    );

    let refusal = refused(
        api.import_data(
            &request(FileKind::Csv, true, Some(mapping(&[(Field::TagName, 7)]))),
            "readings.csv",
            b"name,value\n",
        )
        .await
        .expect("POST /import/csv (out of bounds)"),
        "column out of bounds",
    );
    assert_eq!(refusal.status, 400);
    assert!(refusal.error.contains("out of bounds"), "{}", refusal.error);

    let refusal = refused(
        api.import_data(
            &request(FileKind::Csv, true, None),
            "readings.txt",
            b"name,value\n",
        )
        .await
        .expect("POST /import/csv (not a CSV)"),
        "not a CSV",
    );
    assert_eq!(refusal.status, 400);
    println!("imports: bad mapping and bad file name refused");
}

async fn excel(api: &VoltrusApi) {
    let workbook = xlsx(
        "Readings",
        &[
            vec![
                Cell::Text("name"),
                Cell::Text("value"),
                Cell::Text("device"),
            ],
            vec![Cell::Text(TAG), Cell::Number(1.5), Cell::Text(DEVICE)],
            vec![Cell::Text(TAG), Cell::Text("abc"), Cell::Text(DEVICE)],
        ],
    );
    let dry = done(
        api.import_data(
            &request(FileKind::Excel, true, None),
            "readings.xlsx",
            &workbook,
        )
        .await
        .expect("POST /import/excel (dry run)"),
        "Excel dry run",
    );
    assert_eq!((dry.total_rows, dry.imported, dry.skipped), (2, 1, 1));
    assert_eq!(dry.sheet.as_deref(), Some("Readings"));
    assert_eq!(dry.available_sheets, vec!["Readings"]);
    let detected = ColumnMapping::from_server(&dry.column_mapping);
    assert_eq!(detected.get(Field::TagName), Some(0));
    assert_eq!(detected.get(Field::DeviceId), Some(2));
    println!(
        "imports: Excel dry run, sheet {:?} of {:?}, would import {}",
        dry.sheet, dry.available_sheets, dry.imported
    );

    let mut named = request(
        FileKind::Excel,
        true,
        Some(mapping(&[(Field::TagName, 0), (Field::Value, 1)])),
    );
    named.sheet = "Readings".into();
    let mapped = done(
        api.import_data(&named, "readings.xlsx", &workbook)
            .await
            .expect("POST /import/excel (sheet, mapping)"),
        "Excel mapped dry run",
    );
    assert_eq!(mapped.imported, 1);
    assert_eq!(mapped.sheet.as_deref(), Some("Readings"));

    named.sheet = "Nope".into();
    let refusal = refused(
        api.import_data(&named, "readings.xlsx", &workbook)
            .await
            .expect("POST /import/excel (unknown sheet)"),
        "unknown sheet",
    );
    assert_eq!(refusal.status, 400);
    assert_eq!(refusal.available_sheets, vec!["Readings"]);
    println!("imports: Excel sheet chosen; unknown sheet refused");

    let skipped_only = xlsx(
        "Readings",
        &[
            vec![Cell::Text("name"), Cell::Text("value")],
            vec![Cell::Text(TAG), Cell::Text("not-a-number")],
        ],
    );
    let run = done(
        api.import_data(
            &request(FileKind::Excel, false, None),
            "readings.xlsx",
            &skipped_only,
        )
        .await
        .expect("POST /import/excel"),
        "Excel import",
    );
    assert!(!run.dry_run);
    assert_eq!((run.total_rows, run.imported, run.skipped), (1, 0, 1));
    println!(
        "imports: Excel import ran, {} skipped, nothing written",
        run.skipped
    );
}

/// How many items a list answer holds (`[…]` or `{"flows": […]}`).
async fn count(api: &VoltrusApi, path: &str, key: Option<&str>) -> usize {
    let answer: Value = api.get_json(path).await.expect(path);
    let list = match key {
        Some(key) => &answer[key],
        None => &answer,
    };
    list.as_array().map_or(0, Vec::len)
}

async fn node_red(api: &VoltrusApi) {
    let flows_before = count(api, "/api/v1/flows", Some("flows")).await;
    let devices_before = count(api, "/api/v1/modbus-devices", None).await;
    let sources_before = count(api, "/api/v1/data-sources", None).await;

    let flows = r#"[
        {"id":"nt1","type":"tab","label":"Native e2e"},
        {"id":"nm1","type":"mqtt in","name":"Subscribe","topic":"native/e2e/#","broker":"b1","z":"nt1"},
        {"id":"ng1","type":"ui_gauge","name":"Pressure","gmin":0,"gmax":100,"label":"PSI","z":"nt1"},
        {"id":"nf1","type":"function","name":"Scale","func":"return msg;","z":"nt1"},
        {"id":"nr1","type":"native-e2e-unknown","name":"Odd","z":"nt1"},
        {"id":"nb1","type":"modbus-read","name":"Read","server":"native-e2e-modbus","fc":3,"unitid":1,"address":"0","quantity":"2","z":"nt1"},
        {"id":"no1","type":"opcua-client","name":"Boiler","z":"nt1"}
    ]"#;
    let local = parse_flows(flows).expect("the flows parse locally");
    let result = api
        .import_node_red("flows.json", flows.as_bytes())
        .await
        .expect("POST /import/node-red");
    assert_eq!(result.parse.total_nodes, local.total());
    assert_eq!(
        result.parse.convertible,
        local.count(NodeCategory::Convertible)
    );
    assert_eq!(
        result.parse.manual_review,
        local.count(NodeCategory::ManualReview)
    );
    assert_eq!(
        result.parse.ui_decoration,
        local.count(NodeCategory::UiDecoration)
    );
    assert_eq!(
        result.parse.unsupported,
        local.count(NodeCategory::Unsupported)
    );
    assert_eq!(result.conversion.modbus_devices, 1);
    assert_eq!(result.conversion.mqtt_sources, 1);
    assert_eq!(result.conversion.widgets, 1);
    assert_eq!(result.conversion.function_nodes, 1);
    // The unknown node, and the OPC UA client (left to the OPC UA manager).
    assert_eq!(result.conversion.skipped_nodes, 2);
    assert_eq!(result.details.mqtt_sources[0].topic, "native/e2e/#");
    assert_eq!(result.details.widgets[0].label.as_deref(), Some("PSI"));
    assert_eq!(
        result.details.modbus_devices[0].device_id,
        "native-e2e-modbus"
    );
    assert_eq!(result.license.new_devices, 1);
    assert_eq!(
        usize::try_from(result.license.current_devices).ok(),
        Some(devices_before)
    );
    // The answer's warnings are the converter's (OPC UA and S7 nodes); the
    // parser's manual-review and unsupported warnings are not sent.
    assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
    assert!(result.warnings[0].contains("Boiler"));
    println!(
        "imports: Node-RED {} nodes, {} convertible, {} warnings",
        result.parse.total_nodes,
        result.parse.convertible,
        result.warnings.len()
    );

    assert!(
        api.import_node_red("flows.json", b"not json")
            .await
            .is_err(),
        "a broken flows.json is refused"
    );

    assert_eq!(
        count(api, "/api/v1/flows", Some("flows")).await,
        flows_before
    );
    assert_eq!(
        count(api, "/api/v1/modbus-devices", None).await,
        devices_before
    );
    assert_eq!(
        count(api, "/api/v1/data-sources", None).await,
        sources_before
    );
    println!("imports: Node-RED import created nothing (preview only)");
}

// ── A minimal .xlsx ──────────────────────────────────────────────────────

enum Cell<'a> {
    Text(&'a str),
    Number(f64),
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn sheet_xml(rows: &[Vec<Cell>]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (index, row) in rows.iter().enumerate() {
        let number = index + 1;
        xml.push_str(&format!(r#"<row r="{number}">"#));
        for (column, cell) in row.iter().enumerate() {
            let reference = format!("{}{number}", column_letters(column));
            match cell {
                Cell::Text(text) => xml.push_str(&format!(
                    r#"<c r="{reference}" t="inlineStr"><is><t>{}</t></is></c>"#,
                    escape(text)
                )),
                Cell::Number(value) => {
                    xml.push_str(&format!(r#"<c r="{reference}"><v>{value}</v></c>"#));
                }
            }
        }
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData></worksheet>");
    xml
}

/// A workbook with one sheet `name` of `rows`, zipped without compression.
fn xlsx(name: &str, rows: &[Vec<Cell>]) -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"></Default><Default Extension="xml" ContentType="application/xml"></Default><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"></Override><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"></Override></Types>"#;
    let root_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"></Relationship></Relationships>"#;
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="{}" sheetId="1" r:id="rId1"></sheet></sheets></workbook>"#,
        escape(name)
    );
    let workbook_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"></Relationship></Relationships>"#;
    let sheet = sheet_xml(rows);
    zip_stored(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ])
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn u16_le(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&u16::try_from(value).expect("fits u16").to_le_bytes());
}

fn u32_le(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&u32::try_from(value).expect("fits u32").to_le_bytes());
}

/// A zip archive of `entries`, stored (no compression).
fn zip_stored(entries: &[(&str, &[u8])]) -> Vec<u8> {
    const DOS_DATE: usize = 0x0021; // 1980-01-01
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = out.len();
        let crc = crc32(data) as usize;
        // Local file header.
        u32_le(&mut out, 0x0403_4b50);
        u16_le(&mut out, 20); // version needed
        u16_le(&mut out, 0); // flags
        u16_le(&mut out, 0); // stored
        u16_le(&mut out, 0); // time
        u16_le(&mut out, DOS_DATE);
        u32_le(&mut out, crc);
        u32_le(&mut out, data.len());
        u32_le(&mut out, data.len());
        u16_le(&mut out, name.len());
        u16_le(&mut out, 0); // extra
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        // Its central directory entry.
        u32_le(&mut central, 0x0201_4b50);
        u16_le(&mut central, 20); // made by
        u16_le(&mut central, 20); // needed
        u16_le(&mut central, 0);
        u16_le(&mut central, 0);
        u16_le(&mut central, 0);
        u16_le(&mut central, DOS_DATE);
        u32_le(&mut central, crc);
        u32_le(&mut central, data.len());
        u32_le(&mut central, data.len());
        u16_le(&mut central, name.len());
        u16_le(&mut central, 0); // extra
        u16_le(&mut central, 0); // comment
        u16_le(&mut central, 0); // disk
        u16_le(&mut central, 0); // internal attributes
        u32_le(&mut central, 0); // external attributes
        u32_le(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = out.len();
    let central_size = central.len();
    out.extend_from_slice(&central);
    u32_le(&mut out, 0x0605_4b50);
    u16_le(&mut out, 0);
    u16_le(&mut out, 0);
    u16_le(&mut out, entries.len());
    u16_le(&mut out, entries.len());
    u32_le(&mut out, central_size);
    u32_le(&mut out, central_offset);
    u16_le(&mut out, 0);
    out
}
