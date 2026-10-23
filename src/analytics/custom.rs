use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::cache::DeviceCache;
use crate::expression;
use crate::models::{CustomMetric, CustomMetricValue};
use crate::storage::Storage;
use crate::timeutil;

/// Parse the formula string to extract identifiers (variable names).
/// Skips numbers (e.g. "100", "1.8"), operators, parentheses, and function names.
/// Returns a deduplicated list of tag key names preserving first-seen order.
///
/// Example: "PUMP_FLOW / PUMP_RATED * 100" -> ["PUMP_FLOW", "PUMP_RATED"]
pub fn extract_tag_refs(formula: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    let chars: Vec<char> = formula.chars().collect();
    let len = chars.len();
    let mut i = 0;

    let functions = ["sqrt", "abs", "round", "clamp", "avg", "if"];

    while i < len {
        match chars[i] {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
            }
            '{' => {
                // Register syntax: {NAME} - extract the name
                let start = i + 1;
                let mut end = start;
                while end < len && chars[end] != '}' {
                    end += 1;
                }
                if end > start && end < len {
                    let name: String = chars[start..end].iter().collect();
                    if !name.is_empty() && seen.insert(name.clone()) {
                        result.push(name);
                    }
                    i = end + 1;
                } else {
                    i += 1;
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                // Skip known function names
                if !functions.contains(&name.as_str()) && seen.insert(name.clone()) {
                    result.push(name);
                }
            }
            c if c.is_ascii_digit() || c == '.' => {
                // Skip numbers
                while i < len && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
            }
            _ => {
                // Operators, parentheses, commas, etc.
                i += 1;
            }
        }
    }

    result
}

/// Convert a formula with bare identifiers to register syntax for the expression engine.
/// E.g. "PUMP_FLOW / PUMP_RATED * 100" -> "{PUMP_FLOW} / {PUMP_RATED} * 100"
fn formula_to_register_syntax(formula: &str) -> String {
    let tag_refs = extract_tag_refs(formula);
    let mut result = formula.to_string();
    for tag in &tag_refs {
        let mut new_result = String::new();
        let chars: Vec<char> = result.chars().collect();
        let len = chars.len();
        let mut i = 0;
        while i < len {
            if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
                let start = i;
                while i < len && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                if word == *tag {
                    new_result.push('{');
                    new_result.push_str(tag);
                    new_result.push('}');
                } else {
                    new_result.push_str(&word);
                }
            } else if chars[i] == '{' {
                // Already in register syntax, pass through
                new_result.push(chars[i]);
                i += 1;
                while i < len && chars[i] != '}' {
                    new_result.push(chars[i]);
                    i += 1;
                }
                if i < len {
                    new_result.push(chars[i]);
                    i += 1;
                }
            } else {
                new_result.push(chars[i]);
                i += 1;
            }
        }
        result = new_result;
    }
    result
}

