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
fn help_lists_the_implemented_commands_and_no_later_ones() {
    let output = run(&["--help"]);
    assert_success(&output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    for implemented in [
        "init", "scan", "protect", "status", "check", "clean", "history",
    ] {
        assert!(
            stdout.contains(&format!("  {implemented}")),
            "{implemented} must be listed"
        );
    }
    // Scheduling is Day 5 and must not appear before it exists.
    for later_command in ["enable", "disable"] {
        assert!(
            !stdout.contains(&format!("  {later_command}")),
            "{later_command} must not be listed yet"
        );
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

/// The Day 4 gate, driven through the real binary: a volume declared to be
/// under pressure runs only proven actions, touches no project, records a
/// complete journal, and reports a safe shortfall rather than reaching further.
#[cfg(target_os = "linux")]
#[test]
fn a_pressured_run_executes_only_proven_actions_and_journals_them() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("projects");
    let workspace = root.join("api");
    std::fs::create_dir_all(&workspace).unwrap();
    for marker in ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"] {
        std::fs::write(workspace.join(marker), b"fixture").unwrap();
    }
    let lockfile = workspace.join("pnpm-lock.yaml");
    let lockfile_before = std::fs::read(&lockfile).unwrap();

    let fake_bin = home.path().join("fake-bin");
    std::fs::create_dir(&fake_bin).unwrap();
    let invocations = home.path().join("invocations.log");
    use std::os::unix::fs::PermissionsExt;
    for executable in ["pnpm", "git", "sh", "bash", "npm", "node"] {
        let path = fake_bin.join(executable);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '{executable} %s\\n' \"$*\" >> '{}'\n\
                 if [ \"$1\" = \"--version\" ]; then echo 11.2.3; fi\n",
                invocations.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // A threshold no real machine can satisfy, so the run is genuinely under
    // pressure without anyone having to fill a disk.
    let init = command(home.path())
        .args([
            "init",
            "--root",
            root.to_str().unwrap(),
            "--pnpm",
            fake_bin.join("pnpm").to_str().unwrap(),
            "--minimum-free",
            "900000GiB",
            "--target-free",
            "950000GiB",
            "--json",
        ])
        .output()
        .unwrap();
    assert_success(&init);

    let scan = command(home.path())
        .env("PATH", &fake_bin)
        .args(["scan", "--json"])
        .output()
        .unwrap();
    assert_success(&scan);

    let check = command(home.path())
        .env("PATH", &fake_bin)
        .args(["check", "--json"])
        .output()
        .unwrap();
    let check_json: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();

    // Every safe action was used and the target is still unreachable, which is
    // a correct outcome, not a failure to try harder.
    assert_eq!(check_json["result"], "SHORTFALL_SAFE_ACTIONS_EXHAUSTED");
    assert_eq!(check.status.code(), Some(1));

    // The workspace was observed moments ago, so it can never be cleaned here.
    let actions = check_json["actions"].as_array().unwrap();
    assert!(
        actions
            .iter()
            .all(|action| action["action"] != "PNPM_WORKSPACE_CLEAN"),
        "a newly observed workspace must not be cleaned: {actions:?}"
    );
    assert!(
        actions
            .iter()
            .any(|action| action["action"] == "CLEAN_TERMINAL_JANITOR_STATE"),
        "own-state cleanup must run"
    );

    let log = std::fs::read_to_string(&invocations).unwrap_or_default();
    for line in log.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        assert!(
            matches!(tokens.first(), Some(&"pnpm") | Some(&"git")),
            "unexpected external program: {line:?}"
        );
        for token in &tokens[1..] {
            assert!(
                !matches!(*token, "clean" | "--lockfile" | "-l" | "--force"),
                "{token:?} must never be passed; invocation was {line:?}"
            );
        }
    }
    assert!(
        log.contains("pnpm store prune"),
        "the delegated store prune must run; log was {log:?}"
    );
    assert_eq!(
        std::fs::read(&lockfile).unwrap(),
        lockfile_before,
        "the lockfile is the recovery proof and must survive"
    );

    // The journal must hold a complete, finished record of what happened.
    let history = command(home.path())
        .args(["history", "--json"])
        .output()
        .unwrap();
    assert_success(&history);
    let history_json: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    assert_eq!(history_json["result"], "HISTORY");
    let runs = history_json["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run["result"], "SHORTFALL_SAFE_ACTIONS_EXHAUSTED");
    assert!(
        run["finished_at_millis"].is_number(),
        "the run must be closed"
    );
    let journalled = run["actions"].as_array().unwrap();
    assert_eq!(journalled.len(), actions.len());
    for action in journalled {
        let state = action["state"].as_str().unwrap();
        assert!(
            state != "PLANNED" && state != "VALIDATING" && state != "RUNNING",
            "no action may be left in flight: {action:?}"
        );
    }
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
    // Every external tool is replaced by a script that appends its own name
    // and arguments to one log, so the test can assert exactly what ran rather
    // than only that something did.
    let fake_bin = home.path().join("fake-bin");
    std::fs::create_dir(&fake_bin).unwrap();
    let invocations = home.path().join("invocations.log");
    use std::os::unix::fs::PermissionsExt;
    for executable in ["pnpm", "npm", "node", "git", "sh", "bash"] {
        let path = fake_bin.join(executable);
        // pnpm answers --version so enrolment can succeed; git answers an
        // empty porcelain status, which means a clean worktree.
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '{executable} %s\\n' \"$*\" >> '{}'\n\
                 if [ \"$1\" = \"--version\" ]; then echo 11.2.3; fi\n",
                invocations.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let root_text = root.to_str().unwrap();
    let workspace_text = workspace.to_str().unwrap();
    let fake_pnpm = fake_bin.join("pnpm");

    let init = command(home.path())
        .args([
            "init",
            "--root",
            root_text,
            "--pnpm",
            fake_pnpm.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_success(&init);
    let init_json: serde_json::Value = serde_json::from_slice(&init.stdout).unwrap();
    assert_eq!(init_json["result"], "INIT_COMPLETE");
    assert_eq!(init_json["scheduling_enabled"], false);
    assert_eq!(init_json["scan_performed"], false);
    assert_eq!(init_json["pnpm_enrolled"], true);
    assert_eq!(init_json["pnpm_version"], "11.2.3");

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
            "dry_run",
            "plans",
        ])
    );
    assert_eq!(scan_json["registered"].as_array().unwrap().len(), 1);
    assert_eq!(scan_json["cleanup_performed"], false);
    assert_eq!(scan_json["dry_run"], false);

    // A workspace registered moments ago can never be cleaned, and the plan
    // must say so with one exact gate.
    let plans = scan_json["plans"].as_array().unwrap();
    assert_eq!(plans.len(), 1);
    let skipped = plans[0]["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0]["gate"], "OBSERVATION_WINDOW_NOT_MET");
    assert!(!skipped[0]["reason"].as_str().unwrap().is_empty());
    assert!(
        plans[0]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|action| action["action"] != "PNPM_WORKSPACE_CLEAN"),
        "no workspace clean may be planned for a newly observed workspace"
    );

    // Day 3 may ask questions. It may not clean anything. Every line is
    // checked token by token, so no forbidden argument can hide inside a
    // longer word.
    let log = std::fs::read_to_string(&invocations).unwrap_or_default();
    for line in log.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        assert!(
            matches!(tokens.first(), Some(&"pnpm") | Some(&"git")),
            "unexpected external program invoked: {line:?}"
        );
        for token in &tokens[1..] {
            assert!(
                !matches!(*token, "clean" | "--lockfile" | "-l" | "--force"),
                "{token:?} must never be passed; the invocation was {line:?}"
            );
        }
    }
    assert!(
        log.contains("pnpm --version"),
        "enrolment must read the pnpm version; log was {log:?}"
    );
    assert!(
        log.contains("git --no-optional-locks status"),
        "planning must ask Git for worktree state; log was {log:?}"
    );

    // A dry run reports without writing, so the ledger is untouched by it.
    let state_file = home.path().join("data/terminal_janitor/state.sqlite3");
    let before_state = std::fs::read(&state_file).unwrap();
    let dry = command(home.path())
        .env("PATH", &fake_bin)
        .args(["scan", "--json", "--dry-run"])
        .output()
        .unwrap();
    assert_success(&dry);
    let dry_json: serde_json::Value = serde_json::from_slice(&dry.stdout).unwrap();
    assert_eq!(dry_json["dry_run"], true);
    assert!(dry_json["registered"].as_array().unwrap().is_empty());
    assert!(dry_json["updated"].as_array().unwrap().is_empty());
    assert_eq!(std::fs::read(&state_file).unwrap(), before_state);

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
