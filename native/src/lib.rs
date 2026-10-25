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
//! - [`screens`]: screens, their card layouts and the 24-column card grid
//!   (compaction, drag, resize, auto-arrange), shared with the mimic viewer.
//! - [`tag_api`]: tag writes and write policies, the tag tree, naming
//!   convention, annotations and the tag detail's history queries.
//! - [`mimic`]: HMI mimic screens: the server's SCADA rows, the editable
//!   screen model, the symbol library's geometry, live states, connection
//!   routes and the faceplate's command rules.
pub mod api;
pub mod mimic;
pub mod perf;
pub mod runtime;
pub mod screens;
pub mod stress;
pub mod tag_api;
pub mod ws_types;
