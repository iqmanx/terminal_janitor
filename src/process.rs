//! Process liveness: the G3 gate's live half.
//!
//! Day 3 shipped a prover that always answered `Unknown`, because an
//! unfinished gate must refuse rather than pass. This module replaces it with
//! a real one: it enumerates running processes and asks whether any of them is
//! working inside a workspace.
//!
//! The conservative rule is deliberate. Enumeration failing, a process this
//! user owns whose working directory cannot be read, or a package-manager
//! process whose directory is unknown all produce `Unknown`, and `Unknown`
//! skips (`SAFETY.md` sections 2 and 5, G3). Only a complete, readable picture
//! that contains nothing using the workspace produces `Inactive`.

use std::path::{Path, PathBuf};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::planner::{Liveness, LivenessProver};

/// Executable names that operate on a workspace directly. A process running
/// one of these whose working directory cannot be read is exactly the case
/// that must not be guessed at.
pub const PACKAGE_TOOL_NAMES: &[&str] = &["pnpm", "npm", "yarn", "bun", "node", "npx", "corepack"];

/// One inspected process. Only what the gate needs is kept: no environment,
/// no command history, nothing stored (`SAFETY.md` section 12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    /// `None` when the working directory could not be read.
    pub cwd: Option<PathBuf>,
    pub command: Vec<String>,
    pub executable: Option<PathBuf>,
    /// True when this process belongs to the user running `terminal_janitor`.
    pub own_user: bool,
}

impl ProcessInfo {
    /// True when the executable is a package manager or Node.
    pub fn is_package_tool(&self) -> bool {
        let from_executable = self
            .executable
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|stem| stem.to_str())
            .map(str::to_ascii_lowercase);
        let from_command = self
            .command
            .first()
            .map(|first| Path::new(first).to_path_buf())
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|stem| stem.to_str())
            .map(str::to_ascii_lowercase);
        [from_executable, from_command]
            .into_iter()
            .flatten()
            .any(|name| PACKAGE_TOOL_NAMES.contains(&name.as_str()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub processes: Vec<ProcessInfo>,
    /// False when the process table could not be enumerated at all. A failed
    /// enumeration protects every workspace.
    pub complete: bool,
}

impl ProcessSnapshot {
    pub fn failed() -> Self {
        Self {
            processes: Vec::new(),
            complete: false,
        }
    }
}

/// Injectable process enumeration (`AGENTS.md` section 6).
pub trait ProcessProvider {
    fn snapshot(&self) -> ProcessSnapshot;
}

/// Production enumeration through the platform's own process table. No shell,
/// no `ps`, no output parsing.
#[derive(Debug, Default)]
pub struct SystemProcessProvider;

impl ProcessProvider for SystemProcessProvider {
    fn snapshot(&self) -> ProcessSnapshot {
        let mut system = System::new();
        let refresh = ProcessRefreshKind::nothing()
            .with_cwd(UpdateKind::Always)
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always)
            .with_user(UpdateKind::Always);
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);

        let processes = system.processes();
        // An empty process table is impossible on a running machine, so it
        // means enumeration did not work rather than that nothing is running.
        if processes.is_empty() {
            return ProcessSnapshot::failed();
        }

        let own_user = system
            .process(Pid::from_u32(std::process::id()))
            .and_then(|process| process.user_id())
            .cloned();

        let collected = processes
            .iter()
            .map(|(pid, process)| ProcessInfo {
                pid: pid.as_u32(),
                cwd: process.cwd().map(Path::to_path_buf),
                command: process
                    .cmd()
                    .iter()
                    .filter_map(|argument| argument.to_str().map(str::to_owned))
                    .collect(),
                executable: process.exe().map(Path::to_path_buf),
                // With no identity of our own to compare against, every
                // process is treated as ours, which is the cautious reading.
                own_user: own_user
                    .as_ref()
                    .is_none_or(|own| process.user_id() == Some(own)),
            })
            .collect();

        ProcessSnapshot {
            processes: collected,
            complete: true,
        }
    }
}

/// The real G3 liveness prover.
pub struct ProcessLivenessProver<'a> {
    provider: &'a dyn ProcessProvider,
}

impl<'a> ProcessLivenessProver<'a> {
    pub fn new(provider: &'a dyn ProcessProvider) -> Self {
        Self { provider }
    }
}

impl LivenessProver for ProcessLivenessProver<'_> {
    fn liveness(&self, workspace: &Path) -> Liveness {
        evaluate(&self.provider.snapshot(), workspace)
    }
}

/// Decides liveness from one snapshot.
///
/// `Active` wins over `Unknown` only because it is the more useful report;
/// both refuse the workspace.
pub fn evaluate(snapshot: &ProcessSnapshot, workspace: &Path) -> Liveness {
    if !snapshot.complete {
        return Liveness::Unknown;
    }

    let mut blind_spot = false;
    let mut active = false;
    for process in &snapshot.processes {
        match &process.cwd {
            Some(cwd) if cwd.starts_with(workspace) => active = true,
            Some(_) => {}
            None => {
                // A directory we cannot read is a process we cannot rule out.
                // It matters when the process is this user's — another user's
                // daemon is not what a per-user tool can or should judge — and
                // it always matters for a package manager.
                if process.own_user || process.is_package_tool() {
                    blind_spot = true;
                }
            }
        }
        if references_workspace(&process.command, workspace) {
            active = true;
        }
    }

    if active {
        Liveness::Active
    } else if blind_spot {
        Liveness::Unknown
    } else {
        Liveness::Inactive
    }
}

