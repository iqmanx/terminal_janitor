//! Canonical filesystem identity types for the state ledger.
//!
//! Authority in `terminal_janitor` is bound to *where a thing physically is*,
//! not to a path string. Before any path is stored as an identity it must be
//! absolute, symlink-resolved, and paired with the native identity of the
//! containing volume and of the file itself. String equality alone never
//! proves filesystem identity.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::platform;

/// An absolute, canonicalised, UTF-8-representable filesystem path.
///
/// The inner value is stored as a `String` so ledger persistence is lossless
/// for every representable path; paths that cannot be represented in UTF-8
/// are rejected explicitly rather than converted lossily.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalPath(String);

impl CanonicalPath {
    /// Canonicalises `path` against the real filesystem.
    ///
    /// Relative input is rejected outright: a relative path's meaning depends
    /// on the process working directory and must never become an identity.
    /// On Windows the verbatim `\\?\` prefix is stripped when safe so one
    /// directory always canonicalises to one spelling.
    pub fn resolve(path: &Path) -> Result<Self, IdentityError> {
        if !path.is_absolute() {
            return Err(IdentityError::NotAbsolute(path.to_path_buf()));
        }
        let canonical =
            dunce::canonicalize(path).map_err(|source| IdentityError::Canonicalise {
                path: path.to_path_buf(),
                source,
            })?;
        Self::from_verified(canonical)
    }

    /// Accepts a path that is already canonical (for example one loaded back
    /// from the ledger). The absolute and UTF-8 invariants are re-checked;
    /// no filesystem access occurs.
    pub fn from_verified(path: PathBuf) -> Result<Self, IdentityError> {
        if !path.is_absolute() {
            return Err(IdentityError::NotAbsolute(path));
        }
        match path.into_os_string().into_string() {
            Ok(text) => Ok(Self(text)),
            Err(os) => Err(IdentityError::NotUnicode(PathBuf::from(os))),
        }
    }

    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CanonicalPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Opaque best-available identity of the volume containing a path
/// (`unix-dev:<st_dev>` on Linux/macOS, `winvol-serial:<hex>` on Windows).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VolumeId(String);

impl VolumeId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityError::EmptyIdentityComponent("volume identity"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VolumeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Opaque best-available identity of the file or directory itself
/// (`unix-ino:<st_ino>` on Linux/macOS, `winfile-index:<hex>` on Windows).
/// Used to distinguish a replaced directory from the directory originally
/// registered at the same path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(String);

impl FileId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityError::EmptyIdentityComponent("file identity"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The complete identity of one filesystem location: canonical path plus the
/// native volume and file identities captured together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathIdentity {
    pub canonical: CanonicalPath,
    pub volume: VolumeId,
    pub file: FileId,
}

impl PathIdentity {
    /// Resolves the full identity of `path` against the real filesystem.
    /// Any failure — nonexistent path, relative input, non-UTF-8 path, or
    /// unavailable volume identity — is an explicit error; identity is never
    /// guessed or substituted with a constant.
    pub fn resolve(path: &Path) -> Result<Self, IdentityError> {
        let canonical = CanonicalPath::resolve(path)?;
        let (volume, file) = platform::file_identity(canonical.as_path()).map_err(|source| {
            IdentityError::VolumeIdentityUnavailable {
                path: canonical.as_path().to_path_buf(),
                source,
            }
        })?;
        Ok(Self {
            canonical,
            volume: VolumeId::new(volume)?,
            file: FileId::new(file)?,
        })
    }

    /// Assembles an identity from already-verified parts (ledger loads and
    /// tests with mocked volumes). Performs no filesystem access.
    pub fn from_parts(canonical: CanonicalPath, volume: VolumeId, file: FileId) -> Self {
        Self {
            canonical,
            volume,
            file,
        }
    }
}

#[derive(Debug)]
pub enum IdentityError {
    /// Relative paths are never identities.
    NotAbsolute(PathBuf),
    /// The path cannot be represented losslessly in UTF-8 for persistence.
    NotUnicode(PathBuf),
    Canonicalise {
        path: PathBuf,
        source: io::Error,
    },
    VolumeIdentityUnavailable {
        path: PathBuf,
        source: io::Error,
    },
    EmptyIdentityComponent(&'static str),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAbsolute(path) => {
                write!(
                    f,
                    "path {} is not absolute and cannot become an identity",
                    path.display()
                )
            }
            Self::NotUnicode(path) => write!(
                f,
                "path {} cannot be represented losslessly and is rejected",
                path.display()
            ),
            Self::Canonicalise { path, source } => {
                write!(f, "could not canonicalise {}: {source}", path.display())
            }
            Self::VolumeIdentityUnavailable { path, source } => write!(
                f,
                "volume identity is unavailable for {}: {source}",
                path.display()
            ),
            Self::EmptyIdentityComponent(component) => {
                write!(f, "{component} must not be empty")
            }
        }
    }
}

