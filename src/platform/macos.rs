//! macOS capacity, resolved rather than refused.
//!
//! On APFS the raw `statvfs` availability figure is ambiguous: it excludes
//! space the system holds as purgeable and space pinned by Time Machine local
//! snapshots, either of which the operating system will release when a real
//! request needs it (`RESEARCH.md` 4.4/4.7, `ACCEPTANCE.md` section K). A tool
//! that deletes things must not act on a figure whose meaning it cannot state.
//!
//! Foundation publishes the resolved figure as
//! `NSURLVolumeAvailableCapacityForImportantUsageKey`: the space the system
//! would actually make available for something the user asked for, purgeable
//! space included. That is the number this module reports.
//!
//! **Why the larger figure is the safe one.** It is never smaller than the raw
//! one, so using it makes this product believe there is *more* free space, and
//! therefore clean *less*. Reporting the smaller raw figure would invent
//! pressure that does not exist and delete a workspace to relieve it. When the
//! key cannot be read the module falls back to the raw figure and reports
//! [`CapacityConfidence::SnapshotUncertain`], which blocks workspace cleaning
//! exactly as before.
//!
//! Nothing here deletes or thins a snapshot. This module only reads, and it
//! runs no command: `tmutil`, `diskutil`, and `apfs` appear nowhere in this
//! product.

use std::path::Path;

use objc2_foundation::{
    NSArray, NSNumber, NSString, NSURL, NSURLResourceKey,
    NSURLVolumeAvailableCapacityForImportantUsageKey,
};

use super::CapacityConfidence;
use crate::model::{DiskCapacity, DiskError};

/// Reports capacity for the volume containing `path`.
///
/// The total comes from the same cross-platform measurement every platform
/// uses; only the available figure is snapshot-aware. When Foundation cannot
/// answer, the raw measurement is returned unchanged and
/// [`capacity_confidence`] reports the uncertainty.
pub fn capacity_for(path: &Path) -> Result<DiskCapacity, DiskError> {
    let raw = super::measure(path)?;
    match snapshot_aware_available(path, raw.total_bytes) {
        Some(available_bytes) => DiskCapacity::new(raw.total_bytes, available_bytes),
        None => Ok(raw),
    }
}

/// Whether the free-space figure for this volume can be trusted.
///
/// `Confident` exactly when the snapshot-aware figure was readable and
/// coherent. Every other outcome — a non-UTF-8 path, a path Foundation refuses,
/// a missing or non-numeric value, or a figure larger than the volume itself —
/// is uncertainty, and uncertainty blocks workspace cleaning.
pub fn capacity_confidence(path: &Path) -> CapacityConfidence {
    let Ok(raw) = super::measure(path) else {
        return CapacityConfidence::SnapshotUncertain;
    };
    match snapshot_aware_available(path, raw.total_bytes) {
        Some(_) => CapacityConfidence::Confident,
        None => CapacityConfidence::SnapshotUncertain,
    }
}

pub fn file_identity(path: &Path) -> Result<(String, String), std::io::Error> {
    super::unix_file_identity(path)
}

/// The snapshot-aware available figure, or `None` if it cannot be established.
///
/// A value larger than the volume's own total is incoherent, so it is refused
/// rather than clamped: a clamped figure would look like a measurement.
fn snapshot_aware_available(path: &Path, total_bytes: u64) -> Option<u64> {
    // SAFETY: these are Foundation's own exported constant `NSString`s, read
    // and never written. Reading an `extern "C"` static is the only unsafe
    // operation in this crate; the Objective-C calls below are safe bindings.
    let key: &NSURLResourceKey = unsafe { NSURLVolumeAvailableCapacityForImportantUsageKey };

    // A path this product cannot spell in UTF-8 is never measured; identity
    // rejects such paths too, so this fails closed consistently.
    let path = NSString::from_str(path.to_str()?);
    let url = NSURL::fileURLWithPath(&path);
    let values = url
        .resourceValuesForKeys_error(&NSArray::from_slice(&[key]))
        .ok()?;
    let value = values.objectForKey(key)?;
    let bytes = value.downcast_ref::<NSNumber>()?.longLongValue();

    let bytes = u64::try_from(bytes).ok()?;
    (bytes <= total_bytes).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs on whatever Mac is executing. This is the claim that matters: a
    /// real macOS machine can answer the snapshot-aware question, so macOS is
    /// no longer blocked from cleaning a workspace.
    #[test]
    fn this_mac_answers_the_snapshot_aware_question() {
        let path = std::env::temp_dir();
        let capacity = capacity_for(&path).expect("a temp directory must be measurable");

        assert_eq!(
            capacity_confidence(&path),
            CapacityConfidence::Confident,
            "a real Mac must resolve the purgeable/snapshot ambiguity"
        );
        assert!(capacity.available_bytes <= capacity.total_bytes);
        assert!(capacity.total_bytes > 0);
        assert_eq!(
            capacity.used_bytes,
            capacity.total_bytes - capacity.available_bytes
        );
    }

    /// The resolved figure counts space the raw one hides, so it is never the
    /// smaller of the two. If this ever fails, the key's meaning has changed
    /// and the safety argument for preferring it no longer holds.
    #[test]
    fn the_resolved_figure_is_never_smaller_than_the_raw_one() {
        let path = std::env::temp_dir();
        let raw = super::super::measure(&path).unwrap();
        let resolved = capacity_for(&path).unwrap();
        assert!(
            resolved.available_bytes >= raw.available_bytes,
            "resolved {} must not be below raw {}",
            resolved.available_bytes,
            raw.available_bytes
        );
    }

    /// A path Foundation cannot resolve must not silently become a confident
    /// measurement.
    #[test]
    fn an_unmeasurable_path_stays_uncertain() {
        let missing = std::env::temp_dir().join("terminal_janitor-no-such-volume-path");
        assert!(capacity_for(&missing).is_err());
        assert_eq!(
            capacity_confidence(&missing),
            CapacityConfidence::SnapshotUncertain
        );
    }
}
