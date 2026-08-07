//! Read-only Git worktree cleanliness.
//!
//! Day 2 recorded worktree state as `unknown` because it invoked nothing. Day 3
//! proves cleanliness by asking Git itself, read-only, with a fixed argument
//! array and no shell.
//!
//! Git is resolved at plan time rather than enrolled like pnpm. The difference
//! is authority: pnpm is the delegate that deletes, so its identity is stored
//! and revalidated; Git only answers a question, and an unavailable, unreadable,
//! or slow answer is treated as `Unknown`, which skips the workspace.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::adapters::{CommandRequest, CommandRunner};
use crate::state::GitState;

/// Porcelain v1 with all untracked files listed. Untracked work must count as
/// dirty (`ACCEPTANCE.md` section F), which the default summary would hide.
pub const STATUS_ARGS: &[&str] = &[
    "--no-optional-locks",
    "status",
    "--porcelain=v1",
    "--untracked-files=all",
];

/// A status query that has not answered within this bound is unknown, and
/// unknown skips.
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(60);

/// Proves whether `workspace` has a clean Git worktree.
///
/// Returns [`GitState::Unknown`] for every failure mode — missing Git, a
/// non-zero exit, a timeout, truncated output, or a workspace that is not a Git
/// worktree at all. `Unknown` is not an error to be handled later; it is the
/// answer that causes the liveness gate to skip.
pub fn worktree_state(workspace: &Path, git: &Path, runner: &dyn CommandRunner) -> GitState {
    let outcome = runner.run(&CommandRequest {
        program: git,
        args: STATUS_ARGS,
        working_directory: Some(workspace),
        timeout: STATUS_TIMEOUT,
    });
    let Ok(outcome) = outcome else {
        return GitState::Unknown;
    };
    if !outcome.succeeded() || outcome.output_truncated {
        return GitState::Unknown;
    }
    if outcome.stdout.trim().is_empty() {
        GitState::Clean
    } else {
        GitState::Dirty
    }
}

/// Finds a Git executable without invoking a shell, applying the same
/// interpreter-wrapper refusal as pnpm enrolment.
pub fn locate_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for name in candidate_file_names() {
            let candidate = directory.join(name);
            if fs::metadata(&candidate).is_ok_and(|metadata| metadata.is_file()) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
fn candidate_file_names() -> &'static [&'static str] {
    &["git.exe"]
}

#[cfg(not(windows))]
fn candidate_file_names() -> &'static [&'static str] {
    &["git"]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{CommandError, CommandOutcome};
    use std::cell::RefCell;

    /// Program, arguments, and working directory of one recorded invocation.
    type Invocation = (PathBuf, Vec<String>, Option<PathBuf>);

    struct FakeRunner {
        outcome: Result<CommandOutcome, ()>,
        requests: RefCell<Vec<Invocation>>,
    }

    impl FakeRunner {
        fn returning(stdout: &str, exit_code: i32) -> Self {
            Self {
                outcome: Ok(CommandOutcome {
                    exit_code: Some(exit_code),
                    stdout: stdout.to_owned(),
                    stderr: String::new(),
                    output_truncated: false,
                }),
                requests: RefCell::new(Vec::new()),
            }
        }

        fn failing() -> Self {
            Self {
                outcome: Err(()),
                requests: RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, request: &CommandRequest<'_>) -> Result<CommandOutcome, CommandError> {
            self.requests.borrow_mut().push((
                request.program.to_path_buf(),
                request.args.iter().map(|arg| (*arg).to_owned()).collect(),
                request.working_directory.map(Path::to_path_buf),
            ));
            self.outcome
                .clone()
                .map_err(|()| CommandError::Io(std::io::Error::other("fake failure")))
        }
    }

    fn state_for(runner: &FakeRunner) -> GitState {
        worktree_state(
            Path::new("/approved/api"),
            Path::new("/usr/bin/git"),
            runner,
        )
    }

    #[test]
    fn empty_porcelain_output_is_clean() {
        assert_eq!(state_for(&FakeRunner::returning("", 0)), GitState::Clean);
        assert_eq!(state_for(&FakeRunner::returning("\n", 0)), GitState::Clean);
    }

    #[test]
    fn modified_or_untracked_work_is_dirty() {
        assert_eq!(
            state_for(&FakeRunner::returning(" M src/main.rs\n", 0)),
            GitState::Dirty
        );
        assert_eq!(
            state_for(&FakeRunner::returning("?? notes.txt\n", 0)),
            GitState::Dirty
        );
    }

    #[test]
    fn every_failure_mode_is_unknown_rather_than_clean() {
        assert_eq!(
            state_for(&FakeRunner::returning("", 128)),
            GitState::Unknown
        );
        assert_eq!(state_for(&FakeRunner::failing()), GitState::Unknown);

        let mut truncated = FakeRunner::returning("", 0);
        if let Ok(outcome) = truncated.outcome.as_mut() {
            outcome.output_truncated = true;
        }
        assert_eq!(state_for(&truncated), GitState::Unknown);
    }

    #[test]
    fn the_query_runs_in_the_workspace_with_a_fixed_read_only_argument_array() {
        let runner = FakeRunner::returning("", 0);
        state_for(&runner);
        let requests = runner.requests.borrow();
        assert_eq!(requests.len(), 1);
        let (program, args, working_directory) = &requests[0];
        assert_eq!(program, Path::new("/usr/bin/git"));
        assert_eq!(
            args,
            &[
                "--no-optional-locks".to_owned(),
                "status".to_owned(),
                "--porcelain=v1".to_owned(),
                "--untracked-files=all".to_owned(),
            ]
        );
        assert_eq!(
            working_directory.as_deref(),
            Some(Path::new("/approved/api"))
        );
        // Nothing in the array can write: no add, commit, checkout, clean, or
        // gc, and no configuration override.
        for forbidden in ["clean", "gc", "checkout", "reset", "-c", "--exec-path"] {
            assert!(!args.iter().any(|arg| arg == forbidden));
        }
    }
}
