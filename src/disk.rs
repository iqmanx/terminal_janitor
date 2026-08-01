use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::{DiskCapacity, DiskError};
use crate::platform;

/// Reports the capacity of the volume containing a given path.
pub trait DiskProvider {
    fn capacity_for(&self, path: &Path) -> Result<DiskCapacity, DiskError>;
}

/// Production [`DiskProvider`] backed by the platform's native capacity
/// query. Performs no shell commands, output parsing, privilege escalation,
/// directory scanning, or filesystem mutation.
#[derive(Debug, Default)]
pub struct SystemDiskProvider;

impl DiskProvider for SystemDiskProvider {
    fn capacity_for(&self, path: &Path) -> Result<DiskCapacity, DiskError> {
        platform::capacity_for(path)
    }
}

/// Deterministic [`DiskProvider`] for tests. Paths not explicitly injected
/// resolve to [`DiskError::PathNotFound`] rather than a fabricated value.
#[derive(Debug, Default)]
pub struct FakeDiskProvider {
    capacities: HashMap<PathBuf, DiskCapacity>,
}

impl FakeDiskProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_capacity(&mut self, path: impl Into<PathBuf>, capacity: DiskCapacity) -> &mut Self {
        self.capacities.insert(path.into(), capacity);
        self
    }
}

impl DiskProvider for FakeDiskProvider {
    fn capacity_for(&self, path: &Path) -> Result<DiskCapacity, DiskError> {
        self.capacities
            .get(path)
            .copied()
            .ok_or_else(|| DiskError::PathNotFound(path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_provider_returns_injected_capacity() {
        let mut fake = FakeDiskProvider::new();
        let cap = DiskCapacity::new(2000, 500).unwrap();
        fake.set_capacity("/workspace/project", cap);

        let result = fake.capacity_for(Path::new("/workspace/project")).unwrap();
        assert_eq!(result, cap);
    }

    #[test]
    fn fake_provider_unknown_path_errors() {
        let fake = FakeDiskProvider::new();
        let err = fake.capacity_for(Path::new("/unknown")).unwrap_err();
        assert!(matches!(err, DiskError::PathNotFound(_)));
    }

    #[test]
    fn system_provider_rejects_nonexistent_path() {
        let provider = SystemDiskProvider;
        let missing = std::env::temp_dir().join("terminal_janitor_test_missing_xyz123");

        let err = provider.capacity_for(&missing).unwrap_err();
        assert!(matches!(
            err,
            DiskError::PathNotFound(_) | DiskError::MeasurementFailed { .. }
        ));
    }

    #[test]
    fn system_provider_measures_path_containing_spaces() {
        let dir = tempfile::Builder::new()
            .prefix("tj test dir")
            .tempdir()
            .unwrap();
        let provider = SystemDiskProvider;

        let cap = provider.capacity_for(dir.path()).unwrap();
        assert!(cap.available_bytes <= cap.total_bytes);
        assert_eq!(cap.used_bytes, cap.total_bytes - cap.available_bytes);
    }

    #[test]
    fn system_provider_measures_unicode_path() {
        let dir = tempfile::Builder::new()
            .prefix("tj-Ünïcödé-日本語-")
            .tempdir();
        let dir = match dir {
            Ok(dir) => dir,
            // Some CI filesystems reject non-ASCII names; that is a
            // filesystem limitation, not a terminal_janitor defect.
            Err(_) => return,
        };
        let provider = SystemDiskProvider;

        let cap = provider.capacity_for(dir.path()).unwrap();
        assert!(cap.available_bytes <= cap.total_bytes);
    }

    #[test]
    fn system_provider_result_is_deterministic_for_stable_volume() {
        let provider = SystemDiskProvider;
        let dir = tempfile::tempdir().unwrap();

        let first = provider.capacity_for(dir.path()).unwrap();
        let second = provider.capacity_for(dir.path()).unwrap();
        assert_eq!(first.total_bytes, second.total_bytes);
    }
}
