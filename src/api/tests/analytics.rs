#[test]
fn test_analytics_change_percent() {
    let current_avg = 110.0_f64;
    let previous_avg = 100.0_f64;
    let change = ((current_avg - previous_avg) / previous_avg) * 100.0;
    assert!((change - 10.0).abs() < 0.001);
}

#[test]
fn test_analytics_change_percent_zero_division() {
    let current_avg = 5.0_f64;
    let previous_avg = 0.0_f64;
    let change = if previous_avg.abs() > f64::EPSILON {
        ((current_avg - previous_avg) / previous_avg) * 100.0
    } else {
        0.0
    };
    assert_eq!(change, 0.0);
}

#[test]
fn test_analytics_change_percent_negative() {
    let current_avg = 80.0_f64;
    let previous_avg = 100.0_f64;
    let change = ((current_avg - previous_avg) / previous_avg) * 100.0;
    assert!((change - (-20.0)).abs() < 0.001);
}

#[test]
fn test_recipe_values_json_parse() {
    let json = r#"{"temp": 25.0, "pressure": 1.5}"#;
    let values: std::collections::HashMap<String, f64> = serde_json::from_str(json).unwrap();
    assert_eq!(values.len(), 2);
    assert!((values["temp"] - 25.0).abs() < 0.001);
    assert!((values["pressure"] - 1.5).abs() < 0.001);
}
