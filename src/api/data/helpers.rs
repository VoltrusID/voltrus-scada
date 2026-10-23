/// Parse a time parameter that can be either a unix timestamp (seconds) or an ISO 8601 string.
/// Returns unix timestamp in seconds.
pub(crate) fn parse_time_param(value: &str) -> Result<i64, String> {
    // Try parsing as integer unix timestamp first
    if let Ok(ts) = value.parse::<i64>() {
        return Ok(ts);
    }
    // Try ISO 8601 via the `time` crate
    if let Ok(dt) =
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
    {
        return Ok(dt.unix_timestamp());
    }
    // Try a more lenient ISO format (without timezone)
    if let Ok(dt) = time::PrimitiveDateTime::parse(
        value,
        &time::format_description::well_known::Iso8601::DEFAULT,
    ) {
        // Assume UTC if no timezone specified
        let utc = dt.assume_utc();
        return Ok(utc.unix_timestamp());
    }
    Err(format!(
        "Invalid time format '{}'. Use unix timestamp (seconds) or ISO 8601 string.",
        value
    ))
}

/// Validate that the aggregation parameter is one of the allowed values.
pub(crate) fn validate_aggregation(agg: &str) -> bool {
    matches!(agg, "avg" | "min" | "max" | "count" | "sum")
}

/// Validate that the interval parameter is one of the allowed values.
pub(crate) fn validate_interval(interval: &str) -> bool {
    matches!(interval, "1m" | "5m" | "15m" | "1h" | "1d")
}

pub(crate) const DEFAULT_EXPORT_LIMIT: usize = 1000;
pub(crate) const MAX_EXPORT_LIMIT: usize = 10000;
