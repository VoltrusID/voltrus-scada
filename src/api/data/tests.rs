use super::*;

// -- parse_time_param tests --

#[test]
fn test_parse_time_param_unix_timestamp() {
    assert_eq!(helpers::parse_time_param("1700000000").unwrap(), 1700000000);
    assert_eq!(helpers::parse_time_param("0").unwrap(), 0);
    assert_eq!(helpers::parse_time_param("9999999999").unwrap(), 9999999999);
}

#[test]
fn test_parse_time_param_iso8601_rfc3339() {
    // 2024-01-15T10:30:00Z -> unix timestamp
    let result = helpers::parse_time_param("2024-01-15T10:30:00Z");
    assert!(result.is_ok());
    // Verify it's a reasonable unix timestamp for Jan 2024
    let ts = result.unwrap();
    assert!(ts > 1700000000 && ts < 1710000000);
}

#[test]
fn test_parse_time_param_iso8601_with_offset() {
    // ISO 8601 with timezone offset
    let result = helpers::parse_time_param("2024-01-15T10:30:00+05:30");
    assert!(result.is_ok());
}

#[test]
fn test_parse_time_param_iso8601_no_timezone() {
    // Lenient ISO 8601 without timezone (assumed UTC)
    let result = helpers::parse_time_param("2024-01-15T10:30:00");
    assert!(result.is_ok());
    let ts = result.unwrap();
    assert!(ts > 1700000000 && ts < 1710000000);
}

#[test]
fn test_parse_time_param_invalid() {
    assert!(helpers::parse_time_param("").is_err());
    assert!(helpers::parse_time_param("not-a-date").is_err());
    assert!(helpers::parse_time_param("2024-13-01").is_err()); // invalid month
    assert!(helpers::parse_time_param("abc123").is_err());
}

#[test]
fn test_parse_time_param_negative_accepted_as_unix_ts() {
    // Negative numbers parse as valid (though unusual) unix timestamps
    assert_eq!(helpers::parse_time_param("-1").unwrap(), -1);
}

// -- validate_aggregation tests --

#[test]
fn test_validate_aggregation_valid() {
    assert!(helpers::validate_aggregation("avg"));
    assert!(helpers::validate_aggregation("min"));
    assert!(helpers::validate_aggregation("max"));
    assert!(helpers::validate_aggregation("count"));
    assert!(helpers::validate_aggregation("sum"));
}

#[test]
fn test_validate_aggregation_invalid() {
    assert!(!helpers::validate_aggregation(""));
    assert!(!helpers::validate_aggregation("AVG")); // case-sensitive
    assert!(!helpers::validate_aggregation("Avg"));
    assert!(!helpers::validate_aggregation("mean"));
    assert!(!helpers::validate_aggregation("median"));
    assert!(!helpers::validate_aggregation("total"));
    assert!(!helpers::validate_aggregation(" stddev"));
}

// -- validate_interval tests --

#[test]
fn test_validate_interval_valid() {
    assert!(helpers::validate_interval("1m"));
    assert!(helpers::validate_interval("5m"));
    assert!(helpers::validate_interval("15m"));
    assert!(helpers::validate_interval("1h"));
    assert!(helpers::validate_interval("1d"));
}

#[test]
fn test_validate_interval_invalid() {
    assert!(!helpers::validate_interval(""));
    assert!(!helpers::validate_interval("10m")); // not in the allowed set
    assert!(!helpers::validate_interval("30m")); // not in the allowed set
    assert!(!helpers::validate_interval("2h")); // not in the allowed set
    assert!(!helpers::validate_interval("7d")); // not in the allowed set
    assert!(!helpers::validate_interval("1s")); // seconds not supported
    assert!(!helpers::validate_interval("1w")); // weeks not supported
    assert!(!helpers::validate_interval("minute")); // not abbreviated
}
