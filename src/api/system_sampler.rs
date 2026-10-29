//! One shared, briefly cached sample of the host and of this process, behind
//! `GET /api/v1/system` and `GET /api/v1/system/memory`.
//!
//! The old handler built `System::new_all()` and called `refresh_all()` on every
//! request, which reads `/proc/<pid>/*` for every process on the machine (about
//! 40 ms and ~300 file descriptors on a busy host). The sampler refreshes only
//! what the endpoints return (CPU, memory, this process, the disk list) on a
//! long-lived [`System`], so CPU percentages are real deltas between samples,
//! and serves the same snapshot to every caller for [`SAMPLE_TTL`]. Concurrent
//! callers of a stale snapshot wait on one refresh instead of each running
//! their own.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sysinfo::{
    DiskRefreshKind, Disks, MINIMUM_CPU_UPDATE_INTERVAL, Pid, ProcessRefreshKind,
    ProcessesToUpdate, System,
};

/// How long one sample is reused. Short enough that a dashboard polling every
/// few seconds sees fresh numbers, long enough that bursts cost one refresh.
pub const SAMPLE_TTL: Duration = Duration::from_secs(2);

/// Disk the endpoint reports: the root mount when there is one, else the first.
#[derive(Debug, Clone, Copy)]
pub struct DiskSample {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// This process.
#[derive(Debug, Clone)]
pub struct ProcessSample {
    pub pid: u32,
    /// Resident set size in bytes. `None` when the OS did not report the process.
    pub rss_bytes: Option<u64>,
    /// Percent of one core, like `top`.
    pub cpu_percent: f32,
    /// Process start, seconds since the epoch.
    pub start_time_secs: Option<u64>,
    pub binary_size_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct HostSample {
    pub hostname: String,
    pub os: String,
    pub uptime_secs: u64,
    pub cpu_usage: f32,
    pub memory_total_bytes: u64,
    /// Whole-system used memory, not this process.
    pub memory_used_bytes: u64,
    pub disk: Option<DiskSample>,
    pub process: ProcessSample,
}

/// The sysinfo handles that survive between refreshes.
struct Probe {
    sys: System,
    disks: Disks,
    pid: Pid,
    /// CPU usage is a delta between two refreshes; the first refresh only
    /// establishes the baseline.
    primed: bool,
}

impl Probe {
    fn new() -> Self {
        Self {
            sys: System::new(),
            disks: Disks::new(),
            pid: Pid::from(std::process::id() as usize),
            primed: false,
        }
    }

    fn refresh_cpu_counters(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
    }

    fn sample(&mut self) -> HostSample {
        if !self.primed {
            self.refresh_cpu_counters();
            std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
            self.primed = true;
        }
        self.refresh_cpu_counters();
        self.sys.refresh_memory();
        self.disks
            .refresh_specifics(true, DiskRefreshKind::nothing().with_storage());

        let disk = self
            .disks
            .list()
            .iter()
            .find(|d| d.mount_point().to_str() == Some("/"))
            .or_else(|| self.disks.list().first())
            .map(|d| DiskSample {
                total_bytes: d.total_space(),
                available_bytes: d.available_space(),
            });

        let proc = self.sys.process(self.pid);
        let binary_size_bytes = std::env::current_exe()
            .and_then(std::fs::metadata)
            .map(|m| m.len())
            .unwrap_or(0);

        HostSample {
            hostname: System::host_name().unwrap_or_else(|| "unknown".to_string()),
            os: System::name().unwrap_or_else(|| "unknown".to_string()),
            uptime_secs: System::uptime(),
            cpu_usage: self.sys.global_cpu_usage(),
            memory_total_bytes: self.sys.total_memory(),
            memory_used_bytes: self.sys.used_memory(),
            disk,
            process: ProcessSample {
                pid: std::process::id(),
                rss_bytes: proc.map(|p| p.memory()),
                cpu_percent: proc.map_or(0.0, |p| p.cpu_usage()),
                start_time_secs: proc.map(|p| p.start_time()),
                binary_size_bytes,
            },
        }
    }
}

/// Single-flight, time-limited cache of one [`HostSample`].
pub struct SystemSampler {
    ttl: Duration,
    probe: Arc<Mutex<Probe>>,
    /// Held across a refresh, so callers that arrive while one runs wait for
    /// its result instead of starting another.
    cached: tokio::sync::Mutex<Option<(Instant, Arc<HostSample>)>>,
    refreshes: AtomicU64,
}

impl SystemSampler {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            probe: Arc::new(Mutex::new(Probe::new())),
            cached: tokio::sync::Mutex::new(None),
            refreshes: AtomicU64::new(0),
        }
    }

