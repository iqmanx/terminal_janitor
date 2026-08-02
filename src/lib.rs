//! terminal_janitor read-only foundation (Day 1).
//!
//! Exposes configuration, platform-neutral storage status, rendering, and a
//! testable [`disk::DiskProvider`] abstraction. This remains read-only: no
//! discovery, planning, cleanup execution, or scheduling lives here yet.

pub mod cli;
pub mod config;
pub mod disk;
pub mod model;
mod platform;
pub mod status;
