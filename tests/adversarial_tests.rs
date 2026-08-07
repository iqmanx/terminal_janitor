//! Day 6 — adversarial hardening.
//!
//! These tests add no product behaviour. They try to break the safety model
//! from outside the crate, through the real binary and the real filesystem.

// Each helper below is gated to the platforms whose tests use it. The suite is
// deliberately uneven — driving the real binary needs a fixture that redirects
// the platform's config and state directories, which only the Unix targets have
// here — and an ungated helper would be dead code elsewhere under `-D warnings`.
#[cfg(target_os = "linux")]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Command, Output};

#[cfg(unix)]
fn command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_terminal_janitor"));
    command.current_dir(home);
    #[cfg(target_os = "linux")]
    command
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"));
    #[cfg(target_os = "macos")]
    command.env("HOME", home);
    command
}

#[cfg(unix)]
fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed with {:?}; stdout={:?}; stderr={:?}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Every regular file under `root`, with its bytes and modification time.
///
/// Paths under `ignore` are excluded: those are the places this product is
/// permitted to write.
#[cfg(target_os = "linux")]
fn snapshot(
    root: &Path,
    ignore: &[PathBuf],
) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    let mut found = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        if ignore.iter().any(|skip| directory.starts_with(skip)) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if ignore.iter().any(|skip| path.starts_with(skip)) {
                continue;
            }
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                stack.push(path);
            } else if metadata.is_file() {
                let bytes = std::fs::read(&path).unwrap_or_default();
                let modified = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
                found.insert(path, (bytes, modified));
            }
        }
    }
    found
}

