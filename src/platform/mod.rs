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

/// How far a free-space figure for this volume can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityConfidence {
    /// The reported free space is what a caller would actually get.
    Confident,
    /// The figure may include space the operating system holds in snapshots or
    /// counts as purgeable, so it cannot be relied on to decide that deleting
    /// something was worthwhile.
    SnapshotUncertain,
}

/// Reports whether free space on the volume containing `path` is unambiguous.
///
/// macOS is the reason this exists. On APFS the raw available figure excludes
/// purgeable space and space held by Time Machine local snapshots, so it does
/// not describe what a caller would actually get. [`macos`] resolves that
/// through Foundation and reports `Confident` when it succeeds; when it cannot,
/// the ambiguity is unresolved and `SnapshotUncertain` blocks workspace
/// cleaning, as `ACCEPTANCE.md` section K requires. `terminal_janitor` never
/// deletes or thins a snapshot under any circumstances.
pub fn capacity_confidence(path: &Path) -> CapacityConfidence {
    #[cfg(target_os = "macos")]
    {
        macos::capacity_confidence(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        CapacityConfidence::Confident
    }
}

/// Best-available native identity for the volume containing `path` and for
/// the file or directory itself, as `(volume, file)` opaque strings.
///
/// Uses only native filesystem metadata: `stat` device/inode numbers on Linux
/// and macOS, and the volume serial number plus file index from
/// `GetFileInformationByHandle` on Windows. No shell command, output parsing,
/// or privilege escalation is involved. Unsupported platforms and any
/// metadata failure return an explicit error; identity is never guessed.
pub fn file_identity(path: &Path) -> Result<(String, String), io::Error> {
    #[cfg(target_os = "linux")]
    {
        linux::file_identity(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::file_identity(path)
    }
    #[cfg(target_os = "windows")]
    {
        windows::file_identity(path)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err(io::Error::other(
            "file identity is not supported on this platform",
        ))
    }
}

/// Shared Unix implementation: the `st_dev`/`st_ino` pair from native
/// metadata is the strongest identity available without elevated privileges.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unix_file_identity(path: &Path) -> Result<(String, String), io::Error> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path)?;
    Ok((
        format!("unix-dev:{}", metadata.dev()),
        format!("unix-ino:{}", metadata.ino()),
    ))
}

#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", target_os = "windows")),
    allow(dead_code)
)]
fn measure(path: &Path) -> Result<DiskCapacity, DiskError> {
    // Windows GetDiskFreeSpaceExW may successfully resolve the volume for a
    // nonexistent child path. Validate the requested measurement target first
    // so every platform preserves DiskProvider's exact-path contract.
    std::fs::metadata(path).map_err(|source| to_disk_error(path, source))?;
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
