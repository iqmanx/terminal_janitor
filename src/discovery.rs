//! Bounded, read-only pnpm workspace discovery beneath approved roots.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::activity::ActivityProbe;
use crate::identity::{IdentityError, PathIdentity};
use crate::state::{ApprovedRootRecord, ValidatedRoot, WorkspaceObservation};

pub const MAX_TRAVERSAL_DEPTH: usize = 64;
pub const MAX_DIAGNOSTICS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReason {
    OutsideApprovedRoot,
    RootUnavailable,
    RootIdentityChanged,
    SymlinkNotFollowed,
    PermissionDenied,
    MissingPackageJson,
    MissingPnpmWorkspace,
    MissingPnpmLockfile,
    CanonicalisationFailed,
    VolumeIdentityUnavailable,
    DuplicateIdentity,
    PathChangedDuringScan,
    UnsupportedPath,
}

impl fmt::Display for ExclusionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = serde_json::to_value(self).map_err(|_| fmt::Error)?;
        f.write_str(value.as_str().ok_or(fmt::Error)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExcludedPath {
    pub path: PathBuf,
    pub reason: ExclusionReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnavailableRoot {
    pub path: PathBuf,
    pub reason: ExclusionReason,
}

#[derive(Debug, Clone)]
pub struct DiscoveryBatch {
    pub approved_roots: usize,
    pub validated_roots: Vec<ValidatedRoot>,
    pub observations: Vec<WorkspaceObservation>,
    pub excluded: Vec<ExcludedPath>,
    pub unavailable_roots: Vec<UnavailableRoot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub file_name: String,
    pub kind: EntryKind,
}

pub trait DiscoveryProvider {
    fn entry_kind(&self, path: &Path) -> io::Result<EntryKind>;
    fn read_directory(&self, path: &Path) -> io::Result<Vec<DirectoryEntry>>;
    fn resolve_identity(&self, path: &Path) -> Result<PathIdentity, IdentityError>;
}

#[derive(Debug, Default)]
pub struct SystemDiscoveryProvider;

impl DiscoveryProvider for SystemDiscoveryProvider {
    fn entry_kind(&self, path: &Path) -> io::Result<EntryKind> {
        let file_type = fs::symlink_metadata(path)?.file_type();
        Ok(kind_from_file_type(file_type))
    }

    fn read_directory(&self, path: &Path) -> io::Result<Vec<DirectoryEntry>> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory changed type before traversal",
            ));
        }
        let canonical = dunce::canonicalize(path)?;
        if canonical != path {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory identity changed before traversal",
            ));
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&canonical)? {
            let entry = entry?;
            let file_name = entry.file_name().into_string().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory entry is not representable in UTF-8",
                )
            })?;
            entries.push(DirectoryEntry {
                path: entry.path(),
                file_name,
                kind: kind_from_file_type(entry.file_type()?),
            });
        }
        entries.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        Ok(entries)
    }

    fn resolve_identity(&self, path: &Path) -> Result<PathIdentity, IdentityError> {
        PathIdentity::resolve(path)
    }
}

