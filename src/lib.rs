//! terminal_janitor systems foundation (Day 1A).
//!
//! Exposes a platform-neutral disk-capacity model and a testable
//! [`disk::DiskProvider`] abstraction. This is read-only: no configuration,
//! discovery, planning, execution, or scheduling lives here yet.

pub mod disk;
pub mod model;
mod platform;
