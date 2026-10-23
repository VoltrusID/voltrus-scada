use super::evaluator::{evaluate, evaluate_json};
use super::token::tokenize;
use std::collections::HashMap;

fn vals() -> HashMap<String, f64> {
    HashMap::from([
        ("Voltage_L1".into(), 220.0),
        ("Voltage_L2".into(), 221.0),
        ("Voltage_L3".into(), 219.0),
        ("Active_Power".into(), 5000.0),
    ])
}

#[test]
fn test_single_register() {
    assert_eq!(evaluate("{Voltage_L1}", &vals()).unwrap(), 220.0);
}

#[test]
fn test_addition() {
    assert_eq!(
        evaluate("{Voltage_L1} + {Voltage_L2}", &vals()).unwrap(),
        441.0
    );
}

#[test]
fn test_multiplication_with_constant() {
    assert_eq!(evaluate("{Active_Power} * 0.001", &vals()).unwrap(), 5.0);
}

#[test]
fn test_precedence() {
    assert_eq!(
        evaluate("{Voltage_L1} + {Voltage_L2} * 0.5", &vals()).unwrap(),
        330.5
    );
}

#[test]
fn test_parentheses() {
    assert_eq!(
        evaluate("({Voltage_L1} + {Voltage_L2}) * 0.5", &vals()).unwrap(),
        220.5
    );
}

#[test]
fn test_subtraction_and_division() {
    assert_eq!(
        evaluate("({Active_Power} - {Voltage_L1}) / 2", &vals()).unwrap(),
        2390.0
    );
}

#[test]
fn test_three_registers() {
    assert_eq!(
        evaluate("{Voltage_L1} + {Voltage_L2} + {Voltage_L3}", &vals()).unwrap(),
        660.0
    );
}

#[test]
fn test_unary_minus() {
    assert_eq!(evaluate("-{Voltage_L1}", &vals()).unwrap(), -220.0);
}

#[test]
fn test_unknown_register() {
    assert!(evaluate("{Unknown_Reg}", &vals()).is_err());
}

#[test]
fn test_division_by_zero() {
    assert!(evaluate("{Voltage_L1} / 0", &vals()).is_err());
}

#[test]
fn test_unclosed_brace() {
    assert!(tokenize("{Voltage_L1").is_err());
}

#[test]
fn test_empty_expression() {
    assert!(evaluate("", &vals()).is_err());
}

#[test]
fn test_invalid_character() {
    assert!(tokenize("{Voltage_L1} & {Voltage_L2}").is_err());
}

#[test]
fn test_whitespace_ignored() {
    assert_eq!(
        evaluate("  {Voltage_L1}  +  {Voltage_L2}  ", &vals()).unwrap(),
        441.0
    );
}

#[test]
fn test_sqrt() {
    let v = evaluate("sqrt(9)", &HashMap::new()).unwrap();
    assert!((v - 3.0).abs() < 1e-10);
}

#[test]
fn test_sqrt_with_register() {
    let v = evaluate("sqrt({Active_Power})", &vals()).unwrap();
    assert!((v - 70.7107).abs() < 0.01);
}

#[test]
fn test_sqrt_in_expression() {
    let v = evaluate("sqrt({Active_Power} * {Active_Power})", &vals()).unwrap();
    assert!((v - 5000.0).abs() < 0.1);
}

#[test]
fn test_abs() {
    let v = evaluate("abs(-{Voltage_L1})", &vals()).unwrap();
    assert_eq!(v, 220.0);
}

#[test]
fn test_unknown_function() {
    assert!(evaluate("foo(1)", &HashMap::new()).is_err());
}

#[test]
fn test_sqrt_line_voltage() {
    let v = evaluate("sqrt(3) * {Voltage_L1}", &vals()).unwrap();
    assert!((v - 381.05).abs() < 0.5);
}

// New tests for JSON and extended features
#[test]
fn test_json_path() {
    let context = serde_json::json!({
        "payload": {
            "metrics": {
                "temperature": 85.0
            }
        }
    });
    let result = evaluate_json("payload.metrics.temperature", &context).unwrap();
    assert_eq!(result, serde_json::json!(85.0));
}

#[test]
fn test_comparison_gt() {
    let context = serde_json::json!({
        "payload": {
            "metrics": {
                "temperature": 85.0
            }
        }
    });
    let result = evaluate_json("payload.metrics.temperature > 80", &context).unwrap();
    assert_eq!(result, serde_json::json!(true));
}

#[test]
fn test_comparison_lt() {
    let context = serde_json::json!({
        "temp": 75.0
    });
    let result = evaluate_json("temp < 80", &context).unwrap();
    assert_eq!(result, serde_json::json!(true));
}

#[test]
fn test_comparison_eq() {
    let context = serde_json::json!({
        "temp": 80.0
    });
    let result = evaluate_json("temp == 80", &context).unwrap();
    assert_eq!(result, serde_json::json!(true));
}

