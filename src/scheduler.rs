//! Native per-user scheduling.
//!
//! Automatic operation is a user-level timer, never a daemon and never
//! anything that needs administrator rights (`SAFETY.md` section 11). Each
//! platform gets its own supported mechanism: a systemd user timer, a macOS
//! LaunchAgent, or a per-user Windows scheduled task.
//!
//! Three rules hold everywhere. The schedule invokes the exact installed
//! binary path, so a later `PATH` change cannot redirect it. It invokes
//! `terminal_janitor check` with an argument array and no shell pipeline.
//! Enable and disable are both idempotent, so repeating either is safe.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::adapters::{CommandRequest, CommandRunner};

/// The scheduled run happens hourly (`PLANS.md`). A no-pressure run exits in
/// milliseconds, so this is cheap.
pub const SCHEDULE_INTERVAL: &str = "hourly";

/// The only argument the schedule ever passes.
pub const SCHEDULED_ARGS: &[&str] = &["check"];

/// Scheduler commands are quick; one that hangs is a failure, not a wait.
pub const SCHEDULER_TIMEOUT: Duration = Duration::from_secs(60);

/// Stable identifier used for the unit, agent, and task names.
pub const SCHEDULE_NAME: &str = "terminal_janitor";

/// macOS reverse-DNS label for the LaunchAgent.
pub const LAUNCH_AGENT_LABEL: &str = "io.terminal-janitor.check";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScheduleReport {
    pub result: &'static str,
    pub mechanism: &'static str,
    /// Files this operation wrote or removed.
    pub files: Vec<PathBuf>,
    /// The exact binary the schedule invokes.
    pub binary: Option<PathBuf>,
    pub detail: Option<String>,
}

