use std::path::Path;

use clap::{Parser, Subcommand};
use serde::Serialize;

use crate::config::{ConfigError, load_config_at};
use crate::disk::DiskProvider;
use crate::model::DiskError;
use crate::status::{StorageStatus, render_human, render_json};

pub const EXIT_SUCCESS: u8 = 0;
pub const EXIT_FAILED_CONFIGURATION: u8 = 2;
pub const EXIT_FAILED_STORAGE_MEASUREMENT: u8 = 3;
pub const EXIT_FAILED_OUTPUT: u8 = 4;

#[derive(Debug, Parser)]
#[command(
    name = "terminal_janitor",
    version,
    about = "Conservative storage status for developer machines"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Report storage pressure without scanning or cleanup
    Status {
        /// Emit stable machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u8,
}

impl CommandOutput {
    fn success(stdout: String) -> Self {
        Self {
            stdout,
            stderr: String::new(),
            exit_code: EXIT_SUCCESS,
        }
    }
}

pub fn execute_status(
    json: bool,
    config_path: &Path,
    volume_path: &Path,
    disk_provider: &dyn DiskProvider,
) -> CommandOutput {
    let loaded = match load_config_at(config_path) {
        Ok(loaded) => loaded,
        Err(error) => {
            return render_failure(
                json,
                "FAILED_CONFIGURATION",
                &error,
                EXIT_FAILED_CONFIGURATION,
            );
        }
    };
    let capacity = match disk_provider.capacity_for(volume_path) {
        Ok(capacity) => capacity,
        Err(error) => {
            return render_failure(
                json,
                "FAILED_STORAGE_MEASUREMENT",
                &error,
                EXIT_FAILED_STORAGE_MEASUREMENT,
            );
        }
    };
    let status = StorageStatus::calculate(capacity, loaded.config, loaded.source);

    if json {
        match render_json(&status) {
            Ok(output) => CommandOutput::success(output),
            Err(error) => render_failure(json, "FAILED_OUTPUT", &error, EXIT_FAILED_OUTPUT),
        }
    } else {
        CommandOutput::success(render_human(&status))
    }
}

pub fn configuration_path_failure(json: bool, error: &ConfigError) -> CommandOutput {
    render_failure(
        json,
        "FAILED_CONFIGURATION",
        error,
        EXIT_FAILED_CONFIGURATION,
    )
}

pub fn storage_path_failure(json: bool, error: &DiskError) -> CommandOutput {
    render_failure(
        json,
        "FAILED_STORAGE_MEASUREMENT",
        error,
        EXIT_FAILED_STORAGE_MEASUREMENT,
    )
}

fn render_failure(
    json: bool,
    result: &'static str,
    error: &dyn std::fmt::Display,
    exit_code: u8,
) -> CommandOutput {
    if json {
        #[derive(Serialize)]
        struct JsonFailure<'a> {
            result: &'static str,
            error: &'a str,
        }

        let message = error.to_string();
        let stdout = serde_json::to_string_pretty(&JsonFailure {
            result,
            error: &message,
        })
        .map(|mut output| {
            output.push('\n');
            output
        })
        .unwrap_or_else(|_| format!("{{\"result\":\"{result}\"}}\n"));
        CommandOutput {
            stdout,
            stderr: String::new(),
            exit_code,
        }
    } else {
        CommandOutput {
            stdout: String::new(),
            stderr: format!("{result}: {error}\n"),
            exit_code,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use clap::error::ErrorKind;

    use super::*;
    use crate::disk::FakeDiskProvider;
    use crate::model::DiskCapacity;

    fn fake_for(path: &Path, available: u64) -> FakeDiskProvider {
        let mut fake = FakeDiskProvider::new();
        let total = available.max(100);
        fake.set_capacity(path, DiskCapacity::new(total, available).unwrap());
        fake
    }

    #[test]
    fn help_and_version_are_real_clap_outputs() {
        let help = Cli::try_parse_from(["terminal_janitor", "--help"]).unwrap_err();
        assert_eq!(help.kind(), ErrorKind::DisplayHelp);
        assert!(help.to_string().contains("status"));
        assert!(!help.to_string().contains("\n  clean "));

        let version = Cli::try_parse_from(["terminal_janitor", "--version"]).unwrap_err();
        assert_eq!(version.kind(), ErrorKind::DisplayVersion);
        assert!(version.to_string().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn human_and_json_status_use_fake_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("missing.toml");
        let fake = fake_for(dir.path(), 20 * 1024_u64.pow(3));

        let human = execute_status(false, &config_path, dir.path(), &fake);
        assert_eq!(human.exit_code, EXIT_SUCCESS);
        assert!(human.stdout.contains("State:         Healthy"));
        assert!(human.stdout.contains("Configuration: Defaults"));

        let json = execute_status(true, &config_path, dir.path(), &fake);
        assert_eq!(json.exit_code, EXIT_SUCCESS);
        let value: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
        assert_eq!(value["result"], "OK_NO_PRESSURE");
        assert_eq!(value["available_bytes"], 20 * 1024_u64.pow(3));
    }

    #[test]
    fn invalid_configuration_has_meaningful_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, "minimum_free = [").unwrap();
        let fake = fake_for(dir.path(), 100);

        let output = execute_status(false, &config_path, dir.path(), &fake);
        assert_eq!(output.exit_code, EXIT_FAILED_CONFIGURATION);
        assert!(output.stderr.starts_with("FAILED_CONFIGURATION:"));
    }

    #[test]
    fn disk_failure_has_meaningful_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let output = execute_status(
            true,
            &dir.path().join("missing.toml"),
            dir.path(),
            &FakeDiskProvider::new(),
        );
        assert_eq!(output.exit_code, EXIT_FAILED_STORAGE_MEASUREMENT);
        let value: serde_json::Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(value["result"], "FAILED_STORAGE_MEASUREMENT");
    }

    #[test]
    fn cli_handles_config_path_with_spaces_and_unicode() {
        let root = tempfile::tempdir().unwrap();
        let config_dir = root.path().join("space Ünïcödé 日本語");
        fs::create_dir(&config_dir).unwrap();
        let config_path = config_dir.join("config.toml");
        fs::write(
            &config_path,
            "minimum_free = \"1B\"\ntarget_free = \"2B\"\n",
        )
        .unwrap();
        let fake = fake_for(root.path(), 100);

        let output = execute_status(false, &config_path, root.path(), &fake);
        assert_eq!(output.exit_code, EXIT_SUCCESS);
        assert!(output.stdout.contains("Configuration: File"));
    }
}
