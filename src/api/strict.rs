//! Strict request bodies for the create and update routes that take untyped
//! JSON: an unknown field or an enumerated field with a value outside its list
//! is answered with 400 and a message naming the field, instead of being
//! ignored (or stored and silently read as a default later).
//!
//! The allowed values come from the project format (`project_yaml::spec`), so
//! the REST API, the MCP tools that call these routes, the YAML import and the
//! JSON Schema agree.

use serde_json::Value;

use super::AppError;
use crate::project_yaml::spec;

/// The fields a body may carry and the allowed values of its enumerated ones.
pub struct Shape {
    /// What the body describes, for messages ("data source", "register 2").
    pub what: &'static str,
    pub fields: &'static [&'static str],
    pub enums: &'static [(&'static str, &'static [&'static str])],
}

/// The problem with a body, naming the field.
pub fn problem(body: &Value, shape: &Shape, what: &str) -> Option<String> {
    let Some(obj) = body.as_object() else {
        return Some(format!("{what}: expected a JSON object"));
    };
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    for k in keys {
        if !shape.fields.contains(&k.as_str()) {
            return Some(format!(
                "{what}: unknown field '{k}' (allowed: {})",
                shape.fields.join(", ")
            ));
        }
    }
    for (field, allowed) in shape.enums {
        match obj.get(*field) {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if allowed.contains(&s.as_str()) => {}
            // The routes treat an empty endianness as the default.
            Some(Value::String(s)) if s.is_empty() && *field == "endianness" => {}
            Some(v) => {
                return Some(format!(
                    "{what}: {field} {v} is not one of {}",
                    allowed.join(", ")
                ));
            }
        }
    }
    None
}

/// [`problem`] as a 400 answer.
pub fn check(body: &Value, shape: &Shape) -> Result<(), AppError> {
    match problem(body, shape, shape.what) {
        Some(msg) => Err(AppError::bad_request(msg)),
        None => Ok(()),
    }
}

/// Every item of the list field `list` of `body` (absent or null is fine).
pub fn check_items(body: &Value, list: &str, shape: &Shape) -> Result<(), AppError> {
    let items = match body.get(list) {
        None | Some(Value::Null) => return Ok(()),
        Some(items) => items,
    };
    let Some(items) = items.as_array() else {
        return Err(AppError::bad_request(format!("{list}: expected a list")));
    };
    for (i, item) in items.iter().enumerate() {
        if let Some(msg) = problem(item, shape, &format!("{} {}", shape.what, i + 1)) {
            return Err(AppError::bad_request(msg));
        }
    }
    Ok(())
}

/// A data source (tag) as `POST` and `PUT /api/v1/data-sources` take it: the
/// fields `GET` returns, so a read-modify-write round trip passes.
pub const DATA_SOURCE: Shape = Shape {
    what: "data source",
    fields: &[
        "id",
        "key",
        "name",
        "unit",
        "color",
        "min_val",
        "max_val",
        "warn_val",
        "crit_val",
        "base_val",
        "sort_order",
        "decimals",
        "source_type",
        "source_config",
        "device_id",
        "equipment_id",
        "chart_type",
        "path",
        "on_delay_secs",
        "off_delay_secs",
    ],
    // `chart_type` and `source_type` stay open: older versions and the
    // setup wizard store chart types the client draws as a sparkline, and
    // each protocol module adds its own source types.
    enums: &[],
};

/// A screen as `POST` and `PUT /api/v1/screens` take it. `GET` names the
/// kind `type` (older clients sent `device_type`); both are ignored.
pub const SCREEN: Shape = Shape {
    what: "screen",
    fields: &["id", "name", "sort_order", "type", "device_type"],
    enums: &[],
};

/// A Modbus device as `POST` and `PUT /api/v1/modbus-devices` take it.
pub const MODBUS_DEVICE: Shape = Shape {
    what: "Modbus device",
    fields: &[
        "id",
        "name",
        "ip",
        "port",
        "unit_id",
        "interval_ms",
        "endianness",
        "enabled",
        "registers",
        "template",
        "register_count",
        "running",
        "created_at",
        "updated_at",
    ],
    enums: &[("endianness", spec::ENDIANNESS)],
};

/// One register of a Modbus device body. `id`, `device_id` and `sort_order`
/// are what `GET` returns and are ignored.
pub const MODBUS_REGISTER: Shape = Shape {
    what: "register",
    fields: &[
        "name",
        "address",
        "offset",
        "zero_based",
        "register_type",
        "register_area",
        "scale",
        "id",
        "device_id",
        "sort_order",
    ],
    enums: &[
        ("register_type", spec::REGISTER_TYPES),
        ("register_area", spec::REGISTER_AREAS),
    ],
};

/// A Modbus device body and its registers.
pub fn check_modbus_device(body: &Value) -> Result<(), AppError> {
    check(body, &MODBUS_DEVICE)?;
    check_items(body, "registers", &MODBUS_REGISTER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_fields_and_bad_enum_values_are_named() {
        let p = problem(
            &json!({"name": "a", "colour": "red"}),
            &DATA_SOURCE,
            "data source",
        );
        assert!(
            p.unwrap()
                .starts_with("data source: unknown field 'colour'")
        );
        assert!(problem(&json!({"chart_type": "gauge", "id": 3}), &DATA_SOURCE, "x").is_none());
        assert!(problem(&json!([1]), &SCREEN, "screen").is_some());
    }

    #[test]
    fn modbus_registers_are_checked_one_by_one() {
        let body = json!({"id": "m", "name": "m", "ip": "1.2.3.4", "registers": [
            {"name": "a", "address": 0},
            {"name": "b", "address": 2, "register_type": "uint32"}
        ]});
        let err = check_modbus_device(&body).unwrap_err().0.to_string();
        assert!(
            err.contains("register 2: register_type \"uint32\" is not one of int16, uint16, int32, float32, float64"),
            "{err}"
        );
        let err = check_modbus_device(&json!({"endianness": "middle"}))
            .unwrap_err()
            .0
            .to_string();
        assert!(err.contains("endianness \"middle\""), "{err}");
        assert!(check_modbus_device(&json!({"endianness": ""})).is_ok());
    }
}
