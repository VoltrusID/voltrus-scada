//! Which node of a redundant pair is acting.
//!
//! A single node is always active: it polls devices, writes to them, persists
//! history, evaluates alarms and runs the scheduled jobs. An add-on edition that
//! runs two nodes as a pair (see [`crate::edition::Extension::resolve_node_role`])
//! flips this switch so that only one node of the pair does those things at a
//! time. The core only reads it:
//!
//! * background jobs (history persistence, alarm evaluation, retention, egress,
//!   OEE, the command queue, flows) skip their work while the node is not active;
//! * every low-level device write refuses while the node is not active, whatever
//!   path asked for it (REST, WebSocket, command queue, flow, MCP);
//! * device pollers run only while [`polls_devices`] is true. An add-on may keep
//!   them running on the passive node for live read-only values;
//! * the HTTP API of a passive node is read-only: see [`request_allowed`].
//!
//! The state is process-wide, like the add-on runtime that drives it.

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::http::Method;

static ACTIVE: AtomicBool = AtomicBool::new(true);
static POLLING: AtomicBool = AtomicBool::new(true);
static EXEMPT_PREFIXES: RwLock<Vec<&'static str>> = RwLock::new(Vec::new());

/// Why a passive node refuses a device write.
pub const NOT_ACTIVE_MESSAGE: &str =
    "This node is the standby of a redundant pair and does not write to devices";

/// What a process starts as, decided before any device poller starts. The
/// default is a lone active node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Start {
    pub active: bool,
    /// Keep device pollers running while passive.
    pub passive_polling: bool,
}

impl Default for Start {
    fn default() -> Self {
        Self {
            active: true,
            passive_polling: false,
        }
    }
}

/// True when this node is the active one: the node that owns the devices and
/// the history. Always true unless an add-on edition has set it otherwise.
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// True when device pollers should be running on this node.
pub fn polls_devices() -> bool {
    POLLING.load(Ordering::SeqCst)
}

/// Set the node role. `polling` says whether device pollers run on a passive
/// node (read-only values for the UI); an active node always polls.
pub fn set(active: bool, passive_polling: bool) {
    ACTIVE.store(active, Ordering::SeqCst);
    POLLING.store(active || passive_polling, Ordering::SeqCst);
}

/// Path prefixes a passive node still accepts non-read requests on (sign-in
/// and the add-on's own control endpoints). Replaces the previous list.
pub fn set_passive_write_exemptions(prefixes: &[&'static str]) {
    *EXEMPT_PREFIXES.write().unwrap_or_else(|e| e.into_inner()) = prefixes.to_vec();
}

/// Whether the HTTP API accepts this request on this node. An active node
/// accepts everything. A passive node serves reads and the exempt prefixes,
/// and refuses every other write so configuration cannot diverge from the
/// active node's.
pub fn request_allowed(method: &Method, path: &str) -> bool {
    if is_active() {
        return true;
    }
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    EXEMPT_PREFIXES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|p| path.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The state is process-wide: serialize the tests that flip it.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn passive_node_serves_reads_and_exempt_paths_only() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_passive_write_exemptions(&["/api/v1/auth/", "/api/v1/ha/"]);
        set(false, false);

        assert!(request_allowed(&Method::GET, "/api/v1/devices"));
        assert!(request_allowed(&Method::POST, "/api/v1/auth/login"));
        assert!(request_allowed(&Method::POST, "/api/v1/ha/force-sync"));
        assert!(!request_allowed(&Method::POST, "/api/v1/modbus-devices"));
        assert!(!request_allowed(&Method::DELETE, "/api/v1/screens/1"));
        assert!(!polls_devices());

        set(false, true);
        assert!(polls_devices(), "passive polling keeps pollers running");

        set(true, false);
        assert!(request_allowed(&Method::DELETE, "/api/v1/screens/1"));
        assert!(polls_devices(), "an active node always polls");
        set_passive_write_exemptions(&[]);
    }
}
