//! JSON Schema (draft 2020-12) of the project format, generated from the same
//! field lists that drive parsing (`spec.rs`), so the schema cannot drift from
//! what `voltrus project validate` and the import accept.
//!
//! The schema checks shape only: field names, types, required fields and the
//! allowed values of enumerated fields. Cross-entity rules (references to tags,
//! scripts and templates, duplicate keys) stay in `validate.rs`.

use serde_json::{Map, Value, json};

use super::model::FORMAT;
use super::spec::{self, Field, Layout, Req, Ty};

/// `$id` of the published schema.
pub const SCHEMA_ID: &str = "https://voltrus.id/schema/voltrus-project-1.json";

fn field_schema(f: &Field) -> Value {
    let mut s = match f.ty {
        Ty::Text => {
            if f.one_of.is_empty() {
                json!({"type": "string"})
            } else {
                json!({"type": "string", "enum": f.one_of})
            }
        }
        Ty::Int => json!({"type": "integer"}),
        Ty::Real => json!({"type": "number"}),
        Ty::Bool => json!({"type": "boolean"}),
        Ty::Json => json!({}),
        Ty::Obj(sub) => object_schema(sub),
        Ty::List(sub) => json!({"type": "array", "items": object_schema(sub)}),
    };
    // A field may always be written as null: it then takes its default.
    if !matches!(f.req, Req::Required)
        && let Value::Object(m) = &mut s
        && let Some(t) = m.get("type").cloned()
    {
        m.insert("type".into(), json!([t, "null"]));
        if let Some(Value::Array(values)) = m.get_mut("enum") {
            values.push(Value::Null);
        }
    }
    s
}

/// Schema of one mapping: every listed field, nothing else.
pub fn object_schema(fields: &[Field]) -> Value {
    let mut props = Map::new();
    let mut required = Vec::new();
    for f in fields {
        props.insert(f.name.to_string(), field_schema(f));
        if matches!(f.req, Req::Required) {
            required.push(Value::String(f.name.to_string()));
        }
    }
    let mut s = json!({
        "type": "object",
        "properties": props,
        "additionalProperties": false,
    });
    if !required.is_empty() {
        s["required"] = Value::Array(required);
    }
    s
}

/// The whole single-file project (`voltrus project export site.yaml`).
pub fn project_schema() -> Value {
    let mut props = Map::new();
    props.insert("format".into(), json!({"const": FORMAT}));
    props.insert("server_version".into(), json!({"type": "string"}));
    props.insert("generated_at".into(), json!({"type": "string"}));
    let mut defs = Map::new();
    for s in spec::SECTIONS {
        defs.insert(s.name.to_string(), object_schema(s.fields));
        let reference = json!({"$ref": format!("#/$defs/{}", s.name)});
        let section = match s.layout {
            Layout::Single => reference,
            Layout::Dir | Layout::File => json!({"type": "array", "items": reference}),
        };
        props.insert(s.name.to_string(), section);
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": SCHEMA_ID,
        "title": format!("Voltrus project ({FORMAT})"),
        "description": "Single-file form of a Voltrus project. In the directory form each file holds one entity of a section (or the section's list); the entity schemas are under $defs. See docs/project-format.md.",
        "type": "object",
        "properties": props,
        "required": ["format"],
        "additionalProperties": false,
        "$defs": defs,
    })
}

/// The schema as pretty JSON with a trailing newline, as committed in
/// `docs/project-schema.json`.
pub fn project_schema_text() -> String {
    let mut s = serde_json::to_string_pretty(&project_schema()).expect("schema serialises");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity_errors(section: &str, yaml: &str) -> Vec<String> {
        let s = spec::section(section).unwrap();
        let m: serde_yaml_neo::Mapping = serde_yaml_neo::from_str(yaml).unwrap();
        let mut errs = Vec::new();
        super::super::norm::normalize(s.fields, &m, section, &mut errs);
        errs
    }

    /// `docs/project-schema.json` is generated: a change to the format must
    /// regenerate it with `voltrus project schema docs/project-schema.json`.
    #[test]
    fn committed_schema_matches_the_spec() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/project-schema.json");
        if std::env::var_os("VOLTRUS_UPDATE_SCHEMA").is_some() {
            std::fs::write(&path, project_schema_text()).unwrap();
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == project_schema_text(),
            "docs/project-schema.json is out of date; run `voltrus project schema docs/project-schema.json` (or this test with VOLTRUS_UPDATE_SCHEMA=1)"
        );
    }

    #[test]
    fn schema_lists_every_section_and_closes_every_object() {
        let schema = project_schema();
        for s in spec::SECTIONS {
            assert!(schema["properties"].get(s.name).is_some(), "{}", s.name);
            assert_eq!(schema["$defs"][s.name]["additionalProperties"], false);
        }
        let reg = &schema["$defs"]["modbus_devices"]["properties"]["registers"]["items"];
        assert_eq!(reg["additionalProperties"], false);
        let types = reg["properties"]["register_type"]["enum"]
            .as_array()
            .unwrap();
        assert!(types.contains(&json!("float32")));
        assert!(types.contains(&Value::Null));
        assert_eq!(
            schema["$defs"]["modbus_devices"]["required"],
            json!(["id", "name", "ip"])
        );
    }

    #[test]
    fn enum_fields_reject_unknown_values_naming_the_field_and_the_choices() {
        let errs = entity_errors(
            "modbus_devices",
            "{id: m1, name: M1, ip: 10.0.0.1, endianness: middle, registers: [{name: a, address: 0, register_type: uint32}]}",
        );
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert!(
            errs[0].contains("modbus_devices.endianness")
                && errs[0].contains("'middle' is not one of big, little"),
            "{errs:?}"
        );
        assert!(
            errs[1].contains("registers[0].register_type")
                && errs[1].contains("int16, uint16, int32, float32, float64"),
            "{errs:?}"
        );
        let ok = entity_errors(
            "modbus_devices",
            "{id: m1, name: M1, ip: 10.0.0.1, registers: [{name: a, address: 0, register_type: int16, register_area: coil}]}",
        );
        assert!(ok.is_empty(), "{ok:?}");
    }

    #[test]
    fn unknown_fields_are_rejected_at_every_level() {
        let errs = entity_errors(
            "screens",
            "{id: s1, name: S1, colour: red, scada: {equipment: [{equipment_id: e, equipment_type: pump, sensor: [a]}], connections: [{connection_id: c, from_equipment: e, to_equipment: f, connection_type: steam}]}}",
        );
        let all = errs.join("\n");
        assert!(all.contains("unknown field 'colour'"), "{all}");
        assert!(
            all.contains("scada.equipment[0]: unknown field 'sensor'"),
            "{all}"
        );
        assert!(
            all.contains("connection_type: 'steam' is not one of power, data, flow"),
            "{all}"
        );
    }
}
