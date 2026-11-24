//! Runtime state of the MCP guardrails: per-credential rate limits and the
//! write calls waiting for confirmation. Lives in `AppState` (compiled with or
//! without the `mcp` feature) so every server and every test app has its own.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::models::{ConfirmationMode, McpConfig};

/// Pending confirmations kept at most; past it the oldest go. A client that
/// proposes without ever confirming cannot grow memory.
const MAX_PENDING: usize = 1_000;

/// A write call proposed and waiting for its confirmation token.
#[derive(Debug, Clone)]
pub struct PendingCall {
    /// The credential that proposed it; only it may confirm.
    pub principal: String,
    pub tool: String,
    pub arguments: Value,
    expires_at: Instant,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn full(per_minute: u32, now: Instant) -> Self {
        Self {
            tokens: f64::from(per_minute),
            last: now,
        }
    }

    fn refill(&mut self, per_minute: u32, now: Instant) {
        let rate = f64::from(per_minute) / 60.0;
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * rate).min(f64::from(per_minute));
        self.last = now;
    }

    /// Seconds until one token is available.
    fn wait_secs(&self, per_minute: u32) -> u64 {
        let rate = f64::from(per_minute) / 60.0;
        ((1.0 - self.tokens) / rate).ceil().max(1.0) as u64
    }
}

#[derive(Debug)]
struct Buckets {
    calls: Bucket,
    writes: Bucket,
}

/// Why a confirmation token was not accepted.
#[derive(Debug, PartialEq, Eq)]
pub enum ConfirmError {
    Unknown,
    Expired,
    OtherPrincipal,
}

impl std::fmt::Display for ConfirmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ConfirmError::Unknown => {
                "unknown or already used confirmation token; call the tool again for a new proposal"
            }
            ConfirmError::Expired => {
                "confirmation token expired; call the tool again for a new proposal"
            }
            ConfirmError::OtherPrincipal => {
                "this confirmation token was issued to another credential"
            }
        })
    }
}

pub struct McpGuard {
    config: McpConfig,
    buckets: Mutex<HashMap<String, Buckets>>,
    pending: Mutex<HashMap<String, PendingCall>>,
}

impl Default for McpGuard {
    fn default() -> Self {
        Self::new(McpConfig::default())
    }
}

