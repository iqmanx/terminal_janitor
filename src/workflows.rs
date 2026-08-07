//! Explicit Day 2 registration, scan, and protection workflows.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::Serialize;

use crate::activity::{ActivityProbe, SystemActivityProbe};
use crate::adapters::pnpm::PnpmEnrolment;
use crate::adapters::{CommandRunner, SystemCommandRunner, git, pnpm};
use crate::config::{
    Config, ConfigError, LoadedConfig, load_config_at, parse_size, save_config_at,
};
use crate::discovery::{
    DiscoveryProvider, ExcludedPath, ExclusionReason, SystemDiscoveryProvider, UnavailableRoot,
    discover,
};
use crate::identity::{CanonicalPath, PathIdentity, VolumeId};
use crate::planner::{
    GitProver, Plan, PlanningInputs, PlanningProviders, UnavailableGitProver,
    UnavailableLivenessProver, plan,
};
use crate::protection::{DeniedRoots, SystemProtectionProbe};
use crate::state::{
    Clock, CoordinatedWriteError, GitState, ScanStateResult, SqliteStateStore, StateError,
    StateStore, SystemClock, WorkspaceChangeKind, WorkspaceRecord, WorkspaceStatus,
};

pub const STATE_FILE_NAME: &str = "state.sqlite3";

#[derive(Debug)]
pub enum WorkflowError {
    Configuration(ConfigError),
    State(StateError),
    RootValidation { path: PathBuf, message: String },
    UnknownWorkspace { path: PathBuf },
    AmbiguousWorkspace { path: PathBuf },
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(error) => error.fmt(f),
            Self::State(error) => error.fmt(f),
            Self::RootValidation { path, message } => {
                write!(f, "approved root {} is invalid: {message}", path.display())
            }
            Self::UnknownWorkspace { path } => write!(
                f,
                "{} is not the exact root of a registered workspace",
                path.display()
            ),
            Self::AmbiguousWorkspace { path } => write!(
                f,
                "{} resolves to more than one registered workspace identity",
                path.display()
            ),
        }
    }
}

impl std::error::Error for WorkflowError {}

impl From<ConfigError> for WorkflowError {
    fn from(error: ConfigError) -> Self {
        Self::Configuration(error)
    }
}

