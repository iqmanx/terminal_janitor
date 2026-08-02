use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const DEFAULT_MINIMUM_FREE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
pub const DEFAULT_TARGET_FREE_BYTES: u64 = 15 * 1024 * 1024 * 1024;
pub const CONFIG_FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub minimum_free_bytes: u64,
    pub target_free_bytes: u64,
    /// Explicitly user-approved project roots as canonical absolute paths.
    /// Day 1 configuration files without this field load as an empty list.
    pub approved_roots: Vec<PathBuf>,
}

impl Config {
    pub fn new(minimum_free_bytes: u64, target_free_bytes: u64) -> Result<Self, ConfigError> {
        if minimum_free_bytes == 0 {
            return Err(ConfigError::InvalidThreshold(
                "minimum_free must be greater than zero".to_owned(),
            ));
        }
        if target_free_bytes <= minimum_free_bytes {
            return Err(ConfigError::InvalidThreshold(
                "target_free must be greater than minimum_free".to_owned(),
            ));
        }
        Ok(Self {
            minimum_free_bytes,
            target_free_bytes,
            approved_roots: Vec::new(),
        })
    }

    /// Replaces the approved roots with a validated, deterministically
    /// ordered, duplicate-free list. Every root must be an absolute path
    /// representable in UTF-8; canonicalisation against the live filesystem
    /// is the registration workflow's responsibility, not the config model's.
    pub fn set_approved_roots(&mut self, roots: Vec<PathBuf>) -> Result<(), ConfigError> {
        self.approved_roots = normalize_approved_roots(roots)?;
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            minimum_free_bytes: DEFAULT_MINIMUM_FREE_BYTES,
            target_free_bytes: DEFAULT_TARGET_FREE_BYTES,
            approved_roots: Vec::new(),
        }
    }
}

