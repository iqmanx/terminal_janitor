//! Location protection: the G4 gate from `SAFETY.md`.
//!
//! Protection answers one question — may this workspace be touched at all,
//! ignoring how much space it might return. It is deliberately conservative:
//! a location that merely looks like a synchronised or personal root is
//! refused just as firmly as one that proves it, because `SAFETY.md` section 2
//! makes every ambiguous check a skip.
//!
//! Nothing here reads a file's contents. Detection uses the canonical path and,
//! within the approved root only, the presence of a marker directory.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

/// Why a location may not be operated on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtectionReason {
    /// The owner protected this exact registered workspace.
    ExplicitlyProtected,
    /// A recognised cloud-synchronisation provider owns this location.
    CloudSynchronised {
        provider: &'static str,
        evidence: String,
    },
    /// Something indicates synchronisation but the evidence is not conclusive.
    /// Ambiguity is refused, not resolved (`ACCEPTANCE.md` section F).
    CloudSyncAmbiguous { evidence: String },
    /// A personal, system, or generic-cache root from the permanent automatic
    /// denylist (`SAFETY.md` section 6).
    DeniedRoot { kind: &'static str, root: PathBuf },
}

impl fmt::Display for ProtectionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExplicitlyProtected => {
                write!(f, "the workspace is explicitly protected by the owner")
            }
            Self::CloudSynchronised { provider, evidence } => write!(
                f,
                "the workspace is inside a {provider} synchronised location ({evidence})"
            ),
            Self::CloudSyncAmbiguous { evidence } => write!(
                f,
                "cloud synchronisation could not be ruled out ({evidence})"
            ),
            Self::DeniedRoot { kind, root } => write!(
                f,
                "the workspace is inside a protected {kind} location ({})",
                root.display()
            ),
        }
    }
}

/// A recognised synchronisation provider and the signals that identify it.
struct CloudProvider {
    name: &'static str,
    /// Directory names that, on their own, only raise suspicion.
    directory_names: &'static [&'static str],
    /// Marker entries that confirm the provider when found beside a suspicious
    /// directory name.
    markers: &'static [&'static str],
}

const CLOUD_PROVIDERS: &[CloudProvider] = &[
    CloudProvider {
        name: "Dropbox",
        directory_names: &["dropbox"],
        markers: &[".dropbox", ".dropbox.cache"],
    },
    CloudProvider {
        name: "OneDrive",
        directory_names: &["onedrive"],
        markers: &[".849c9593-d756-4e56-8d6e-42412f2a707b"],
    },
    CloudProvider {
        name: "iCloud Drive",
        directory_names: &["com~apple~clouddocs", "mobile documents"],
        markers: &[],
    },
    CloudProvider {
        name: "Google Drive",
        directory_names: &["my drive", "googledrive", "google drive"],
        markers: &[".tmp.driveupload"],
    },
];

/// Directory-name fragments that indicate synchronisation without naming a
/// provider. They are enough to refuse, never enough to identify.
const AMBIGUOUS_SYNC_FRAGMENTS: &[&str] =
    &["icloud", "nextcloud", "syncthing", "pcloud", "box sync"];

/// Filesystem questions protection needs. Injecting them keeps tests off the
/// real home directory and lets a test model an unreadable path.
pub trait ProtectionProbe {
    /// True when `path` exists as a directory entry of any kind.
    fn entry_exists(&self, path: &Path) -> bool;
}

#[derive(Debug, Default)]
pub struct SystemProtectionProbe;

impl ProtectionProbe for SystemProtectionProbe {
    fn entry_exists(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }
}

/// Canonical roots that automatic authority may never enter.
#[derive(Debug, Clone, Default)]
pub struct DeniedRoots {
    roots: Vec<(PathBuf, &'static str)>,
}

impl DeniedRoots {
    /// Builds the denylist from the platform's own user directories, so the
    /// list is correct for localised and relocated folders instead of being
    /// guessed from English names.
    pub fn from_user_directories() -> Self {
        let mut roots = Vec::new();
        if let Some(base) = directories::BaseDirs::new() {
            push_root(&mut roots, base.cache_dir(), "cache");
            push_root(&mut roots, base.config_dir(), "application configuration");
            push_root(&mut roots, base.data_dir(), "application state");
            push_root(&mut roots, base.data_local_dir(), "application state");
            push_root(&mut roots, base.preference_dir(), "application state");
        }
        if let Some(user) = directories::UserDirs::new() {
            push_root(&mut roots, user.home_dir().join(".ssh"), "credential");
            push_root(&mut roots, user.home_dir().join(".gnupg"), "credential");
            for (directory, kind) in [
                (user.document_dir(), "personal documents"),
                (user.download_dir(), "downloads"),
                (user.desktop_dir(), "desktop"),
                (user.picture_dir(), "personal media"),
                (user.video_dir(), "personal media"),
                (user.audio_dir(), "personal media"),
            ] {
                if let Some(directory) = directory {
                    push_root(&mut roots, directory, kind);
                }
            }
        }
        roots.sort();
        roots.dedup();
        Self { roots }
    }

