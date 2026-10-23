//! Entitlement model.
//!
//! The open core has no licence checks of its own. It only carries the
//! vocabulary an add-on edition uses to say which add-on features are unlocked:
//! [`LicenseTier`] (what is enabled) and [`LicenseStatus`] (why).
//! A plain core build always runs as [`LicenseTier::Community`].

pub mod status;
pub mod tier;

#[cfg(test)]
mod tests;

pub use status::*;
pub use tier::*;
