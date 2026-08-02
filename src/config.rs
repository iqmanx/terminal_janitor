use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::Deserialize;

pub const DEFAULT_MINIMUM_FREE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
pub const DEFAULT_TARGET_FREE_BYTES: u64 = 15 * 1024 * 1024 * 1024;
pub const CONFIG_FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub minimum_free_bytes: u64,
    pub target_free_bytes: u64,
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
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            minimum_free_bytes: DEFAULT_MINIMUM_FREE_BYTES,
            target_free_bytes: DEFAULT_TARGET_FREE_BYTES,
        }
    }
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
    Read { path: PathBuf, source: io::Error },
    InvalidToml { path: PathBuf, message: String },
    InvalidSize { field: &'static str, value: String },
    InvalidThreshold(String),
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
    let config = Config::new(minimum_free_bytes, target_free_bytes)?;

    Ok(LoadedConfig {
        config,
        source: ConfigSource::File,
    })
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
