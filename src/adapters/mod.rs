//! Typed cleanup authority and the narrow command interface behind it.
//!
//! Day 3 builds plans and never executes a cleanup action. The action type and
//! the command runner live here so that the Day 4 executor cannot invent an
//! action or an argument array of its own: both are fixed at compile time.

pub mod git;
pub mod pnpm;

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::state::WorkspaceId;

/// The complete automatic action boundary from `SAFETY.md` section 3.
///
/// There is deliberately no `DeletePath`, `RunShell`, `RunConfiguredCommand`,
/// or `RemoveMatchingGlob` variant, and no variant carries a caller-supplied
/// path or command string. `PnpmWorkspaceClean` names a registered workspace by
/// ledger identity; the executor resolves that identity itself and refuses a
/// workspace whose identity no longer matches.
///
/// Extending this enum requires explicit owner approval plus matching changes
/// to `SAFETY.md`, `ACCEPTANCE.md`, and `PLANS.md`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AllowedAction {
    /// Remove `terminal_janitor`'s own expired state. Touches only the
    /// product's private state directory.
    CleanTerminalJanitorState,
    /// Delegate shared-store reclamation to pnpm. Rust never deletes store
    /// content directly (`SAFETY.md` section 5, G2).
    PnpmStorePrune,
    /// Delegate workspace reclamation to pnpm for one registered workspace.
    PnpmWorkspaceClean { workspace_id: WorkspaceId },
}

/// Argument arrays that a pnpm action is permitted to use.
///
/// `pnpm pm clean` is the explicit built-in form. The bare `pnpm clean` form is
/// never produced, because a project-defined `clean` script could be selected
/// instead of pnpm's own command (`RESEARCH.md` section 6).
pub const PNPM_WORKSPACE_CLEAN_ARGS: &[&str] = &["pm", "clean"];
pub const PNPM_STORE_PRUNE_ARGS: &[&str] = &["store", "prune"];

/// Argument spellings that must never reach pnpm (`SAFETY.md` section 7).
pub const FORBIDDEN_PNPM_ARGS: &[&str] = &["--lockfile", "-l", "--force"];

impl AllowedAction {
    /// The stable machine-readable name used in plans, journals, and JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CleanTerminalJanitorState => "CLEAN_TERMINAL_JANITOR_STATE",
            Self::PnpmStorePrune => "PNPM_STORE_PRUNE",
            Self::PnpmWorkspaceClean { .. } => "PNPM_WORKSPACE_CLEAN",
        }
    }

    /// The fixed argument array for a pnpm action, or `None` for the action
    /// that runs no command at all. The array is a compiled constant; no
    /// caller, configuration value, or model output contributes to it.
    pub fn pnpm_args(&self) -> Option<&'static [&'static str]> {
        match self {
            Self::CleanTerminalJanitorState => None,
            Self::PnpmStorePrune => Some(PNPM_STORE_PRUNE_ARGS),
            Self::PnpmWorkspaceClean { .. } => Some(PNPM_WORKSPACE_CLEAN_ARGS),
        }
    }
}

impl fmt::Display for AllowedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Maximum bytes captured from each of stdout and stderr. Output beyond this
/// is drained and discarded so a chatty child cannot fill memory or block on a
/// full pipe (`ACCEPTANCE.md` section H).
pub const MAX_CAPTURED_OUTPUT_BYTES: u64 = 64 * 1024;

/// One command invocation. The program is an exact canonical executable path
/// and the arguments are a borrowed array. There is no command string, so
/// there is nothing for a shell to re-parse even if one were involved.
#[derive(Debug, Clone)]
pub struct CommandRequest<'a> {
    pub program: &'a Path,
    pub args: &'a [&'a str],
    /// Working directory for the child. Cleanup actions set this to the exact
    /// verified workspace; enrolment leaves it unset.
    pub working_directory: Option<&'a Path>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// True when either stream reached [`MAX_CAPTURED_OUTPUT_BYTES`]. Truncated
    /// output is ambiguous output, and callers treat it as such.
    pub output_truncated: bool,
}

impl CommandOutcome {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

#[derive(Debug)]
pub enum CommandError {
    Spawn {
        program: PathBuf,
        source: io::Error,
    },
    Io(io::Error),
    /// The child did not finish within the request timeout and was killed.
    TimedOut {
        program: PathBuf,
        timeout: Duration,
    },
    /// Output was not valid UTF-8. Unreadable output is ambiguous output.
    NotUnicode {
        program: PathBuf,
        stream: &'static str,
    },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, source } => {
                write!(f, "could not run {}: {source}", program.display())
            }
            Self::Io(source) => write!(f, "command io failed: {source}"),
            Self::TimedOut { program, timeout } => write!(
                f,
                "{} did not finish within {} ms and was stopped",
                program.display(),
                timeout.as_millis()
            ),
            Self::NotUnicode { program, stream } => write!(
                f,
                "{} produced {stream} that is not valid UTF-8",
                program.display()
            ),
        }
    }
}

impl std::error::Error for CommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            Self::Io(source) => Some(source),
            _ => None,
        }
    }
}

/// Injectable command execution (`AGENTS.md` section 6). Tests supply a fake
/// runner or a generated fake executable; production uses
/// [`SystemCommandRunner`].
pub trait CommandRunner {
    fn run(&self, request: &CommandRequest<'_>) -> Result<CommandOutcome, CommandError>;
}

/// Direct process execution with an argument array.
///
/// No shell is involved: `std::process::Command` passes arguments to the
/// operating system directly, so `bash`, `sh`, `cmd.exe`, and PowerShell never
/// see the arguments and no metacharacter is ever interpreted.
#[derive(Debug, Default)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, request: &CommandRequest<'_>) -> Result<CommandOutcome, CommandError> {
        let mut command = Command::new(request.program);
        command
            .args(request.args.iter().map(OsStr::new))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = request.working_directory {
            command.current_dir(directory);
        }

