//! Live collaboration in the Voltrus Designer (docs/collaboration.md): a
//! screen's draft rows as a Yjs document ([`document`]), stacking order as
//! fractional indexes ([`fractional`]) and the y-sync and awareness messages
//! ([`wire`]). The server keeps the authoritative document per open screen;
//! the native console joins with the same code. The web Designer uses Yjs,
//! whose encoding this shares.

pub mod document;
pub mod fractional;
pub mod wire;

pub use yrs;

/// The collaboration protocol version, sent in `DesignJoin`. A server and a
/// client that disagree do not join (the client reloads).
pub const PROTOCOL: u32 = 1;