/// A pressured run must write nowhere except its own state directory and,
/// through pnpm, the exact verified workspace. This asserts the first half
/// directly: with a pnpm that changes nothing, the entire tree outside the
/// product's own state is byte-for-byte and timestamp-for-timestamp identical.
///
/// It also proves the Rust engine never recursively deletes a project path:
/// every project file is still present afterwards.
#[cfg(target_os = "linux")]
#[test]
fn a_pressured_run_writes_nothing_outside_its_own_state_directory() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("projects");
    let workspace = root.join("api");
    std::fs::create_dir_all(workspace.join("node_modules/left-pad")).unwrap();
    std::fs::create_dir_all(workspace.join("src")).unwrap();

    // A project that defines its own `clean` script. Selecting it instead of
    // pnpm's built-in command is exactly what `pnpm pm clean` prevents.
    let script_evidence = home.path().join("project-clean-script-ran");
    std::fs::write(
        workspace.join("package.json"),
        format!(
            r#"{{"name":"api","scripts":{{"clean":"touch {}"}}}}"#,
            script_evidence.display()
        ),
    )
    .unwrap();
    std::fs::write(workspace.join("pnpm-workspace.yaml"), b"packages: []\n").unwrap();
    std::fs::write(workspace.join("pnpm-lock.yaml"), b"lockfileVersion: 9\n").unwrap();
    std::fs::write(
        workspace.join("src/main.ts"),
        b"export const answer = 42;\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("node_modules/left-pad/index.js"),
        b"module\n",
    )
    .unwrap();
    std::fs::write(home.path().join("personal-notes.txt"), b"do not touch\n").unwrap();

    let fake_bin = home.path().join("fake-bin");
    std::fs::create_dir(&fake_bin).unwrap();
    let invocations = home.path().join("invocations.log");
    for executable in ["pnpm", "git", "node", "npm", "sh", "bash"] {
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

    assert_success(
        &command(home.path())
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
            .unwrap(),
    );
    assert_success(
        &command(home.path())
            .env("PATH", &fake_bin)
            .args(["scan", "--json"])
            .output()
            .unwrap(),
    );

    // Everything the product is allowed to write to.
    let permitted = vec![
        home.path().join("data"),
        home.path().join("config"),
        invocations.clone(),
    ];
    let before = snapshot(home.path(), &permitted);
    assert!(
        before.contains_key(&workspace.join("pnpm-lock.yaml")),
        "the fixture must be captured"
    );

    let check = command(home.path())
        .env("PATH", &fake_bin)
        .args(["check", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert!(
        report["result"].as_str().is_some(),
        "check must report a result: {report:?}"
    );

    let after = snapshot(home.path(), &permitted);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "no file outside the product's state may appear or disappear"
    );
    for (path, before_entry) in &before {
        assert_eq!(
            before_entry,
            after.get(path).unwrap(),
            "{} was modified",
            path.display()
        );
    }
    assert!(
        !script_evidence.exists(),
        "a project-defined clean script must never be able to run"
    );
    assert!(
        workspace.join("node_modules/left-pad/index.js").exists(),
        "the Rust engine must not delete project paths itself"
    );
}

/// A run must not be able to write outside an approved root even when the
/// workspace is reached through a symlink, and a symlinked project root must
/// not be registrable at all.
#[cfg(unix)]
#[test]
fn a_symlinked_root_is_refused_and_a_symlink_loop_is_survived() {
    let home = tempfile::tempdir().unwrap();
    let real = home.path().join("real-projects");
    std::fs::create_dir_all(real.join("api")).unwrap();
    for marker in ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"] {
        std::fs::write(real.join("api").join(marker), b"fixture").unwrap();
    }

    // A loop inside the tree must not hang or overflow the traversal.
    let loop_dir = real.join("api").join("loop");
    std::os::unix::fs::symlink(&real, &loop_dir).unwrap();

    let linked_root = home.path().join("linked-projects");
    std::os::unix::fs::symlink(&real, &linked_root).unwrap();

    let refused = command(home.path())
        .args(["init", "--root", linked_root.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(
        !refused.status.success(),
        "a root whose final component is a symlink must be refused"
    );

    assert_success(
        &command(home.path())
            .args([
                "init",
                "--root",
                real.to_str().unwrap(),
                "--pnpm",
                "/terminal_janitor-test-no-such-pnpm",
                "--json",
            ])
            .output()
            .unwrap(),
    );
    let scan = command(home.path())
        .args(["scan", "--json"])
        .output()
        .unwrap();
    assert_success(&scan);
    let report: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert_eq!(
        report["registered"].as_array().unwrap().len(),
        1,
        "the loop must not produce repeated registrations"
    );
}

/// One physical directory must never acquire two identities, whatever the
/// filesystem does about case. On a case-insensitive filesystem the alternate
/// spelling resolves and must match; on a case-sensitive one it simply does
/// not exist. Both are safe; two different identities would not be.
#[test]
fn case_differences_never_produce_a_second_identity() {
    let home = tempfile::tempdir().unwrap();
    let mixed = home.path().join("CaseSensitivity");
    std::fs::create_dir(&mixed).unwrap();
    let lowered = home.path().join("casesensitivity");

    let Ok(direct) = dunce::canonicalize(&mixed) else {
        return;
    };
    let Ok(alternate) = dunce::canonicalize(&lowered) else {
        // Case-sensitive filesystem: the alternate spelling is a different,
        // absent path, which is the safe outcome.
        return;
    };
    assert_eq!(
        direct, alternate,
        "one directory must canonicalise to one spelling"
    );
}

/// `history` must stay readable and truthful when the ledger has never been
/// written, rather than inventing a run or failing obscurely.
#[cfg(target_os = "linux")]
#[test]
fn history_before_any_run_is_empty_rather_than_invented() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("projects");
    std::fs::create_dir_all(&root).unwrap();
    assert_success(
        &command(home.path())
            .args([
                "init",
                "--root",
                root.to_str().unwrap(),
                "--pnpm",
                "/terminal_janitor-test-no-such-pnpm",
                "--json",
            ])
            .output()
            .unwrap(),
    );

    let history = command(home.path())
        .args(["history", "--json"])
        .output()
        .unwrap();
    assert_success(&history);
    let report: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    assert_eq!(report["result"], "HISTORY");
    assert!(report["runs"].as_array().unwrap().is_empty());
}

/// A corrupt ledger must fail closed and must not be silently recreated,
/// because recreating it would erase protection state.
#[cfg(target_os = "linux")]
#[test]
fn a_corrupt_ledger_fails_closed_without_being_recreated() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("projects");
    std::fs::create_dir_all(&root).unwrap();
    assert_success(
        &command(home.path())
            .args([
                "init",
                "--root",
                root.to_str().unwrap(),
                "--pnpm",
                "/terminal_janitor-test-no-such-pnpm",
                "--json",
            ])
            .output()
            .unwrap(),
    );

    let state = home.path().join("data/terminal_janitor/state.sqlite3");
    assert!(state.is_file(), "init must have created the ledger");
    std::fs::write(&state, b"this is not a database").unwrap();
    let corrupt_bytes = std::fs::read(&state).unwrap();

    for arguments in [
        vec!["scan", "--json"],
        vec!["check", "--json"],
        vec!["history", "--json"],
    ] {
        let output = command(home.path()).args(&arguments).output().unwrap();
        assert!(
            !output.status.success(),
            "{arguments:?} must fail on a corrupt ledger"
        );
        assert_eq!(
            std::fs::read(&state).unwrap(),
            corrupt_bytes,
            "{arguments:?} must not rewrite or recreate the ledger"
        );
    }
}