/// Validate a formula by evaluating it with all extracted tags set to 1.0.
/// Returns Ok(()) if the formula evaluates successfully, or Err with the error message.
pub fn validate_formula(formula: &str) -> Result<(), String> {
    let tag_refs = extract_tag_refs(formula);
    let mut test_values = HashMap::new();
    for tag in &tag_refs {
        test_values.insert(tag.clone(), 1.0);
    }

    let register_formula = formula_to_register_syntax(formula);
    match expression::evaluate(&register_formula, &test_values) {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Resolve tag values from the device cache.
/// Gets all device snapshots and searches for matching metric keys.
pub async fn resolve_tag_values(cache: &DeviceCache, tag_refs: &[String]) -> HashMap<String, f64> {
    let snapshots = cache.get_all().await;
    let mut values = HashMap::new();

    for tag in tag_refs {
        for snapshot in &snapshots {
            if let Some(v) = snapshot.metrics.get(tag) {
                values.insert(tag.clone(), *v);
                break;
            }
        }
    }

    values
}

/// Evaluate a single custom metric against the current device cache.
pub async fn evaluate_metric(
    cache: &DeviceCache,
    metric: &CustomMetric,
) -> Result<CustomMetricValue> {
    let tag_refs: Vec<String> =
        serde_json::from_str(&metric.tag_refs).context("Failed to parse tag_refs JSON")?;

    let tag_values = resolve_tag_values(cache, &tag_refs).await;

    let register_formula = formula_to_register_syntax(&metric.formula);

    let value = expression::evaluate(&register_formula, &tag_values)
        .map_err(|e| anyhow::anyhow!("Formula evaluation failed: {}", e))?;

    Ok(CustomMetricValue {
        id: metric.id,
        name: metric.name.clone(),
        value,
        unit: metric.unit.clone(),
        computed_at: timeutil::now_secs(),
        tag_values,
    })
}

/// Evaluate all custom metrics from storage.
/// Skips metrics that have a validation_error set.
pub async fn evaluate_all_metrics(
    storage: &Storage,
    cache: &DeviceCache,
) -> Result<Vec<CustomMetricValue>> {
    let metrics = storage.list_custom_metrics().await?;
    let mut results = Vec::new();

    for metric in &metrics {
        // Skip metrics with validation errors
        if metric.validation_error.is_some() {
            continue;
        }

        match evaluate_metric(cache, metric).await {
            Ok(value) => results.push(value),
            Err(e) => {
                tracing::warn!(
                    "Failed to evaluate custom metric '{}' (id={}): {}",
                    metric.name,
                    metric.id,
                    e
                );
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tag_refs() {
        let refs = extract_tag_refs("PUMP_FLOW / PUMP_RATED * 100");
        assert_eq!(refs, vec!["PUMP_FLOW", "PUMP_RATED"]);
    }

    #[test]
    fn test_extract_tag_refs_with_numbers() {
        let refs = extract_tag_refs("temp * 1.8 + 32");
        assert_eq!(refs, vec!["temp"]);
        assert!(!refs.contains(&"1.8".to_string()));
        assert!(!refs.contains(&"32".to_string()));
    }

    #[test]
    fn test_extract_tag_refs_complex() {
        let refs = extract_tag_refs("(VOLTAGE_L1 + VOLTAGE_L2 + VOLTAGE_L3) / 3");
        assert_eq!(refs, vec!["VOLTAGE_L1", "VOLTAGE_L2", "VOLTAGE_L3"]);
    }

    #[test]
    fn test_extract_tag_refs_with_braces() {
        let refs = extract_tag_refs("{Voltage_L1} + {Voltage_L2}");
        assert_eq!(refs, vec!["Voltage_L1", "Voltage_L2"]);
    }

    #[test]
    fn test_extract_tag_refs_skips_functions() {
        let refs = extract_tag_refs("sqrt(POWER) + abs(VOLTAGE)");
        assert_eq!(refs, vec!["POWER", "VOLTAGE"]);
    }

    #[test]
    fn test_extract_tag_refs_dedup() {
        let refs = extract_tag_refs("A + A * A");
        assert_eq!(refs, vec!["A"]);
    }

    #[test]
    fn test_validate_formula_valid() {
        assert!(validate_formula("A + B * C").is_ok());
    }

    #[test]
    fn test_validate_formula_with_numbers() {
        assert!(validate_formula("temp * 1.8 + 32").is_ok());
    }

    #[test]
    fn test_validate_formula_invalid() {
        // Division by zero should fail
        assert!(validate_formula("A / (B - B)").is_err());
    }

    #[test]
    fn test_validate_formula_empty() {
        assert!(validate_formula("").is_err());
    }

    #[test]
    fn test_formula_to_register_syntax() {
        let result = formula_to_register_syntax("PUMP_FLOW / PUMP_RATED * 100");
        assert_eq!(result, "{PUMP_FLOW} / {PUMP_RATED} * 100");
    }

    #[test]
    fn test_formula_to_register_syntax_already_braced() {
        let result = formula_to_register_syntax("{Voltage_L1} + {Voltage_L2}");
        assert_eq!(result, "{Voltage_L1} + {Voltage_L2}");
    }

    #[test]
    fn test_evaluate_simple() {
        let mut values = HashMap::new();
        values.insert("A".to_string(), 10.0);
        values.insert("B".to_string(), 2.0);

        let result = expression::evaluate("{A} / {B} * 100", &values).unwrap();
        assert!((result - 500.0).abs() < 0.001);
    }

    #[test]
    fn test_evaluate_with_formula_conversion() {
        let mut values = HashMap::new();
        values.insert("A".to_string(), 10.0);
        values.insert("B".to_string(), 2.0);

        let formula = "A / B * 100";
        let register_formula = formula_to_register_syntax(formula);
        let result = expression::evaluate(&register_formula, &values).unwrap();
        assert!((result - 500.0).abs() < 0.001);
    }

    #[test]
    fn test_evaluate_with_functions() {
        let mut values = HashMap::new();
        values.insert("POWER".to_string(), 25.0);

        let formula = "sqrt(POWER)";
        let register_formula = formula_to_register_syntax(formula);
        let result = expression::evaluate(&register_formula, &values).unwrap();
        assert!((result - 5.0).abs() < 1e-10);
    }

    #[tokio::test]
    async fn test_resolve_tag_values() {
        use crate::cache::DeviceCache;

        let cache = DeviceCache::new();
        let metrics = HashMap::from([
            ("PUMP_FLOW".to_string(), 150.0),
            ("PUMP_RATED".to_string(), 200.0),
        ]);
        cache.update("pump_1", metrics, false).await;

        let tag_refs = vec!["PUMP_FLOW".to_string(), "PUMP_RATED".to_string()];
        let values = resolve_tag_values(&cache, &tag_refs).await;

        assert_eq!(values.get("PUMP_FLOW"), Some(&150.0));
        assert_eq!(values.get("PUMP_RATED"), Some(&200.0));
    }

    #[tokio::test]
    async fn test_resolve_tag_values_missing() {
        use crate::cache::DeviceCache;

        let cache = DeviceCache::new();
        let metrics = HashMap::from([("VOLTAGE".to_string(), 220.0)]);
        cache.update("dev1", metrics, false).await;

        let tag_refs = vec!["VOLTAGE".to_string(), "MISSING".to_string()];
        let values = resolve_tag_values(&cache, &tag_refs).await;

        assert_eq!(values.get("VOLTAGE"), Some(&220.0));
        assert!(values.get("MISSING").is_none());
    }

    #[tokio::test]
    async fn test_evaluate_metric() {
        use crate::cache::DeviceCache;
        use crate::models::CustomMetric;

        let cache = DeviceCache::new();
        let metrics = HashMap::from([
            ("PUMP_FLOW".to_string(), 150.0),
            ("PUMP_RATED".to_string(), 200.0),
        ]);
        cache.update("pump_1", metrics, false).await;

        let metric = CustomMetric {
            id: 1,
            name: "Pump Efficiency".to_string(),
            description: "Pump flow efficiency".to_string(),
            formula: "PUMP_FLOW / PUMP_RATED * 100".to_string(),
            unit: Some("%".to_string()),
            tag_refs: r#"["PUMP_FLOW","PUMP_RATED"]"#.to_string(),
            validation_error: None,
            created_at: 1000,
            updated_at: 1000,
        };

        let result = evaluate_metric(&cache, &metric).await.unwrap();
        assert_eq!(result.id, 1);
        assert_eq!(result.name, "Pump Efficiency");
        assert!((result.value - 75.0).abs() < 0.001);
        assert_eq!(result.unit, Some("%".to_string()));
        assert_eq!(result.tag_values.get("PUMP_FLOW"), Some(&150.0));
        assert_eq!(result.tag_values.get("PUMP_RATED"), Some(&200.0));
    }
}