fn kind_from_file_type(file_type: fs::FileType) -> EntryKind {
    if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

pub fn discover(
    roots: &[ApprovedRootRecord],
    provider: &dyn DiscoveryProvider,
    activity: &dyn ActivityProbe,
) -> DiscoveryBatch {
    let mut sorted_roots = roots.to_vec();
    sorted_roots.sort_by(|a, b| a.canonical_path.cmp(&b.canonical_path));
    let approved_roots = sorted_roots.len();
    let mut validated_roots = Vec::new();
    let mut unavailable_roots = Vec::new();

    for root in sorted_roots {
        let path = root.canonical_path.as_path();
        match validate_root(&root, provider) {
            Ok(identity) => validated_roots.push(ValidatedRoot {
                id: root.id,
                identity,
            }),
            Err(reason) => unavailable_roots.push(UnavailableRoot {
                path: path.to_path_buf(),
                reason,
            }),
        }
    }

    validated_roots.sort_by(|a, b| a.identity.canonical.cmp(&b.identity.canonical));
    unavailable_roots.sort_by(|a, b| a.path.cmp(&b.path));

    // A top-level approved root already covers nested roots. Validate every
    // root, but traverse each physical subtree once and bind a workspace to
    // its most specific containing root below.
    let mut traversal_roots: Vec<&ValidatedRoot> = Vec::new();
    for root in &validated_roots {
        if !traversal_roots.iter().any(|parent| {
            root.identity
                .canonical
                .as_path()
                .starts_with(parent.identity.canonical.as_path())
        }) {
            traversal_roots.push(root);
        }
    }

    let mut observations_by_identity = BTreeMap::new();
    let mut excluded = Vec::new();
    for root in traversal_roots {
        walk_root(
            root,
            &validated_roots,
            provider,
            activity,
            &mut observations_by_identity,
            &mut excluded,
        );
    }
    excluded.sort_by(|a, b| a.path.cmp(&b.path).then(a.reason.cmp(&b.reason)));
    excluded.dedup();
    let mut observations: Vec<_> = observations_by_identity.into_values().collect();
    observations.sort_by(|a, b| a.identity.canonical.cmp(&b.identity.canonical));

    DiscoveryBatch {
        approved_roots,
        validated_roots,
        observations,
        excluded,
        unavailable_roots,
    }
}

fn validate_root(
    root: &ApprovedRootRecord,
    provider: &dyn DiscoveryProvider,
) -> Result<PathIdentity, ExclusionReason> {
    match provider.entry_kind(root.canonical_path.as_path()) {
        Ok(EntryKind::Directory) => {}
        Ok(_) => return Err(ExclusionReason::RootIdentityChanged),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            return Err(ExclusionReason::PermissionDenied);
        }
        Err(_) => return Err(ExclusionReason::RootUnavailable),
    }
    let identity = provider
        .resolve_identity(root.canonical_path.as_path())
        .map_err(identity_reason)?;
    if identity.canonical != root.canonical_path
        || identity.volume != root.volume_id
        || identity.file != root.file_id
    {
        return Err(ExclusionReason::RootIdentityChanged);
    }
    Ok(identity)
}