impl std::error::Error for IdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Canonicalise { source, .. } | Self::VolumeIdentityUnavailable { source, .. } => {
                Some(source)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_rejected() {
        for path in ["relative/project", ".", "../parent"] {
            assert!(matches!(
                CanonicalPath::resolve(Path::new(path)),
                Err(IdentityError::NotAbsolute(_))
            ));
            assert!(matches!(
                PathIdentity::resolve(Path::new(path)),
                Err(IdentityError::NotAbsolute(_))
            ));
        }
    }

    #[test]
    fn from_verified_rejects_relative_paths() {
        assert!(matches!(
            CanonicalPath::from_verified(PathBuf::from("relative")),
            Err(IdentityError::NotAbsolute(_))
        ));
    }

    #[test]
    fn nonexistent_path_is_an_explicit_error() {
        let missing = std::env::temp_dir().join("terminal_janitor_identity_missing_xyz");
        assert!(matches!(
            PathIdentity::resolve(&missing),
            Err(IdentityError::Canonicalise { .. })
        ));
    }

    #[test]
    fn resolve_produces_stable_identity_for_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first = PathIdentity::resolve(dir.path()).unwrap();
        let second = PathIdentity::resolve(dir.path()).unwrap();
        assert_eq!(first, second);
        assert!(first.canonical.as_path().is_absolute());
        assert!(!first.volume.as_str().is_empty());
        assert!(!first.file.as_str().is_empty());
    }

    #[test]
    fn symlink_alias_resolves_to_one_canonical_identity() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("real");
        std::fs::create_dir(&target).unwrap();

        #[cfg(unix)]
        {
            let link = root.path().join("alias");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let direct = PathIdentity::resolve(&target).unwrap();
            let via_link = PathIdentity::resolve(&link).unwrap();
            assert_eq!(direct, via_link);
        }
    }

    #[test]
    fn paths_with_spaces_and_unicode_resolve_losslessly() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("space dir Ünïcödé 日本語");
        if std::fs::create_dir(&dir).is_err() {
            // Some filesystems reject non-ASCII names; that is a host
            // limitation, not an identity defect.
            return;
        }
        let identity = PathIdentity::resolve(&dir).unwrap();
        assert!(identity.canonical.as_str().contains("space dir"));
        assert!(identity.canonical.as_str().contains("日本語"));
    }

    #[test]
    fn distinct_directories_have_distinct_file_identity() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let ia = PathIdentity::resolve(&a).unwrap();
        let ib = PathIdentity::resolve(&b).unwrap();
        assert_eq!(ia.volume, ib.volume, "same temp volume expected");
        assert_ne!(ia.file, ib.file);
    }

    #[test]
    fn empty_identity_components_are_rejected() {
        assert!(VolumeId::new("").is_err());
        assert!(FileId::new("").is_err());
        assert!(VolumeId::new("unix-dev:1").is_ok());
    }
}
