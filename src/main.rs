use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;
use terminal_janitor::cli::{
    Cli, Command, CommandOutput, configuration_path_failure, execute_init, execute_protection,
    execute_scan, execute_status, render_failure, storage_path_failure,
};
use terminal_janitor::config::config_file_path;
use terminal_janitor::disk::SystemDiskProvider;
use terminal_janitor::model::DiskError;
use terminal_janitor::workflows::{InitOptions, state_file_path};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    let dry_run = cli.dry_run;
    let output = match cli.command {
        Command::Init {
            roots,
            minimum_free,
            target_free,
            pnpm,
        } => run_init(
            json,
            InitOptions {
                roots,
                minimum_free,
                target_free,
                pnpm,
            },
        ),
        Command::Scan => run_scan(json, dry_run),
        Command::Protect { command } => run_protection(json, command),
        Command::Status => run_status(json),
    };
    emit(output)
}

fn run_init(json: bool, options: InitOptions) -> CommandOutput {
    let config_path = match config_file_path() {
        Ok(path) => path,
        Err(error) => return configuration_path_failure(json, &error),
    };
    let state_path = match state_file_path() {
        Ok(path) => path,
        Err(error) => return render_failure(json, "FAILED_STATE", &error, 5),
    };
    execute_init(json, &config_path, &state_path, options)
}

fn run_scan(json: bool, dry_run: bool) -> CommandOutput {
    let config_path = match config_file_path() {
        Ok(path) => path,
        Err(error) => return configuration_path_failure(json, &error),
    };
    let state_path = match state_file_path() {
        Ok(path) => path,
        Err(error) => return render_failure(json, "FAILED_STATE", &error, 5),
    };
    execute_scan(json, &config_path, &state_path, dry_run)
}

fn run_protection(json: bool, command: terminal_janitor::cli::ProtectCommand) -> CommandOutput {
    let state_path = match state_file_path() {
        Ok(path) => path,
        Err(error) => return render_failure(json, "FAILED_STATE", &error, 5),
    };
    execute_protection(json, &state_path, command)
}

fn run_status(json: bool) -> CommandOutput {
    let config_path = match config_file_path() {
        Ok(path) => path,
        Err(error) => return configuration_path_failure(json, &error),
    };
    let volume_path = match std::env::current_dir() {
        Ok(path) => path,
        Err(source) => {
            let error = DiskError::MeasurementFailed {
                path: ".".into(),
                source,
            };
            return storage_path_failure(json, &error);
        }
    };
    execute_status(json, &config_path, &volume_path, &SystemDiskProvider)
}

fn emit(output: CommandOutput) -> ExitCode {
    if !output.stdout.is_empty() {
        let _ = io::stdout().write_all(output.stdout.as_bytes());
    }
    if !output.stderr.is_empty() {
        let _ = io::stderr().write_all(output.stderr.as_bytes());
    }
    ExitCode::from(output.exit_code)
}
