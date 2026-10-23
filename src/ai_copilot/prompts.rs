pub fn register_map_system_prompt() -> &'static str {
    r#"You are a Modbus register mapping assistant for a SCADA system. You extract register definitions from device datasheets and return them as structured JSON.

Rules:
- Extract ONLY register addresses, data types, scaling factors, and units
- Use standard Modbus 5-digit notation (e.g., 40001 for holding register 1)
- Valid data types: int16, uint16, int32, uint32, float32, float64, bool
- Valid register areas: holding, input, coil, discrete
- Scale factor is a multiplier (e.g., 0.1 means raw value * 0.1 = engineering value)
- If information is unclear, set scale to 1.0 and note in description

Return a JSON array:
```json
[
  {
    "name": "descriptive_tag_name",
    "address": 40001,
    "register_type": "float32",
    "register_area": "holding",
    "scale": 1.0,
    "unit": "V",
    "description": "Line voltage L1-N"
  }
]
```

Use snake_case for tag names. Be descriptive but concise. Return ONLY the JSON array, no other text."#
}

pub fn register_map_user_prompt(datasheet_text: &str, device_name: &str) -> String {
    format!(
        "Extract all Modbus register definitions from this datasheet for device '{}'.\n\nDatasheet content:\n{}",
        device_name, datasheet_text
    )
}

pub fn dashboard_layout_system_prompt() -> &'static str {
    r#"You are a SCADA dashboard layout assistant. Given a description of plant equipment and available data sources (tags), generate a dashboard layout with appropriate widget types and grid positions.

Widget types available:
- "sparkline": Real-time trend chart (good for continuous values like temperature, flow, pressure)
- "big_number": Large numeric display with optional progress bar (good for totals, key metrics)
- "gauge": Circular gauge with min/max (good for pressure, temperature, tank levels)
- "led": Binary status indicator (good for running/stopped, open/closed)

Layout grid: 12 columns, widgets positioned by {x, y, w, h}.

Rules:
- Group related equipment together (e.g., all pump metrics near each other)
- Use gauges for analog values with clear operating ranges
- Use sparklines for values that trend over time
- Use big_number for totals, counts, energy values
- Use led for binary/digital status points
- Leave reasonable spacing between groups

Return JSON:
```json
{
  "widgets": [
    {
      "data_source_key": "device:tag_name",
      "chart_type": "gauge",
      "label": "Pump 1 Pressure",
      "group": "pump_1"
    }
  ],
  "layout": [
    {
      "data_source_key": "device:tag_name",
      "x": 0, "y": 0, "w": 3, "h": 2
    }
  ]
}
```

Return ONLY the JSON object, no other text."#
}

pub fn dashboard_layout_user_prompt(description: &str, available_tags: &str) -> String {
    format!(
        "Plant description: {}\n\nAvailable data sources (key: name [unit]):\n{}",
        description, available_tags
    )
}

pub fn alarm_threshold_system_prompt() -> &'static str {
    r#"You are an industrial alarm engineering assistant following ISA-18.2 standards. Given a device type and its tags, suggest alarm thresholds.

Rules:
- Suggest warning and critical thresholds based on typical operating ranges for the device type
- Warning threshold: value that indicates abnormal but manageable condition
- Critical threshold: value that indicates dangerous or equipment-damaging condition
- For some tags (like status booleans), alarm thresholds may not apply — skip those
- Provide a brief justification for each threshold

Return JSON:
```json
[
  {
    "source_key": "device:tag_name",
    "warn_val": 80.0,
    "crit_val": 95.0,
    "justification": "Motor temperature exceeding 80°C indicates potential overheating"
  }
]
```

Return ONLY the JSON array, no other text."#
}

pub fn alarm_threshold_user_prompt(device_type: &str, tags: &str) -> String {
    format!(
        "Device type: {}\n\nTags to set thresholds for (key: name [unit] range min-max):\n{}",
        device_type, tags
    )
}

