use std::env;

use terminal_janitor::disk::{DiskProvider, SystemDiskProvider};

/// Day 1A placeholder entry point.
///
/// This is not the product CLI (no subcommands, no configuration, no
/// cleanup). It only exercises the read-only disk-capacity systems API so
/// the crate produces a runnable binary. `clap`, `status`, and friends are
/// Day 1B scope.
fn main() {
    let path = env::current_dir().unwrap_or_else(|_| ".".into());
    let provider = SystemDiskProvider;

    match provider.capacity_for(&path) {
        Ok(capacity) => {
            println!(
                "terminal_janitor systems check ({}): total={} available={} used={} bytes",
                path.display(),
                capacity.total_bytes,
                capacity.available_bytes,
                capacity.used_bytes
            );
        }
        Err(err) => {
            eprintln!("terminal_janitor systems check failed: {err}");
            std::process::exit(1);
        }
    }
}