#[derive(Debug)]
pub enum ScheduleError {
    Unsupported,
    /// The installed binary path could not be determined or verified.
    BinaryUnavailable(String),
    HomeUnavailable,
    Write {
        path: PathBuf,
        source: io::Error,
    },
    Command {
        program: String,
        detail: String,
    },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(
                f,
                "scheduling is not supported on this platform; run terminal_janitor check yourself"
            ),
            Self::BinaryUnavailable(detail) => {
                write!(f, "the installed binary path is unusable: {detail}")
            }
            Self::HomeUnavailable => write!(f, "the user home directory is unavailable"),
            Self::Write { path, source } => {
                write!(f, "could not write {}: {source}", path.display())
            }
            Self::Command { program, detail } => write!(f, "{program} failed: {detail}"),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Where the schedule's files live and which binary it points at.
///
/// The home directory is injected so tests never write into a real profile.
#[derive(Debug, Clone)]
pub struct ScheduleContext {
    pub home: PathBuf,
    pub binary: PathBuf,
}

impl ScheduleContext {
    /// Builds the context from the running executable and the real home.
    ///
    /// The binary path is canonicalised: a schedule must name the installed
    /// file, not a symlink or a relative spelling that could later resolve
    /// somewhere else.
    pub fn resolve() -> Result<Self, ScheduleError> {
        let binary = std::env::current_exe()
            .map_err(|error| ScheduleError::BinaryUnavailable(error.to_string()))?;
        let binary = dunce::canonicalize(&binary)
            .map_err(|error| ScheduleError::BinaryUnavailable(error.to_string()))?;
        let home = directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .ok_or(ScheduleError::HomeUnavailable)?;
        Ok(Self { home, binary })
    }
}

/// Installs the schedule. Running this twice leaves the same result.
pub fn enable(
    context: &ScheduleContext,
    runner: &dyn CommandRunner,
) -> Result<ScheduleReport, ScheduleError> {
    platform::enable(context, runner)
}

/// Removes the schedule. Running this twice, or when nothing is installed,
/// succeeds quietly.
pub fn disable(
    context: &ScheduleContext,
    runner: &dyn CommandRunner,
) -> Result<ScheduleReport, ScheduleError> {
    platform::disable(context, runner)
}

/// Writes `contents` to `path`, creating parents. Rewriting an identical file
/// is what makes `enable` idempotent.
fn write_file(path: &Path, contents: &str) -> Result<(), ScheduleError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ScheduleError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, contents).map_err(|source| ScheduleError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// Removes a file, treating "already absent" as success.
fn remove_file(path: &Path) -> Result<bool, ScheduleError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ScheduleError::Write {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Runs a scheduler command with an argument array.
///
/// No shell is involved and no elevation is requested: `sudo`, `runas`, and
/// their equivalents appear nowhere in this module.
fn run(
    runner: &dyn CommandRunner,
    program: &str,
    args: &[&str],
    tolerate_failure: bool,
) -> Result<(), ScheduleError> {
    let outcome = runner
        .run(&CommandRequest {
            program: Path::new(program),
            args,
            working_directory: None,
            timeout: SCHEDULER_TIMEOUT,
        })
        .map_err(|error| ScheduleError::Command {
            program: program.to_owned(),
            detail: error.to_string(),
        })?;
    if outcome.succeeded() || tolerate_failure {
        return Ok(());
    }
    Err(ScheduleError::Command {
        program: program.to_owned(),
        detail: format!(
            "exit {}: {}",
            outcome
                .exit_code
                .map_or_else(|| "unknown".to_owned(), |code| code.to_string()),
            outcome.stderr.trim()
        ),
    })
}

// ---- Linux: systemd user timer -----------------------------------------

/// The unit text for a systemd user timer.
///
/// `ExecStart` is the canonical binary path followed by one bare argument, so
/// systemd execs it directly. There is no `/bin/sh -c`, no pipeline, and no
/// shell metacharacter anywhere in the line.
pub fn systemd_service_unit(binary: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=terminal_janitor storage threshold check\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart={} check\n",
        binary.display()
    )
}

pub fn systemd_timer_unit() -> String {
    "[Unit]\n\
     Description=Hourly terminal_janitor storage threshold check\n\
     \n\
     [Timer]\n\
     OnCalendar=hourly\n\
     Persistent=true\n\
     \n\
     [Install]\n\
     WantedBy=timers.target\n"
        .to_owned()
}

pub fn systemd_unit_directory(home: &Path) -> PathBuf {
    home.join(".config/systemd/user")
}

// ---- macOS: LaunchAgent -------------------------------------------------

/// The LaunchAgent property list.
///
/// `ProgramArguments` is an array, which is precisely why no shell is
/// involved: launchd execs the first element with the rest as arguments.
pub fn launch_agent_plist(binary: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\
         \t<string>{LAUNCH_AGENT_LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{}</string>\n\
         \t\t<string>check</string>\n\
         \t</array>\n\
         \t<key>StartInterval</key>\n\
         \t<integer>3600</integer>\n\
         \t<key>RunAtLoad</key>\n\
         \t<false/>\n\
         </dict>\n\
         </plist>\n",
        binary.display()
    )
}

pub fn launch_agent_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist"))
}

// ---- Windows: per-user scheduled task -----------------------------------

/// Arguments for creating the scheduled task.
///
/// `/RU` and `/RL` are deliberately absent: without them the task is created
/// for the current user at normal integrity, which is exactly what is wanted.
/// Nothing here requests elevation.
pub fn schtasks_create_args(binary: &Path) -> Vec<String> {
    vec![
        "/Create".to_owned(),
        "/TN".to_owned(),
        SCHEDULE_NAME.to_owned(),
        "/TR".to_owned(),
        format!("\"{}\" check", binary.display()),
        "/SC".to_owned(),
        "HOURLY".to_owned(),
        "/F".to_owned(),
    ]
}

