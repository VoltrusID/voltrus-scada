use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct AuditEvent {
    pub action: String,
    pub username: String,
    pub resource: String,
    pub details: Option<String>,
}

/// Entries waiting for the database writer. Handlers never wait on the audit
/// log, so a stalled database must not grow memory without bound: past this
/// many queued entries new ones are dropped, counted and reported.
pub const AUDIT_QUEUE_CAPACITY: usize = 10_000;

#[derive(Clone)]
pub struct AuditLogger {
    sender: mpsc::Sender<AuditEvent>,
    dropped: Arc<AtomicU64>,
}

impl AuditLogger {
    pub fn new() -> (Self, mpsc::Receiver<AuditEvent>) {
        Self::with_capacity(AUDIT_QUEUE_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> (Self, mpsc::Receiver<AuditEvent>) {
        let (tx, rx) = mpsc::channel(capacity);
        (
            Self {
                sender: tx,
                dropped: Arc::new(AtomicU64::new(0)),
            },
            rx,
        )
    }

    pub fn log(&self, event: AuditEvent) {
        match self.sender.try_send(event) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(event)) => {
                let n = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                // The first drop and then every thousandth: enough to notice
                // without flooding the log the queue is already behind on.
                if n == 1 || n.is_multiple_of(1000) {
                    tracing::error!(
                        dropped_total = n,
                        action = %event.action,
                        "audit queue full ({AUDIT_QUEUE_CAPACITY} entries): audit entry dropped"
                    );
                }
            }
            // No writer (unit tests that do not read the audit log).
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    /// Entries dropped because the queue was full, since start.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Write queued entries to the database until every logger is dropped.
    pub fn spawn_writer(
        storage: crate::storage::Storage,
        mut rx: mpsc::Receiver<AuditEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(e) = storage
                    .log_audit_event(
                        &event.action,
                        &event.username,
                        &event.resource,
                        event.details.as_deref(),
                    )
                    .await
                {
                    tracing::warn!("Failed to write audit log: {}", e);
                }
            }
        })
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new().0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(n: usize) -> AuditEvent {
        AuditEvent {
            action: format!("a{n}"),
            username: "u".into(),
            resource: "r".into(),
            details: None,
        }
    }

    #[test]
    fn a_full_queue_drops_and_counts_instead_of_growing() {
        let (logger, mut rx) = AuditLogger::with_capacity(3);
        for n in 0..5 {
            logger.log(event(n));
        }
        assert_eq!(logger.dropped(), 2);
        let kept: Vec<String> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|e| e.action)
            .collect();
        assert_eq!(kept, ["a0", "a1", "a2"], "the oldest entries are kept");
    }
}