pub fn tag_rename_system_prompt() -> &'static str {
    r#"You are an industrial tag naming assistant. Convert tag names to follow ISA-5.1 or other specified naming conventions.

ISA-5.1 rules:
- Format: UUU-TNNN-S where UUU=unit, T=type, NNN=number, S=suffix
- Types: TT=temperature, PT=pressure, FT=flow, LT=level, MT=motor, VT=valve, etc.
- Use uppercase letters, numbers, and underscores only
- Keep names under 30 characters

Return JSON:
```json
[
  {
    "current": "Pump1_Start",
    "proposed": "PMP001_RUN",
    "type": "motor_status"
  }
]
```

Return ONLY the JSON array, no other text."#
}

pub fn tag_rename_user_prompt(
    tags: &str,
    convention: &str,
    custom_pattern: Option<&str>,
) -> String {
    let mut prompt = format!(
        "Convert these tag names to {} convention.\n\nCurrent tag names:\n{}",
        convention, tags
    );
    if let Some(pattern) = custom_pattern {
        prompt.push_str(&format!("\n\nCustom pattern: {}", pattern));
    }
    prompt
}

pub fn flow_rule_system_prompt() -> &'static str {
    r#"You are a SCADA automation rule assistant. Given a natural language description and available tags, generate a data flow rule.

Available node types:
- "filter": Conditional filtering. Fields: {id, condition}
- "threshold": Threshold checking. Fields: {id, field, rules: [{operator, value, action}]}
- "expression": Calculate values. Fields: {id, output_field, expr}
- "alarm_out": Create alarm. Fields: {id, severity, source_key, message}
- "http_out": Send webhook. Fields: {id, url, headers, timeout_ms}
- "data_source_out": Write to data source. Fields: {id, source_key, name, unit, value_field}

Available trigger types:
- {"type": "modbus", "device_id": "DEVICE_ID"} — triggers on device data update
- {"type": "timer", "interval_ms": 60000} — triggers on time interval

Return JSON:
```json
{
  "name": "Human-readable rule name",
  "trigger_config": {"type": "modbus", "device_id": "DEVICE_ID"},
  "nodes": [
    {"id": "n1", "type": "filter", "condition": "value > 80"},
    {"id": "n2", "type": "alarm_out", "severity": "critical", "source_key": "device:tag", "message": "High value detected"}
  ]
}
```

Rules:
- Each node must have a unique id (n1, n2, n3...)
- Nodes execute in order
- Use actual source_key values from the provided tag list
- Keep rules simple and deterministic

Return ONLY the JSON object, no other text."#
}

pub fn flow_rule_user_prompt(description: &str, available_tags: &str) -> String {
    format!(
        "Rule description: {}\n\nAvailable tags (source_key: name [unit]):\n{}",
        description, available_tags
    )
}

/// Strip potentially sensitive information from prompts before sending to AI provider.
/// Replaces IP address patterns (digits.digits.digits.digits) with [IP_REDACTED].
pub fn scrub_prompt(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            // Try to match IP pattern: d+.d+.d+.d+
            if let Some(end) = try_match_ip(&chars, i) {
                result.push_str("[IP_REDACTED]");
                i = end;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Try to match an IPv4 address starting at position `start`.
/// Returns the end index if matched, or None.
fn try_match_ip(chars: &[char], start: usize) -> Option<usize> {
    let mut pos = start;
    let mut octets = 0u8;

    loop {
        // Parse a number (1-3 digits)
        let num_start = pos;
        while pos < chars.len() && chars[pos].is_ascii_digit() {
            pos += 1;
        }
        let digit_count = pos - num_start;
        if digit_count == 0 || digit_count > 3 {
            return None;
        }
        octets += 1;

        if octets == 4 {
            // Check that the next char is not a digit or dot (word boundary)
            if pos < chars.len() && (chars[pos].is_ascii_digit() || chars[pos] == '.') {
                return None;
            }
            return Some(pos);
        }

        // Expect a dot
        if pos >= chars.len() || chars[pos] != '.' {
            return None;
        }
        pos += 1; // skip the dot
    }
}
