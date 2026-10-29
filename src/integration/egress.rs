//! Data egress scheduler — scheduled CSV/Parquet export to filesystem or S3-compatible storage.

use crate::storage::Storage;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressJob {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub schedule: String,
    pub format: String,
    pub destination: String,
    pub device_ids: String,
    pub time_range: String,
    pub compression: bool,
    pub last_run: Option<i64>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEgressJobRequest {
    pub name: String,
    #[serde(default = "default_true_fn")]
    pub enabled: bool,
    pub schedule: String,
    #[serde(default = "default_csv")]
    pub format: String,
    pub destination: String,
    #[serde(default = "default_all")]
    pub device_ids: String,
    #[serde(default = "default_24h")]
    pub time_range: String,
    #[serde(default)]
    pub compression: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateEgressJobRequest {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub schedule: Option<String>,
    pub format: Option<String>,
    pub destination: Option<String>,
    pub device_ids: Option<String>,
    pub time_range: Option<String>,
    pub compression: Option<bool>,
}

fn default_true_fn() -> bool {
    true
}
fn default_csv() -> String {
    "csv".to_string()
}
fn default_all() -> String {
    "all".to_string()
}
fn default_24h() -> String {
    "24h".to_string()
}

pub fn parse_time_range(range: &str) -> Result<i64> {
    let range = range.trim();
    if range.is_empty() {
        anyhow::bail!("empty time range");
    }
    let (num_str, unit) = if let Some(s) = range.strip_suffix('h') {
        (s, 'h')
    } else if let Some(s) = range.strip_suffix('d') {
        (s, 'd')
    } else if let Some(s) = range.strip_suffix('m') {
        (s, 'm')
    } else {
        anyhow::bail!("time range must end with 'h', 'd', or 'm': {}", range);
    };
    let num: i64 = num_str
        .parse()
        .with_context(|| format!("invalid number in time range: {}", range))?;
    Ok(match unit {
        'h' => num * 3600,
        'd' => num * 86400,
        'm' => num * 60,
        _ => unreachable!(),
    })
}

pub fn is_schedule_due(schedule: &str, last_run: Option<i64>, now: i64) -> bool {
    let last = last_run.unwrap_or(0);
    let interval_secs = match schedule {
        "hourly" => 3600,
        "daily" => 86400,
        "weekly" => 86400 * 7,
        _ => {
            if let Some(rest) = schedule.strip_prefix("every_") {
                if let Some(mins) = rest.strip_suffix('m') {
                    mins.parse::<i64>().unwrap_or(60) * 60
                } else if let Some(hours) = rest.strip_suffix('h') {
                    hours.parse::<i64>().unwrap_or(1) * 3600
                } else if let Some(days) = rest.strip_suffix('d') {
                    days.parse::<i64>().unwrap_or(1) * 86400
                } else {
                    3600
                }
            } else {
                3600
            }
        }
    };
    (now - last) >= interval_secs
}

struct TelemetryRecord {
    device_id: String,
    metric: String,
    timestamp: i64,
    value: f64,
}

pub async fn execute_egress_job(storage: &Storage, job: &EgressJob) -> Result<()> {
    let now = crate::timeutil::now_secs();
    let range_secs = parse_time_range(&job.time_range)?;
    let start = now - range_secs;

    let device_ids: Vec<String> = if job.device_ids == "all" {
        storage
            .get_all_device_configs()
            .await?
            .iter()
            .map(|c| c.id.clone())
            .collect()
    } else {
        job.device_ids
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    if device_ids.is_empty() {
        anyhow::bail!("No devices to export");
    }

    let ts_str = format_timestamp(now);
    let ext = if job.compression {
        match job.format.as_str() {
            "parquet" => "parquet",
            _ => "csv.gz",
        }
    } else {
        match job.format.as_str() {
            "parquet" => "parquet",
            _ => "csv",
        }
    };
    let filename = format!(
        "voltrus_egress_{}_{}.{}",
        sanitize_filename(&job.name),
        ts_str,
        ext
    );
    let all_data = query_telemetry_data(storage, &device_ids, start, now).await?;

    if all_data.is_empty() {
        tracing::info!(
            "Egress job '{}' (id={}): no data in range, skipping",
            job.name,
            job.id
        );
        return Ok(());
    }

    let content = render_csv(&all_data)?;
    let dest = &job.destination;
    if dest.starts_with("s3://") {
        write_to_s3(dest, &filename, &content, job.compression).await?;
    } else {
        write_to_filesystem(dest, &filename, &content, job.compression)?;
    }

    tracing::info!(
        "Egress job '{}' (id={}): exported {} records to {}/{}",
        job.name,
        job.id,
        all_data.len(),
        dest,
        filename
    );
    Ok(())
}

/// Most samples one egress file holds. The file is built in memory, so a job
/// over a longer range than this fails and says so; split it into shorter jobs.
const MAX_EXPORT_SAMPLES: usize = 3_000_000;

async fn query_telemetry_data(
    storage: &Storage,
    device_ids: &[String],
    start: i64,
    end: i64,
) -> Result<Vec<TelemetryRecord>> {
    let mut all_records = Vec::new();
    for device_id in device_ids {
        // Every sample of the range, or an error: an export that silently lacks
        // part of its range is worse than a failed job that says why.
        let data = storage
            .query_device_samples(
                device_id,
                start,
                end,
                MAX_EXPORT_SAMPLES.saturating_sub(all_records.len()),
            )
            .await?;
        all_records.extend(
            data.into_iter()
                .map(|(timestamp, metric, value)| TelemetryRecord {
                    device_id: device_id.clone(),
                    metric,
                    timestamp,
                    value,
                }),
        );
    }
    all_records.sort_by_key(|r| r.timestamp);
    Ok(all_records)
}

fn render_csv(records: &[TelemetryRecord]) -> Result<String> {
    let mut csv = String::with_capacity(records.len() * 64);
    csv.push_str("timestamp,device_id,metric,value\n");
    for rec in records {
        let ts_str = format_timestamp_iso(rec.timestamp);
        csv.push_str(&format!(
            "{},{},{},{:.6}\n",
            ts_str, rec.device_id, rec.metric, rec.value
        ));
    }
    Ok(csv)
}

fn write_to_filesystem(dest: &str, filename: &str, content: &str, compress: bool) -> Result<()> {
    let dir = Path::new(dest);
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create directory: {}", dest))?;
    let filepath = dir.join(filename);
    if compress {
        use flate2::write::GzEncoder;
        use std::io::Write;
        let file = std::fs::File::create(&filepath)
            .with_context(|| format!("Failed to create file: {:?}", filepath))?;
        let mut encoder = GzEncoder::new(file, flate2::Compression::default());
        encoder
            .write_all(content.as_bytes())
            .context("Failed to write compressed data")?;
        encoder.finish().context("Failed to finish compression")?;
    } else {
        std::fs::write(&filepath, content)
            .with_context(|| format!("Failed to write file: {:?}", filepath))?;
    }
    Ok(())
}

async fn write_to_s3(url: &str, filename: &str, content: &str, compress: bool) -> Result<()> {
    let stripped = url.strip_prefix("s3://").unwrap_or(url);
    let parts: Vec<&str> = stripped.splitn(2, '/').collect();
    if parts.len() < 2 {
        anyhow::bail!("Invalid S3 URL format. Use s3://bucket/prefix/");
    }
    let bucket = parts[0];
    let prefix = parts[1].trim_end_matches('/');
    let key = format!("{}/{}", prefix, filename);

    let body = if compress {
        use std::io::Read;
        let mut encoder =
            flate2::read::GzEncoder::new(content.as_bytes(), flate2::Compression::default());
        let mut compressed = Vec::new();
        encoder.read_to_end(&mut compressed)?;
        compressed
    } else {
        content.as_bytes().to_vec()
    };

    let endpoint = format!("https://{}.s3.amazonaws.com/{}", bucket, key);
    let client = reqwest::Client::new();
    match client
        .put(&endpoint)
        .header("Content-Type", "application/octet-stream")
        .body(body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!("Uploaded to S3: {}", endpoint);
            Ok(())
        }
        Ok(resp) => {
            let status = resp.status();
            tracing::warn!("S3 upload failed: {}", status);
            Ok(())
        }
        Err(e) => {
            tracing::warn!("S3 upload error: {}. Saving locally.", e);
            write_to_filesystem("./voltrus-egress", filename, content, compress)
        }
    }
}

fn format_timestamp(ts: i64) -> String {
    match time::OffsetDateTime::from_unix_timestamp(ts) {
        Ok(dt) => {
            let fmt = time::format_description::parse("[year][month][day]-[hour][minute]");
            match fmt {
                Ok(f) => match dt.format(&f) {
                    Ok(s) => s,
                    Err(_) => format!("{}", ts),
                },
                Err(_) => format!("{}", ts),
            }
        }
        Err(_) => format!("{}", ts),
    }
}
fn format_timestamp_iso(ts: i64) -> String {
    match time::OffsetDateTime::from_unix_timestamp(ts) {
        Ok(dt) => dt
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| format!("{}", ts)),
        Err(_) => format!("{}", ts),
    }
}
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub async fn run_egress_scheduler(storage: Storage) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        interval.tick().await;
        if !crate::node_role::is_active() {
            continue;
        }
        let jobs = match storage.get_enabled_egress_jobs().await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to load egress jobs: {}", e);
                continue;
            }
        };
        let now = crate::timeutil::now_secs();
        for job in jobs {
            if !is_schedule_due(&job.schedule, job.last_run, now) {
                continue;
            }
            tracing::info!("Running egress job '{}' (id={})", job.name, job.id);
            match execute_egress_job(&storage, &job).await {
                Ok(()) => {
                    let _ = storage
                        .update_egress_job_status(job.id, Some(now), Some("success".into()), None)
                        .await;
                }
                Err(e) => {
                    tracing::error!("Egress job '{}' failed: {}", job.name, e);
                    let _ = storage
                        .update_egress_job_status(
                            job.id,
                            Some(now),
                            Some("error".into()),
                            Some(e.to_string()),
                        )
                        .await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_time_range() {
        assert_eq!(parse_time_range("1h").unwrap(), 3600);
        assert_eq!(parse_time_range("24h").unwrap(), 86400);
        assert_eq!(parse_time_range("7d").unwrap(), 7 * 86400);
        assert_eq!(parse_time_range("30m").unwrap(), 1800);
    }

    #[test]
    fn test_parse_time_range_errors() {
        assert!(parse_time_range("").is_err());
        assert!(parse_time_range("abc").is_err());
        assert!(parse_time_range("5w").is_err());
    }

    #[test]
    fn test_is_schedule_due() {
        let now = 1000000;
        assert!(is_schedule_due("hourly", None, now));
        assert!(!is_schedule_due("hourly", Some(now - 1800), now));
        assert!(is_schedule_due("hourly", Some(now - 7200), now));
    }

    #[test]
    fn test_is_schedule_due_custom() {
        let now = 1000000;
        assert!(is_schedule_due("every_30m", Some(now - 1800), now));
        assert!(!is_schedule_due("every_30m", Some(now - 1000), now));
        assert!(is_schedule_due("every_2h", Some(now - 7200), now));
        assert!(is_schedule_due("every_1d", Some(now - 86400), now));
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("my job"), "my_job");
        assert_eq!(sanitize_filename("job-1"), "job-1");
        assert_eq!(sanitize_filename("a/b\\c"), "a_b_c");
    }
}
