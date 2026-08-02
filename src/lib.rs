//! terminal_janitor foundation (Days 1–2A).
//!
//! Exposes configuration (with atomic writes), platform-neutral storage
//! status, a testable [`disk::DiskProvider`] abstraction, canonical
//! filesystem [`identity`] types, and the persistent metadata [`state`]
//! ledger. Nothing here performs discovery, planning, cleanup execution, or
//! scheduling: the ledger stores metadata handed to it, and the only
//! filesystem writes are the ledger file itself and atomic configuration
//! replacement.

pub mod cli;
pub mod config;
pub mod disk;
pub mod identity;
pub mod model;
mod platform;
pub mod state;
pub mod status;