        let mut child = command.spawn().map_err(|source| CommandError::Spawn {
            program: request.program.to_path_buf(),
            source,
        })?;

        // Both streams are drained on their own threads. Waiting on the child
        // while a pipe buffer is full would otherwise deadlock, and the
        // deadlock would outlive the timeout because the child never exits.
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (sender, receiver) = mpsc::channel();
        let stdout_sender = sender.clone();
        let stdout_reader = thread::spawn(move || {
            let captured = stdout
                .map(bounded_capture)
                .unwrap_or(Ok((Vec::new(), false)));
            let _ = stdout_sender.send(());
            captured
        });
        let stderr_reader = thread::spawn(move || {
            let captured = stderr
                .map(bounded_capture)
                .unwrap_or(Ok((Vec::new(), false)));
            let _ = sender.send(());
            captured
        });

        let deadline = Instant::now() + request.timeout;
        let status = loop {
            match child.try_wait().map_err(CommandError::Io)? {
                Some(status) => break status,
                None => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        // A timeout is a safety stop, not a retry. Kill the
                        // child, reap it so no zombie is left behind, and let
                        // the reader threads finish on the closed pipes.
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = stdout_reader.join();
                        let _ = stderr_reader.join();
                        return Err(CommandError::TimedOut {
                            program: request.program.to_path_buf(),
                            timeout: request.timeout,
                        });
                    }
                    // Wake on either stream closing or a short poll interval,
                    // so a fast child is not charged the full interval.
                    let _ = receiver.recv_timeout(remaining.min(Duration::from_millis(25)));
                }
            }
        };

        let (stdout_bytes, stdout_truncated) = stdout_reader
            .join()
            .map_err(|_| reader_panicked(request.program))?
            .map_err(CommandError::Io)?;
        let (stderr_bytes, stderr_truncated) = stderr_reader
            .join()
            .map_err(|_| reader_panicked(request.program))?
            .map_err(CommandError::Io)?;

        Ok(CommandOutcome {
            exit_code: status.code(),
            stdout: decode(stdout_bytes, request.program, "stdout")?,
            stderr: decode(stderr_bytes, request.program, "stderr")?,
            output_truncated: stdout_truncated || stderr_truncated,
        })
    }
}

fn reader_panicked(program: &Path) -> CommandError {
    CommandError::Spawn {
        program: program.to_path_buf(),
        source: io::Error::other("output reader thread stopped unexpectedly"),
    }
}

fn decode(bytes: Vec<u8>, program: &Path, stream: &'static str) -> Result<String, CommandError> {
    String::from_utf8(bytes).map_err(|_| CommandError::NotUnicode {
        program: program.to_path_buf(),
        stream,
    })
}

/// Reads at most [`MAX_CAPTURED_OUTPUT_BYTES`], then keeps draining into a sink
/// so the child is never blocked by a full pipe. Returns the captured bytes and
/// whether anything was discarded.
fn bounded_capture(mut stream: impl Read) -> io::Result<(Vec<u8>, bool)> {
    let mut captured = Vec::new();
    let taken = (&mut stream)
        .take(MAX_CAPTURED_OUTPUT_BYTES)
        .read_to_end(&mut captured)?;
    let mut truncated = false;
    if taken as u64 == MAX_CAPTURED_OUTPUT_BYTES {
        let discarded = io::copy(&mut stream, &mut io::sink())?;
        truncated = discarded > 0;
    }
    Ok((captured, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_clean_uses_the_explicit_built_in_pnpm_command() {
        let action = AllowedAction::PnpmWorkspaceClean {
            workspace_id: WorkspaceId::for_tests("workspace-1"),
        };
        assert_eq!(action.pnpm_args(), Some(PNPM_WORKSPACE_CLEAN_ARGS));
        assert_eq!(action.pnpm_args().unwrap(), &["pm", "clean"]);
    }

    #[test]
    fn no_action_can_produce_a_bare_clean_or_a_forbidden_argument() {
        let actions = [
            AllowedAction::CleanTerminalJanitorState,
            AllowedAction::PnpmStorePrune,
            AllowedAction::PnpmWorkspaceClean {
                workspace_id: WorkspaceId::for_tests("workspace-1"),
            },
        ];
        for action in actions {
            let Some(args) = action.pnpm_args() else {
                continue;
            };
            assert_ne!(args, &["clean"], "bare pnpm clean must never be produced");
            for forbidden in FORBIDDEN_PNPM_ARGS {
                assert!(
                    !args.contains(forbidden),
                    "{action} must never pass {forbidden}"
                );
            }
        }
    }

    #[test]
    fn state_cleanup_runs_no_command() {
        assert_eq!(AllowedAction::CleanTerminalJanitorState.pnpm_args(), None);
    }

    #[test]
    fn action_names_are_stable() {
        assert_eq!(
            AllowedAction::CleanTerminalJanitorState.as_str(),
            "CLEAN_TERMINAL_JANITOR_STATE"
        );
        assert_eq!(AllowedAction::PnpmStorePrune.as_str(), "PNPM_STORE_PRUNE");
        assert_eq!(
            AllowedAction::PnpmWorkspaceClean {
                workspace_id: WorkspaceId::for_tests("workspace-1"),
            }
            .as_str(),
            "PNPM_WORKSPACE_CLEAN"
        );
    }
}
