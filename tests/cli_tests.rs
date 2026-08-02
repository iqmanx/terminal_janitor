use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

fn command(isolated_config_home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_terminal_janitor"));
    command.current_dir(isolated_config_home);

    #[cfg(target_os = "linux")]
    command.env("XDG_CONFIG_HOME", isolated_config_home);
    #[cfg(target_os = "macos")]
    command.env("HOME", isolated_config_home);
    #[cfg(target_os = "windows")]
    {
        // ProjectDirs uses the Known Folder API on Windows. CI runs in a
        // clean account; these variables also isolate fallback resolution.
        command.env("APPDATA", isolated_config_home);
        command.env("USERPROFILE", isolated_config_home);
    }

    command
}

fn run(args: &[&str]) -> Output {
    let home = tempfile::tempdir().unwrap();
    command(home.path()).args(args).output().unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed with {:?}; stdout={:?}; stderr={:?}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn help_lists_only_implemented_command() {
    let output = run(&["--help"]);
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("status"));
    for later_command in ["init", "scan", "check", "clean", "protect", "history"] {
        assert!(!stdout.contains(&format!("  {later_command}")));
    }
}

#[test]
fn version_reports_package_version() {
    let output = run(&["--version"]);
    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        concat!("terminal_janitor ", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn human_status_is_read_only_and_labels_configuration() {
    let output = run(&["status"]);
    assert_success(&output);
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    for label in [
        "Storage status",
        "State:",
        "Total:",
        "Used:",
        "Available:",
        "Minimum free:",
        "Target free:",
        "Configuration: Defaults",
    ] {
        assert!(stdout.contains(label), "missing {label:?} in {stdout:?}");
    }
}

#[test]
fn json_status_is_valid_and_has_stable_schema() {
    let output = run(&["status", "--json"]);
    assert_success(&output);
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let fields: BTreeSet<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let expected = BTreeSet::from([
        "result",
        "state",
        "total_bytes",
        "used_bytes",
        "available_bytes",
        "minimum_free_bytes",
        "target_free_bytes",
        "config_source",
    ]);
    assert_eq!(fields, expected);
    assert!(matches!(
        value["result"].as_str(),
        Some("OK_NO_PRESSURE" | "PRESSURE_DETECTED")
    ));
    assert!(matches!(
        value["state"].as_str(),
        Some("healthy" | "pressure")
    ));
    assert_eq!(value["config_source"], "defaults");
}

#[cfg(target_os = "linux")]
#[test]
fn invalid_existing_platform_config_exits_nonzero_and_fails_closed() {
    let home = tempfile::tempdir().unwrap();
    let config_dir = home.path().join("terminal_janitor");
    std::fs::create_dir(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), "minimum_free = [").unwrap();

    let output = command(home.path())
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["result"], "FAILED_CONFIGURATION");
}
