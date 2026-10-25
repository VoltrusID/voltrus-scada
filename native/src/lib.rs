//! voltrus-scada-native: the network side of the GPUI operator console, with
//! no UI, so it can be tested on its own.
//!
//! - [`api`]: REST client (sign-in cookie, tags, alarms, history).
//! - [`ws_types`]: the WebSocket wire format, mirrored from the server.
//! - [`runtime`]: the actor that owns the session and streams events.
pub mod api;
pub mod runtime;
pub mod ws_types;