/// Two ordinary hostile filesystem shapes inside an approved root: a link with
/// nothing behind it, and a tree deeper than the traversal's limit. Neither may
/// stop discovery, and neither may produce a registration.
#[cfg(target_os = "linux")]
#[test]
fn a_broken_link_and_an_over_deep_tree_register_nothing_and_stop_nothing() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("projects");
    std::fs::create_dir_all(&root).unwrap();

    std::os::unix::fs::symlink(root.join("never-existed"), root.join("dangling")).unwrap();

    // Past the depth limit of 64, with a complete set of markers at the very
    // bottom: reachability, not validity, is what must refuse this one.
    let mut deep = root.join("deep");
    for _ in 0..80 {
        deep = deep.join("d");
    }
    std::fs::create_dir_all(&deep).unwrap();

    // One genuine workspace at a reachable depth, so a scan that found nothing
    // at all cannot pass this test by accident.
    let reachable = root.join("api");
    std::fs::create_dir_all(&reachable).unwrap();
    for directory in [&deep, &reachable] {
        for marker in ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"] {
            std::fs::write(directory.join(marker), b"fixture").unwrap();
        }
    }

    assert_success(
        &command(home.path())
            .args([
                "init",
                "--root",
                root.to_str().unwrap(),
                "--pnpm",
                "/terminal_janitor-test-no-such-pnpm",
                "--json",
            ])
            .output()
            .unwrap(),
    );
    let scan = command(home.path())
        .args(["scan", "--json"])
        .output()
        .unwrap();
    assert_success(&scan);

    let report: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    let registered = report["registered"].as_array().unwrap();
    assert_eq!(
        registered.len(),
        1,
        "only the reachable workspace may register: {registered:?}"
    );
    let text = serde_json::to_string(registered).unwrap();
    assert!(
        text.contains("/api"),
        "the reachable workspace must be the registered one: {text}"
    );
    assert!(
        !text.contains("/deep/"),
        "a workspace past the depth limit must not register: {text}"
    );
}

/// `SAFETY.md` caps logs at 10 MiB. This product meets that by writing no log
/// at all: the SQLite journal is the only record, and it is capped separately
/// at 200 runs and 20 MiB. That is worth pinning down rather than assuming, so
/// a log file added later has to arrive with its cap.
#[cfg(target_os = "linux")]
#[test]
fn a_run_writes_no_log_only_its_ledger_and_its_lock() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path().join("projects");
    std::fs::create_dir_all(root.join("api")).unwrap();
    for marker in ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"] {
        std::fs::write(root.join("api").join(marker), b"fixture").unwrap();
    }

    assert_success(
        &command(home.path())
            .args([
                "init",
                "--root",
                root.to_str().unwrap(),
                "--pnpm",
                "/terminal_janitor-test-no-such-pnpm",
                "--minimum-free",
                "900000GiB",
                "--target-free",
                "950000GiB",
                "--json",
            ])
            .output()
            .unwrap(),
    );
    assert_success(
        &command(home.path())
            .args(["scan", "--json"])
            .output()
            .unwrap(),
    );
    // A pressured run: it journals, takes its lock, and finds nothing eligible.
    let _ = command(home.path())
        .args(["check", "--json"])
        .output()
        .unwrap();

    let state = home.path().join("data/terminal_janitor");
    let mut stack = vec![state.clone()];
    let mut found = Vec::new();
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    assert!(!found.is_empty(), "the run must have written its ledger");

    for path in &found {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let permitted = name == "state.sqlite3"
            || name.starts_with("state.sqlite3-")     // SQLite -wal/-shm sidecars
            || (name.starts_with("volume-") && name.ends_with(".lock"));
        assert!(
            permitted,
            "{} is neither the ledger nor a lock; an uncapped log would live here",
            path.display()
        );
    }
}

/// Windows reaches the same shape through reparse points rather than POSIX
/// symlinks. The final-component refusal that protects an approved root keys
/// off `is_symlink`, so this asserts that assumption holds for a real reparse
/// point, and that a directory reached through one still canonicalises to a
/// single identity.
#[cfg(windows)]
#[test]
fn a_windows_reparse_point_is_seen_as_a_link_and_grants_no_second_identity() {
    let home = tempfile::tempdir().unwrap();
    let target = home.path().join("real-projects");
    std::fs::create_dir(&target).unwrap();
    let link = home.path().join("linked-projects");

    // Deliberately not skipped when the fixture cannot be created. A skipped
    // body and a passing body are both reported as one passing test, so a
    // graceful return here would make `ACCEPTANCE.md`'s reparse-point item
    // unprovable — the tick would rest on "it probably ran". Creating a
    // directory symlink needs SeCreateSymbolicLinkPrivilege, which CI runners
    // and developer machines in Developer Mode both have; without it this
    // fails loudly and says why, which is the honest outcome.
    std::os::windows::fs::symlink_dir(&target, &link).expect(
        "could not create a directory reparse point: this needs \
         SeCreateSymbolicLinkPrivilege, granted by Developer Mode or an \
         elevated shell. The reparse-point safety property cannot be proven \
         without it, so this test refuses to pass silently.",
    );

    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "a directory reparse point must be visible as a link, or the \
         final-component refusal cannot see it"
    );
    assert_eq!(
        dunce::canonicalize(&link).unwrap(),
        dunce::canonicalize(&target).unwrap(),
        "a reparse point must not produce a second identity for one directory"
    );
}
