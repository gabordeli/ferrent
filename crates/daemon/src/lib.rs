//! Ferrent daemon library: persistence and, later, the RPC server.

/// Version reported over `GetStatus`, so a client can tell which build answers.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod autoadd;
pub mod automation;
pub mod blocklist;
pub mod compat;
pub mod config;
pub mod control;
pub mod diskguard;
pub mod events;
pub mod hooks;
pub mod labels;
pub mod metrics;
pub mod netbind;
pub mod notify;
pub mod persist;
pub mod platform;
pub mod rpc;
pub mod rss;
pub mod rss_service;
pub mod scheduler;
pub mod state;
pub mod store;