pub fn schtasks_delete_args() -> Vec<String> {
    vec![
        "/Delete".to_owned(),
        "/TN".to_owned(),
        SCHEDULE_NAME.to_owned(),
        "/F".to_owned(),
    ]
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    pub(super) fn enable(
        context: &ScheduleContext,
        runner: &dyn CommandRunner,
    ) -> Result<ScheduleReport, ScheduleError> {
        let directory = systemd_unit_directory(&context.home);
        let service = directory.join(format!("{SCHEDULE_NAME}.service"));
        let timer = directory.join(format!("{SCHEDULE_NAME}.timer"));
        write_file(&service, &systemd_service_unit(&context.binary))?;
        write_file(&timer, &systemd_timer_unit())?;

        run(runner, "systemctl", &["--user", "daemon-reload"], false)?;
        run(
            runner,
            "systemctl",
            &["--user", "enable", "--now", "terminal_janitor.timer"],
            false,
        )?;

        Ok(ScheduleReport {
            result: "SCHEDULE_ENABLED",
            mechanism: "systemd user timer",
            files: vec![service, timer],
            binary: Some(context.binary.clone()),
            detail: None,
        })
    }

    pub(super) fn disable(
        context: &ScheduleContext,
        runner: &dyn CommandRunner,
    ) -> Result<ScheduleReport, ScheduleError> {
        // Disabling something already absent is not a failure, so systemctl's
        // complaint is tolerated while filesystem removal stays authoritative.
        run(
            runner,
            "systemctl",
            &["--user", "disable", "--now", "terminal_janitor.timer"],
            true,
        )?;

        let directory = systemd_unit_directory(&context.home);
        let service = directory.join(format!("{SCHEDULE_NAME}.service"));
        let timer = directory.join(format!("{SCHEDULE_NAME}.timer"));
        let mut removed = Vec::new();
        if remove_file(&timer)? {
            removed.push(timer);
        }
        if remove_file(&service)? {
            removed.push(service);
        }
        run(runner, "systemctl", &["--user", "daemon-reload"], true)?;

        Ok(ScheduleReport {
            result: "SCHEDULE_DISABLED",
            mechanism: "systemd user timer",
            files: removed,
            binary: None,
            detail: None,
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    pub(super) fn enable(
        context: &ScheduleContext,
        runner: &dyn CommandRunner,
    ) -> Result<ScheduleReport, ScheduleError> {
        let plist = launch_agent_path(&context.home);
        write_file(&plist, &launch_agent_plist(&context.binary))?;

        let plist_text = plist.to_string_lossy().into_owned();
        // Unloading first makes re-enabling idempotent when an older agent is
        // already registered; its failure when nothing is loaded is expected.
        run(runner, "launchctl", &["unload", &plist_text], true)?;
        run(runner, "launchctl", &["load", "-w", &plist_text], false)?;

        Ok(ScheduleReport {
            result: "SCHEDULE_ENABLED",
            mechanism: "macOS LaunchAgent",
            files: vec![plist],
            binary: Some(context.binary.clone()),
            detail: None,
        })
    }

    pub(super) fn disable(
        context: &ScheduleContext,
        runner: &dyn CommandRunner,
    ) -> Result<ScheduleReport, ScheduleError> {
        let plist = launch_agent_path(&context.home);
        let plist_text = plist.to_string_lossy().into_owned();
        run(runner, "launchctl", &["unload", "-w", &plist_text], true)?;

        let mut removed = Vec::new();
        if remove_file(&plist)? {
            removed.push(plist);
        }
        Ok(ScheduleReport {
            result: "SCHEDULE_DISABLED",
            mechanism: "macOS LaunchAgent",
            files: removed,
            binary: None,
            detail: None,
        })
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;

    pub(super) fn enable(
        context: &ScheduleContext,
        runner: &dyn CommandRunner,
    ) -> Result<ScheduleReport, ScheduleError> {
        let args = schtasks_create_args(&context.binary);
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        // /F makes creation idempotent by replacing an existing task.
        run(runner, "schtasks.exe", &borrowed, false)?;
        Ok(ScheduleReport {
            result: "SCHEDULE_ENABLED",
            mechanism: "Windows Task Scheduler",
            files: Vec::new(),
            binary: Some(context.binary.clone()),
            detail: Some(format!("per-user scheduled task {SCHEDULE_NAME}")),
        })
    }

    pub(super) fn disable(
        _context: &ScheduleContext,
        runner: &dyn CommandRunner,
    ) -> Result<ScheduleReport, ScheduleError> {
        let args = schtasks_delete_args();
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        // Deleting an absent task reports an error; that is still "disabled".
        run(runner, "schtasks.exe", &borrowed, true)?;
        Ok(ScheduleReport {
            result: "SCHEDULE_DISABLED",
            mechanism: "Windows Task Scheduler",
            files: Vec::new(),
            binary: None,
            detail: Some(format!("per-user scheduled task {SCHEDULE_NAME}")),
        })
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    pub(super) fn enable(
        _context: &ScheduleContext,
        _runner: &dyn CommandRunner,
    ) -> Result<ScheduleReport, ScheduleError> {
        Err(ScheduleError::Unsupported)
    }

    pub(super) fn disable(
        _context: &ScheduleContext,
        _runner: &dyn CommandRunner,
    ) -> Result<ScheduleReport, ScheduleError> {
        Err(ScheduleError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{CommandError, CommandOutcome};
    use std::cell::RefCell;

    struct FakeRunner {
        exit_code: i32,
        calls: RefCell<Vec<(String, Vec<String>)>>,
    }

    impl FakeRunner {
        fn ok() -> Self {
            Self {
                exit_code: 0,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn failing() -> Self {
            Self {
                exit_code: 1,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn programs(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .map(|(program, _)| program.clone())
                .collect()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, request: &CommandRequest<'_>) -> Result<CommandOutcome, CommandError> {
            self.calls.borrow_mut().push((
                request.program.to_string_lossy().into_owned(),
                request.args.iter().map(|arg| (*arg).to_owned()).collect(),
            ));
            Ok(CommandOutcome {
                exit_code: Some(self.exit_code),
                stdout: String::new(),
                stderr: "injected".to_owned(),
                output_truncated: false,
            })
        }
    }

    fn context(home: &Path) -> ScheduleContext {
        ScheduleContext {
            home: home.to_path_buf(),
            binary: home.join("bin").join("terminal_janitor"),
        }
    }

    #[test]
    fn the_systemd_unit_names_the_exact_binary_with_no_shell() {
        let binary = Path::new("/opt/terminal_janitor/bin/terminal_janitor");
        let unit = systemd_service_unit(binary);
        assert!(unit.contains("ExecStart=/opt/terminal_janitor/bin/terminal_janitor check"));
        for shell in ["/bin/sh", "-c", "bash", "|", "&&", ";"] {
            assert!(!unit.contains(shell), "{shell} must not appear in the unit");
        }
        assert!(!unit.to_lowercase().contains("sudo"));
        let timer = systemd_timer_unit();
        assert!(timer.contains("OnCalendar=hourly"));
    }

    #[test]
    fn the_launch_agent_uses_an_argument_array_with_the_exact_binary() {
        let binary = Path::new("/usr/local/bin/terminal_janitor");
        let plist = launch_agent_plist(binary);
        assert!(plist.contains("<string>/usr/local/bin/terminal_janitor</string>"));
        assert!(plist.contains("<string>check</string>"));
        assert!(plist.contains("<key>ProgramArguments</key>"));
        // A shell would have to appear as the first array element.
        for shell in ["/bin/sh", "/bin/bash", "-c"] {
            assert!(
                !plist.contains(shell),
                "{shell} must not appear in the plist"
            );
        }
        assert!(plist.contains("<integer>3600</integer>"));
    }

    #[test]
    fn the_windows_task_names_the_exact_binary_and_requests_no_elevation() {
        let binary = Path::new("C:\\Program Files\\terminal_janitor\\terminal_janitor.exe");
        let args = schtasks_create_args(binary);
        assert!(args.contains(&"/Create".to_owned()));
        assert!(args.contains(&"HOURLY".to_owned()));
        // /F is what makes a repeated enable idempotent.
        assert!(args.contains(&"/F".to_owned()));
        let target = args
            .iter()
            .find(|arg| arg.contains("terminal_janitor.exe"))
            .unwrap();
        assert!(target.ends_with(" check"));
        // Nothing may raise the task's rights or run it as another account.
        for forbidden in ["/RL", "HIGHEST", "/RU", "SYSTEM", "runas"] {
            assert!(
                !args.iter().any(|arg| arg.contains(forbidden)),
                "{forbidden} must not appear"
            );
        }
        assert!(schtasks_delete_args().contains(&"/Delete".to_owned()));
    }

    #[test]
    fn no_scheduler_command_ever_requests_elevation() {
        let home = tempfile::tempdir().unwrap();
        let runner = FakeRunner::ok();
        let _ = enable(&context(home.path()), &runner);
        let _ = disable(&context(home.path()), &runner);
        for program in runner.programs() {
            let lowered = program.to_lowercase();
            for forbidden in ["sudo", "runas", "pkexec", "osascript"] {
                assert!(
                    !lowered.contains(forbidden),
                    "{program} must not request elevation"
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn enable_is_idempotent_and_writes_both_units() {
        let home = tempfile::tempdir().unwrap();
        let context = context(home.path());
        let runner = FakeRunner::ok();

        let first = enable(&context, &runner).unwrap();
        assert_eq!(first.result, "SCHEDULE_ENABLED");
        assert_eq!(first.files.len(), 2);
        let contents: Vec<_> = first
            .files
            .iter()
            .map(|path| fs::read_to_string(path).unwrap())
            .collect();

        let second = enable(&context, &runner).unwrap();
        assert_eq!(first.files, second.files);
        for (path, before) in second.files.iter().zip(contents) {
            assert_eq!(fs::read_to_string(path).unwrap(), before);
        }
        assert_eq!(runner.programs(), vec!["systemctl"; 4]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn disable_is_idempotent_and_succeeds_when_nothing_is_installed() {
        let home = tempfile::tempdir().unwrap();
        let context = context(home.path());
        let runner = FakeRunner::ok();

        // Nothing installed yet.
        let empty = disable(&context, &runner).unwrap();
        assert_eq!(empty.result, "SCHEDULE_DISABLED");
        assert!(empty.files.is_empty());

        enable(&context, &runner).unwrap();
        let removed = disable(&context, &runner).unwrap();
        assert_eq!(removed.files.len(), 2);
        for path in &removed.files {
            assert!(!path.exists());
        }

        let again = disable(&context, &runner).unwrap();
        assert!(again.files.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_failing_enable_is_reported_rather_than_ignored() {
        let home = tempfile::tempdir().unwrap();
        let runner = FakeRunner::failing();
        let error = enable(&context(home.path()), &runner).unwrap_err();
        assert!(matches!(error, ScheduleError::Command { .. }));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_failing_disable_still_removes_the_units() {
        // systemctl complains when the timer was never enabled. Removal of the
        // files is what actually disables it, so that must still happen.
        let home = tempfile::tempdir().unwrap();
        let context = context(home.path());
        enable(&context, &FakeRunner::ok()).unwrap();

        let report = disable(&context, &FakeRunner::failing()).unwrap();
        assert_eq!(report.files.len(), 2);
        for path in &report.files {
            assert!(!path.exists());
        }
    }

    #[test]
    fn the_scheduled_arguments_are_exactly_one_bare_subcommand() {
        assert_eq!(SCHEDULED_ARGS, &["check"]);
        assert_eq!(SCHEDULE_INTERVAL, "hourly");
    }
}
