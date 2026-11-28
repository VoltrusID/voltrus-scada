use serde_json::Value;

/// Extract JSON from an AI response that may be wrapped in markdown code blocks.
pub fn extract_json(response: &str) -> Result<String, super::error::AiError> {
    let trimmed = response.trim();

    // Try direct parse first
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Ok(serde_json::to_string(&v).unwrap());
    }

    // Try extracting from markdown code block
    if let Some(json) = extract_from_code_block(trimmed)
        && let Ok(v) = serde_json::from_str::<Value>(&json)
    {
        return Ok(serde_json::to_string(&v).unwrap());
    }

    // Try finding JSON array or object in the response
    if let Some(json) = find_json_structure(trimmed) {
        return Ok(json);
    }

    Err(super::error::AiError::MalformedResponse(
        "Could not extract valid JSON from AI response".into(),
    ))
}

fn extract_from_code_block(text: &str) -> Option<String> {
    // Match ```json ... ``` or ``` ... ```
    let patterns = ["```json", "```JSON", "```"];
    for pattern in patterns {
        if let Some(start_idx) = text.find(pattern) {
            let content_start = start_idx + pattern.len();
            if let Some(end_idx) = text[content_start..].find("```") {
                return Some(
                    text[content_start..content_start + end_idx]
                        .trim()
                        .to_string(),
                );
            }
        }
    }
    None
}

fn find_json_structure(text: &str) -> Option<String> {
    // Find first [ or { and match to closing bracket
    let start = text.find(['[', '{'])?;
    let open = text.chars().nth(start)?;
    let close = if open == '[' { ']' } else { '}' };

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let chars: Vec<char> = text[start..].chars().collect();

    for (i, &ch) in chars.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                let candidate: String = chars[..=i].iter().collect();
                if serde_json::from_str::<Value>(&candidate).is_ok() {
                    return Some(candidate);
                }
                return None;
            }
        }
    }
    None
}

/// Parse a register map and check every register against what the Modbus
/// driver can poll (see [`check_register_map`]).
pub fn parse_register_map(json: &str) -> Result<Vec<serde_json::Value>, super::error::AiError> {
    let v: Value = serde_json::from_str(json).map_err(|e| {
        super::error::AiError::MalformedResponse(format!("Invalid register map JSON: {e}"))
    })?;
    check_register_map(&v).map_err(super::error::AiError::MalformedResponse)
}

/// Check a register map and return it normalized: each register gets
/// `name`, `address` (5-digit reference) or `offset` (0-based),
/// `register_type`, `register_area` and `scale`, plus `unit` and
/// `description` when given. Anything the driver cannot poll as given is an
/// error naming the register: no silent defaults, truncation or remapping.
pub fn check_register_map(v: &Value) -> Result<Vec<Value>, String> {
    let items = v
        .as_array()
        .ok_or("A register map must be a JSON array of registers")?;
    if items.is_empty() {
        return Err("The register map has no registers".into());
    }
    let mut names = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for (i, reg) in items.iter().enumerate() {
        let r = check_register(reg).map_err(|e| format!("register {}: {e}", i + 1))?;
        let name = r["name"].as_str().unwrap_or_default().to_string();
        if !names.insert(name.clone()) {
            return Err(format!(
                "register {}: the name '{name}' is used twice",
                i + 1
            ));
        }
        out.push(r);
    }
    Ok(out)
}

