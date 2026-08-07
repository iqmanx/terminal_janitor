//! Pnpm executable enrolment and version proof.
//!
//! Enrolment is the only place `terminal_janitor` decides which pnpm it will
//! ever run. It records one canonical executable path plus its volume and file
//! identity, so a later run can prove the same executable is still there rather
//! than trusting `PATH` at execution time.
//!
//! Day 3 runs exactly one pnpm command, `pnpm --version`, and it is read-only.
//! No cleanup command is executed before Day 4.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::adapters::{CommandError, CommandRequest, CommandRunner};
use crate::identity::{IdentityError, PathIdentity};
use crate::state::{Clock, PnpmEnrolmentRecord, Timestamp};

/// Workspace cleanup requires pnpm 11 or newer (`ACCEPTANCE.md` section D).
pub const MINIMUM_PNPM_MAJOR: u64 = 11;

/// The only pnpm arguments Day 3 ever passes.
pub const VERSION_ARGS: &[&str] = &["--version"];

/// A version query that has not answered within this bound is ambiguous, and
/// ambiguity fails closed.
pub const VERSION_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

/// A parsed, ordered pnpm version.
///
/// The raw text is retained because it is what the executable actually
/// reported; the parsed fields are what gates compare.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PnpmVersion {
    major: u64,
    minor: u64,
    patch: u64,
    raw: String,
}

impl PnpmVersion {
    /// Parses the output of `pnpm --version`.
    ///
    /// Accepts exactly one non-empty line holding `major.minor.patch`, with an
    /// optional `-prerelease` or `+build` suffix. Anything else — no output,
    /// extra lines such as an update notice, a leading `v`, or a partial
    /// version — is ambiguous output and is rejected rather than guessed at.
    pub fn parse(output: &str) -> Result<Self, PnpmError> {
        let mut lines = output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        let Some(line) = lines.next() else {
            return Err(PnpmError::EmptyVersionOutput);
        };
        if lines.next().is_some() {
            return Err(PnpmError::AmbiguousVersionOutput {
                output: output.trim().to_owned(),
            });
        }

        let core = line
            .split_once(['-', '+'])
            .map_or(line, |(core, _suffix)| core);
        let components: Vec<&str> = core.split('.').collect();
        let unreadable = || PnpmError::UnreadableVersion {
            output: line.to_owned(),
        };
        let [major, minor, patch] = components.as_slice() else {
            return Err(unreadable());
        };
        let (Ok(major), Ok(minor), Ok(patch)) = (
            major.parse::<u64>(),
            minor.parse::<u64>(),
            patch.parse::<u64>(),
        ) else {
            return Err(unreadable());
        };

        Ok(Self {
            major,
            minor,
            patch,
            raw: line.to_owned(),
        })
    }

    pub fn major(&self) -> u64 {
        self.major
    }

    pub fn minor(&self) -> u64 {
        self.minor
    }

    pub fn patch(&self) -> u64 {
        self.patch
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// True when this version may perform workspace cleanup.
    pub fn meets_minimum(&self) -> bool {
        self.major >= MINIMUM_PNPM_MAJOR
    }
}

impl fmt::Display for PnpmVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// One enrolled pnpm executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PnpmEnrolment {
    /// Canonical path plus volume and file identity of the executable itself.
    pub identity: PathIdentity,
    pub version: PnpmVersion,
    pub enrolled_at: Timestamp,
}

impl PnpmEnrolment {
    /// The ledger form of this enrolment.
    pub fn to_record(&self) -> PnpmEnrolmentRecord {
        PnpmEnrolmentRecord {
            identity: self.identity.clone(),
            version: self.version.as_str().to_owned(),
            enrolled_at: self.enrolled_at,
        }
    }

    /// Rebuilds an enrolment from the ledger.
    ///
    /// The stored version text is re-parsed rather than trusted, so a row that
    /// no longer means anything cannot silently grant authority.
    pub fn from_record(record: &PnpmEnrolmentRecord) -> Result<Self, PnpmError> {
        Ok(Self {
            identity: record.identity.clone(),
            version: PnpmVersion::parse(&record.version)?,
            enrolled_at: record.enrolled_at,
        })
    }
}