#[test]
fn test_logical_and() {
    let context = serde_json::json!({
        "a": true,
        "b": true
    });
    let result = evaluate_json("a && b", &context).unwrap();
    assert_eq!(result, serde_json::json!(true));
}

#[test]
fn test_logical_and_short_circuit() {
    let context = serde_json::json!({
        "a": false,
        "b": true
    });
    let result = evaluate_json("a && b", &context).unwrap();
    assert_eq!(result, serde_json::json!(false));
}

#[test]
fn test_logical_or() {
    let context = serde_json::json!({
        "a": false,
        "b": true
    });
    let result = evaluate_json("a || b", &context).unwrap();
    assert_eq!(result, serde_json::json!(true));
}

#[test]
fn test_logical_or_short_circuit() {
    let context = serde_json::json!({
        "a": true,
        "b": false
    });
    let result = evaluate_json("a || b", &context).unwrap();
    assert_eq!(result, serde_json::json!(true));
}

#[test]
fn test_if_function() {
    let context = serde_json::json!({
        "payload": {
            "metrics": {
                "temperature": 85.0
            }
        }
    });
    let result = evaluate_json(
        "if(payload.metrics.temperature > 80, \"CRITICAL\", \"OK\")",
        &context,
    )
    .unwrap();
    assert_eq!(result, serde_json::json!("CRITICAL"));
}

#[test]
fn test_if_function_false() {
    let context = serde_json::json!({
        "payload": {
            "metrics": {
                "temperature": 75.0
            }
        }
    });
    let result = evaluate_json(
        "if(payload.metrics.temperature > 80, \"CRITICAL\", \"OK\")",
        &context,
    )
    .unwrap();
    assert_eq!(result, serde_json::json!("OK"));
}

#[test]
fn test_clamp_function() {
    let context = serde_json::json!({
        "val": 150.0
    });
    let result = evaluate_json("clamp(val, 0, 100)", &context).unwrap();
    assert_eq!(result, serde_json::json!(100.0));
}

#[test]
fn test_avg_function() {
    let context = serde_json::json!({
        "a": 10.0,
        "b": 20.0,
        "c": 30.0
    });
    let result = evaluate_json("avg(a, b, c)", &context).unwrap();
    assert_eq!(result, serde_json::json!(20.0));
}

#[test]
fn test_complex_combined_expression() {
    let context = serde_json::json!({
        "payload": {
            "metrics": {
                "temperature": 85.0,
                "pressure": 90.0
            }
        }
    });
    let expr = "payload.metrics.temperature > 80 && payload.metrics.pressure < 100";
    let result = evaluate_json(expr, &context).unwrap();
    assert_eq!(result, serde_json::json!(true));
}

#[test]
fn test_string_literal_comparison() {
    let context = serde_json::json!({
        "status": "CRITICAL"
    });
    let result = evaluate_json("status == \"CRITICAL\"", &context).unwrap();
    assert_eq!(result, serde_json::json!(true));
}

#[test]
fn test_gte_lte_operators() {
    let context = serde_json::json!({
        "temp": 80.0
    });
    assert_eq!(
        evaluate_json("temp >= 80", &context).unwrap(),
        serde_json::json!(true)
    );
    assert_eq!(
        evaluate_json("temp <= 80", &context).unwrap(),
        serde_json::json!(true)
    );
    assert_eq!(
        evaluate_json("temp >= 81", &context).unwrap(),
        serde_json::json!(false)
    );
    assert_eq!(
        evaluate_json("temp <= 79", &context).unwrap(),
        serde_json::json!(false)
    );
}

#[test]
fn test_null_comparison() {
    let context = serde_json::json!({
        "a": null,
        "b": null,
        "c": 1.0
    });
    assert_eq!(
        evaluate_json("a == b", &context).unwrap(),
        serde_json::json!(true)
    );
    assert_eq!(
        evaluate_json("a == c", &context).unwrap(),
        serde_json::json!(false)
    );
    assert_eq!(
        evaluate_json("a > c", &context).unwrap(),
        serde_json::json!(false)
    );
}

#[test]
fn test_old_evaluate_still_works_with_functions() {
    let mut map = HashMap::new();
    map.insert("x".into(), 25.0);
    let v = evaluate("sqrt({x})", &map).unwrap();
    assert!((v - 5.0).abs() < 1e-10);
}

#[test]
fn test_round_function() {
    let context = serde_json::json!({
        "val": 3.7
    });
    let result = evaluate_json("round(val)", &context).unwrap();
    assert_eq!(result, serde_json::json!(4.0));
}

#[test]
fn test_old_sqrt_abs_in_json() {
    let context = serde_json::json!({
        "val": -9.0
    });
    let result = evaluate_json("sqrt(abs(val))", &context).unwrap();
    assert_eq!(result, serde_json::json!(3.0));
}