fn check_register(reg: &Value) -> Result<Value, String> {
    use crate::models::{REGISTER_AREAS, RegisterType};

    let obj = reg.as_object().ok_or("must be a JSON object")?;
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or("'name' is missing or empty")?;
    let ctx = |msg: String| format!("'{name}': {msg}");

    let whole = |key: &str| -> Result<Option<u64>, String> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_u64() {
                Some(n) => Ok(Some(n)),
                None => Err(ctx(format!(
                    "'{key}' must be a whole number of 0 or more, not {v}"
                ))),
            },
        }
    };
    let address = whole("address")?;
    let offset = whole("offset")?;

    let area = match obj.get("register_area") {
        None | Some(Value::Null) => None,
        Some(Value::String(a)) => Some(a.trim().to_ascii_lowercase()),
        Some(v) => return Err(ctx(format!("'register_area' must be a string, not {v}"))),
    };
    if let Some(a) = &area
        && !REGISTER_AREAS.contains(&a.as_str())
    {
        return Err(ctx(format!(
            "register_area '{a}' is not supported; use one of {}",
            REGISTER_AREAS.join(", ")
        )));
    }

    let mut out = serde_json::Map::new();
    out.insert("name".into(), Value::from(name));
    let area = match (address, offset) {
        (Some(_), Some(_)) => return Err(ctx("give 'address' or 'offset', not both".into())),
        (None, None) => return Err(ctx("'address' or 'offset' is required".into())),
        (None, Some(off)) => {
            if off > u16::MAX as u64 {
                return Err(ctx(format!("offset {off} is out of range (0-65535)")));
            }
            out.insert("offset".into(), Value::from(off));
            area.ok_or_else(|| ctx("'register_area' is required with 'offset'".into()))?
        }
        (Some(addr), None) => {
            let implied = match addr {
                30001..=39999 => "input",
                40001..=65535 => "holding",
                65536.. => {
                    return Err(ctx(format!(
                        "address {addr} is out of range: 6-digit references are not supported (the largest address is 65535); give the 0-based 'offset' instead"
                    )));
                }
                _ => {
                    return Err(ctx(format!(
                        "address {addr} is not a 5-digit holding (40001-65535) or input (30001-39999) reference; give the 0-based 'offset' with 'register_area' instead"
                    )));
                }
            };
            if let Some(a) = &area
                && a != implied
            {
                return Err(ctx(format!(
                    "address {addr} is a {implied} register reference but register_area is '{a}'"
                )));
            }
            out.insert("address".into(), Value::from(addr));
            area.unwrap_or_else(|| implied.to_string())
        }
    };

    let reg_type = match obj.get("register_type") {
        Some(Value::String(t)) => t.trim().to_ascii_lowercase(),
        None | Some(Value::Null) if area == "coil" || area == "discrete" => "uint16".into(),
        None | Some(Value::Null) => return Err(ctx("'register_type' is required".into())),
        Some(v) => return Err(ctx(format!("'register_type' must be a string, not {v}"))),
    };
    if !RegisterType::NAMES.contains(&reg_type.as_str()) {
        return Err(ctx(format!(
            "register_type '{reg_type}' is not supported by the Modbus driver; use one of {}",
            RegisterType::NAMES.join(", ")
        )));
    }

    let scale = match obj.get("scale") {
        None | Some(Value::Null) => 1.0,
        Some(v) => v
            .as_f64()
            .filter(|s| s.is_finite() && *s != 0.0)
            .ok_or_else(|| ctx(format!("'scale' must be a non-zero number, not {v}")))?,
    };

    out.insert("register_type".into(), Value::from(reg_type));
    out.insert("register_area".into(), Value::from(area));
    out.insert("scale".into(), Value::from(scale));
    for key in ["unit", "description"] {
        if let Some(Value::String(text)) = obj.get(key) {
            out.insert(key.into(), Value::from(text.as_str()));
        }
    }
    Ok(Value::Object(out))
}

/// Parse alarm thresholds JSON (see [`check_alarm_thresholds`]).
pub fn parse_alarm_thresholds(json: &str) -> Result<Vec<serde_json::Value>, super::error::AiError> {
    let v: Value = serde_json::from_str(json).map_err(|e| {
        super::error::AiError::MalformedResponse(format!("Invalid alarm thresholds JSON: {e}"))
    })?;
    check_alarm_thresholds(&v).map_err(super::error::AiError::MalformedResponse)
}

/// Check alarm thresholds: each names its tag by `source_key` (and `tag_id`
/// when known), gives `warn_val` and/or `crit_val` as numbers, and a warning
/// below the critical limit (both are high limits; 0 disables one).
pub fn check_alarm_thresholds(v: &Value) -> Result<Vec<Value>, String> {
    let items = v
        .as_array()
        .ok_or("Alarm thresholds must be a JSON array")?;
    for (i, t) in items.iter().enumerate() {
        let n = i + 1;
        let key = t
            .get("source_key")
            .and_then(Value::as_str)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| format!("threshold {n}: 'source_key' is missing"))?;
        match t.get("tag_id") {
            None | Some(Value::Null) => {}
            Some(id) if id.as_i64().is_some() => {}
            Some(id) => {
                return Err(format!(
                    "threshold {n} ('{key}'): 'tag_id' must be a whole number, not {id}"
                ));
            }
        }
        let num = |field: &str| -> Result<Option<f64>, String> {
            match t.get(field) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_f64()
                    .filter(|f| f.is_finite())
                    .map(Some)
                    .ok_or_else(|| {
                        format!("threshold {n} ('{key}'): '{field}' must be a number, not {v}")
                    }),
            }
        };
        let warn = num("warn_val")?;
        let crit = num("crit_val")?;
        if warn.is_none() && crit.is_none() {
            return Err(format!(
                "threshold {n} ('{key}'): give 'warn_val' and/or 'crit_val'"
            ));
        }
        if let (Some(w), Some(c)) = (warn, crit)
            && w != 0.0
            && c != 0.0
            && w >= c
        {
            return Err(format!(
                "threshold {n} ('{key}'): warn_val {w} must be below crit_val {c} (both are high limits)"
            ));
        }
    }
    Ok(items.clone())
}

