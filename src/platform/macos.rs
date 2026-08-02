use std::path::Path;

use crate::model::{DiskCapacity, DiskError};

// NOTE: macOS APFS purgeable space and Time Machine local snapshots can make
// raw statvfs() availability figures ambiguous. Day 1A does not attempt to
// resolve that ambiguity (see RESEARCH.md 4.4/4.7 and SAFETY.md/ACCEPTANCE.md
// section K) — it only reports the raw measurement. Snapshot-aware handling
// is later-day scope.
pub fn capacity_for(path: &Path) -> Result<DiskCapacity, DiskError> {
    super::measure(path)
}

pub fn file_identity(path: &Path) -> Result<(String, String), std::io::Error> {
    super::unix_file_identity(path)
}
