//! Cleanup of `terminal_janitor`'s own state.
//!
//! This is the smallest authority the product has, and the only action that
//! deletes bytes with Rust rather than delegating. It is allowed to because
//! every byte it touches is the product's own: the ledger it wrote and the
//! lock files it created. It never leaves the product's private state
//! directory, and it never touches configuration or protection data.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Directory holding per-volume lock files, inside the product state directory.
pub const LOCK_DIRECTORY: &str = "locks";

/// A lock file older than this cannot belong to a live run on any reasonable
/// machine, so it is a leftover from a killed process.
pub const STALE_LOCK_AGE_MILLIS: u64 = 24 * 60 * 60 * 1000;

/// Removes lock files that no live run can still hold.
///
/// A lock file that is currently held is left alone: the check is whether the
/// file can be locked, not whether it merely exists. Anything unreadable is
/// left in place rather than guessed at.
pub fn remove_stale_locks(state_directory: &Path, now_millis: u64) -> io::Result<usize> {
    let locks = state_directory.join(LOCK_DIRECTORY);
    let entries = match fs::read_dir(&locks) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };

    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !is_own_lock_file(&path) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        if !older_than(&metadata, now_millis, STALE_LOCK_AGE_MILLIS) {
            continue;
        }
        // Age alone is not proof. A run that has been going for a day would
        // still hold its lock, so the file is only removed when it can be
        // locked, which means nothing holds it.
        if !can_take_exclusively(&path) {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Only files this product created are candidates. The name pattern is fixed
/// by [`lock_file_path`], so nothing a user placed here is ever removed.
fn is_own_lock_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("volume-") && name.ends_with(".lock"))
}

fn older_than(metadata: &fs::Metadata, now_millis: u64, age_millis: u64) -> bool {
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let Ok(elapsed) = modified.duration_since(std::time::UNIX_EPOCH) else {
        return false;
    };
    let modified_millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    now_millis.saturating_sub(modified_millis) > age_millis
}

fn can_take_exclusively(path: &Path) -> bool {
    let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(path) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            true
        }
        Err(_) => false,
    }
}

/// The lock file for one volume.
///
/// The volume identity is hashed rather than used directly: a native volume
/// identity can contain characters no filesystem accepts in a name, and a
/// fixed-width hash keeps one file per volume without that risk.
pub fn lock_file_path(state_directory: &Path, volume_id: &str) -> PathBuf {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in volume_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    state_directory
        .join(LOCK_DIRECTORY)
        .join(format!("volume-{hash:016x}.lock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(LOCK_DIRECTORY)).unwrap();
        dir
    }

    fn now() -> u64 {
        u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    }

    #[test]
    fn one_volume_maps_to_one_stable_lock_file() {
        let dir = state_dir();
        let first = lock_file_path(dir.path(), "unix-dev:66");
        let second = lock_file_path(dir.path(), "unix-dev:66");
        assert_eq!(first, second);
        assert_ne!(first, lock_file_path(dir.path(), "unix-dev:67"));
        // The native identity's punctuation must not reach the filename.
        let name = first.file_name().unwrap().to_str().unwrap();
        assert!(!name.contains(':'));
        assert!(name.starts_with("volume-") && name.ends_with(".lock"));
    }

    #[test]
    fn a_missing_lock_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(remove_stale_locks(dir.path(), now()).unwrap(), 0);
    }

    #[test]
    fn a_fresh_lock_file_is_left_alone() {
        let dir = state_dir();
        let path = lock_file_path(dir.path(), "unix-dev:66");
        fs::write(&path, b"").unwrap();
        assert_eq!(remove_stale_locks(dir.path(), now()).unwrap(), 0);
        assert!(path.exists());
    }

    #[test]
    fn an_old_unheld_lock_file_is_removed() {
        let dir = state_dir();
        let path = lock_file_path(dir.path(), "unix-dev:66");
        fs::write(&path, b"").unwrap();
        // Ask as though a great deal of time had passed, rather than making
        // the test wait or rewrite filesystem timestamps.
        let far_future = now() + 10 * STALE_LOCK_AGE_MILLIS;
        assert_eq!(remove_stale_locks(dir.path(), far_future).unwrap(), 1);
        assert!(!path.exists());
    }

    #[test]
    fn an_old_but_still_held_lock_file_is_left_alone() {
        let dir = state_dir();
        let path = lock_file_path(dir.path(), "unix-dev:66");
        let held = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        held.try_lock().unwrap();

        let far_future = now() + 10 * STALE_LOCK_AGE_MILLIS;
        assert_eq!(remove_stale_locks(dir.path(), far_future).unwrap(), 0);
        assert!(path.exists(), "a held lock must survive cleanup");
        let _ = held.unlock();
    }

    #[test]
    fn files_this_product_did_not_create_are_never_removed() {
        let dir = state_dir();
        let foreign = dir.path().join(LOCK_DIRECTORY).join("notes.txt");
        fs::write(&foreign, b"a user file").unwrap();
        let far_future = now() + 10 * STALE_LOCK_AGE_MILLIS;
        assert_eq!(remove_stale_locks(dir.path(), far_future).unwrap(), 0);
        assert!(foreign.exists());
    }
}