#[derive(Debug)]
pub enum PnpmError {
    NotFoundOnPath,
    Identity(IdentityError),
    NotAFile(PathBuf),
    /// On Windows an `npm`-installed pnpm is a `.cmd`, `.bat`, or `.ps1`
    /// wrapper that only an interpreter can run. Running one would mean
    /// invoking `cmd.exe` or PowerShell, which `SAFETY.md` section 5 forbids,
    /// so the wrapper is refused and the real executable is required.
    NotDirectlyExecutable {
        path: PathBuf,
        extension: String,
    },
    Command(CommandError),
    VersionQueryFailed {
        path: PathBuf,
        exit_code: Option<i32>,
        stderr: String,
    },
    VersionOutputTruncated(PathBuf),
    EmptyVersionOutput,
    AmbiguousVersionOutput {
        output: String,
    },
    UnreadableVersion {
        output: String,
    },
    BelowMinimumVersion {
        found: PnpmVersion,
        required_major: u64,
    },
}

impl fmt::Display for PnpmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFoundOnPath => write!(
                f,
                "no pnpm executable was found on PATH; enrol one explicitly with --pnpm <path>"
            ),
            Self::Identity(error) => error.fmt(f),
            Self::NotAFile(path) => {
                write!(f, "{} is not a regular file", path.display())
            }
            Self::NotDirectlyExecutable { path, extension } => write!(
                f,
                "{} is a .{extension} wrapper that requires an interpreter; \
                 enrol the pnpm executable itself so no shell is involved",
                path.display()
            ),
            Self::Command(error) => error.fmt(f),
            Self::VersionQueryFailed {
                path,
                exit_code,
                stderr,
            } => {
                let code = exit_code.map_or_else(|| "no exit code".to_owned(), |c| c.to_string());
                write!(
                    f,
                    "{} --version failed ({code}): {}",
                    path.display(),
                    stderr.trim()
                )
            }
            Self::VersionOutputTruncated(path) => write!(
                f,
                "{} --version produced more output than can be read safely",
                path.display()
            ),
            Self::EmptyVersionOutput => write!(f, "pnpm --version produced no output"),
            Self::AmbiguousVersionOutput { output } => {
                write!(f, "pnpm --version produced ambiguous output: {output}")
            }
            Self::UnreadableVersion { output } => {
                write!(f, "pnpm reported an unreadable version: {output}")
            }
            Self::BelowMinimumVersion {
                found,
                required_major,
            } => write!(
                f,
                "pnpm {found} is below the required major version {required_major}"
            ),
        }
    }
}

impl std::error::Error for PnpmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Identity(error) => Some(error),
            Self::Command(error) => Some(error),
            _ => None,
        }
    }
}

impl From<IdentityError> for PnpmError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}

impl From<CommandError> for PnpmError {
    fn from(error: CommandError) -> Self {
        Self::Command(error)
    }
}

/// Enrols the pnpm executable at `path`.
///
/// The path is canonicalised first, so the recorded identity is the executable
/// that will actually run rather than the spelling the user typed. The version
/// is then read from that exact canonical path.
pub fn enrol(
    path: &Path,
    runner: &dyn CommandRunner,
    clock: &dyn Clock,
) -> Result<PnpmEnrolment, PnpmError> {
    let identity = PathIdentity::resolve(path)?;
    let canonical = identity.canonical.as_path();

    let metadata = fs::symlink_metadata(canonical).map_err(|source| {
        PnpmError::Identity(IdentityError::Canonicalise {
            path: canonical.to_path_buf(),
            source,
        })
    })?;
    if !metadata.is_file() {
        return Err(PnpmError::NotAFile(canonical.to_path_buf()));
    }
    reject_interpreter_wrapper(canonical)?;

    let outcome = runner.run(&CommandRequest {
        program: canonical,
        args: VERSION_ARGS,
        working_directory: None,
        timeout: VERSION_QUERY_TIMEOUT,
    })?;
    if !outcome.succeeded() {
        return Err(PnpmError::VersionQueryFailed {
            path: canonical.to_path_buf(),
            exit_code: outcome.exit_code,
            stderr: outcome.stderr,
        });
    }
    if outcome.output_truncated {
        return Err(PnpmError::VersionOutputTruncated(canonical.to_path_buf()));
    }

    let version = PnpmVersion::parse(&outcome.stdout)?;
    if !version.meets_minimum() {
        return Err(PnpmError::BelowMinimumVersion {
            found: version,
            required_major: MINIMUM_PNPM_MAJOR,
        });
    }

    Ok(PnpmEnrolment {
        identity,
        version,
        enrolled_at: clock.now(),
    })
}

