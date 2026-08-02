//! terminal_janitor foundation and read-only registration (Days 1–2B).
//!
//! Exposes configuration (with atomic writes), platform-neutral storage
//! status, a testable [`disk::DiskProvider`] abstraction, canonical
//! filesystem [`identity`] types, and the persistent metadata [`state`]
//! ledger. Discovery only reads approved roots. Nothing here performs cleanup,
//! planning, pnpm execution, project mutation, or scheduling.

pub mod activity;
pub mod cli;
pub mod config;
pub mod discovery;
pub mod disk;
pub mod identity;
pub mod model;
mod platform;
pub mod state;
pub mod status;
pub mod workflows;
