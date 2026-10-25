//! voltrus-scada-native: the network side of the GPUI operator console, with
//! no UI, so it can be tested on its own.
//!
//! - [`api`]: REST client (sign-in cookies, generic JSON helpers, tags,
//!   alarms, history).
//! - [`ws_types`]: the WebSocket wire format, mirrored from the server.
//! - [`runtime`]: the actor that owns the session and streams events,
//!   including generic REST calls tied to the session.
//! - [`stress`]: a deterministic synthetic plant for stress tests.
//! - [`perf`]: frame statistics for the HUD and the stress report.
pub mod api;
pub mod perf;
pub mod runtime;
pub mod stress;
pub mod ws_types;