/// Extensions that only an interpreter can execute. Accepting one would put
/// `cmd.exe` or PowerShell between `terminal_janitor` and pnpm.
const INTERPRETER_WRAPPER_EXTENSIONS: &[&str] = &["cmd", "bat", "ps1", "psm1", "vbs", "js"];

fn reject_interpreter_wrapper(path: &Path) -> Result<(), PnpmError> {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return Ok(());
    };
    let lowered = extension.to_ascii_lowercase();
    if INTERPRETER_WRAPPER_EXTENSIONS.contains(&lowered.as_str()) {
        return Err(PnpmError::NotDirectlyExecutable {
            path: path.to_path_buf(),
            extension: lowered,
        });
    }
    Ok(())
}

/// Finds a candidate pnpm executable on `PATH` without invoking a shell.
///
/// `PATH` is only a hint for enrolment. What is stored, and what is later
/// revalidated, is the canonical identity of the file this finds — the
/// executable is never re-resolved through `PATH` at execution time.
pub fn locate_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for candidate_name in candidate_file_names() {
            let candidate = directory.join(candidate_name);
            match fs::symlink_metadata(&candidate) {
                Ok(metadata) if metadata.is_file() => return Some(candidate),
                // A symlinked entry is normal for a package manager; resolve it
                // and accept it only when it points at a regular file.
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if fs::metadata(&candidate).is_ok_and(|target| target.is_file()) {
                        return Some(candidate);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

#[cfg(windows)]
fn candidate_file_names() -> &'static [&'static str] {
    // Only real executable images. The .cmd and .ps1 wrappers npm installs are
    // deliberately absent; see `reject_interpreter_wrapper`.
    &["pnpm.exe", "pnpm.com"]
}

#[cfg(not(windows))]
fn candidate_file_names() -> &'static [&'static str] {
    &["pnpm"]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::CommandOutcome;
    use crate::state::FixedClock;
    use std::cell::RefCell;

    /// Program, arguments, working directory, and timeout of one invocation.
    type Invocation = (PathBuf, Vec<String>, Option<PathBuf>, Duration);

    /// Records every request so tests can assert exactly what was run.
    struct FakeRunner {
        outcome: RefCell<Result<CommandOutcome, ()>>,
        requests: RefCell<Vec<Invocation>>,
    }

    impl FakeRunner {
        fn returning(stdout: &str, exit_code: i32) -> Self {
            Self {
                outcome: RefCell::new(Ok(CommandOutcome {
                    exit_code: Some(exit_code),
                    stdout: stdout.to_owned(),
                    stderr: String::new(),
                    output_truncated: false,
                })),
                requests: RefCell::new(Vec::new()),
            }
        }

        fn truncated(stdout: &str) -> Self {
            let runner = Self::returning(stdout, 0);
            if let Ok(outcome) = runner.outcome.borrow_mut().as_mut() {
                outcome.output_truncated = true;
            }
            runner
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, request: &CommandRequest<'_>) -> Result<CommandOutcome, CommandError> {
            self.requests.borrow_mut().push((
                request.program.to_path_buf(),
                request.args.iter().map(|arg| (*arg).to_owned()).collect(),
                request.working_directory.map(Path::to_path_buf),
                request.timeout,
            ));
            self.outcome
                .borrow()
                .clone()
                .map_err(|()| CommandError::Io(std::io::Error::other("fake failure")))
        }
    }

    fn executable(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, b"fake").unwrap();
        path
    }

    #[test]
    fn plain_release_versions_parse() {
        let version = PnpmVersion::parse("11.2.3\n").unwrap();
        assert_eq!(
            (version.major(), version.minor(), version.patch()),
            (11, 2, 3)
        );
        assert_eq!(version.as_str(), "11.2.3");
        assert!(version.meets_minimum());
    }

    #[test]
    fn prerelease_and_build_suffixes_parse_to_their_release_numbers() {
        assert_eq!(PnpmVersion::parse("11.0.0-beta.1").unwrap().major(), 11);
        assert_eq!(PnpmVersion::parse("12.4.1+build.7").unwrap().minor(), 4);
    }

    #[test]
    fn version_ten_does_not_meet_the_minimum() {
        assert!(!PnpmVersion::parse("10.99.99").unwrap().meets_minimum());
    }

    #[test]
    fn unreadable_and_ambiguous_version_output_is_rejected() {
        assert!(matches!(
            PnpmVersion::parse(""),
            Err(PnpmError::EmptyVersionOutput)
        ));
        assert!(matches!(
            PnpmVersion::parse("11.2.3\nUpdate available!"),
            Err(PnpmError::AmbiguousVersionOutput { .. })
        ));
        for unreadable in ["v11.2.3", "11.2", "11.2.3.4", "eleven", "11.x.3"] {
            assert!(
                matches!(
                    PnpmVersion::parse(unreadable),
                    Err(PnpmError::UnreadableVersion { .. })
                ),
                "{unreadable} must not parse"
            );
        }
    }

    #[test]
    fn enrolment_records_canonical_identity_and_queries_only_the_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = executable(&dir, "pnpm");
        let runner = FakeRunner::returning("11.2.3\n", 0);
        let clock = FixedClock::new(1_700_000_000_000);

        let enrolment = enrol(&path, &runner, &clock).unwrap();

        assert_eq!(enrolment.version.as_str(), "11.2.3");
        assert_eq!(
            enrolment.enrolled_at,
            Timestamp::from_unix_millis(1_700_000_000_000)
        );
        let requests = runner.requests.borrow();
        assert_eq!(requests.len(), 1);
        let (program, args, working_directory, timeout) = &requests[0];
        assert_eq!(program, enrolment.identity.canonical.as_path());
        assert_eq!(args, &["--version".to_owned()]);
        assert_eq!(working_directory, &None);
        assert_eq!(*timeout, VERSION_QUERY_TIMEOUT);
    }

    #[test]
    fn enrolment_resolves_a_linked_spelling_to_the_real_executable() {
        let dir = tempfile::tempdir().unwrap();
        let real = executable(&dir, "pnpm");
        let linked_dir = dir.path().join("link-parent");
        fs::create_dir(&linked_dir).unwrap();

        let runner = FakeRunner::returning("11.2.3", 0);
        let clock = FixedClock::new(0);
        let direct = enrol(&real, &runner, &clock).unwrap();
        let through_dots = enrol(&linked_dir.join("..").join("pnpm"), &runner, &clock).unwrap();
        assert_eq!(direct.identity, through_dots.identity);
    }

    #[test]
    fn pnpm_below_eleven_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = executable(&dir, "pnpm");
        let runner = FakeRunner::returning("10.14.0", 0);
        let error = enrol(&path, &runner, &FixedClock::new(0)).unwrap_err();
        assert!(matches!(
            error,
            PnpmError::BelowMinimumVersion {
                required_major: MINIMUM_PNPM_MAJOR,
                ..
            }
        ));
    }

    #[test]
    fn a_failed_or_truncated_version_query_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = executable(&dir, "pnpm");
        let failing = FakeRunner::returning("", 1);
        assert!(matches!(
            enrol(&path, &failing, &FixedClock::new(0)).unwrap_err(),
            PnpmError::VersionQueryFailed { .. }
        ));

        let truncated = FakeRunner::truncated("11.2.3");
        assert!(matches!(
            enrol(&path, &truncated, &FixedClock::new(0)).unwrap_err(),
            PnpmError::VersionOutputTruncated(_)
        ));
    }

    #[test]
    fn interpreter_wrappers_are_refused_so_no_shell_is_involved() {
        let dir = tempfile::tempdir().unwrap();
        let runner = FakeRunner::returning("11.2.3", 0);
        for wrapper in ["pnpm.cmd", "pnpm.bat", "pnpm.ps1"] {
            let path = executable(&dir, wrapper);
            let error = enrol(&path, &runner, &FixedClock::new(0)).unwrap_err();
            assert!(
                matches!(error, PnpmError::NotDirectlyExecutable { .. }),
                "{wrapper} must be refused"
            );
        }
        assert!(runner.requests.borrow().is_empty(), "no wrapper may be run");
    }

    #[test]
    fn a_directory_is_not_an_executable() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("pnpm");
        fs::create_dir(&nested).unwrap();
        let runner = FakeRunner::returning("11.2.3", 0);
        assert!(matches!(
            enrol(&nested, &runner, &FixedClock::new(0)).unwrap_err(),
            PnpmError::NotAFile(_)
        ));
        assert!(runner.requests.borrow().is_empty());
    }

    #[test]
    fn a_missing_executable_is_an_explicit_error() {
        let dir = tempfile::tempdir().unwrap();
        let runner = FakeRunner::returning("11.2.3", 0);
        assert!(matches!(
            enrol(&dir.path().join("absent"), &runner, &FixedClock::new(0)).unwrap_err(),
            PnpmError::Identity(_)
        ));
    }
}