    /// Builds an explicit denylist. Tests use this; production uses
    /// [`DeniedRoots::from_user_directories`].
    pub fn from_paths(roots: Vec<(PathBuf, &'static str)>) -> Self {
        let mut accepted = Vec::with_capacity(roots.len());
        for (path, kind) in roots {
            push_root(&mut accepted, path, kind);
        }
        accepted.sort();
        accepted.dedup();
        Self { roots: accepted }
    }

    /// The denied root containing `path`, if any. A workspace that *is* a
    /// denied root is refused as well as one nested inside it.
    pub fn covering(&self, path: &Path) -> Option<(&Path, &'static str)> {
        self.roots
            .iter()
            .find(|(root, _)| path.starts_with(root))
            .map(|(root, kind)| (root.as_path(), *kind))
    }
}

fn push_root(
    roots: &mut Vec<(PathBuf, &'static str)>,
    path: impl Into<PathBuf>,
    kind: &'static str,
) {
    let path = path.into();
    // A relative or empty directory cannot be compared safely, and the
    // filesystem root would deny everything; both are dropped rather than
    // stored as a bogus boundary.
    if path.is_absolute() && path.parent().is_some() {
        roots.push((path, kind));
    }
}

/// Evaluates the G4 protection gate for one workspace.
///
/// `workspace` and `approved_root` must both be canonical. Marker probing is
/// confined to the approved root and below: `terminal_janitor` never inspects
/// a directory the owner did not approve, so a provider marker that lives
/// above the approved root is judged by path name alone.
pub fn evaluate(
    workspace: &Path,
    approved_root: &Path,
    explicitly_protected: bool,
    denied: &DeniedRoots,
    probe: &dyn ProtectionProbe,
) -> Option<ProtectionReason> {
    if explicitly_protected {
        return Some(ProtectionReason::ExplicitlyProtected);
    }
    if let Some((root, kind)) = denied.covering(workspace) {
        return Some(ProtectionReason::DeniedRoot {
            kind,
            root: root.to_path_buf(),
        });
    }
    cloud_sync_reason(workspace, approved_root, probe)
}

fn cloud_sync_reason(
    workspace: &Path,
    approved_root: &Path,
    probe: &dyn ProtectionProbe,
) -> Option<ProtectionReason> {
    for (ancestor, name) in named_ancestors(workspace) {
        let lowered = name.to_ascii_lowercase();

        for provider in CLOUD_PROVIDERS {
            if !provider
                .directory_names
                .iter()
                .any(|candidate| lowered == *candidate)
            {
                continue;
            }
            // A marker confirms the provider, but only where the owner has
            // approved inspection. Everywhere else the name alone is enough to
            // refuse; it is simply reported as unconfirmed.
            if ancestor.starts_with(approved_root) {
                for marker in provider.markers {
                    if probe.entry_exists(&ancestor.join(marker)) {
                        return Some(ProtectionReason::CloudSynchronised {
                            provider: provider.name,
                            evidence: format!("{}/{marker}", ancestor.display()),
                        });
                    }
                }
            }
            return Some(ProtectionReason::CloudSynchronised {
                provider: provider.name,
                evidence: format!("directory named {name}"),
            });
        }

        if let Some(fragment) = AMBIGUOUS_SYNC_FRAGMENTS
            .iter()
            .find(|fragment| lowered.contains(**fragment))
        {
            return Some(ProtectionReason::CloudSyncAmbiguous {
                evidence: format!("directory name {name} contains {fragment}"),
            });
        }
    }
    None
}

/// Every ancestor of `path`, deepest first, paired with its own final
/// component name. The filesystem root has no name and is skipped.
fn named_ancestors(path: &Path) -> Vec<(PathBuf, String)> {
    let mut result = Vec::new();
    let mut current = Some(path);
    while let Some(ancestor) = current {
        if let Some(Component::Normal(name)) = ancestor.components().next_back()
            && let Some(name) = name.to_str()
        {
            result.push((ancestor.to_path_buf(), name.to_owned()));
        }
        current = ancestor.parent();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Test fixtures are written in one POSIX spelling and mapped to a single
    /// absolute platform path, because Windows does not treat a rootless path
    /// as absolute and a denied root must be absolute to be a boundary.
    fn abs(path: &str) -> PathBuf {
        #[cfg(windows)]
        {
            if path == "/" {
                return PathBuf::from("C:\\");
            }
            if !path.starts_with('/') {
                return PathBuf::from(path);
            }
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        }
        #[cfg(not(windows))]
        {
            PathBuf::from(path)
        }
    }

    #[derive(Default)]
    struct FakeProbe {
        entries: BTreeSet<PathBuf>,
    }

    impl FakeProbe {
        fn with(entries: &[&str]) -> Self {
            Self {
                entries: entries.iter().map(|entry| abs(entry)).collect(),
            }
        }
    }

    impl ProtectionProbe for FakeProbe {
        fn entry_exists(&self, path: &Path) -> bool {
            self.entries.contains(path)
        }
    }

    fn evaluate_path(
        workspace: &str,
        root: &str,
        probe: &dyn ProtectionProbe,
    ) -> Option<ProtectionReason> {
        evaluate(
            &abs(workspace),
            &abs(root),
            false,
            &DeniedRoots::default(),
            probe,
        )
    }

    #[test]
    fn an_ordinary_project_under_an_approved_root_is_not_protected() {
        assert_eq!(
            evaluate_path("/approved/projects/api", "/approved", &FakeProbe::default()),
            None
        );
    }

    #[test]
    fn explicit_protection_wins_before_any_other_check() {
        let reason = evaluate(
            &abs("/approved/api"),
            &abs("/approved"),
            true,
            &DeniedRoots::default(),
            &FakeProbe::default(),
        );
        assert_eq!(reason, Some(ProtectionReason::ExplicitlyProtected));
    }

    #[test]
    fn a_confirmed_provider_marker_names_the_provider() {
        let probe = FakeProbe::with(&["/approved/Dropbox/.dropbox"]);
        let reason = evaluate_path("/approved/Dropbox/work/api", "/approved", &probe);
        assert!(matches!(
            reason,
            Some(ProtectionReason::CloudSynchronised {
                provider: "Dropbox",
                ..
            })
        ));
    }

    #[test]
    fn a_provider_directory_name_alone_still_refuses() {
        let reason = evaluate_path("/approved/OneDrive/api", "/approved", &FakeProbe::default());
        assert!(matches!(
            reason,
            Some(ProtectionReason::CloudSynchronised {
                provider: "OneDrive",
                ..
            })
        ));
    }

    #[test]
    fn markers_above_the_approved_root_are_never_probed() {
        let probe = FakeProbe::with(&["/home/person/Dropbox/.dropbox"]);
        // The name still refuses the workspace; the point is that the refusal
        // does not depend on inspecting an unapproved directory.
        let reason = evaluate_path(
            "/home/person/Dropbox/approved/api",
            "/home/person/Dropbox/approved",
            &probe,
        );
        assert!(matches!(
            reason,
            Some(ProtectionReason::CloudSynchronised { .. })
        ));
    }

    #[test]
    fn an_unidentifiable_sync_name_fails_closed_as_ambiguous() {
        let reason = evaluate_path(
            "/approved/Nextcloud-work/api",
            "/approved",
            &FakeProbe::default(),
        );
        assert!(matches!(
            reason,
            Some(ProtectionReason::CloudSyncAmbiguous { .. })
        ));
    }

    #[test]
    fn a_denied_root_refuses_the_workspace_and_names_the_root() {
        let denied = DeniedRoots::from_paths(vec![(abs("/home/person/Downloads"), "downloads")]);
        let reason = evaluate(
            &abs("/home/person/Downloads/api"),
            &abs("/home/person"),
            false,
            &denied,
            &FakeProbe::default(),
        );
        assert!(matches!(
            reason,
            Some(ProtectionReason::DeniedRoot {
                kind: "downloads",
                ..
            })
        ));
    }

    #[test]
    fn a_workspace_that_is_itself_a_denied_root_is_refused() {
        let denied = DeniedRoots::from_paths(vec![(abs("/home/person/.cache"), "cache")]);
        assert!(
            evaluate(
                &abs("/home/person/.cache"),
                &abs("/home/person"),
                false,
                &denied,
                &FakeProbe::default(),
            )
            .is_some()
        );
    }

    #[test]
    fn a_sibling_of_a_denied_root_is_not_refused_by_prefix_text() {
        let denied = DeniedRoots::from_paths(vec![(abs("/home/person/Downloads"), "downloads")]);
        assert_eq!(
            evaluate(
                &abs("/home/person/Downloads-archive/api"),
                &abs("/home/person"),
                false,
                &denied,
                &FakeProbe::default(),
            ),
            None
        );
    }

    #[test]
    fn the_filesystem_root_is_never_stored_as_a_denied_boundary() {
        let denied =
            DeniedRoots::from_paths(vec![(abs("/"), "root"), (abs("relative"), "relative")]);
        assert_eq!(denied.covering(&abs("/approved/api")), None);
    }
}
