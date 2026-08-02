use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;
use terminal_janitor::cli::{
    Cli, Command, CommandOutput, configuration_path_failure, execute_status, storage_path_failure,
};
use terminal_janitor::config::config_file_path;
use terminal_janitor::disk::SystemDiskProvider;
use terminal_janitor::model::DiskError;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let output = match cli.command {
        Command::Status { json } => run_status(json),
    };
    emit(output)
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