fn walk_root(
    traversal_root: &ValidatedRoot,
    all_roots: &[ValidatedRoot],
    provider: &dyn DiscoveryProvider,
    activity: &dyn ActivityProbe,
    observations: &mut BTreeMap<(String, String), WorkspaceObservation>,
    excluded: &mut Vec<ExcludedPath>,
) {
    let root_path = traversal_root.identity.canonical.as_path();
    let mut stack = vec![(root_path.to_path_buf(), 0_usize)];
    let mut visited = BTreeSet::new();

    while let Some((path, depth)) = stack.pop() {
        let kind = match provider.entry_kind(&path) {
            Ok(kind) => kind,
            Err(error) => {
                push_exclusion(excluded, path, io_reason(&error));
                continue;
            }
        };
        if kind == EntryKind::Symlink {
            push_exclusion(excluded, path, ExclusionReason::SymlinkNotFollowed);
            continue;
        }
        if kind != EntryKind::Directory {
            continue;
        }

        let directory_identity = match provider.resolve_identity(&path) {
            Ok(identity) => identity,
            Err(error) => {
                push_exclusion(excluded, path, identity_reason(error));
                continue;
            }
        };
        if !directory_identity
            .canonical
            .as_path()
            .starts_with(root_path)
        {
            push_exclusion(
                excluded,
                directory_identity.canonical.as_path().to_path_buf(),
                ExclusionReason::OutsideApprovedRoot,
            );
            continue;
        }
        let visit_key = (
            directory_identity.volume.as_str().to_owned(),
            directory_identity.file.as_str().to_owned(),
        );
        if !visited.insert(visit_key) {
            push_exclusion(
                excluded,
                directory_identity.canonical.as_path().to_path_buf(),
                ExclusionReason::DuplicateIdentity,
            );
            continue;
        }

        let entries = match provider.read_directory(directory_identity.canonical.as_path()) {
            Ok(entries) => entries,
            Err(error) => {
                push_exclusion(
                    excluded,
                    directory_identity.canonical.as_path().to_path_buf(),
                    io_reason(&error),
                );
                continue;
            }
        };

        let package = marker_present(&entries, "package.json");
        let workspace = marker_present(&entries, "pnpm-workspace.yaml");
        let lockfile = marker_present(&entries, "pnpm-lock.yaml");
        let candidate = entries.iter().any(|entry| {
            matches!(
                entry.file_name.as_str(),
                "package.json" | "pnpm-workspace.yaml" | "pnpm-lock.yaml"
            )
        });
        if candidate {
            let reason = if !package {
                Some(ExclusionReason::MissingPackageJson)
            } else if !workspace {
                Some(ExclusionReason::MissingPnpmWorkspace)
            } else if !lockfile {
                Some(ExclusionReason::MissingPnpmLockfile)
            } else {
                None
            };
            if let Some(reason) = reason {
                push_exclusion(
                    excluded,
                    directory_identity.canonical.as_path().to_path_buf(),
                    reason,
                );
            } else if let Some(owner) = owning_root(all_roots, &directory_identity) {
                match activity.observe(directory_identity.canonical.as_path()) {
                    Ok(activity) => {
                        let key = (
                            directory_identity.volume.as_str().to_owned(),
                            directory_identity.file.as_str().to_owned(),
                        );
                        observations.entry(key).or_insert(WorkspaceObservation {
                            identity: directory_identity.clone(),
                            approved_root_id: owner.id.clone(),
                            activity_at: activity.activity_at,
                            git_state: activity.git_state,
                            git_head_fingerprint: activity.git_head_fingerprint,
                            git_index_fingerprint: activity.git_index_fingerprint,
                        });
                    }
                    Err(error) => push_exclusion(
                        excluded,
                        directory_identity.canonical.as_path().to_path_buf(),
                        io_reason(&error),
                    ),
                }
            } else {
                push_exclusion(
                    excluded,
                    directory_identity.canonical.as_path().to_path_buf(),
                    ExclusionReason::OutsideApprovedRoot,
                );
            }
        }

        if depth >= MAX_TRAVERSAL_DEPTH {
            push_exclusion(
                excluded,
                directory_identity.canonical.as_path().to_path_buf(),
                ExclusionReason::UnsupportedPath,
            );
            continue;
        }
        for entry in entries.into_iter().rev() {
            match entry.kind {
                EntryKind::Directory
                    if entry.file_name != ".git" && entry.file_name != "node_modules" =>
                {
                    stack.push((entry.path, depth + 1));
                }
                EntryKind::Symlink => {
                    push_exclusion(excluded, entry.path, ExclusionReason::SymlinkNotFollowed)
                }
                _ => {}
            }
        }
    }
}

fn marker_present(entries: &[DirectoryEntry], name: &str) -> bool {
    entries
        .iter()
        .any(|entry| entry.file_name == name && entry.kind == EntryKind::File)
}

fn owning_root<'a>(
    roots: &'a [ValidatedRoot],
    workspace: &PathIdentity,
) -> Option<&'a ValidatedRoot> {
    roots
        .iter()
        .filter(|root| {
            workspace
                .canonical
                .as_path()
                .starts_with(root.identity.canonical.as_path())
        })
        .max_by(|a, b| {
            a.identity
                .canonical
                .as_path()
                .components()
                .count()
                .cmp(&b.identity.canonical.as_path().components().count())
                .then_with(|| b.identity.canonical.cmp(&a.identity.canonical))
        })
}

fn push_exclusion(excluded: &mut Vec<ExcludedPath>, path: PathBuf, reason: ExclusionReason) {
    if excluded.len() < MAX_DIAGNOSTICS {
        excluded.push(ExcludedPath { path, reason });
    }
}

fn io_reason(error: &io::Error) -> ExclusionReason {
    match error.kind() {
        io::ErrorKind::PermissionDenied => ExclusionReason::PermissionDenied,
        io::ErrorKind::NotFound => ExclusionReason::PathChangedDuringScan,
        io::ErrorKind::InvalidData | io::ErrorKind::Unsupported => ExclusionReason::UnsupportedPath,
        _ => ExclusionReason::PathChangedDuringScan,
    }
}