/// Sorts, deduplicates, and validates approved roots so identical inputs
/// always produce one identical stored list.
fn normalize_approved_roots(roots: Vec<PathBuf>) -> Result<Vec<PathBuf>, ConfigError> {
    let mut normalized = Vec::with_capacity(roots.len());
    for root in roots {
        if !root.is_absolute() {
            return Err(ConfigError::InvalidRoot {
                value: root.display().to_string(),
                message: "approved roots must be absolute paths".to_owned(),
            });
        }
        if root.to_str().is_none() {
            return Err(ConfigError::InvalidRoot {
                value: root.display().to_string(),
                message: "approved roots must be representable in UTF-8".to_owned(),
            });
        }
        normalized.push(root);
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Defaults,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfig {
    pub config: Config,
    pub source: ConfigSource,
}

#[derive(Debug)]
pub enum ConfigError {
    DirectoryUnavailable,
    Read {
        path: PathBuf,
        source: io::Error,
    },
    InvalidToml {
        path: PathBuf,
        message: String,
    },
    InvalidSize {
        field: &'static str,
        value: String,
    },
    InvalidThreshold(String),
    InvalidRoot {
        value: String,
        message: String,
    },
    /// The atomic replacement of the configuration file failed. The previous
    /// valid file, if any, is preserved.
    Write {
        path: PathBuf,
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectoryUnavailable => write!(
                f,
                "the platform user configuration directory is unavailable"
            ),
            Self::Read { path, source } => {
                write!(
                    f,
                    "could not read configuration {}: {source}",
                    path.display()
                )
            }
            Self::InvalidToml { path, message } => write!(
                f,
                "configuration {} is not valid TOML: {message}",
                path.display()
            ),
            Self::InvalidSize { field, value } => write!(
                f,
                "configuration field {field} has invalid size {value:?}; expected unsigned whole bytes followed by one of B, KiB, MiB, GiB, TiB"
            ),
            Self::InvalidThreshold(message) => write!(f, "invalid thresholds: {message}"),
            Self::InvalidRoot { value, message } => {
                write!(f, "invalid approved root {value:?}: {message}")
            }
            Self::Write { path, message } => write!(
                f,
                "could not atomically write configuration {}: {message}; \
                 the previous configuration file is preserved",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    minimum_free: String,
    target_free: String,
    /// Absent in Day 1 configuration files; defaults to no approved roots.
    #[serde(default)]
    approved_roots: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct FileConfigOut<'a> {
    minimum_free: String,
    target_free: String,
    approved_roots: Vec<&'a str>,
}

pub fn config_file_path() -> Result<PathBuf, ConfigError> {
    ProjectDirs::from("", "", "terminal_janitor")
        .map(|dirs| dirs.config_dir().join(CONFIG_FILE_NAME))
        .ok_or(ConfigError::DirectoryUnavailable)
}

pub fn load_config() -> Result<LoadedConfig, ConfigError> {
    load_config_at(&config_file_path()?)
}

pub fn load_config_at(path: &Path) -> Result<LoadedConfig, ConfigError> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(LoadedConfig {
                config: Config::default(),
                source: ConfigSource::Defaults,
            });
        }
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let raw: FileConfig = toml::from_str(&contents).map_err(|error| ConfigError::InvalidToml {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let minimum_free_bytes = parse_size("minimum_free", &raw.minimum_free)?;
    let target_free_bytes = parse_size("target_free", &raw.target_free)?;
    let mut config = Config::new(minimum_free_bytes, target_free_bytes)?;
    config.set_approved_roots(raw.approved_roots)?;

    Ok(LoadedConfig {
        config,
        source: ConfigSource::File,
    })
}

/// Atomically writes the complete validated configuration to `path`.
///
/// The serialised file is written to a temporary file in the destination
/// directory, flushed and synced, and then atomically moved over the
/// destination. On any failure the previous valid file is preserved and the
/// temporary file is removed; the destination is never left partially
/// written. Validation happens before any filesystem access, so an invalid
/// configuration cannot cause a partial update.
pub fn save_config_at(path: &Path, config: &Config) -> Result<(), ConfigError> {
    // Re-validate the invariants: the fields are public and could have been
    // mutated since construction.
    Config::new(config.minimum_free_bytes, config.target_free_bytes)?;
    let roots = normalize_approved_roots(config.approved_roots.clone())?;
    let contents = toml::to_string(&FileConfigOut {
        minimum_free: format_size(config.minimum_free_bytes),
        target_free: format_size(config.target_free_bytes),
        approved_roots: roots
            .iter()
            .map(|root| root.to_str().expect("validated UTF-8 above"))
            .collect(),
    })
    .map_err(|error| ConfigError::Write {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;

    let parent = path.parent().ok_or_else(|| ConfigError::Write {
        path: path.to_path_buf(),
        message: "configuration path has no parent directory".to_owned(),
    })?;
    create_config_dir(parent).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        message: format!("could not create {}: {source}", parent.display()),
    })?;

    let write_error = |message: String| ConfigError::Write {
        path: path.to_path_buf(),
        message,
    };
    let mut temp = tempfile::Builder::new()
        .prefix(".config-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|error| write_error(format!("could not create temporary file: {error}")))?;
    use std::io::Write as _;
    temp.write_all(contents.as_bytes())
        .and_then(|()| temp.flush())
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|error| write_error(format!("could not write temporary file: {error}")))?;
    // `persist` replaces the destination atomically (rename on Unix,
    // MoveFileExW with MOVEFILE_REPLACE_EXISTING on Windows). On failure the
    // temporary file is cleaned up and the destination is untouched.
    temp.persist(path)
        .map_err(|error| write_error(format!("could not replace destination: {}", error.error)))?;

    // Best-effort durability of the rename itself; atomicity and the
    // completeness of whichever file survives are already guaranteed.
    #[cfg(unix)]
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Creates the configuration directory (user-only where supported).
fn create_config_dir(parent: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(parent)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(parent)
    }
}

/// Renders a byte count in the strict configuration syntax, using the
/// largest binary unit that divides it exactly so parsing round-trips.
fn format_size(bytes: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1024_u64.pow(4), "TiB"),
        (1024_u64.pow(3), "GiB"),
        (1024_u64.pow(2), "MiB"),
        (1024, "KiB"),
    ];
    for (multiplier, unit) in UNITS {
        if bytes != 0 && bytes.is_multiple_of(multiplier) {
            return format!("{}{unit}", bytes / multiplier);
        }
    }
    format!("{bytes}B")
}

pub fn parse_size(field: &'static str, input: &str) -> Result<u64, ConfigError> {
    let digit_count = input.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 || digit_count == input.len() {
        return Err(invalid_size(field, input));
    }

    let (number, unit) = input.split_at(digit_count);
    let multiplier = match unit {
        "B" => 1,
        "KiB" => 1024,
        "MiB" => 1024_u64.pow(2),
        "GiB" => 1024_u64.pow(3),
        "TiB" => 1024_u64.pow(4),
        _ => return Err(invalid_size(field, input)),
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|value| value.checked_mul(multiplier))
        .ok_or_else(|| invalid_size(field, input))
}

fn invalid_size(field: &'static str, input: &str) -> ConfigError {
    ConfigError::InvalidSize {
        field,
        value: input.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(dir: &tempfile::TempDir, contents: &str) -> PathBuf {
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn parses_supported_sizes_and_boundaries() {
        assert_eq!(parse_size("test", "10GiB").unwrap(), 10 * 1024_u64.pow(3));
        assert_eq!(parse_size("test", "1B").unwrap(), 1);
        assert_eq!(
            parse_size("test", "18446744073709551615B").unwrap(),
            u64::MAX
        );
        assert_eq!(parse_size("test", "1KiB").unwrap(), 1024);
        assert_eq!(parse_size("test", "1MiB").unwrap(), 1024_u64.pow(2));
        assert_eq!(parse_size("test", "1TiB").unwrap(), 1024_u64.pow(4));
    }

    #[test]
    fn rejects_unknown_case_decimal_negative_malformed_and_overflow() {
        for value in [
            "10GB",
            "10gib",
            "1.5GiB",
            "-1GiB",
            "GiB",
            "10",
            " 10GiB",
            "10GiB ",
            "18446744073709551615KiB",
            "18446744073709551616B",
        ] {
            assert!(matches!(
                parse_size("test", value),
                Err(ConfigError::InvalidSize { .. })
            ));
        }
    }

    #[test]
    fn validates_threshold_boundaries() {
        assert_eq!(Config::new(1, 2).unwrap().minimum_free_bytes, 1);
        assert!(matches!(
            Config::new(0, 1),
            Err(ConfigError::InvalidThreshold(_))
        ));
        assert!(matches!(
            Config::new(10, 10),
            Err(ConfigError::InvalidThreshold(_))
        ));
        assert!(matches!(
            Config::new(10, 9),
            Err(ConfigError::InvalidThreshold(_))
        ));
    }

    #[test]
    fn missing_config_uses_labelled_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let loaded = load_config_at(&path).unwrap();
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.source, ConfigSource::Defaults);
        assert!(!path.exists(), "loading defaults must not create config");
    }

    #[test]
    fn valid_config_uses_file_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, "minimum_free = \"10GiB\"\ntarget_free = \"15GiB\"\n");
        let loaded = load_config_at(&path).unwrap();
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.source, ConfigSource::File);
    }

    #[test]
    fn valid_file_accepts_full_u64_byte_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(
            &dir,
            "minimum_free = \"1B\"\ntarget_free = \"18446744073709551615B\"\n",
        );
        let loaded = load_config_at(&path).unwrap();
        assert_eq!(loaded.config.minimum_free_bytes, 1);
        assert_eq!(loaded.config.target_free_bytes, u64::MAX);
    }

    #[test]
    fn corrupt_or_invalid_existing_config_never_falls_back() {
        for contents in [
            "not toml at all [",
            "minimum_free = \"0B\"\ntarget_free = \"15GiB\"\n",
            "minimum_free = \"10GiB\"\ntarget_free = \"10GiB\"\n",
            "minimum_free = \"10GiB\"\ntarget_free = \"9GiB\"\n",
            "minimum_free = \"10GiB\"\ntarget_free = \"15GiB\"\nfuture = true\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = write_config(&dir, contents);
            assert!(load_config_at(&path).is_err());
        }
    }

    fn absolute(path: &str) -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        }
        #[cfg(not(windows))]
        {
            PathBuf::from(path)
        }
    }

    #[test]
    fn day1_config_without_approved_roots_stays_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(&dir, "minimum_free = \"10GiB\"\ntarget_free = \"15GiB\"\n");
        let loaded = load_config_at(&path).unwrap();
        assert_eq!(loaded.config.approved_roots, Vec::<PathBuf>::new());
        assert_eq!(loaded.source, ConfigSource::File);
    }

    #[test]
    fn format_size_round_trips_through_the_parser() {
        for bytes in [
            1,
            1023,
            1024,
            10 * 1024_u64.pow(3),
            1024_u64.pow(4),
            3 * 1024_u64.pow(2) + 1,
            u64::MAX,
        ] {
            let rendered = format_size(bytes);
            assert_eq!(parse_size("test", &rendered).unwrap(), bytes, "{rendered}");
        }
    }

    #[test]
    fn saved_config_is_read_back_by_the_loader() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let mut config = Config::new(1024, 10 * 1024_u64.pow(3)).unwrap();
        config
            .set_approved_roots(vec![
                absolute("/projects/space dir Ünïcödé 日本語"),
                absolute("/projects/alpha"),
            ])
            .unwrap();

        save_config_at(&path, &config).unwrap();
        let loaded = load_config_at(&path).unwrap();
        assert_eq!(loaded.config, config);
        assert_eq!(loaded.source, ConfigSource::File);
        // Deterministic ordering: sorted, not insertion order.
        assert_eq!(loaded.config.approved_roots[0], absolute("/projects/alpha"));
    }

    #[test]
    fn approved_root_duplicates_collapse_deterministically() {
        let mut config = Config::new(1, 2).unwrap();
        config
            .set_approved_roots(vec![
                absolute("/projects/beta"),
                absolute("/projects/alpha"),
                absolute("/projects/beta"),
            ])
            .unwrap();
        assert_eq!(
            config.approved_roots,
            vec![absolute("/projects/alpha"), absolute("/projects/beta")]
        );
    }

    #[test]
    fn relative_approved_roots_are_rejected() {
        let mut config = Config::new(1, 2).unwrap();
        assert!(matches!(
            config.set_approved_roots(vec![PathBuf::from("relative/root")]),
            Err(ConfigError::InvalidRoot { .. })
        ));

        let dir = tempfile::tempdir().unwrap();
        let path = write_config(
            &dir,
            "minimum_free = \"1B\"\ntarget_free = \"2B\"\napproved_roots = [\"relative/root\"]\n",
        );
        assert!(matches!(
            load_config_at(&path),
            Err(ConfigError::InvalidRoot { .. })
        ));
    }

    #[test]
    fn invalid_new_config_leaves_previous_file_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let valid = Config::new(1024, 2048).unwrap();
        save_config_at(&path, &valid).unwrap();
        let before = fs::read_to_string(&path).unwrap();

        let mut broken = valid.clone();
        broken.target_free_bytes = broken.minimum_free_bytes;
        assert!(save_config_at(&path, &broken).is_err());
        let mut bad_root = valid.clone();
        bad_root.approved_roots = vec![PathBuf::from("relative")];
        assert!(save_config_at(&path, &bad_root).is_err());

        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(load_config_at(&path).unwrap().config, valid);
    }

    #[cfg(unix)]
    #[test]
    fn failed_temporary_write_leaves_previous_config_intact() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let valid = Config::new(1024, 2048).unwrap();
        save_config_at(&path, &valid).unwrap();
        let before = fs::read_to_string(&path).unwrap();

        // A read-only directory makes the same-directory temporary file
        // creation fail before the destination could be touched.
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let replacement = Config::new(4096, 8192).unwrap();
        let result = save_config_at(&path, &replacement);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        if result.is_ok() {
            // Running as root (some CI sandboxes) bypasses the permission
            // denial; the atomicity claim cannot be exercised here.
            return;
        }

        assert!(matches!(result, Err(ConfigError::Write { .. })));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(load_config_at(&path).unwrap().config, valid);
    }

    #[test]
    fn successful_replacement_leaves_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        save_config_at(&path, &Config::new(1024, 2048).unwrap()).unwrap();
        save_config_at(&path, &Config::new(2048, 4096).unwrap()).unwrap();

        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from(CONFIG_FILE_NAME)]);
    }

    #[test]
    fn save_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("nested")
            .join("deeper")
            .join(CONFIG_FILE_NAME);
        save_config_at(&path, &Config::default()).unwrap();
        assert_eq!(load_config_at(&path).unwrap().config, Config::default());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700, "config directories are user-only");
        }
    }

    #[test]
    fn loads_paths_with_spaces_and_unicode() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("config space Ünïcödé 日本語");
        fs::create_dir(&dir).unwrap();
        let path = dir.join(CONFIG_FILE_NAME);
        fs::write(&path, "minimum_free = \"1B\"\ntarget_free = \"2B\"\n").unwrap();
        let loaded = load_config_at(&path).unwrap();
        assert_eq!(loaded.source, ConfigSource::File);
    }
}
