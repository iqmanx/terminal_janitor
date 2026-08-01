use std::io;
use std::path::Path;

use crate::model::{DiskCapacity, DiskError};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Dispatches to the platform-specific capacity query.
///
/// Each `#[cfg(target_os = "...")]` module is a dedicated seam for future
/// platform-specific handling (for example macOS APFS/snapshot capacity
/// ambiguity), even though all three currently delegate to the same
/// cross-platform syscall wrapper.
pub fn capacity_for(path: &Path) -> Result<DiskCapacity, DiskError> {
    #[cfg(target_os = "linux")]
    {
        linux::capacity_for(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::capacity_for(path)
    }
    #[cfg(target_os = "windows")]
    {
        windows::capacity_for(path)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err(DiskError::UnsupportedPlatform)
    }
}

#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
fn measure(path: &Path) -> Result<DiskCapacity, DiskError> {
    let total = fs4::total_space(path).map_err(|source| to_disk_error(path, source))?;
    let available = fs4::available_space(path).map_err(|source| to_disk_error(path, source))?;
    DiskCapacity::new(total, available)
}

#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
fn to_disk_error(path: &Path, source: io::Error) -> DiskError {
    if source.kind() == io::ErrorKind::NotFound {
        DiskError::PathNotFound(path.to_path_buf())
    } else {
        DiskError::MeasurementFailed {
            path: path.to_path_buf(),
            source,
        }
    }
}