fn identity_reason(error: IdentityError) -> ExclusionReason {
    match error {
        IdentityError::Canonicalise { source, .. }
            if source.kind() == io::ErrorKind::PermissionDenied =>
        {
            ExclusionReason::PermissionDenied
        }
        IdentityError::Canonicalise { source, .. } if source.kind() == io::ErrorKind::NotFound => {
            ExclusionReason::PathChangedDuringScan
        }
        IdentityError::Canonicalise { .. } => ExclusionReason::CanonicalisationFailed,
        IdentityError::VolumeIdentityUnavailable { .. } => {
            ExclusionReason::VolumeIdentityUnavailable
        }
        IdentityError::NotAbsolute(_) | IdentityError::NotUnicode(_) => {
            ExclusionReason::UnsupportedPath
        }
        IdentityError::EmptyIdentityComponent(_) => ExclusionReason::VolumeIdentityUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::SystemActivityProbe;
    use crate::identity::{PathIdentity, VolumeId};
    use crate::state::{FixedClock, NewApprovedRoot, SqliteStateStore, StateStore};

    fn add_root(path: &Path) -> ApprovedRootRecord {
        let db_dir = tempfile::tempdir().unwrap();
        let mut store = SqliteStateStore::open_with_clock(
            &db_dir.path().join("state.sqlite3"),
            Box::new(FixedClock::new(1_000)),
        )
        .unwrap();
        store
            .add_approved_root(NewApprovedRoot {
                identity: PathIdentity::resolve(path).unwrap(),
            })
            .unwrap()
    }

    fn make_workspace(path: &Path, markers: &[&str]) {
        fs::create_dir_all(path).unwrap();
        for marker in markers {
            fs::write(path.join(marker), b"fixture").unwrap();
        }
    }

    #[test]
    fn valid_workspace_registers_and_partial_candidates_have_exact_reasons() {
        let root = tempfile::tempdir().unwrap();
        make_workspace(
            &root.path().join("valid"),
            &["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"],
        );
        make_workspace(
            &root.path().join("no-package"),
            &["pnpm-workspace.yaml", "pnpm-lock.yaml"],
        );
        make_workspace(
            &root.path().join("no-workspace"),
            &["package.json", "pnpm-lock.yaml"],
        );
        make_workspace(
            &root.path().join("no-lock"),
            &["package.json", "pnpm-workspace.yaml"],
        );
        fs::create_dir(root.path().join("ordinary")).unwrap();
        let record = add_root(root.path());

        let result = discover(&[record], &SystemDiscoveryProvider, &SystemActivityProbe);
        assert_eq!(result.observations.len(), 1);
        assert!(
            result.observations[0]
                .identity
                .canonical
                .as_path()
                .ends_with("valid")
        );
        let reasons: BTreeSet<_> = result
            .excluded
            .iter()
            .map(|excluded| excluded.reason)
            .collect();
        assert_eq!(
            reasons,
            BTreeSet::from([
                ExclusionReason::MissingPackageJson,
                ExclusionReason::MissingPnpmWorkspace,
                ExclusionReason::MissingPnpmLockfile,
            ])
        );
        assert!(
            result
                .excluded
                .iter()
                .all(|excluded| !excluded.path.ends_with("ordinary"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlinks_and_loops_are_not_followed_or_escaped() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        make_workspace(
            &outside.path().join("outside-workspace"),
            &["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"],
        );
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        std::os::unix::fs::symlink(root.path(), root.path().join("loop")).unwrap();

        let result = discover(
            &[add_root(root.path())],
            &SystemDiscoveryProvider,
            &SystemActivityProbe,
        );
        assert!(result.observations.is_empty());
        assert_eq!(result.excluded.len(), 2);
        assert!(
            result
                .excluded
                .iter()
                .all(|excluded| excluded.reason == ExclusionReason::SymlinkNotFollowed)
        );
    }

    #[test]
    fn nested_and_overlapping_roots_scan_once_and_choose_most_specific_owner() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        make_workspace(
            &nested.join("workspace"),
            &["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"],
        );
        let outer_record = add_root(root.path());
        let nested_record = add_root(&nested);
        let result = discover(
            &[outer_record, nested_record.clone()],
            &SystemDiscoveryProvider,
            &SystemActivityProbe,
        );
        assert_eq!(result.validated_roots.len(), 2);
        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].approved_root_id, nested_record.id);
    }

    struct VolumeOverrideProvider {
        mount: PathBuf,
    }

    impl DiscoveryProvider for VolumeOverrideProvider {
        fn entry_kind(&self, path: &Path) -> io::Result<EntryKind> {
            SystemDiscoveryProvider.entry_kind(path)
        }

        fn read_directory(&self, path: &Path) -> io::Result<Vec<DirectoryEntry>> {
            SystemDiscoveryProvider.read_directory(path)
        }

        fn resolve_identity(&self, path: &Path) -> Result<PathIdentity, IdentityError> {
            let identity = PathIdentity::resolve(path)?;
            if identity.canonical.as_path().starts_with(&self.mount) {
                Ok(PathIdentity::from_parts(
                    identity.canonical,
                    VolumeId::new("injected-mounted-volume").unwrap(),
                    identity.file,
                ))
            } else {
                Ok(identity)
            }
        }
    }

    #[test]
    fn child_volume_identity_is_retained_through_injected_provider() {
        let root = tempfile::tempdir().unwrap();
        let mount = root.path().join("mount");
        let workspace = mount.join("workspace");
        make_workspace(
            &workspace,
            &["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"],
        );
        let provider = VolumeOverrideProvider {
            mount: dunce::canonicalize(&mount).unwrap(),
        };
        let result = discover(&[add_root(root.path())], &provider, &SystemActivityProbe);
        assert_eq!(result.observations.len(), 1);
        assert_eq!(
            result.observations[0].identity.volume.as_str(),
            "injected-mounted-volume"
        );
    }

    struct FailingReadProvider {
        fail_path: PathBuf,
        kind: io::ErrorKind,
    }

    impl DiscoveryProvider for FailingReadProvider {
        fn entry_kind(&self, path: &Path) -> io::Result<EntryKind> {
            SystemDiscoveryProvider.entry_kind(path)
        }

        fn read_directory(&self, path: &Path) -> io::Result<Vec<DirectoryEntry>> {
            if path == self.fail_path {
                Err(io::Error::new(self.kind, "injected traversal failure"))
            } else {
                SystemDiscoveryProvider.read_directory(path)
            }
        }

        fn resolve_identity(&self, path: &Path) -> Result<PathIdentity, IdentityError> {
            SystemDiscoveryProvider.resolve_identity(path)
        }
    }

    #[test]
    fn permission_and_disappearing_path_errors_fail_safely() {
        for (kind, expected) in [
            (
                io::ErrorKind::PermissionDenied,
                ExclusionReason::PermissionDenied,
            ),
            (
                io::ErrorKind::NotFound,
                ExclusionReason::PathChangedDuringScan,
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let child = root.path().join("child");
            fs::create_dir(&child).unwrap();
            let provider = FailingReadProvider {
                fail_path: dunce::canonicalize(&child).unwrap(),
                kind,
            };
            let result = discover(&[add_root(root.path())], &provider, &SystemActivityProbe);
            assert!(
                result
                    .excluded
                    .iter()
                    .any(|excluded| excluded.reason == expected)
            );
        }
    }

    #[test]
    fn ordering_spaces_unicode_and_repeated_discovery_are_deterministic() {
        let root = tempfile::tempdir().unwrap();
        for name in ["z workspace", "a Ünïcödé 日本語"] {
            make_workspace(
                &root.path().join(name),
                &["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"],
            );
        }
        let record = add_root(root.path());
        let first = discover(
            std::slice::from_ref(&record),
            &SystemDiscoveryProvider,
            &SystemActivityProbe,
        );
        let second = discover(
            std::slice::from_ref(&record),
            &SystemDiscoveryProvider,
            &SystemActivityProbe,
        );
        let first_paths: Vec<_> = first
            .observations
            .iter()
            .map(|observation| observation.identity.canonical.clone())
            .collect();
        let second_paths: Vec<_> = second
            .observations
            .iter()
            .map(|observation| observation.identity.canonical.clone())
            .collect();
        assert_eq!(first_paths, second_paths);
        assert!(first_paths[0].as_str().contains("Ünïcödé"));
    }
}
