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

/// Parse register map JSON into typed structure.
pub fn parse_register_map(json: &str) -> Result<Vec<serde_json::Value>, super::error::AiError> {
    let v: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| {
        super::error::AiError::MalformedResponse(format!("Invalid register map JSON: {e}"))
    })?;

    for reg in &v {
        if reg.get("name").and_then(|n| n.as_str()).is_none() {
            return Err(super::error::AiError::MalformedResponse(
                "Register missing 'name' field".into(),
            ));
        }
        if reg.get("address").and_then(|n| n.as_i64()).is_none() {
            return Err(super::error::AiError::MalformedResponse(
                "Register missing valid 'address' field".into(),
            ));
        }
    }

    Ok(v)
}

/// Parse alarm thresholds JSON.
pub fn parse_alarm_thresholds(json: &str) -> Result<Vec<serde_json::Value>, super::error::AiError> {
    let v: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| {
        super::error::AiError::MalformedResponse(format!("Invalid alarm thresholds JSON: {e}"))
    })?;

    for threshold in &v {
        if threshold
            .get("source_key")
            .and_then(|n| n.as_str())
            .is_none()
        {
            return Err(super::error::AiError::MalformedResponse(
                "Threshold missing 'source_key' field".into(),
            ));
        }
    }

    Ok(v)
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
}