/// Check the configuration of a proposal of `proposal_type` before it is
/// approved (an edited configuration) or applied.
pub fn check_proposal_config(proposal_type: &str, config: &Value) -> Result<(), String> {
    match proposal_type {
        "register_map" => check_register_map(config).map(drop),
        "alarm_thresholds" => check_alarm_thresholds(config).map(drop),
        "tag_rename" => parse_tag_rename(&config.to_string())
            .map(drop)
            .map_err(|e| e.to_string()),
        "dashboard_layout" => parse_dashboard_layout(&config.to_string())
            .map(drop)
            .map_err(|e| e.to_string()),
        "flow_rule" => parse_flow_rule(&config.to_string())
            .map(drop)
            .map_err(|e| e.to_string()),
        "equipment_template" => check_equipment_template(config).map(drop),
        other => Err(format!("Unknown proposal type: {other}")),
    }
}

/// Check an equipment template proposal and return the tags it makes as
/// `(name, tag)`: a JSON object whose `tags` is a non-empty array, each tag an
/// object with a non-empty `name_pattern` (`{n}` becomes 1) and, when given,
/// a string `unit`, a Modbus `register_area` and a Modbus `default_type`. Two
/// tags with one name are an error.
pub fn check_equipment_template(config: &Value) -> Result<Vec<(String, Value)>, String> {
    use crate::models::{REGISTER_AREAS, RegisterType};

    let obj = config
        .as_object()
        .ok_or("An equipment template must be a JSON object")?;
    let tags = obj
        .get("tags")
        .and_then(Value::as_array)
        .ok_or("The equipment template has no 'tags' array")?;
    if tags.is_empty() {
        return Err("The equipment template has no tags".into());
    }
    let mut names = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(tags.len());
    for (i, tag) in tags.iter().enumerate() {
        let n = i + 1;
        let t = tag
            .as_object()
            .ok_or_else(|| format!("tag {n}: must be a JSON object"))?;
        let name = t
            .get("name_pattern")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| format!("tag {n}: 'name_pattern' is missing or empty"))?
            .replace("{n}", "1");
        let text = |key: &str| -> Result<Option<String>, String> {
            match t.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
                Some(v) => Err(format!(
                    "tag {n} ('{name}'): '{key}' must be a string, not {v}"
                )),
            }
        };
        text("unit")?;
        if let Some(area) = text("register_area")?
            && !REGISTER_AREAS.contains(&area.to_ascii_lowercase().as_str())
        {
            return Err(format!(
                "tag {n} ('{name}'): register_area '{area}' is not supported; use one of {}",
                REGISTER_AREAS.join(", ")
            ));
        }
        if let Some(ty) = text("default_type")?
            && !RegisterType::NAMES.contains(&ty.to_ascii_lowercase().as_str())
        {
            return Err(format!(
                "tag {n} ('{name}'): default_type '{ty}' is not supported; use one of {}",
                RegisterType::NAMES.join(", ")
            ));
        }
        if !names.insert(name.clone()) {
            return Err(format!("the template makes the tag '{name}' twice"));
        }
        out.push((name, tag.clone()));
    }
    Ok(out)
}

/// Parse tag rename mapping JSON.
pub fn parse_tag_rename(json: &str) -> Result<Vec<serde_json::Value>, super::error::AiError> {
    let v: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| {
        super::error::AiError::MalformedResponse(format!("Invalid tag rename JSON: {e}"))
    })?;

    for entry in &v {
        if entry.get("current").and_then(|n| n.as_str()).is_none() {
            return Err(super::error::AiError::MalformedResponse(
                "Rename entry missing 'current' field".into(),
            ));
        }
        if entry.get("proposed").and_then(|n| n.as_str()).is_none() {
            return Err(super::error::AiError::MalformedResponse(
                "Rename entry missing 'proposed' field".into(),
            ));
        }
        if entry
            .get("tag_id")
            .is_some_and(|id| !id.is_null() && id.as_i64().is_none())
        {
            return Err(super::error::AiError::MalformedResponse(
                "Rename entry 'tag_id' must be a whole number".into(),
            ));
        }
    }

    Ok(v)
}

