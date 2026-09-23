//! RunwayBar core library: provider discovery, quota polling, snapshot model, cache.
//!
//! Invariants upheld here (see the plan bundle's IMPLICIT_SPEC):
//! read-only credential access, redacted secrets, unknown-stays-unknown window
//! normalisation, per-provider failure isolation, bounded fetches.

pub mod autostart;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod error;
pub mod http;
pub mod icon;
pub mod ipc;
pub mod model;
pub mod notify;
pub mod providers;
pub mod secret;
pub mod singleinst;
pub mod state;
pub mod store;
pub mod timefmt;
pub mod tray;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const USER_AGENT: &str = concat!("RunwayBar/", env!("CARGO_PKG_VERSION"));