/// True when any argument names the workspace or something inside it.
fn references_workspace(command: &[String], workspace: &Path) -> bool {
    let Some(workspace_text) = workspace.to_str() else {
        // A workspace path we cannot compare as text is one we cannot clear.
        return true;
    };
    command.iter().any(|argument| {
        Path::new(argument).starts_with(workspace) || argument.contains(workspace_text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(path: &str) -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        }
        #[cfg(not(windows))]
        {
            PathBuf::from(path)
        }
    }

    fn process(cwd: Option<PathBuf>, command: &[&str], own_user: bool) -> ProcessInfo {
        ProcessInfo {
            pid: 42,
            cwd,
            command: command.iter().map(|part| (*part).to_owned()).collect(),
            executable: command.first().map(PathBuf::from),
            own_user,
        }
    }

    fn snapshot(processes: Vec<ProcessInfo>) -> ProcessSnapshot {
        ProcessSnapshot {
            processes,
            complete: true,
        }
    }

    #[test]
    fn a_failed_enumeration_protects_the_workspace() {
        assert_eq!(
            evaluate(&ProcessSnapshot::failed(), &abs("/approved/api")),
            Liveness::Unknown
        );
    }

    #[test]
    fn a_working_directory_inside_the_workspace_is_active() {
        let workspace = abs("/approved/api");
        let inside = snapshot(vec![process(
            Some(workspace.join("packages/web")),
            &["node"],
            true,
        )]);
        assert_eq!(evaluate(&inside, &workspace), Liveness::Active);

        let exactly = snapshot(vec![process(Some(workspace.clone()), &["bash"], true)]);
        assert_eq!(evaluate(&exactly, &workspace), Liveness::Active);
    }

    #[test]
    fn a_command_referencing_the_workspace_is_active() {
        let workspace = abs("/approved/api");
        let elsewhere = abs("/somewhere/else");
        let referencing = snapshot(vec![process(
            Some(elsewhere),
            &["pnpm", "--dir", workspace.to_str().unwrap(), "install"],
            true,
        )]);
        assert_eq!(evaluate(&referencing, &workspace), Liveness::Active);
    }

    #[test]
    fn an_unrelated_process_leaves_the_workspace_inactive() {
        let workspace = abs("/approved/api");
        let unrelated = snapshot(vec![
            process(Some(abs("/somewhere/else")), &["node", "server.js"], true),
            process(Some(abs("/")), &["systemd"], false),
        ]);
        assert_eq!(evaluate(&unrelated, &workspace), Liveness::Inactive);
    }

    #[test]
    fn a_sibling_directory_is_not_inside_the_workspace() {
        let workspace = abs("/approved/api");
        let sibling = snapshot(vec![process(
            Some(abs("/approved/api-tools")),
            &["node"],
            true,
        )]);
        assert_eq!(evaluate(&sibling, &workspace), Liveness::Inactive);
    }

    #[test]
    fn an_unreadable_directory_on_this_users_process_is_a_blind_spot() {
        let workspace = abs("/approved/api");
        let blind = snapshot(vec![process(None, &["something"], true)]);
        assert_eq!(evaluate(&blind, &workspace), Liveness::Unknown);
    }

    #[test]
    fn another_users_unreadable_process_does_not_block_a_clean() {
        let workspace = abs("/approved/api");
        // A root daemon's directory is routinely unreadable to a per-user
        // tool. Treating that as uncertainty would mean never cleaning
        // anything on any normal machine.
        let other = snapshot(vec![process(None, &["systemd"], false)]);
        assert_eq!(evaluate(&other, &workspace), Liveness::Inactive);
    }

    #[test]
    fn an_unreadable_package_tool_is_a_blind_spot_whoever_owns_it() {
        let workspace = abs("/approved/api");
        for tool in ["pnpm", "npm", "yarn", "bun", "node", "npx", "corepack"] {
            let hidden = snapshot(vec![process(None, &[tool], false)]);
            assert_eq!(
                evaluate(&hidden, &workspace),
                Liveness::Unknown,
                "an unreadable {tool} must not be cleared"
            );
        }
    }

    #[test]
    fn an_active_process_outranks_a_blind_spot_in_the_report() {
        let workspace = abs("/approved/api");
        let both = snapshot(vec![
            process(None, &["something"], true),
            process(Some(workspace.clone()), &["node"], true),
        ]);
        assert_eq!(evaluate(&both, &workspace), Liveness::Active);
    }

    #[test]
    fn an_empty_readable_process_table_still_needs_completeness() {
        let workspace = abs("/approved/api");
        assert_eq!(
            evaluate(&snapshot(Vec::new()), &workspace),
            Liveness::Inactive
        );
        assert_eq!(
            evaluate(&ProcessSnapshot::failed(), &workspace),
            Liveness::Unknown
        );
    }

    #[test]
    fn the_real_process_table_can_be_enumerated_on_this_platform() {
        let snapshot = SystemProcessProvider.snapshot();
        assert!(snapshot.complete, "process enumeration must work");
        assert!(
            snapshot
                .processes
                .iter()
                .any(|process| process.pid == std::process::id()),
            "this test's own process must be visible"
        );
    }

    /// The platform question the G3 gate depends on: can this tool read the
    /// working directory of a process it owns? Where it cannot, every
    /// workspace is permanently `Unknown` and no workspace can ever be
    /// cleaned on that platform.
    #[test]
    fn this_platform_reports_the_working_directory_of_our_own_process() {
        let snapshot = SystemProcessProvider.snapshot();
        let own = snapshot
            .processes
            .iter()
            .find(|process| process.pid == std::process::id())
            .expect("our own process must be visible");
        assert!(
            own.cwd.is_some(),
            "this platform cannot report a working directory, so liveness can \
             never be proven here"
        );
    }
}