/// Parse dashboard layout JSON.
pub fn parse_dashboard_layout(json: &str) -> Result<serde_json::Value, super::error::AiError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| {
        super::error::AiError::MalformedResponse(format!("Invalid dashboard layout JSON: {e}"))
    })?;

    if v.get("widgets").is_none() {
        return Err(super::error::AiError::MalformedResponse(
            "Dashboard layout missing 'widgets' field".into(),
        ));
    }

    Ok(v)
}

/// Parse flow rule JSON.
pub fn parse_flow_rule(json: &str) -> Result<serde_json::Value, super::error::AiError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| {
        super::error::AiError::MalformedResponse(format!("Invalid flow rule JSON: {e}"))
    })?;

    if v.get("trigger_config").is_none() {
        return Err(super::error::AiError::MalformedResponse(
            "Flow rule missing 'trigger_config' field".into(),
        ));
    }
    if v.get("nodes").is_none() {
        return Err(super::error::AiError::MalformedResponse(
            "Flow rule missing 'nodes' field".into(),
        ));
    }

    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_direct() {
        let input = r#"{"key": "value"}"#;
        let result = extract_json(input).unwrap();
        assert_eq!(result, r#"{"key":"value"}"#);
    }

    #[test]
    fn test_extract_json_from_code_block() {
        let input = "Here are the registers:\n```json\n[{\"name\": \"temp\"}]\n```\nThat's all.";
        let result = extract_json(input).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed[0]["name"], "temp");
    }

    #[test]
    fn test_extract_json_from_code_block_no_lang() {
        let input = "```\n[{\"name\": \"voltage\"}]\n```";
        let result = extract_json(input).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed[0]["name"], "voltage");
    }

    #[test]
    fn test_extract_json_embedded() {
        let input = "Here is the result: [{\"name\": \"pressure\"}] end.";
        let result = extract_json(input).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed[0]["name"], "pressure");
    }

    #[test]
    fn test_extract_json_failure() {
        let input = "No JSON here at all";
        assert!(extract_json(input).is_err());
    }

    #[test]
    fn test_parse_register_map_valid() {
        let json = r#"[{"name": "voltage", "address": 40001, "register_type": "float32", "register_area": "holding", "scale": 1.0, "unit": "V"}]"#;
        let result = parse_register_map(json).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["name"], "voltage");
    }

    #[test]
    fn test_parse_register_map_missing_name() {
        let json = r#"[{"address": 40001}]"#;
        assert!(parse_register_map(json).is_err());
    }

    #[test]
    fn test_parse_tag_rename_valid() {
        let json = r#"[{"current": "Pump1_Start", "proposed": "PMP001_RUN"}]"#;
        let result = parse_tag_rename(json).unwrap();
        assert_eq!(result[0]["current"], "Pump1_Start");
        assert_eq!(result[0]["proposed"], "PMP001_RUN");
    }

    #[test]
    fn test_scrub_ips() {
        use super::super::prompts::scrub_prompt;
        let input = "Device at 192.168.1.100 and 10.0.0.1";
        let scrubbed = scrub_prompt(input);
        assert!(!scrubbed.contains("192.168.1.100"));
        assert!(!scrubbed.contains("10.0.0.1"));
        assert!(scrubbed.contains("[IP_REDACTED]"));
    }

    #[test]
    fn register_map_rejects_types_the_driver_lacks() {
        for t in ["uint8", "bool", "string", "int128"] {
            let json = format!(r#"[{{"name": "v", "address": 40001, "register_type": "{t}"}}]"#);
            let err = parse_register_map(&json).unwrap_err().to_string();
            assert!(err.contains(&format!("'{t}' is not supported")), "{err}");
            assert!(
                err.contains("int16, uint16, int32, uint32, int64, uint64, float32, float64"),
                "{err}"
            );
        }
        for t in crate::models::RegisterType::NAMES {
            let json = format!(r#"[{{"name": "v", "address": 40001, "register_type": "{t}"}}]"#);
            assert!(parse_register_map(&json).is_ok(), "{t}");
        }
        let err = parse_register_map(
            r#"[{"name": "v", "offset": 3, "register_area": "eeprom", "register_type": "int16"}]"#,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("register_area 'eeprom' is not supported"),
            "{err}"
        );
        // A missing type is an error, not a silent float32.
        assert!(parse_register_map(r#"[{"name": "v", "address": 40001}]"#).is_err());
    }

    #[test]
    fn register_addresses_out_of_range_are_rejected_not_truncated() {
        for (addr, want) in [
            ("400001", "6-digit references are not supported"),
            ("65536", "6-digit references are not supported"),
            ("-1", "whole number"),
            ("40001.5", "whole number"),
            ("\"40001\"", "whole number"),
            ("0", "not a 5-digit"),
            ("10001", "not a 5-digit"),
        ] {
            let json = format!(r#"[{{"name": "v", "address": {addr}, "register_type": "int16"}}]"#);
            let err = parse_register_map(&json).unwrap_err().to_string();
            assert!(err.contains(want), "{addr}: {err}");
        }
        let err =
            parse_register_map(r#"[{"name": "v", "offset": 70000, "register_area": "coil"}]"#)
                .unwrap_err()
                .to_string();
        assert!(err.contains("out of range (0-65535)"), "{err}");
        let err = parse_register_map(r#"[{"name": "v", "address": 30010, "register_area": "holding", "register_type": "int16"}]"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("input register reference"), "{err}");
        assert!(
            parse_register_map(
                r#"[{"name": "v", "address": 40001, "offset": 0, "register_type": "int16"}]"#
            )
            .is_err()
        );
        assert!(parse_register_map(r#"[{"name": "v"}]"#).is_err());

        let ok = parse_register_map(
            r#"[{"name": "a", "address": 65535, "register_type": "uint16"},
                {"name": "b", "address": 30001, "register_type": "int16", "unit": "V"},
                {"name": "c", "offset": 0, "register_area": "coil"}]"#,
        )
        .unwrap();
        assert_eq!(ok[0]["register_area"], "holding");
        assert_eq!(ok[1]["register_area"], "input");
        assert_eq!(ok[1]["unit"], "V");
        assert_eq!(ok[2]["offset"], 0);
        assert_eq!(ok[2]["register_type"], "uint16");
        assert_eq!(ok[0]["scale"], 1.0);
    }

    #[test]
    fn register_names_must_be_unique() {
        let err = parse_register_map(
            r#"[{"name": "v", "address": 40001, "register_type": "int16"},
                {"name": "v", "address": 40002, "register_type": "int16"}]"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("used twice"), "{err}");
    }

    #[test]
    fn alarm_thresholds_need_numbers_and_warn_below_crit() {
        assert!(
            parse_alarm_thresholds(r#"[{"source_key": "t", "warn_val": 80, "crit_val": 95}]"#)
                .is_ok()
        );
        assert!(
            parse_alarm_thresholds(r#"[{"source_key": "t", "tag_id": 3, "crit_val": 95}]"#).is_ok()
        );
        for bad in [
            r#"[{"source_key": "t", "warn_val": 95, "crit_val": 80}]"#,
            r#"[{"source_key": "t", "warn_val": "80"}]"#,
            r#"[{"source_key": "t"}]"#,
            r#"[{"source_key": "t", "tag_id": "x", "warn_val": 1}]"#,
            r#"[{"warn_val": 1}]"#,
            r#"{"source_key": "t"}"#,
        ] {
            assert!(parse_alarm_thresholds(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn proposal_configs_are_checked_by_type() {
        use serde_json::json;
        assert!(
            check_proposal_config(
                "register_map",
                &json!([{"name": "v", "address": 40001, "register_type": "int16"}])
            )
            .is_ok()
        );
        assert!(
            check_proposal_config(
                "register_map",
                &json!([{"name": "v", "address": 400001, "register_type": "int16"}])
            )
            .is_err()
        );
        assert!(check_proposal_config("register_map", &json!("not a map")).is_err());
        assert!(check_proposal_config("tag_rename", &json!([{"current": "a"}])).is_err());
        assert!(check_proposal_config("flow_rule", &json!({"nodes": []})).is_err());
        assert!(check_proposal_config("dashboard_layout", &json!({"layout": []})).is_err());
        assert!(check_proposal_config("nope", &json!({})).is_err());
    }
}
