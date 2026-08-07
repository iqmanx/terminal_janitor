//! Conservative, read-only Day 2 activity observations.
//!
//! Only the three workspace markers plus narrow `.git/HEAD`, its loose ref,
//! `packed-refs`, and `.git/index` probes are inspected. No command is run and
//! filesystem access time is never consulted. Worktree state therefore stays
//! explicitly unknown until a later proof layer can establish it safely.

use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::state::{GitState, Timestamp};

const MAX_HEAD_BYTES: u64 = 4 * 1024;
const MAX_REF_BYTES: u64 = 4 * 1024;
const MAX_PACKED_REFS_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivitySnapshot {
    pub activity_at: Option<Timestamp>,
    pub git_state: GitState,
    pub git_head_fingerprint: Option<String>,
    pub git_index_fingerprint: Option<String>,
}

pub trait ActivityProbe {
    fn observe(&self, workspace: &Path) -> io::Result<ActivitySnapshot>;
}

#[derive(Debug, Default)]
pub struct SystemActivityProbe;

impl ActivityProbe for SystemActivityProbe {
    fn observe(&self, workspace: &Path) -> io::Result<ActivitySnapshot> {
        observe_system(workspace)
    }
}

fn observe_system(workspace: &Path) -> io::Result<ActivitySnapshot> {
    let mut newest = None;
    for marker in ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"] {
        newest = max_optional(newest, modified_at(&workspace.join(marker))?);
    }

    let git_path = workspace.join(".git");
    // `.git` itself must not be a symlink, but the approved root is canonicalised
    // before any containment check. An uncanonicalised root would reject its own
    // children wherever an ancestor is a link or an alias: macOS reaches
    // temporary and user directories through `/var -> /private/var`, and Windows
    // hands out 8.3 short names such as `RUNNER~1`.
    let git_root = match fs::symlink_metadata(&git_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => None,
        Ok(_) => Some(dunce::canonicalize(&git_path)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };

    let mut head_fingerprint = None;
    let mut index_fingerprint = None;
    if let Some(git_dir) = git_root {
        let head_path = git_dir.join("HEAD");
        if let Some((bytes, modified)) =
            read_optional_bounded(&head_path, &git_dir, MAX_HEAD_BYTES)?
        {
            newest = max_optional(newest, modified);
            let mut fingerprint_bytes = bytes.clone();
            if let Some(reference) = parse_head_reference(&bytes) {
                let ref_path = git_dir.join(reference);
                if ref_path.starts_with(&git_dir)
                    && let Some((ref_bytes, ref_modified)) =
                        read_optional_bounded(&ref_path, &git_dir, MAX_REF_BYTES)?
                {
                    newest = max_optional(newest, ref_modified);
                    fingerprint_bytes.extend_from_slice(&ref_bytes);
                } else if let Some((packed, packed_modified)) = read_optional_bounded(
                    &git_dir.join("packed-refs"),
                    &git_dir,
                    MAX_PACKED_REFS_BYTES,
                )? {
                    newest = max_optional(newest, packed_modified);
                    fingerprint_bytes.extend_from_slice(&packed);
                }
            }
            head_fingerprint = Some(fingerprint(&fingerprint_bytes));
        }

        let index_path = git_dir.join("index");
        match fs::symlink_metadata(&index_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {}
            Ok(metadata) => {
                let canonical = dunce::canonicalize(&index_path)?;
                if canonical.starts_with(&git_dir) {
                    let modified = system_time_millis(metadata.modified()?);
                    newest = max_optional(newest, modified);
                    index_fingerprint = Some(format!(
                        "mtime:{}:len:{}",
                        modified.as_unix_millis(),
                        metadata.len()
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }

    Ok(ActivitySnapshot {
        activity_at: newest,
        git_state: GitState::Unknown,
        git_head_fingerprint: head_fingerprint,
        git_index_fingerprint: index_fingerprint,
    })
}

fn read_optional_bounded(
    path: &Path,
    allowed_root: &Path,
    limit: u64,
) -> io::Result<Option<(Vec<u8>, Timestamp)>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Ok(None);
        }
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.len() > limit {
        return Ok(None);
    }
    let canonical = dunce::canonicalize(path)?;
    if !canonical.starts_with(allowed_root) {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(canonical)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Ok(None);
    }
    Ok(Some((bytes, system_time_millis(metadata.modified()?))))
}

fn parse_head_reference(bytes: &[u8]) -> Option<PathBuf> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let reference = text.strip_prefix("ref: ")?;
    let path = Path::new(reference);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn modified_at(path: &Path) -> io::Result<Timestamp> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace marker is not a regular file",
        ));
    }
    Ok(system_time_millis(metadata.modified()?))
}

