use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use serde::Serialize;

use crate::config::{ConfigError, load_config_at};
use crate::disk::DiskProvider;
use crate::executor::RunReport;
use crate::model::DiskError;
use crate::status::{StorageStatus, render_human, render_json};
use crate::workflows::{
    HistoryReport, InitOptions, InitReport, ProtectionListReport, ProtectionReport, ScanReport,
    WorkflowError, check, history, initialise, list_protection, scan, set_protection,
};

pub const EXIT_SUCCESS: u8 = 0;
pub const EXIT_FAILED_CONFIGURATION: u8 = 2;
pub const EXIT_FAILED_STORAGE_MEASUREMENT: u8 = 3;
pub const EXIT_FAILED_OUTPUT: u8 = 4;
pub const EXIT_FAILED_STATE: u8 = 5;
pub const EXIT_FAILED_ROOT_VALIDATION: u8 = 6;
pub const EXIT_FAILED_DISCOVERY: u8 = 7;

#[derive(Debug, Parser)]
#[command(
    name = "terminal_janitor",
    version,
    about = "Conservative storage governance for developer machines"
)]
pub struct Cli {
    /// Emit stable machine-readable JSON
    #[arg(long, global = true)]
    pub json: bool,
    /// Read and report without writing state
    #[arg(long, global = true)]
    pub dry_run: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Explicitly register one or more approved project roots
    Init {
        /// Approved project root; repeat for more roots
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
        /// Cleanup trigger threshold (for example 10GiB)
        #[arg(long)]
        minimum_free: Option<String>,
        /// Recovery target threshold (for example 15GiB)
        #[arg(long)]
        target_free: Option<String>,
        /// Pnpm executable to enrol; PATH is searched when this is omitted
        #[arg(long)]
        pnpm: Option<PathBuf>,
    },
    /// Discover pnpm workspaces read-only beneath approved roots
    Scan,
    /// Manage exact registered-workspace protection
    Protect {
        #[command(subcommand)]
        command: ProtectCommand,
    },
    /// Report storage pressure without scanning or cleanup
    Status,
    /// Non-interactive threshold run; the scheduler entry point
    Check,
    /// Same authority as check, asked for by a person
    Clean,
    /// Show journalled runs, newest first
    History {
        /// Maximum runs to show
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProtectCommand {
    /// Protect an exact registered workspace root
    Add { path: PathBuf },
    /// Remove protection from an exact registered workspace root
    Remove { path: PathBuf },
    /// List protected registered workspaces
    List,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: u8,
}

pub fn execute_init(
    json: bool,
    config_path: &Path,
    state_path: &Path,
    options: InitOptions,
) -> CommandOutput {
    match initialise(config_path, state_path, options) {
        Ok(report) => render_serializable(json, &report, render_init_human(&report)),
        Err(error) => workflow_failure(json, &error),
    }
}

pub fn execute_scan(
    json: bool,
    config_path: &Path,
    state_path: &Path,
    dry_run: bool,
) -> CommandOutput {
    match scan(config_path, state_path, dry_run) {
        Ok(report) => render_serializable(json, &report, render_scan_human(&report)),
        Err(error) => workflow_failure(json, &error),
    }
}

/// Runs the threshold loop and exits with the result's own code, so a
/// scheduler can distinguish a healthy machine from a shortfall or a failure.
pub fn execute_check(
    json: bool,
    config_path: &Path,
    state_path: &Path,
    volume_path: &Path,
    dry_run: bool,
) -> CommandOutput {
    match check(config_path, state_path, volume_path, dry_run) {
        Ok(report) => {
            let exit_code = report.result.exit_code();
            let rendered = render_serializable(json, &report, render_check_human(&report));
            CommandOutput {
                exit_code,
                ..rendered
            }
        }
        Err(error) => workflow_failure(json, &error),
    }
}

pub fn execute_history(json: bool, state_path: &Path, limit: usize) -> CommandOutput {
    match history(state_path, limit) {
        Ok(report) => render_serializable(json, &report, render_history_human(&report)),
        Err(error) => workflow_failure(json, &error),
    }
}

pub fn execute_protection(json: bool, state_path: &Path, command: ProtectCommand) -> CommandOutput {
    match command {
        ProtectCommand::Add { path } => match set_protection(state_path, &path, true) {
            Ok(report) => render_serializable(json, &report, render_protection_human(&report)),
            Err(error) => workflow_failure(json, &error),
        },
        ProtectCommand::Remove { path } => match set_protection(state_path, &path, false) {
            Ok(report) => render_serializable(json, &report, render_protection_human(&report)),
            Err(error) => workflow_failure(json, &error),
        },
        ProtectCommand::List => match list_protection(state_path) {
            Ok(report) => render_serializable(json, &report, render_protection_list_human(&report)),
            Err(error) => workflow_failure(json, &error),
        },
    }
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

pub fn render_failure(
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

fn render_serializable<T: Serialize>(json: bool, value: &T, human: String) -> CommandOutput {
    if !json {
        return CommandOutput::success(human);
    }
    match serde_json::to_string_pretty(value) {
        Ok(mut stdout) => {
            stdout.push('\n');
            CommandOutput::success(stdout)
        }
        Err(error) => render_failure(true, "FAILED_OUTPUT", &error, EXIT_FAILED_OUTPUT),
    }
}

fn workflow_failure(json: bool, error: &WorkflowError) -> CommandOutput {
    match error {
        WorkflowError::Configuration(_) => render_failure(
            json,
            "FAILED_CONFIGURATION",
            error,
            EXIT_FAILED_CONFIGURATION,
        ),
        WorkflowError::State(_) => render_failure(json, "FAILED_STATE", error, EXIT_FAILED_STATE),
        WorkflowError::RootValidation { .. }
        | WorkflowError::UnknownWorkspace { .. }
        | WorkflowError::AmbiguousWorkspace { .. } => render_failure(
            json,
            "FAILED_ROOT_VALIDATION",
            error,
            EXIT_FAILED_ROOT_VALIDATION,
        ),
    }
}

fn render_init_human(report: &InitReport) -> String {
    let mut output = format!(
        "terminal_janitor initialised\nApproved roots: {}\nMinimum free: {} bytes\nTarget free: {} bytes\n",
        report.approved_roots.len(),
        report.minimum_free_bytes,
        report.target_free_bytes
    );
    for root in &report.approved_roots {
        output.push_str(&format!("{}\n", root.display()));
    }
    match (&report.pnpm_path, &report.pnpm_version) {
        (Some(path), Some(version)) => {
            output.push_str(&format!("Enrolled pnpm: {} ({version})\n", path.display()))
        }
        _ => output.push_str("Enrolled pnpm: none\n"),
    }
    if let Some(note) = &report.pnpm_note {
        output.push_str(&format!("{note}\n"));
    }
    output.push_str("Scheduling was not enabled. Scan was not run.\n");
    output
}

fn render_plans_human(report: &ScanReport) -> String {
    let mut output = String::new();
    for plan in &report.plans {
        output.push_str(&format!(
            "\nPlan {} (policy {})\nActions: {}\n",
            plan.plan_id,
            plan.policy_hash,
            plan.actions.len()
        ));
        for action in &plan.actions {
            match &action.path {
                Some(path) => {
                    output.push_str(&format!("{} {}\n", action.action, path.display()));
                }
                None => output.push_str(&format!("{}\n", action.action)),
            }
        }
        if !plan.skipped.is_empty() {
            output.push_str("Skipped\n");
            for skipped in &plan.skipped {
                output.push_str(&format!(
                    "{}\nGate: {} — {}\n",
                    skipped.path.display(),
                    skipped.gate,
                    skipped.reason
                ));
            }
        }
    }
    output.push_str("\nNothing was cleaned. Planning does not execute.\n");
    output
}

fn render_scan_human(report: &ScanReport) -> String {
    let mut output = format!(
        "Workspace scan\nApproved roots: {}\nRoots scanned: {}\nWorkspaces registered: {}\nWorkspaces updated: {}\nExcluded candidates: {}\nUnavailable roots: {}\nMissing workspaces: {}\nProtected workspaces: {}\n",
        report.approved_roots,
        report.roots_scanned,
        report.registered.len(),
        report.updated.len(),
        report.excluded.len(),
        report.unavailable_roots.len(),
        report.missing.len(),
        report.protected_workspaces,
    );
    if !report.registered.is_empty() {
        output.push_str("\nRegistered\n");
        for workspace in &report.registered {
            output.push_str(&format!(
                "{}{}\n",
                workspace.path.display(),
                if workspace.protected {
                    " [protected]"
                } else {
                    ""
                }
            ));
        }
    }
    if !report.updated.is_empty() {
        output.push_str("\nUpdated\n");
        for workspace in &report.updated {
            output.push_str(&format!(
                "{}{}\n",
                workspace.path.display(),
                if workspace.protected {
                    " [protected]"
                } else {
                    ""
                }
            ));
        }
    }
    if !report.excluded.is_empty() {
        output.push_str("\nExcluded\n");
        for exclusion in &report.excluded {
            output.push_str(&format!(
                "{}\nReason: {}\n",
                exclusion.path.display(),
                exclusion.reason
            ));
        }
    }
    if !report.unavailable_roots.is_empty() {
        output.push_str("\nUnavailable approved roots\n");
        for root in &report.unavailable_roots {
            output.push_str(&format!(
                "{}\nReason: {}\n",
                root.path.display(),
                root.reason
            ));
        }
    }
    if !report.missing.is_empty() {
        output.push_str("\nMissing\n");
        for workspace in &report.missing {
            output.push_str(&format!("{}\n", workspace.path.display()));
        }
    }
    if report.dry_run {
        output.push_str("\nDry run: no observation was written to the ledger.\n");
    }
    output.push_str(&render_plans_human(report));
    output.push_str("\nNo cleanup was performed.\n");
    output
}

fn render_check_human(report: &RunReport) -> String {
    let mut output = format!("Threshold run\nResult: {}\n", report.result);
    match (report.free_before_bytes, report.free_after_bytes) {
        (Some(before), Some(after)) => {
            output.push_str(&format!(
                "Free before: {before} bytes\nFree after:  {after} bytes\n"
            ));
            // Only a measured change is ever called recovery.
            if let Some(recovered) = report.measured_recovery_bytes() {
                output.push_str(&format!("Measured recovery: {recovered} bytes\n"));
            }
        }
        _ => output.push_str("Free space was not measured.\n"),
    }
    output.push_str(&format!(
        "Minimum free: {} bytes\nTarget free:  {} bytes\n",
        report.minimum_free_bytes, report.target_free_bytes
    ));
    if let Some(detail) = &report.detail {
        output.push_str(&format!("{detail}\n"));
    }
    if report.actions.is_empty() {
        output.push_str("\nNothing was touched.\n");
        return output;
    }
    output.push_str("\nActions\n");
    for action in &report.actions {
        let target = action
            .path
            .as_ref()
            .map_or_else(String::new, |path| format!(" {}", path.display()));
        output.push_str(&format!("{} {}{target}\n", action.state, action.action));
        if let Some(detail) = &action.detail {
            output.push_str(&format!("  {detail}\n"));
        }
    }
    output
}

fn render_history_human(report: &HistoryReport) -> String {
    if report.runs.is_empty() {
        return "No runs have been journalled.\n".to_owned();
    }
    let mut output = String::new();
    for run in &report.runs {
        let result = run
            .result
            .map_or_else(|| "INTERRUPTED".to_owned(), |result| result.to_string());
        output.push_str(&format!(
            "{}\nResult: {result}\nVolume: {}\nFree before: {} bytes\n",
            run.run_id, run.volume_id, run.free_before_bytes
        ));
        if let Some(after) = run.free_after_bytes {
            output.push_str(&format!("Free after:  {after} bytes\n"));
        }
        for action in &run.actions {
            output.push_str(&format!("  {} {}\n", action.state, action.action));
            if let Some(recovery) = &action.recovery {
                output.push_str(&format!("    recovery: {recovery}\n"));
            }
        }
        // An interrupted run must never read as either success or failure.
        let unresolved = run.unresolved_actions();
        if !unresolved.is_empty() {
            output.push_str(&format!(
                "  {} action(s) were interrupted; their outcome is unknown and \
                 they were not replayed.\n",
                unresolved.len()
            ));
        }
        output.push('\n');
    }
    output
}

fn render_protection_human(report: &ProtectionReport) -> String {
    format!(
        "{}\nWorkspace: {}\nProtected: {}\n",
        report.result,
        report.workspace.path.display(),
        report.workspace.protected
    )
}

fn render_protection_list_human(report: &ProtectionListReport) -> String {
    let mut output = format!("Protected workspaces: {}\n", report.protected.len());
    for workspace in &report.protected {
        output.push_str(&format!(
            "{} [{}]\n",
            workspace.path.display(),
            workspace.status
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use std::fs;

    use clap::error::ErrorKind;

    use super::*;

    /// A path that cannot exist, so these tests never search `PATH` for pnpm
    /// and never run a real one.
    const ABSENT_PNPM: &str = "/terminal_janitor-test-no-such-pnpm";
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
        let help = help.to_string();
        for command in [
            "init", "scan", "protect", "status", "check", "clean", "history",
        ] {
            assert!(help.contains(command), "{command} must be listed");
        }
        // Scheduling is Day 5; it must not be advertised before it exists.
        for later in ["enable", "disable"] {
            assert!(
                !help.contains(&format!("\n  {later}")),
                "{later} must not be listed yet"
            );
        }

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

    #[test]
    fn day_two_human_outputs_are_truthful() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        for marker in ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"] {
            fs::write(workspace.join(marker), b"fixture").unwrap();
        }
        let config = dir.path().join("config/config.toml");
        let state = dir.path().join("state/state.sqlite3");
        let missing_root = execute_init(
            true,
            &config,
            &state,
            InitOptions {
                roots: vec![],
                minimum_free: None,
                target_free: None,
                pnpm: Some(PathBuf::from(ABSENT_PNPM)),
            },
        );
        assert_eq!(missing_root.exit_code, EXIT_FAILED_ROOT_VALIDATION);
        let failure: serde_json::Value = serde_json::from_str(&missing_root.stdout).unwrap();
        assert_eq!(failure["result"], "FAILED_ROOT_VALIDATION");

        let init = execute_init(
            false,
            &config,
            &state,
            InitOptions {
                roots: vec![root],
                minimum_free: None,
                target_free: None,
                pnpm: Some(PathBuf::from(ABSENT_PNPM)),
            },
        );
        assert_eq!(init.exit_code, EXIT_SUCCESS);
        assert!(init.stdout.contains("terminal_janitor initialised"));
        assert!(init.stdout.contains("Scheduling was not enabled"));
        assert!(init.stdout.contains("Scan was not run"));

        let scan = execute_scan(false, &config, &state, false);
        assert_eq!(scan.exit_code, EXIT_SUCCESS);
        assert!(scan.stdout.contains("Workspace scan"));
        assert!(scan.stdout.contains("Workspaces registered: 1"));
        assert!(scan.stdout.contains("No cleanup was performed"));

        let list = execute_protection(false, &state, ProtectCommand::List);
        assert_eq!(list.exit_code, EXIT_SUCCESS);
        assert_eq!(list.stdout, "Protected workspaces: 0\n");
    }
}