impl McpGuard {
    pub fn new(config: McpConfig) -> Self {
        Self {
            config,
            buckets: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn config(&self) -> &McpConfig {
        &self.config
    }

    /// Whether calling `tool` (by name, destructive flag, read-only access)
    /// first returns a proposal.
    pub fn needs_confirmation(&self, name: &str, destructive: bool, read_only: bool) -> bool {
        if read_only {
            return false;
        }
        match self.config.require_confirmation {
            ConfirmationMode::Off => false,
            ConfirmationMode::All => true,
            ConfirmationMode::Destructive => destructive || name == "write_tag",
        }
    }

    /// Take one call (and one write when `write`) from the credential's
    /// buckets, or say how many seconds to wait. Nothing is taken when the call
    /// is refused, so a client that backs off is not punished twice.
    pub fn check_rate(&self, principal: &str, write: bool) -> Result<(), u64> {
        let calls_limit = self.config.calls_per_minute;
        let writes_limit = self.config.writes_per_minute;
        let now = Instant::now();
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let b = map.entry(principal.to_string()).or_insert_with(|| Buckets {
            calls: Bucket::full(calls_limit, now),
            writes: Bucket::full(writes_limit, now),
        });
        b.calls.refill(calls_limit, now);
        b.writes.refill(writes_limit, now);
        if calls_limit > 0 && b.calls.tokens < 1.0 {
            return Err(b.calls.wait_secs(calls_limit));
        }
        if write && writes_limit > 0 && b.writes.tokens < 1.0 {
            return Err(b.writes.wait_secs(writes_limit));
        }
        if calls_limit > 0 {
            b.calls.tokens -= 1.0;
        }
        if write && writes_limit > 0 {
            b.writes.tokens -= 1.0;
        }
        Ok(())
    }

    /// Record a proposed call and return its confirmation token.
    pub fn propose(&self, principal: &str, tool: &str, arguments: Value) -> String {
        use rand::Rng as _;
        let token: String = rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();
        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|_, p| p.expires_at > now);
        if pending.len() >= MAX_PENDING
            && let Some(oldest) = pending
                .iter()
                .min_by_key(|(_, p)| p.expires_at)
                .map(|(k, _)| k.clone())
        {
            pending.remove(&oldest);
        }
        pending.insert(
            token.clone(),
            PendingCall {
                principal: principal.to_string(),
                tool: tool.to_string(),
                arguments,
                expires_at: now + Duration::from_secs(self.config.confirmation_ttl_secs),
            },
        );
        token
    }

    /// Redeem a token: single use, only by the credential it was issued to.
    pub fn take(&self, token: &str, principal: &str) -> Result<PendingCall, ConfirmError> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let Some(call) = pending.get(token) else {
            return Err(ConfirmError::Unknown);
        };
        if call.principal != principal {
            // Left in place: a guessed or leaked token must not let another
            // credential cancel the owner's proposal either.
            return Err(ConfirmError::OtherPrincipal);
        }
        let call = pending.remove(token).expect("checked above");
        if call.expires_at <= Instant::now() {
            return Err(ConfirmError::Expired);
        }
        Ok(call)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn guard(calls: u32, writes: u32) -> McpGuard {
        McpGuard::new(McpConfig {
            calls_per_minute: calls,
            writes_per_minute: writes,
            ..McpConfig::default()
        })
    }

    #[test]
    fn writes_have_their_own_smaller_bucket() {
        let g = guard(5, 2);
        assert!(g.check_rate("k", true).is_ok());
        assert!(g.check_rate("k", true).is_ok());
        let wait = g.check_rate("k", true).unwrap_err();
        assert!((1..=30).contains(&wait), "{wait}");
        // Reads still pass, until the overall bucket is empty too.
        assert!(g.check_rate("k", false).is_ok());
        assert!(g.check_rate("k", false).is_ok());
        assert!(g.check_rate("k", false).is_ok());
        assert!(g.check_rate("k", false).is_err());
        // Per credential.
        assert!(g.check_rate("other", true).is_ok());
    }

    #[test]
    fn zero_means_unlimited() {
        let g = guard(0, 0);
        for _ in 0..1000 {
            assert!(g.check_rate("k", true).is_ok());
        }
    }

    #[test]
    fn tokens_are_single_use_and_bound_to_the_credential() {
        let g = McpGuard::default();
        let t = g.propose("alice", "write_tag", json!({"tag_id": 1, "value": 2}));
        assert_eq!(g.take(&t, "bob").unwrap_err(), ConfirmError::OtherPrincipal);
        let call = g.take(&t, "alice").unwrap();
        assert_eq!(call.tool, "write_tag");
        assert_eq!(g.take(&t, "alice").unwrap_err(), ConfirmError::Unknown);
    }

    #[test]
    fn expired_tokens_are_refused() {
        let g = McpGuard::new(McpConfig {
            confirmation_ttl_secs: 0,
            ..McpConfig::default()
        });
        let t = g.propose("alice", "delete_tag", json!({"id": 1}));
        assert_eq!(g.take(&t, "alice").unwrap_err(), ConfirmError::Expired);
    }

    #[test]
    fn confirmation_modes() {
        let d = McpGuard::default();
        assert!(d.needs_confirmation("write_tag", false, false));
        assert!(d.needs_confirmation("delete_tag", true, false));
        assert!(!d.needs_confirmation("create_tag", false, false));
        assert!(!d.needs_confirmation("list_tags", false, true));
        let all = McpGuard::new(McpConfig {
            require_confirmation: ConfirmationMode::All,
            ..McpConfig::default()
        });
        assert!(all.needs_confirmation("create_tag", false, false));
        let off = McpGuard::new(McpConfig {
            require_confirmation: ConfirmationMode::Off,
            ..McpConfig::default()
        });
        assert!(!off.needs_confirmation("write_tag", false, false));
    }
}