fn system_time_millis(time: std::time::SystemTime) -> Timestamp {
    match time.duration_since(UNIX_EPOCH) {
        Ok(elapsed) => {
            Timestamp::from_unix_millis(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        }
        Err(before_epoch) => Timestamp::from_unix_millis(
            i64::try_from(before_epoch.duration().as_millis())
                .map(|millis| -millis)
                .unwrap_or(i64::MIN),
        ),
    }
}

fn max_optional(current: Option<Timestamp>, candidate: Timestamp) -> Option<Timestamp> {
    Some(current.map_or(candidate, |current| current.max(candidate)))
}

fn fingerprint(bytes: &[u8]) -> String {
    // Fixed FNV-1a is deterministic across processes and platforms. It is an
    // observation token, not a security hash or identity proof.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for marker in ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"] {
            fs::write(dir.path().join(marker), b"marker").unwrap();
        }
        dir
    }

    #[test]
    fn marker_and_git_observations_are_read_only_and_git_stays_unknown() {
        let dir = workspace();
        let git = dir.path().join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(git.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
        fs::write(git.join("refs/heads/main"), b"abc\n").unwrap();
        fs::write(git.join("index"), b"index").unwrap();

        let before: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        let snapshot = SystemActivityProbe.observe(dir.path()).unwrap();
        let after: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(before, after);
        assert_eq!(snapshot.git_state, GitState::Unknown);
        assert!(snapshot.git_head_fingerprint.is_some());
        assert!(snapshot.git_index_fingerprint.is_some());
    }

    #[test]
    fn head_content_change_changes_fingerprint() {
        let dir = workspace();
        let git = dir.path().join(".git");
        fs::create_dir(&git).unwrap();
        fs::write(git.join("HEAD"), b"first\n").unwrap();
        let first = SystemActivityProbe.observe(dir.path()).unwrap();
        fs::write(git.join("HEAD"), b"second\n").unwrap();
        let second = SystemActivityProbe.observe(dir.path()).unwrap();
        assert_ne!(first.git_head_fingerprint, second.git_head_fingerprint);
    }

    #[test]
    fn manifest_and_lockfile_modifications_advance_activity_observation() {
        let dir = workspace();
        let first = SystemActivityProbe.observe(dir.path()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.path().join("package.json"), b"manifest changed").unwrap();
        let manifest = SystemActivityProbe.observe(dir.path()).unwrap();
        assert!(manifest.activity_at >= first.activity_at);

        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.path().join("pnpm-lock.yaml"), b"lockfile changed").unwrap();
        let lockfile = SystemActivityProbe.observe(dir.path()).unwrap();
        assert!(lockfile.activity_at >= manifest.activity_at);
    }

    #[test]
    fn git_index_change_changes_bounded_fingerprint() {
        let dir = workspace();
        let git = dir.path().join(".git");
        fs::create_dir(&git).unwrap();
        fs::write(git.join("index"), b"index-a").unwrap();
        let first = SystemActivityProbe.observe(dir.path()).unwrap();
        fs::write(git.join("index"), b"index-longer-b").unwrap();
        let second = SystemActivityProbe.observe(dir.path()).unwrap();
        assert_ne!(first.git_index_fingerprint, second.git_index_fingerprint);
    }

    /// A workspace reached through a linked ancestor is the shape macOS presents
    /// for every temporary and user directory (`/var -> /private/var`), and the
    /// shape Windows presents through 8.3 short names.
    #[cfg(unix)]
    #[test]
    fn observations_survive_a_linked_ancestor_directory() {
        let dir = workspace();
        let git = dir.path().join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(git.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
        fs::write(git.join("refs/heads/main"), b"abc\n").unwrap();
        fs::write(git.join("index"), b"index").unwrap();

        let link_parent = tempfile::tempdir().unwrap();
        let linked_workspace = link_parent.path().join("workspace-link");
        std::os::unix::fs::symlink(dir.path(), &linked_workspace).unwrap();

        let direct = SystemActivityProbe.observe(dir.path()).unwrap();
        let linked = SystemActivityProbe.observe(&linked_workspace).unwrap();
        assert!(linked.git_head_fingerprint.is_some());
        assert!(linked.git_index_fingerprint.is_some());
        assert_eq!(direct, linked);
    }

    #[cfg(unix)]
    #[test]
    fn git_symlink_is_not_followed() {
        let dir = workspace();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("HEAD"), b"secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join(".git")).unwrap();
        let snapshot = SystemActivityProbe.observe(dir.path()).unwrap();
        assert_eq!(snapshot.git_head_fingerprint, None);
        assert_eq!(snapshot.git_state, GitState::Unknown);
    }
}