    /// The process-wide sampler the API handlers share.
    pub fn global() -> &'static SystemSampler {
        static GLOBAL: LazyLock<SystemSampler> = LazyLock::new(|| SystemSampler::new(SAMPLE_TTL));
        &GLOBAL
    }

    /// Number of times the OS was actually queried.
    pub fn refresh_count(&self) -> u64 {
        self.refreshes.load(Ordering::Relaxed)
    }

    /// The current sample, refreshed when the cached one is older than the TTL.
    pub async fn sample(&self) -> Result<Arc<HostSample>> {
        let mut cached = self.cached.lock().await;
        if let Some((taken, sample)) = cached.as_ref()
            && taken.elapsed() < self.ttl
        {
            return Ok(sample.clone());
        }

        let probe = self.probe.clone();
        let sample = tokio::task::spawn_blocking(move || {
            probe.lock().unwrap_or_else(|e| e.into_inner()).sample()
        })
        .await
        .context("System sampling task failed")?;
        self.refreshes.fetch_add(1, Ordering::Relaxed);

        let sample = Arc::new(sample);
        *cached = Some((Instant::now(), sample.clone()));
        Ok(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `VmRSS` of this process from the kernel, in bytes.
    #[cfg(target_os = "linux")]
    fn kernel_rss_bytes() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let kb: u64 = status
            .lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .expect("VmRSS line");
        kb * 1024
    }

    #[tokio::test]
    async fn sample_reports_this_process_not_the_machine() {
        let sampler = SystemSampler::new(Duration::from_secs(60));
        let host = sampler.sample().await.unwrap();

        let rss = host.process.rss_bytes.expect("process is reported");
        assert!(rss > 1 << 20, "a test binary holds more than 1 MiB: {rss}");
        assert!(
            rss < host.memory_total_bytes,
            "process RSS {rss} must be below physical memory {}",
            host.memory_total_bytes
        );
        assert!(
            rss < host.memory_used_bytes,
            "process RSS {rss} is part of, not all of, used memory {}",
            host.memory_used_bytes
        );
        assert_eq!(host.process.pid, std::process::id());

        #[cfg(target_os = "linux")]
        {
            let kernel = kernel_rss_bytes();
            let tolerance = (kernel / 4).max(16 << 20);
            assert!(
                rss.abs_diff(kernel) <= tolerance,
                "sampled RSS {rss} vs kernel VmRSS {kernel}"
            );
        }

        // The disk list is refreshed on the long-lived handle, not only at creation.
        #[cfg(target_os = "linux")]
        if std::path::Path::new("/proc/self/mountinfo").exists() {
            let disk = host.disk.expect("a mounted disk is reported");
            assert!(disk.total_bytes > 0 && disk.available_bytes <= disk.total_bytes);
        }

        let start = host.process.start_time_secs.expect("start time");
        let now = crate::timeutil::now_secs() as u64;
        assert!(start <= now && now - start < 24 * 3600, "start {start}");
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_refresh() {
        let sampler = Arc::new(SystemSampler::new(Duration::from_secs(60)));
        let calls: Vec<_> = (0..32)
            .map(|_| {
                let sampler = sampler.clone();
                tokio::spawn(async move { sampler.sample().await.unwrap() })
            })
            .collect();
        let mut samples = Vec::new();
        for call in calls {
            samples.push(call.await.unwrap());
        }

        assert_eq!(
            sampler.refresh_count(),
            1,
            "one /proc refresh for 32 callers"
        );
        assert!(
            samples.iter().all(|s| Arc::ptr_eq(s, &samples[0])),
            "every caller got the same snapshot"
        );
    }

    #[tokio::test]
    async fn a_stale_sample_is_refreshed_once_for_the_next_burst() {
        let sampler = Arc::new(SystemSampler::new(Duration::from_millis(50)));
        sampler.sample().await.unwrap();
        sampler.sample().await.unwrap();
        assert_eq!(sampler.refresh_count(), 1, "second call is inside the TTL");

        tokio::time::sleep(Duration::from_millis(80)).await;
        let calls: Vec<_> = (0..8)
            .map(|_| {
                let sampler = sampler.clone();
                tokio::spawn(async move { sampler.sample().await.unwrap() })
            })
            .collect();
        for call in calls {
            call.await.unwrap();
        }
        assert_eq!(
            sampler.refresh_count(),
            2,
            "the burst after expiry shares one refresh"
        );
    }
}
