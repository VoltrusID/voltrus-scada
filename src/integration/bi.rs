//! BI connector helpers — provides data in formats optimized for external BI tools.

use crate::storage::Storage;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct BiQuery {
    pub devices: Option<String>,
    pub metrics: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub aggregation: Option<String>,
    pub interval: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BiRow {
    pub timestamp: i64,
    pub timestamp_iso: String,
    pub device_id: String,
    pub metric: String,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BiResponse {
    pub total_count: u32,
    pub limit: u32,
    pub offset: u32,
    pub has_more: bool,
    pub data: Vec<BiRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GrafanaResponse {
    pub target: String,
    pub datapoints: Vec<[f64; 2]>,
}

pub fn parse_time(s: &str) -> Option<i64> {
    // Unix timestamp
    if let Ok(ts) = s.parse::<i64>() {
        return Some(ts);
    }
    // ISO 8601
    if let Ok(dt) = time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339) {
        return Some(dt.unix_timestamp());
    }
    // Relative time: "now", "now-1h", "now-30m", "now-7d"
    let s = s.trim();
    if s == "now" {
        return Some(crate::timeutil::now_secs());
    }
    if let Some(rest) = s.strip_prefix("now-") {
        let now = crate::timeutil::now_secs();
        if let Some(num) = rest.strip_suffix('h') {
            if let Ok(n) = num.parse::<i64>() {
                return Some(now - n * 3600);
            }
        } else if let Some(num) = rest.strip_suffix('m') {
            if let Ok(n) = num.parse::<i64>() {
                return Some(now - n * 60);
            }
        } else if let Some(num) = rest.strip_suffix('d') {
            if let Ok(n) = num.parse::<i64>() {
                return Some(now - n * 86400);
            }
        } else if let Ok(n) = rest.parse::<i64>() {
            // Bare number = seconds
            return Some(now - n);
        }
    }
    None
}

/// Most samples one BI query reads. Rows are held in memory to be paged, so
/// the range, not the page size, bounds the cost.
const MAX_QUERY_SAMPLES: usize = 1_000_000;

pub async fn execute_bi_query(storage: &Storage, query: &BiQuery) -> Result<BiResponse> {
    let now = crate::timeutil::now_secs();
    let from = query
        .from
        .as_deref()
        .and_then(parse_time)
        .unwrap_or(now - 3600);
    let to = query.to.as_deref().and_then(parse_time).unwrap_or(now);
    let limit = query.limit.unwrap_or(1000).min(10000);
    let offset = query.offset.unwrap_or(0);

    let all_configs = storage.get_all_device_configs().await?;
    let device_ids: Vec<String> = if let Some(ref devices) = query.devices {
        let filter: std::collections::HashSet<String> =
            devices.split(',').map(|s| s.trim().to_string()).collect();
        all_configs
            .iter()
            .filter(|c| filter.contains(&c.id))
            .map(|c| c.id.clone())
            .collect()
    } else {
        all_configs.iter().map(|c| c.id.clone()).collect()
    };

    let metric_filter: Option<std::collections::HashSet<String>> = query
        .metrics
        .as_ref()
        .map(|m| m.split(',').map(|s| s.trim().to_string()).collect());

    let mut all_rows = Vec::new();
    for device_id in &device_ids {
        // Every sample of the range, or an error: a BI query that silently
        // lacks its newest data is worse than one that asks for a shorter range.
        let data = storage
            .query_device_samples(device_id, from, to, MAX_QUERY_SAMPLES)
            .await?;
        for (ts, metric, value) in data {
            if let Some(ref filter) = metric_filter
                && !filter.contains(&metric)
            {
                continue;
            }
            all_rows.push(BiRow {
                timestamp: ts,
                timestamp_iso: format_timestamp_iso(ts),
                device_id: device_id.clone(),
                metric,
                value,
            });
        }
    }

    all_rows.sort_by_key(|r| r.timestamp);
    let total_count = all_rows.len() as u32;
    let has_more = total_count > offset + limit;
    let data: Vec<BiRow> = all_rows
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();

    Ok(BiResponse {
        total_count,
        limit,
        offset,
        has_more,
        data,
    })
}

pub async fn execute_grafana_query(
    storage: &Storage,
    query: &BiQuery,
) -> Result<Vec<GrafanaResponse>> {
    let bi_result = execute_bi_query(storage, query).await?;
    let mut targets: std::collections::HashMap<String, Vec<[f64; 2]>> =
        std::collections::HashMap::new();
    for row in &bi_result.data {
        let target = format!("{}:{}", row.device_id, row.metric);
        targets
            .entry(target)
            .or_default()
            .push([row.value, (row.timestamp * 1000) as f64]);
    }
    Ok(targets
        .into_iter()
        .map(|(target, datapoints)| GrafanaResponse { target, datapoints })
        .collect())
}

pub fn render_bi_csv(data: &[BiRow]) -> String {
    let mut csv = String::with_capacity(data.len() * 64);
    csv.push_str("timestamp,timestamp_iso,device_id,metric,value\n");
    for row in data {
        csv.push_str(&format!(
            "{},{},{},{},{:.6}\n",
            row.timestamp, row.timestamp_iso, row.device_id, row.metric, row.value
        ));
    }
    csv
}

fn format_timestamp_iso(ts: i64) -> String {
    match time::OffsetDateTime::from_unix_timestamp(ts) {
        Ok(dt) => dt
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| format!("{}", ts)),
        Err(_) => format!("{}", ts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_time_unix() {
        assert_eq!(parse_time("1710000000"), Some(1710000000));
    }

    #[test]
    fn test_parse_time_iso() {
        assert!(parse_time("2024-03-09T20:00:00Z").is_some());
    }

    #[test]
    fn test_parse_time_now() {
        assert!(parse_time("now").is_some());
        let now = crate::timeutil::now_secs();
        assert_eq!(parse_time("now"), Some(now));
    }

    #[test]
    fn test_parse_time_relative() {
        let now = crate::timeutil::now_secs();
        assert_eq!(parse_time("now-1h"), Some(now - 3600));
        assert_eq!(parse_time("now-30m"), Some(now - 1800));
        assert_eq!(parse_time("now-7d"), Some(now - 7 * 86400));
        assert_eq!(parse_time("now-3600"), Some(now - 3600));
    }

    #[test]
    fn test_parse_time_invalid() {
        assert!(parse_time("not-a-time").is_none());
    }
    #[test]
    fn test_render_bi_csv() {
        let rows = vec![BiRow {
            timestamp: 1710000000,
            timestamp_iso: "2024-03-09T19:20:00Z".into(),
            device_id: "plc1".into(),
            metric: "voltage".into(),
            value: 220.5,
        }];
        let csv = render_bi_csv(&rows);
        assert!(csv.contains("220.500000"));
    }
}
