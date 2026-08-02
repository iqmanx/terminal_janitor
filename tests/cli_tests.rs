use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

fn command(isolated_config_home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_terminal_janitor"));
    command.current_dir(isolated_config_home);

    #[cfg(target_os = "linux")]
    command
        .env("XDG_CONFIG_HOME", isolated_config_home.join("config"))
        .env("XDG_DATA_HOME", isolated_config_home.join("data"));
    #[cfg(target_os = "macos")]
    command.env("HOME", isolated_config_home);
    // Windows deliberately receives no profile overrides: ProjectDirs uses
    // the Known Folder API, and changing profile variables can make that API
    // unavailable. GitHub CI supplies a clean account for the default test.

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
fn help_lists_only_day_two_commands() {
    let output = run(&["--help"]);
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    for implemented in ["init", "scan", "protect", "status"] {
        assert!(stdout.contains(&format!("  {implemented}")));
    }
    for later_command in ["check", "clean", "history", "enable", "disable"] {
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
    let config_dir = home.path().join("config/terminal_janitor");
    std::fs::create_dir_all(&config_dir).unwrap();
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

#[cfg(target_os = "linux")]
#[test]
fn init_scan_and_protection_cli_workflows_emit_stable_json() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("approved projects");
    let workspace = root.join("Unicode workspace 日本語");
    std::fs::create_dir_all(&workspace).unwrap();
    for marker in ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"] {
        std::fs::write(workspace.join(marker), b"fixture").unwrap();
    }
    let marker_paths: Vec<_> = ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"]
        .into_iter()
        .map(|marker| workspace.join(marker))
        .collect();
    let before_files: Vec<_> = marker_paths
        .iter()
        .map(|path| {
            (
                std::fs::read(path).unwrap(),
                std::fs::metadata(path).unwrap().modified().unwrap(),
            )
        })
        .collect();
    let fake_bin = home.path().join("fake-bin");
    std::fs::create_dir(&fake_bin).unwrap();
    let invoked = home.path().join("external-command-was-invoked");
    use std::os::unix::fs::PermissionsExt;
    for executable in ["pnpm", "npm", "node", "git", "sh", "bash"] {
        let path = fake_bin.join(executable);
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf invoked > '{}'\n", invoked.display()),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let root_text = root.to_str().unwrap();
    let workspace_text = workspace.to_str().unwrap();

    let init = command(home.path())
        .args(["init", "--root", root_text, "--json"])
        .output()
        .unwrap();
    assert_success(&init);
    let init_json: serde_json::Value = serde_json::from_slice(&init.stdout).unwrap();
    assert_eq!(init_json["result"], "INIT_COMPLETE");
    assert_eq!(init_json["scheduling_enabled"], false);
    assert_eq!(init_json["scan_performed"], false);

    let scan = command(home.path())
        .env("PATH", &fake_bin)
        .args(["scan", "--json"])
        .output()
        .unwrap();
    assert_success(&scan);
    let scan_json: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    let fields: BTreeSet<_> = scan_json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        fields,
        BTreeSet::from([
            "result",
            "approved_roots",
            "roots_scanned",
            "registered",
            "updated",
            "excluded",
            "unavailable_roots",
            "missing",
            "protected_workspaces",
            "cleanup_performed",
        ])
    );
    assert_eq!(scan_json["registered"].as_array().unwrap().len(), 1);
    assert_eq!(scan_json["cleanup_performed"], false);
    assert!(
        !invoked.exists(),
        "scan must invoke no pnpm, package script, Git, Node, or shell process"
    );

    for (args, expected) in [
        (
            vec!["protect", "add", workspace_text, "--json"],
            "PROTECTION_ADDED",
        ),
        (
            vec!["protect", "remove", workspace_text, "--json"],
            "PROTECTION_REMOVED",
        ),
    ] {
        let output = command(home.path()).args(args).output().unwrap();
        assert_success(&output);
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["result"], expected);
    }

    let list = command(home.path())
        .args(["protect", "list", "--json"])
        .output()
        .unwrap();
    assert_success(&list);
    let list_json: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(list_json["result"], "PROTECTION_LIST");
    assert!(list_json["protected"].as_array().unwrap().is_empty());
    for (path, (contents, modified)) in marker_paths.iter().zip(before_files) {
        assert_eq!(std::fs::read(path).unwrap(), contents);
        assert_eq!(
            std::fs::metadata(path).unwrap().modified().unwrap(),
            modified
        );
    }
}