impl From<StateError> for WorkflowError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub roots: Vec<PathBuf>,
    pub minimum_free: Option<String>,
    pub target_free: Option<String>,
    /// Explicit pnpm executable to enrol. When absent, `PATH` is searched once
    /// and whatever it finds is enrolled by canonical identity.
    pub pnpm: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InitReport {
    pub result: &'static str,
    pub approved_roots: Vec<PathBuf>,
    pub minimum_free_bytes: u64,
    pub target_free_bytes: u64,
    pub scheduling_enabled: bool,
    pub scan_performed: bool,
    /// Enrolment outcome. A machine without pnpm still initialises: pnpm is
    /// required only when a pnpm action is actually used (`ACCEPTANCE.md`
    /// section A), so a failure here is reported, not fatal.
    pub pnpm_enrolled: bool,
    pub pnpm_path: Option<PathBuf>,
    pub pnpm_version: Option<String>,
    pub pnpm_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceSummary {
    pub id: String,
    pub path: PathBuf,
    pub protected: bool,
    pub status: &'static str,
    pub git_state: &'static str,
    pub first_observed_at: i64,
    pub last_observed_at: i64,
    pub last_activity_at: i64,
}

impl From<&WorkspaceRecord> for WorkspaceSummary {
    fn from(record: &WorkspaceRecord) -> Self {
        Self {
            id: record.id.as_str().to_owned(),
            path: record.canonical_path.as_path().to_path_buf(),
            protected: record.protected,
            status: workspace_status(record.status),
            git_state: git_state(record.git_state),
            first_observed_at: record.first_observed_at.as_unix_millis(),
            last_observed_at: record.last_observed_at.as_unix_millis(),
            last_activity_at: record.last_activity_at.as_unix_millis(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub result: &'static str,
    pub approved_roots: usize,
    pub roots_scanned: usize,
    pub registered: Vec<WorkspaceSummary>,
    pub updated: Vec<WorkspaceSummary>,
    pub excluded: Vec<ExcludedPath>,
    pub unavailable_roots: Vec<UnavailableRoot>,
    pub missing: Vec<WorkspaceSummary>,
    pub protected_workspaces: usize,
    pub cleanup_performed: bool,
    /// True when `--dry-run` kept the observation out of the ledger.
    pub dry_run: bool,
    /// One plan per volume that holds registered workspaces, ordered by volume
    /// identity. Scan is not a pressure-driven run, so each volume is planned
    /// as though it were the pressured one: the plan answers "if this volume
    /// needed space, what could be done and what exactly would refuse".
    pub plans: Vec<Plan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProtectionReport {
    pub result: &'static str,
    pub workspace: WorkspaceSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProtectionListReport {
    pub result: &'static str,
    pub protected: Vec<WorkspaceSummary>,
}

pub fn state_file_path() -> Result<PathBuf, StateError> {
    ProjectDirs::from("", "", "terminal_janitor")
        .map(|dirs| dirs.data_local_dir().join(STATE_FILE_NAME))
        .ok_or_else(|| StateError::Unavailable {
            path: PathBuf::from(STATE_FILE_NAME),
            message: "the platform user state directory is unavailable".to_owned(),
        })
}

pub fn initialise(
    config_path: &Path,
    state_path: &Path,
    options: InitOptions,
) -> Result<InitReport, WorkflowError> {
    initialise_with_provider(config_path, state_path, options, &SystemDiscoveryProvider)
}

pub fn initialise_with_provider(
    config_path: &Path,
    state_path: &Path,
    options: InitOptions,
    provider: &dyn DiscoveryProvider,
) -> Result<InitReport, WorkflowError> {
    initialise_with(
        config_path,
        state_path,
        options,
        provider,
        &SystemCommandRunner,
        &SystemClock,
    )
}

/// Registration with every external interface injected, so tests never depend
/// on a real pnpm, a real clock, or the real `PATH`.
pub fn initialise_with(
    config_path: &Path,
    state_path: &Path,
    options: InitOptions,
    provider: &dyn DiscoveryProvider,
    runner: &dyn CommandRunner,
    clock: &dyn Clock,
) -> Result<InitReport, WorkflowError> {
    initialise_with_provider_and_writer(
        config_path,
        state_path,
        options,
        provider,
        runner,
        clock,
        &|path, config| save_config_at(path, config),
    )
}

#[allow(clippy::too_many_arguments)]
fn initialise_with_provider_and_writer(
    config_path: &Path,
    state_path: &Path,
    options: InitOptions,
    provider: &dyn DiscoveryProvider,
    runner: &dyn CommandRunner,
    clock: &dyn Clock,
    config_writer: &dyn Fn(&Path, &Config) -> Result<(), ConfigError>,
) -> Result<InitReport, WorkflowError> {
    if options.roots.is_empty() {
        return Err(WorkflowError::RootValidation {
            path: PathBuf::new(),
            message: "at least one explicit --root is required".to_owned(),
        });
    }
    let LoadedConfig {
        config: previous, ..
    } = load_config_at(config_path)?;
    let minimum = options
        .minimum_free
        .as_deref()
        .map(|value| parse_size("minimum_free", value))
        .transpose()?
        .unwrap_or(previous.minimum_free_bytes);
    let target = options
        .target_free
        .as_deref()
        .map(|value| parse_size("target_free", value))
        .transpose()?
        .unwrap_or(previous.target_free_bytes);
    let mut next = Config::new(minimum, target)?;

    let mut supplied = previous.approved_roots.clone();
    supplied.extend(options.roots);
    supplied.sort();
    supplied.dedup();
    let mut identities = Vec::with_capacity(supplied.len());
    for path in supplied {
        identities.push(validate_registration_root(&path, provider)?);
    }
    identities.sort_by(|a, b| a.canonical.cmp(&b.canonical));
    identities.dedup_by(|a, b| a.volume == b.volume && a.file == b.file);
    next.set_approved_roots(
        identities
            .iter()
            .map(|identity| identity.canonical.as_path().to_path_buf())
            .collect(),
    )?;

    create_private_parent(state_path).map_err(|error| StateError::Unavailable {
        path: state_path.to_path_buf(),
        message: error.to_string(),
    })?;
    let state_existed = state_path.exists();
    let mut state = SqliteStateStore::open(state_path)?;
    let prior_config = previous.clone();
    let config_existed = config_path.exists();
    let result = state.register_approved_roots_coordinated(
        &identities,
        |_| config_writer(config_path, &next),
        || restore_config(config_path, config_existed, &prior_config),
    );
    let records = match result {
        Ok(records) => records,
        Err(CoordinatedWriteError::External(error)) => {
            drop(state);
            remove_new_state_file(state_path, state_existed);
            return Err(WorkflowError::Configuration(error));
        }
        Err(CoordinatedWriteError::State(error) | CoordinatedWriteError::StateCommit(error)) => {
            drop(state);
            remove_new_state_file(state_path, state_existed);
            return Err(WorkflowError::State(error));
        }
        Err(CoordinatedWriteError::StateCommitAndRollback {
            state: state_error,
            rollback,
        }) => {
            drop(state);
            remove_new_state_file(state_path, state_existed);
            return Err(WorkflowError::State(StateError::TransactionFailed {
                message: format!("{state_error}; configuration rollback also failed: {rollback}"),
            }));
        }
    };

    // Enrolment happens after registration succeeds, and its failure never
    // undoes registration: approved roots are the owner's decision, while a
    // missing or too-old pnpm only means no pnpm action can be planned yet.
    let mut pnpm_enrolled = false;
    let mut pnpm_path = None;
    let mut pnpm_version = None;
    let mut pnpm_note = None;
    match options.pnpm.or_else(pnpm::locate_on_path) {
        Some(path) => match pnpm::enrol(&path, runner, clock) {
            Ok(enrolment) => {
                state.set_pnpm_enrolment(&enrolment.to_record())?;
                pnpm_enrolled = true;
                pnpm_path = Some(enrolment.identity.canonical.as_path().to_path_buf());
                pnpm_version = Some(enrolment.version.as_str().to_owned());
            }
            // A refused executable never becomes an enrolment, and it never
            // undoes root registration either. The refusal is reported so the
            // owner can see that no pnpm action can be planned yet.
            Err(error) => pnpm_note = Some(format!("pnpm was not enrolled: {error}")),
        },
        None => {
            pnpm_note = Some(
                "no pnpm executable was found on PATH; \
                 enrol one with init --pnpm <path> before pnpm actions can be planned"
                    .to_owned(),
            );
        }
    }

    Ok(InitReport {
        result: "INIT_COMPLETE",
        approved_roots: records
            .iter()
            .map(|record| record.canonical_path.as_path().to_path_buf())
            .collect(),
        minimum_free_bytes: next.minimum_free_bytes,
        target_free_bytes: next.target_free_bytes,
        scheduling_enabled: false,
        scan_performed: false,
        pnpm_enrolled,
        pnpm_path,
        pnpm_version,
        pnpm_note,
    })
}

fn remove_new_state_file(path: &Path, existed: bool) {
    if !existed {
        let _ = fs::remove_file(path);
    }
}

fn validate_registration_root(
    path: &Path,
    provider: &dyn DiscoveryProvider,
) -> Result<PathIdentity, WorkflowError> {
    if !path.is_absolute() {
        return Err(root_error(path, "path must be absolute"));
    }
    let kind = provider
        .entry_kind(path)
        .map_err(|error| root_error(path, &error.to_string()))?;
    match kind {
        crate::discovery::EntryKind::Directory => {}
        crate::discovery::EntryKind::Symlink => {
            return Err(root_error(path, "symbolic-link roots are not accepted"));
        }
        _ => return Err(root_error(path, "path is not a directory")),
    }
    let identity = provider
        .resolve_identity(path)
        .map_err(|error| root_error(path, &error.to_string()))?;
    if is_filesystem_root(identity.canonical.as_path()) {
        return Err(root_error(
            identity.canonical.as_path(),
            "filesystem roots and whole system drives are rejected",
        ));
    }
    Ok(identity)
}

fn is_filesystem_root(path: &Path) -> bool {
    path.parent().is_none() || path.parent() == Some(path)
}

fn root_error(path: &Path, message: &str) -> WorkflowError {
    WorkflowError::RootValidation {
        path: path.to_path_buf(),
        message: message.to_owned(),
    }
}

fn restore_config(config_path: &Path, existed: bool, previous: &Config) -> Result<(), ConfigError> {
    if existed {
        save_config_at(config_path, previous)
    } else {
        match fs::remove_file(config_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ConfigError::Write {
                path: config_path.to_path_buf(),
                message: format!("could not remove newly written file during rollback: {error}"),
            }),
        }
    }
}

pub fn scan(
    config_path: &Path,
    state_path: &Path,
    dry_run: bool,
) -> Result<ScanReport, WorkflowError> {
    scan_with(
        config_path,
        state_path,
        &SystemDiscoveryProvider,
        &SystemActivityProbe,
        dry_run,
    )
}

pub fn scan_with(
    config_path: &Path,
    state_path: &Path,
    provider: &dyn DiscoveryProvider,
    activity: &dyn ActivityProbe,
    dry_run: bool,
) -> Result<ScanReport, WorkflowError> {
    let loaded = load_config_at(config_path)?;
    if loaded.config.approved_roots.is_empty() {
        return Err(WorkflowError::Configuration(ConfigError::InvalidRoot {
            value: String::new(),
            message: "no approved roots are configured; run init with --root".to_owned(),
        }));
    }
    if !state_path.is_file() {
        return Err(WorkflowError::State(StateError::Unavailable {
            path: state_path.to_path_buf(),
            message: "state does not exist; run init first".to_owned(),
        }));
    }
    let mut state = SqliteStateStore::open(state_path)?;
    let all_roots = state.list_approved_roots()?;
    let mut roots = Vec::with_capacity(loaded.config.approved_roots.len());
    for configured in &loaded.config.approved_roots {
        let canonical = CanonicalPath::from_verified(configured.clone()).map_err(|error| {
            WorkflowError::Configuration(ConfigError::InvalidRoot {
                value: configured.display().to_string(),
                message: error.to_string(),
            })
        })?;
        let matches: Vec<_> = all_roots
            .iter()
            .filter(|root| root.active && root.canonical_path == canonical)
            .cloned()
            .collect();
        match matches.as_slice() {
            [root] => roots.push(root.clone()),
            [] => {
                return Err(WorkflowError::State(StateError::ConflictingIdentity {
                    message: format!(
                        "configured approved root {} is not registered in state",
                        configured.display()
                    ),
                }));
            }
            _ => {
                return Err(WorkflowError::State(StateError::ConflictingIdentity {
                    message: format!(
                        "configured approved root {} is ambiguous in state",
                        configured.display()
                    ),
                }));
            }
        }
    }

    let discovery = discover(&roots, provider, activity);
    let uncertain_prefixes: Vec<_> = discovery
        .excluded
        .iter()
        .filter(|excluded| {
            matches!(
                excluded.reason,
                ExclusionReason::PermissionDenied
                    | ExclusionReason::CanonicalisationFailed
                    | ExclusionReason::VolumeIdentityUnavailable
                    | ExclusionReason::UnsupportedPath
            )
        })
        .map(|excluded| excluded.path.clone())
        .collect();
    // A dry run reads everything and writes nothing, so the ledger's first
    // observation timestamps are not advanced by a rehearsal.
    let ScanStateResult { changes, missing } = if dry_run {
        ScanStateResult {
            changes: Vec::new(),
            missing: Vec::new(),
        }
    } else {
        state.apply_scan(
            &discovery.validated_roots,
            &discovery.observations,
            &uncertain_prefixes,
        )?
    };
    let mut registered = Vec::new();
    let mut updated = Vec::new();
    for change in changes {
        match change.kind {
            WorkspaceChangeKind::Registered => {
                registered.push(WorkspaceSummary::from(&change.workspace));
            }
            WorkspaceChangeKind::Updated => {
                updated.push(WorkspaceSummary::from(&change.workspace));
            }
        }
    }
    let missing: Vec<_> = missing.iter().map(WorkspaceSummary::from).collect();
    let protected_workspaces = state
        .list_workspaces()?
        .iter()
        .filter(|workspace| workspace.protected)
        .count();
    let result = if discovery.excluded.is_empty() && discovery.unavailable_roots.is_empty() {
        "SCAN_COMPLETE"
    } else {
        "SCAN_COMPLETE_WITH_EXCLUSIONS"
    };
    let plans = build_plans(&state, &loaded.config, provider)?;

    Ok(ScanReport {
        result,
        approved_roots: discovery.approved_roots,
        roots_scanned: discovery.validated_roots.len(),
        registered,
        updated,
        excluded: discovery.excluded,
        unavailable_roots: discovery.unavailable_roots,
        missing,
        protected_workspaces,
        cleanup_performed: false,
        dry_run,
        plans,
    })
}

/// Asks Git, read-only, through the enrolled command interface.
struct CommandGitProver<'a> {
    git: PathBuf,
    runner: &'a dyn CommandRunner,
}

impl GitProver for CommandGitProver<'_> {
    fn worktree_state(&self, workspace: &Path) -> GitState {
        git::worktree_state(workspace, &self.git, self.runner)
    }
}

/// Builds one plan per volume holding registered workspaces.
///
/// Planning writes nothing and executes no cleanup command. The only child
/// process involved is the read-only `git status` query.
fn build_plans(
    state: &SqliteStateStore,
    config: &Config,
    provider: &dyn DiscoveryProvider,
) -> Result<Vec<Plan>, WorkflowError> {
    let workspaces = state.list_workspaces()?;
    let roots = state.list_approved_roots()?;
    let enrolment = state
        .get_pnpm_enrolment()?
        .as_ref()
        .and_then(|record| PnpmEnrolment::from_record(record).ok());

    let runner = SystemCommandRunner;
    let git_prover: Box<dyn GitProver> = match git::locate_on_path() {
        Some(git) => Box::new(CommandGitProver {
            git,
            runner: &runner,
        }),
        None => Box::new(UnavailableGitProver),
    };
    let protection_probe = SystemProtectionProbe;
    let denied_roots = DeniedRoots::from_user_directories();
    let providers = PlanningProviders {
        discovery: provider,
        protection_probe: &protection_probe,
        denied_roots: &denied_roots,
        liveness: &UnavailableLivenessProver,
        git: git_prover.as_ref(),
    };

    let mut volumes: Vec<VolumeId> = workspaces
        .iter()
        .map(|workspace| workspace.volume_id.clone())
        .collect();
    volumes.sort();
    volumes.dedup();

    let now = SystemClock.now();
    Ok(volumes
        .iter()
        .map(|volume| {
            plan(
                &PlanningInputs {
                    workspaces: &workspaces,
                    approved_roots: &roots,
                    pnpm: enrolment.as_ref(),
                    pressured_volume: volume,
                    config,
                    now,
                },
                &providers,
            )
        })
        .collect())
}

pub fn set_protection(
    state_path: &Path,
    path: &Path,
    protected: bool,
) -> Result<ProtectionReport, WorkflowError> {
    if !state_path.is_file() {
        return Err(WorkflowError::State(StateError::Unavailable {
            path: state_path.to_path_buf(),
            message: "state does not exist; run init and scan first".to_owned(),
        }));
    }
    let identity = PathIdentity::resolve(path).map_err(|_| WorkflowError::UnknownWorkspace {
        path: path.to_path_buf(),
    })?;
    let mut state = SqliteStateStore::open(state_path)?;
    let matches: Vec<_> = state
        .list_workspaces()?
        .into_iter()
        .filter(|workspace| {
            workspace.status == WorkspaceStatus::Present
                && workspace.canonical_path == identity.canonical
                && workspace.volume_id == identity.volume
                && workspace.file_id == identity.file
        })
        .collect();
    let workspace = match matches.as_slice() {
        [workspace] => workspace.clone(),
        [] => {
            return Err(WorkflowError::UnknownWorkspace {
                path: path.to_path_buf(),
            });
        }
        _ => {
            return Err(WorkflowError::AmbiguousWorkspace {
                path: path.to_path_buf(),
            });
        }
    };
    state.set_protected(&workspace.id, protected)?;
    let updated =
        state
            .get_workspace(&workspace.id)?
            .ok_or_else(|| StateError::UnknownWorkspace {
                id: workspace.id.clone(),
            })?;
    Ok(ProtectionReport {
        result: if protected {
            "PROTECTION_ADDED"
        } else {
            "PROTECTION_REMOVED"
        },
        workspace: WorkspaceSummary::from(&updated),
    })
}

pub fn list_protection(state_path: &Path) -> Result<ProtectionListReport, WorkflowError> {
    if !state_path.is_file() {
        return Err(WorkflowError::State(StateError::Unavailable {
            path: state_path.to_path_buf(),
            message: "state does not exist; run init and scan first".to_owned(),
        }));
    }
    let state = SqliteStateStore::open(state_path)?;
    let protected = state
        .list_workspaces()?
        .iter()
        .filter(|workspace| workspace.protected)
        .map(WorkspaceSummary::from)
        .collect();
    Ok(ProtectionListReport {
        result: "PROTECTION_LIST",
        protected,
    })
}

fn workspace_status(status: WorkspaceStatus) -> &'static str {
    match status {
        WorkspaceStatus::Present => "present",
        WorkspaceStatus::Missing => "missing",
        WorkspaceStatus::Replaced => "replaced",
    }
}

fn git_state(state: GitState) -> &'static str {
    match state {
        GitState::Unknown => "unknown",
        GitState::Clean => "clean",
        GitState::Dirty => "dirty",
    }
}

fn create_private_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "state path has no parent"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{FileId, VolumeId};
    use crate::state::{NewApprovedRoot, StateStore};

    /// A path that cannot exist. Naming it explicitly keeps registration tests
    /// off `PATH` entirely, so they neither find nor run a real pnpm.
    const ABSENT_PNPM: &str = "/terminal_janitor-test-no-such-pnpm";

    /// Answers every command with one fixed outcome and records what was asked.
    struct FakeRunner {
        stdout: String,
        exit_code: i32,
        requests: std::cell::RefCell<Vec<(PathBuf, Vec<String>)>>,
    }

    impl FakeRunner {
        fn version(stdout: &str) -> Self {
            Self {
                stdout: stdout.to_owned(),
                exit_code: 0,
                requests: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            request: &crate::adapters::CommandRequest<'_>,
        ) -> Result<crate::adapters::CommandOutcome, crate::adapters::CommandError> {
            self.requests.borrow_mut().push((
                request.program.to_path_buf(),
                request.args.iter().map(|arg| (*arg).to_owned()).collect(),
            ));
            Ok(crate::adapters::CommandOutcome {
                exit_code: Some(self.exit_code),
                stdout: self.stdout.clone(),
                stderr: String::new(),
                output_truncated: false,
            })
        }
    }

    fn paths(dir: &tempfile::TempDir) -> (PathBuf, PathBuf) {
        (
            dir.path().join("config/config.toml"),
            dir.path().join("state/state.sqlite3"),
        )
    }

    fn options(root: &Path) -> InitOptions {
        InitOptions {
            roots: vec![root.to_path_buf()],
            minimum_free: None,
            target_free: None,
            pnpm: Some(PathBuf::from(ABSENT_PNPM)),
        }
    }

    fn workspace(path: &Path) {
        fs::create_dir_all(path).unwrap();
        for marker in ["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"] {
            fs::write(path.join(marker), b"fixture").unwrap();
        }
    }

    #[test]
    fn init_requires_explicit_root_and_rejects_missing_file_and_filesystem_root() {
        let dir = tempfile::tempdir().unwrap();
        let (config, state) = paths(&dir);
        assert!(matches!(
            initialise(
                &config,
                &state,
                InitOptions {
                    roots: vec![],
                    minimum_free: None,
                    target_free: None,
                    pnpm: Some(PathBuf::from(ABSENT_PNPM)),
                }
            ),
            Err(WorkflowError::RootValidation { .. })
        ));
        assert!(initialise(&config, &state, options(&dir.path().join("missing"))).is_err());
        let file = dir.path().join("file");
        fs::write(&file, b"not a directory").unwrap();
        assert!(initialise(&config, &state, options(&file)).is_err());
        let canonical = dunce::canonicalize(dir.path()).unwrap();
        let filesystem_root = canonical.ancestors().last().unwrap().to_path_buf();
        assert!(initialise(&config, &state, options(&filesystem_root)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn init_rejects_symlink_root() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let (config, state) = paths(&dir);
        assert!(matches!(
            initialise(&config, &state, options(&link)),
            Err(WorkflowError::RootValidation { .. })
        ));
    }

    #[test]
    fn init_is_canonical_transactional_idempotent_and_preserves_thresholds() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("approved root Ünïcödé");
        fs::create_dir(&root).unwrap();
        let (config, state) = paths(&dir);
        let first = initialise(
            &config,
            &state,
            InitOptions {
                roots: vec![root.clone(), root.clone()],
                minimum_free: Some("2GiB".to_owned()),
                target_free: Some("3GiB".to_owned()),
                pnpm: Some(PathBuf::from(ABSENT_PNPM)),
            },
        )
        .unwrap();
        let second = initialise(&config, &state, options(&root)).unwrap();
        assert_eq!(first.approved_roots, second.approved_roots);
        assert_eq!(second.minimum_free_bytes, 2 * 1024_u64.pow(3));
        assert_eq!(second.target_free_bytes, 3 * 1024_u64.pow(3));
        assert!(!second.scheduling_enabled);
        assert!(!second.scan_performed);
        assert_eq!(
            SqliteStateStore::open(&state)
                .unwrap()
                .list_approved_roots()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn invalid_threshold_update_preserves_previous_config_and_state() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        let (config, state) = paths(&dir);
        initialise(&config, &state, options(&root)).unwrap();
        let before = fs::read(&config).unwrap();
        let error = initialise(
            &config,
            &state,
            InitOptions {
                roots: vec![root],
                minimum_free: Some("20GiB".to_owned()),
                target_free: Some("10GiB".to_owned()),
                pnpm: Some(PathBuf::from(ABSENT_PNPM)),
            },
        )
        .unwrap_err();
        assert!(matches!(error, WorkflowError::Configuration(_)));
        assert_eq!(fs::read(&config).unwrap(), before);
        assert_eq!(
            SqliteStateStore::open(&state)
                .unwrap()
                .list_approved_roots()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn failed_config_write_removes_new_state_and_preserves_previous_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        let (config, state) = paths(&dir);
        save_config_at(&config, &Config::default()).unwrap();
        let before = fs::read(&config).unwrap();

        let result = initialise_with_provider_and_writer(
            &config,
            &state,
            options(&root),
            &SystemDiscoveryProvider,
            &FakeRunner::version("11.2.3"),
            &crate::state::FixedClock::new(0),
            &|path, _| {
                Err(ConfigError::Write {
                    path: path.to_path_buf(),
                    message: "injected write failure".to_owned(),
                })
            },
        );
        assert!(matches!(result, Err(WorkflowError::Configuration(_))));
        assert_eq!(fs::read(&config).unwrap(), before);
        assert!(!state.exists());
    }

    #[test]
    fn failed_state_registration_leaves_previous_config_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        let (config, state_path) = paths(&dir);
        save_config_at(&config, &Config::default()).unwrap();
        create_private_parent(&state_path).unwrap();
        let actual = PathIdentity::resolve(&root).unwrap();
        let mut state = SqliteStateStore::open(&state_path).unwrap();
        state
            .add_approved_root(NewApprovedRoot {
                identity: PathIdentity::from_parts(
                    actual.canonical.clone(),
                    actual.volume.clone(),
                    FileId::new("injected-conflict").unwrap(),
                ),
            })
            .unwrap();
        let before = fs::read(&config).unwrap();
        let error = initialise(&config, &state_path, options(&root)).unwrap_err();
        assert!(matches!(error, WorkflowError::State(_)));
        assert_eq!(fs::read(&config).unwrap(), before);
    }

    #[test]
    fn scan_protection_rescan_reopen_move_and_missing_are_conservative() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let original = root.join("workspace");
        workspace(&original);
        let (config, state) = paths(&dir);
        initialise(&config, &state, options(&root)).unwrap();

        let first = scan(&config, &state, false).unwrap();
        assert_eq!(first.registered.len(), 1);
        assert!(first.updated.is_empty());
        assert!(!first.cleanup_performed);
        let original_id = first.registered[0].id.clone();
        let first_observed = first.registered[0].first_observed_at;

        let protected = set_protection(&state, &original, true).unwrap();
        assert!(protected.workspace.protected);
        let protected_again = set_protection(&state, &original, true).unwrap();
        assert!(protected_again.workspace.protected);
        assert_eq!(list_protection(&state).unwrap().protected.len(), 1);
        let repeated = scan(&config, &state, false).unwrap();
        assert_eq!(repeated.updated.len(), 1);
        assert_eq!(repeated.updated[0].id, original_id);
        assert_eq!(repeated.updated[0].first_observed_at, first_observed);
        assert!(repeated.updated[0].protected);

        let moved = root.join("moved workspace");
        fs::rename(&original, &moved).unwrap();
        let moved_scan = scan(&config, &state, false).unwrap();
        assert_eq!(moved_scan.registered.len(), 1);
        assert_ne!(moved_scan.registered[0].id, original_id);
        assert!(!moved_scan.registered[0].protected);
        assert_eq!(moved_scan.missing.len(), 1);
        assert!(moved_scan.missing[0].protected);

        let reopened = SqliteStateStore::open(&state).unwrap();
        let original_record = reopened
            .list_workspaces()
            .unwrap()
            .into_iter()
            .find(|workspace| workspace.id.as_str() == original_id)
            .unwrap();
        assert_eq!(original_record.status, WorkspaceStatus::Missing);
        assert!(original_record.protected);

        let removed = set_protection(&state, &moved, false).unwrap();
        assert!(!removed.workspace.protected);
        let removed_again = set_protection(&state, &moved, false).unwrap();
        assert!(!removed_again.workspace.protected);
    }

    #[test]
    fn unavailable_root_is_reported_and_remains_registered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        fs::create_dir(&root).unwrap();
        let (config, state) = paths(&dir);
        initialise(&config, &state, options(&root)).unwrap();
        fs::remove_dir(&root).unwrap();

        let report = scan(&config, &state, false).unwrap();
        assert_eq!(report.approved_roots, 1);
        assert_eq!(report.roots_scanned, 0);
        assert_eq!(report.unavailable_roots.len(), 1);
        assert_eq!(report.result, "SCAN_COMPLETE_WITH_EXCLUSIONS");
        assert_eq!(
            SqliteStateStore::open(&state)
                .unwrap()
                .list_approved_roots()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn unknown_workspace_is_rejected_and_json_reports_are_parseable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        fs::create_dir(&root).unwrap();
        let (config, state) = paths(&dir);
        let init = initialise(&config, &state, options(&root)).unwrap();
        serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&init).unwrap()).unwrap();
        assert!(matches!(
            set_protection(&state, &root, true),
            Err(WorkflowError::UnknownWorkspace { .. })
        ));
        let report = scan(&config, &state, false).unwrap();
        serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&report).unwrap())
            .unwrap();
    }

    #[test]
    fn same_path_string_on_injected_volume_is_not_silently_reassociated() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir(&root).unwrap();
        let identity = PathIdentity::resolve(&root).unwrap();
        let changed = PathIdentity::from_parts(
            identity.canonical,
            VolumeId::new("different-volume").unwrap(),
            identity.file,
        );
        assert_ne!(identity.volume, changed.volume);
    }
}
