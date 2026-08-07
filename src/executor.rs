//! The verified executor and threshold loop.
//!
//! Nothing here decides what may be cleaned; the planner already did that. The
//! executor's job is narrower and more suspicious: hold one lock per volume,
//! prove every action is *still* legal immediately before running it, run it
//! directly with a compiled argument array, measure the volume after every
//! action, and stop the moment the target is reached or anything is unclear.
//!
//! Estimated bytes never appear here. Only a measured free-space change is
//! reported as recovery (`SAFETY.md` section 9).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::adapters::own_state::lock_file_path;
use crate::adapters::pnpm::PnpmEnrolment;
use crate::adapters::{AllowedAction, CommandRequest, CommandRunner, own_state};
use crate::config::Config;
use crate::disk::{CapacityConfidence, DiskProvider};
use crate::identity::VolumeId;
use crate::journal::{ActionOutcome, ActionState, NewAction, NewRun, RunId, RunResult};
use crate::model::DiskError;
use crate::planner::{Plan, PlanningInputs, PlanningProviders, revalidate_workspace};
use crate::state::{ApprovedRootRecord, Clock, StateError, StateStore, Timestamp, WorkspaceRecord};

/// A cleanup command that has not finished within this bound is stopped. A
/// timeout is a safety stop, never a retry.
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// At most one store prune may follow a workspace clean within a run
/// (`SAFETY.md` section 9, `ACCEPTANCE.md` section D).
pub const MAX_POST_CLEAN_STORE_PRUNES: usize = 1;

/// What the executor was asked to do.
pub struct ExecutionInputs<'a> {
    pub plan: &'a Plan,
    pub workspaces: &'a [WorkspaceRecord],
    pub approved_roots: &'a [ApprovedRootRecord],
    pub config: &'a Config,
    /// A path on the volume under pressure; capacity is measured here.
    pub volume_path: &'a Path,
    pub pressured_volume: &'a VolumeId,
    pub pnpm: Option<&'a PnpmEnrolment>,
    /// The product's private state directory, which holds the lock files.
    pub state_directory: &'a Path,
}

/// Everything external the executor touches.
pub struct ExecutionProviders<'a> {
    pub disk: &'a dyn DiskProvider,
    pub runner: &'a dyn CommandRunner,
    pub clock: &'a dyn Clock,
    /// The same providers planning used, so revalidation asks exactly the
    /// questions planning asked.
    pub planning: &'a PlanningProviders<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionReport {
    pub action: String,
    pub workspace_id: Option<String>,
    pub path: Option<PathBuf>,
    pub state: ActionState,
    pub free_before_bytes: u64,
    pub free_after_bytes: Option<u64>,
    /// Measured recovery only. An estimate never lands here.
    pub actual_bytes: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunReport {
    pub result: RunResult,
    pub run_id: Option<String>,
    pub plan_id: String,
    pub free_before_bytes: Option<u64>,
    pub free_after_bytes: Option<u64>,
    pub minimum_free_bytes: u64,
    pub target_free_bytes: u64,
    pub actions: Vec<ActionReport>,
    pub detail: Option<String>,
}

impl RunReport {
    /// Measured recovery across the whole run, or `None` when either end was
    /// not measured.
    pub fn measured_recovery_bytes(&self) -> Option<u64> {
        Some(
            self.free_after_bytes?
                .saturating_sub(self.free_before_bytes?),
        )
    }
}

#[derive(Debug)]
pub enum ExecutionError {
    State(StateError),
    Lock { path: PathBuf, source: io::Error },
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(error) => error.fmt(f),
            Self::Lock { path, source } => {
                write!(
                    f,
                    "could not prepare the lock at {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ExecutionError {}

impl From<StateError> for ExecutionError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

/// An exclusive per-volume lock held for the lifetime of a run.
///
/// The lock is an advisory file lock rather than the file's existence, so a
/// killed process releases it when the operating system closes its handles.
/// A leftover file blocks nothing.
#[derive(Debug)]
pub struct VolumeLock {
    file: fs::File,
    path: PathBuf,
}

impl VolumeLock {
    /// Takes the lock for `volume_id`, or reports that another run holds it.
    pub fn acquire(state_directory: &Path, volume_id: &str) -> Result<Option<Self>, io::Error> {
        let path = lock_file_path(state_directory, volume_id);
        if let Some(parent) = path.parent() {
            create_private_directory(parent)?;
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { file, path })),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => Err(error),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for VolumeLock {
    fn drop(&mut self) {
        // The operating system would release this when the handle closes; the
        // explicit unlock just makes the intent visible.
        let _ = self.file.unlock();
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if path.is_dir() {
        return Ok(());
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

/// Runs the threshold loop.
///
/// The order is fixed by `PLANS.md`: measure, exit above `minimum_free`, take
/// the lock, remeasure, then execute proven actions one at a time, measuring
/// after each and stopping at `target_free`.
pub fn execute(
    inputs: &ExecutionInputs<'_>,
    providers: &ExecutionProviders<'_>,
    state: &mut dyn StateStore,
) -> Result<RunReport, ExecutionError> {
    let minimum = inputs.config.minimum_free_bytes;
    let target = inputs.config.target_free_bytes;

    let free_before_lock = match measure(providers, inputs.volume_path) {
        Ok(free) => free,
        Err(error) => return Ok(measurement_failure(inputs, error)),
    };
    // Above the trigger nothing is scanned, cleaned, locked, or journalled. An
    // hourly scheduled run on a healthy machine leaves no trace beyond exiting.
    if free_before_lock >= minimum {
        return Ok(no_pressure(inputs, free_before_lock));
    }

    let lock = match VolumeLock::acquire(inputs.state_directory, inputs.pressured_volume.as_str()) {
        Ok(Some(lock)) => lock,
        Ok(None) => return Ok(already_running(inputs, free_before_lock)),
        Err(source) => {
            return Err(ExecutionError::Lock {
                path: lock_file_path(inputs.state_directory, inputs.pressured_volume.as_str()),
                source,
            });
        }
    };

    // Free space is remeasured under the lock, because whatever the other run
    // was doing has finished by now.
    let free_before = match measure(providers, inputs.volume_path) {
        Ok(free) => free,
        Err(error) => return Ok(measurement_failure(inputs, error)),
    };
    if free_before >= minimum {
        return Ok(no_pressure(inputs, free_before));
    }

    let started_at = providers.clock.now();
    let run_id = RunId::new(format!(
        "run-{}-{}",
        started_at.as_unix_millis(),
        inputs.plan.plan_id
    ));
    state.begin_run(&NewRun {
        run_id: run_id.clone(),
        plan_id: inputs.plan.plan_id.clone(),
        policy_hash: inputs.plan.policy_hash.clone(),
        volume_id: inputs.pressured_volume.as_str().to_owned(),
        free_before_bytes: free_before,
        minimum_free_bytes: minimum,
        target_free_bytes: target,
        started_at,
    })?;

    let mut loop_state = LoopState {
        free: free_before,
        sequence: 0,
        reports: Vec::new(),
        post_clean_prunes: 0,
        succeeded: 0,
        unattempted: 0,
        command_failure: None,
        snapshot_uncertain: false,
    };

    for action in &inputs.plan.actions {
        if loop_state.free >= target {
            // Everything after this point is deliberately not attempted, and
            // deliberately not journalled: it was never begun.
            loop_state.unattempted += 1;
            continue;
        }
        run_one(
            inputs,
            providers,
            state,
            &run_id,
            &action.typed_action,
            action.path.clone(),
            action.workspace_id.clone(),
            &mut loop_state,
        )?;
        if loop_state.command_failure.is_some() {
            break;
        }

        // The one permitted post-clean store prune: pnpm has just detached a
        // workspace's content, and only pnpm can decide what its store may
        // now release.
        if matches!(
            action.typed_action,
            AllowedAction::PnpmWorkspaceClean { .. }
        ) && loop_state.free < target
            && loop_state.post_clean_prunes < MAX_POST_CLEAN_STORE_PRUNES
            && inputs.pnpm.is_some()
        {
            loop_state.post_clean_prunes += 1;
            run_one(
                inputs,
                providers,
                state,
                &run_id,
                &AllowedAction::PnpmStorePrune,
                None,
                None,
                &mut loop_state,
            )?;
            if loop_state.command_failure.is_some() {
                break;
            }
        }
    }

    let result = classify(&loop_state, target);
    let finished_at = providers.clock.now();
    state.finish_run(&run_id, result, Some(loop_state.free), finished_at)?;
    drop(lock);

    Ok(RunReport {
        result,
        run_id: Some(run_id.as_str().to_owned()),
        plan_id: inputs.plan.plan_id.clone(),
        free_before_bytes: Some(free_before),
        free_after_bytes: Some(loop_state.free),
        minimum_free_bytes: minimum,
        target_free_bytes: target,
        actions: loop_state.reports,
        detail: loop_state.command_failure,
    })
}

struct LoopState {
    free: u64,
    sequence: i64,
    reports: Vec<ActionReport>,
    post_clean_prunes: usize,
    succeeded: usize,
    unattempted: usize,
    command_failure: Option<String>,
    /// True when a workspace clean was refused because the volume's free-space
    /// figure is not trustworthy.
    snapshot_uncertain: bool,
}

#[allow(clippy::too_many_arguments)]
fn run_one(
    inputs: &ExecutionInputs<'_>,
    providers: &ExecutionProviders<'_>,
    state: &mut dyn StateStore,
    run_id: &RunId,
    action: &AllowedAction,
    path: Option<PathBuf>,
    workspace_id: Option<String>,
    loop_state: &mut LoopState,
) -> Result<(), ExecutionError> {
    let free_before = loop_state.free;
    loop_state.sequence += 1;

    // Written down before anything is attempted.
    let action_id = state.record_planned_action(&NewAction {
        run_id: run_id.clone(),
        sequence: loop_state.sequence,
        action: action.as_str(),
        workspace_id: workspace_id.clone(),
        canonical_path: path.clone(),
        free_before_bytes: free_before,
        started_at: providers.clock.now(),
    })?;

    state.advance_action(action_id, ActionState::Validating)?;

    let now = providers.clock.now();

    // Where free space itself is ambiguous, a workspace clean cannot be shown
    // to have been worthwhile, so it is refused rather than attempted
    // (`ACCEPTANCE.md` section K). Own-state cleanup and the delegated store
    // prune are unaffected: neither depends on that figure being exact, and
    // neither touches a user's project.
    if matches!(action, AllowedAction::PnpmWorkspaceClean { .. })
        && providers.disk.capacity_confidence(inputs.volume_path)
            == CapacityConfidence::SnapshotUncertain
    {
        loop_state.snapshot_uncertain = true;
        return finish(
            state,
            loop_state,
            action_id,
            action,
            path,
            workspace_id,
            free_before,
            ActionState::SkippedUnknown,
            Some(
                "free space on this volume may include snapshot or purgeable \
                 space, so a workspace clean cannot be shown to help"
                    .to_owned(),
            ),
            None,
            now,
        );
    }

    if inputs.plan.is_expired(now) {
        return finish(
            state,
            loop_state,
            action_id,
            action,
            path,
            workspace_id,
            free_before,
            ActionState::SkippedChanged,
            Some("the plan expired before this action began".to_owned()),
            None,
            now,
        );
    }

    // Revalidate the workspace itself. Planning proved it was eligible; this
    // proves it still is, immediately before the command runs.
    if let AllowedAction::PnpmWorkspaceClean { workspace_id: id } = action {
        let Some(workspace) = inputs.workspaces.iter().find(|record| &record.id == id) else {
            return finish(
                state,
                loop_state,
                action_id,
                action,
                path,
                workspace_id,
                free_before,
                ActionState::SkippedChanged,
                Some("the workspace is no longer in the ledger".to_owned()),
                None,
                now,
            );
        };
        let planning_inputs = PlanningInputs {
            workspaces: inputs.workspaces,
            approved_roots: inputs.approved_roots,
            pnpm: inputs.pnpm,
            pressured_volume: inputs.pressured_volume,
            config: inputs.config,
            now,
        };
        if let Err(failure) = revalidate_workspace(workspace, &planning_inputs, providers.planning)
        {
            return finish(
                state,
                loop_state,
                action_id,
                action,
                path,
                workspace_id,
                free_before,
                failure.skip_state(),
                Some(format!("{}: {failure}", failure.as_str())),
                None,
                now,
            );
        }
    }

    state.advance_action(action_id, ActionState::Running)?;

    let outcome = perform(inputs, providers, action, path.as_deref());
    let free_after = measure(providers, inputs.volume_path).ok();
    if let Some(free) = free_after {
        loop_state.free = free;
    }
    let actual = free_after.map(|after| after.saturating_sub(free_before));
    let finished_at = providers.clock.now();

    match outcome {
        Ok(()) => {
            loop_state.succeeded += 1;
            if let AllowedAction::PnpmWorkspaceClean { workspace_id: id } = action {
                state.record_workspace_cleaned(id, finished_at)?;
            }
            finish_measured(
                state,
                loop_state,
                action_id,
                action,
                path,
                workspace_id,
                free_before,
                ActionState::Succeeded,
                None,
                recovery_note(action),
                free_after,
                actual,
                finished_at,
            )
        }
        Err(detail) => {
            // A failed, timed-out, or ambiguous command stops the run. It never
            // escalates into a broader attempt.
            loop_state.command_failure = Some(detail.clone());
            finish_measured(
                state,
                loop_state,
                action_id,
                action,
                path,
                workspace_id,
                free_before,
                ActionState::Failed,
                Some(detail),
                recovery_note(action),
                free_after,
                actual,
                finished_at,
            )
        }
    }
}

/// Performs one action. This is the only place a cleanup command is started.
fn perform(
    inputs: &ExecutionInputs<'_>,
    providers: &ExecutionProviders<'_>,
    action: &AllowedAction,
    workspace_path: Option<&Path>,
) -> Result<(), String> {
    match action {
        AllowedAction::CleanTerminalJanitorState => {
            let now = u64::try_from(providers.clock.now().as_unix_millis().max(0)).unwrap_or(0);
            own_state::remove_stale_locks(inputs.state_directory, now)
                .map(|_| ())
                .map_err(|error| format!("own-state cleanup failed: {error}"))
        }
        AllowedAction::PnpmStorePrune | AllowedAction::PnpmWorkspaceClean { .. } => {
            let Some(pnpm) = inputs.pnpm else {
                return Err("no pnpm executable is enrolled".to_owned());
            };
            let Some(args) = action.pnpm_args() else {
                return Err("this action has no pnpm argument array".to_owned());
            };
            // A workspace clean must run in the exact verified workspace; a
            // store prune has no working directory of its own.
            let working_directory = match action {
                AllowedAction::PnpmWorkspaceClean { .. } => match workspace_path {
                    Some(path) => Some(path),
                    None => return Err("the workspace path is missing".to_owned()),
                },
                _ => None,
            };
            let outcome = providers
                .runner
                .run(&CommandRequest {
                    program: pnpm.identity.canonical.as_path(),
                    args,
                    working_directory,
                    timeout: CLEANUP_TIMEOUT,
                })
                .map_err(|error| error.to_string())?;
            if outcome.output_truncated {
                return Err("pnpm produced more output than can be read safely".to_owned());
            }
            if !outcome.succeeded() {
                let code = outcome
                    .exit_code
                    .map_or_else(|| "no exit code".to_owned(), |code| code.to_string());
                return Err(format!(
                    "pnpm exited with {code}: {}",
                    outcome.stderr.trim()
                ));
            }
            Ok(())
        }
    }
}

/// What the owner would do to restore what an action removed.
fn recovery_note(action: &AllowedAction) -> Option<String> {
    match action {
        AllowedAction::CleanTerminalJanitorState => None,
        AllowedAction::PnpmStorePrune => {
            Some("pnpm refetches store content on the next install".to_owned())
        }
        AllowedAction::PnpmWorkspaceClean { .. } => {
            Some("run pnpm install in the workspace; the lockfile was never removed".to_owned())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn finish(
    state: &mut dyn StateStore,
    loop_state: &mut LoopState,
    action_id: crate::journal::ActionId,
    action: &AllowedAction,
    path: Option<PathBuf>,
    workspace_id: Option<String>,
    free_before: u64,
    action_state: ActionState,
    detail: Option<String>,
    recovery: Option<String>,
    finished_at: Timestamp,
) -> Result<(), ExecutionError> {
    finish_measured(
        state,
        loop_state,
        action_id,
        action,
        path,
        workspace_id,
        free_before,
        action_state,
        detail,
        recovery,
        None,
        None,
        finished_at,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_measured(
    state: &mut dyn StateStore,
    loop_state: &mut LoopState,
    action_id: crate::journal::ActionId,
    action: &AllowedAction,
    path: Option<PathBuf>,
    workspace_id: Option<String>,
    free_before: u64,
    action_state: ActionState,
    detail: Option<String>,
    recovery: Option<String>,
    free_after: Option<u64>,
    actual_bytes: Option<u64>,
    finished_at: Timestamp,
) -> Result<(), ExecutionError> {
    state.finish_action(
        action_id,
        &ActionOutcome {
            state: action_state,
            free_after_bytes: free_after,
            actual_bytes,
            detail: detail.clone(),
            recovery,
            finished_at,
        },
    )?;
    loop_state.reports.push(ActionReport {
        action: action.as_str().to_owned(),
        workspace_id,
        path,
        state: action_state,
        free_before_bytes: free_before,
        free_after_bytes: free_after,
        actual_bytes,
        detail,
    });
    Ok(())
}

fn classify(loop_state: &LoopState, target: u64) -> RunResult {
    if loop_state.command_failure.is_some() {
        return RunResult::FailedCommand;
    }
    if loop_state.free >= target {
        return RunResult::OkTargetRestored;
    }
    // An ambiguous capacity figure is the most specific thing that went wrong,
    // so it is reported ahead of the generic shortfall — including when the
    // ancillary actions succeeded. Own-state cleanup and a store prune
    // completing is not the news; the news is that workspace cleaning was
    // blocked and the owner cannot rely on the free-space figure
    // (`ACCEPTANCE.md` section K).
    if loop_state.snapshot_uncertain {
        return RunResult::SkippedSnapshotCapacityUncertain;
    }
    if loop_state.succeeded > 0 {
        // More proven work is waiting, so this run recovered what it safely
        // could rather than exhausting the possibilities.
        if loop_state.unattempted > 0 {
            return RunResult::OkPartialSafeReclaim;
        }
        return RunResult::ShortfallSafeActionsExhausted;
    }

    // Nothing ran. The reason matters: refusing on protection is a different
    // report from being unable to establish a fact.
    let skips: Vec<ActionState> = loop_state
        .reports
        .iter()
        .map(|report| report.state)
        .filter(|state| state.is_skip())
        .collect();
    if !skips.is_empty() {
        if skips
            .iter()
            .all(|state| *state == ActionState::SkippedProtected)
        {
            return RunResult::SkippedProtected;
        }
        if skips.iter().any(|state| {
            matches!(
                state,
                ActionState::SkippedUnknown | ActionState::SkippedActive
            )
        }) {
            return RunResult::SkippedActivityUncertain;
        }
    }
    RunResult::ShortfallSafeActionsExhausted
}

fn measure(providers: &ExecutionProviders<'_>, path: &Path) -> Result<u64, DiskError> {
    providers
        .disk
        .capacity_for(path)
        .map(|capacity| capacity.available_bytes)
}

fn no_pressure(inputs: &ExecutionInputs<'_>, free: u64) -> RunReport {
    RunReport {
        result: RunResult::OkNoPressure,
        run_id: None,
        plan_id: inputs.plan.plan_id.clone(),
        free_before_bytes: Some(free),
        free_after_bytes: Some(free),
        minimum_free_bytes: inputs.config.minimum_free_bytes,
        target_free_bytes: inputs.config.target_free_bytes,
        actions: Vec::new(),
        detail: None,
    }
}

fn already_running(inputs: &ExecutionInputs<'_>, free: u64) -> RunReport {
    RunReport {
        result: RunResult::AlreadyRunning,
        run_id: None,
        plan_id: inputs.plan.plan_id.clone(),
        free_before_bytes: Some(free),
        free_after_bytes: None,
        minimum_free_bytes: inputs.config.minimum_free_bytes,
        target_free_bytes: inputs.config.target_free_bytes,
        actions: Vec::new(),
        detail: Some("another run holds this volume's lock".to_owned()),
    }
}

fn measurement_failure(inputs: &ExecutionInputs<'_>, error: DiskError) -> RunReport {
    RunReport {
        result: RunResult::FailedStorageMeasurement,
        run_id: None,
        plan_id: inputs.plan.plan_id.clone(),
        free_before_bytes: None,
        free_after_bytes: None,
        minimum_free_bytes: inputs.config.minimum_free_bytes,
        target_free_bytes: inputs.config.target_free_bytes,
        actions: Vec::new(),
        detail: Some(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::pnpm::PnpmVersion;
    use crate::adapters::{CommandError, CommandOutcome};
    use crate::discovery::{DirectoryEntry, DiscoveryProvider, EntryKind};
    use crate::identity::{CanonicalPath, FileId, IdentityError, PathIdentity};
    use crate::journal::RunRecord;
    use crate::model::DiskCapacity;
    use crate::planner::{
        GitProver, Liveness, LivenessProver, PlanningInputs as PInputs,
        PlanningProviders as PProviders, plan,
    };
    use crate::protection::{DeniedRoots, ProtectionProbe};
    use crate::state::{FixedClock, GitState, SqliteStateStore};
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, BTreeSet};

    const DAY: i64 = 86_400_000;
    const NOW: i64 = 1_800_000_000_000;
    const VOLUME: &str = "unix-dev:66";
    const MINIMUM: u64 = 1_000;
    const TARGET: u64 = 2_000;

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

    /// Free space that the fake pnpm moves as it "works", so the loop's
    /// measurements are driven by the actions rather than by the test.
    #[derive(Default)]
    struct Volume {
        free: Cell<u64>,
    }

    struct StepDisk<'a> {
        volume: &'a Volume,
        failing: bool,
        confidence: CapacityConfidence,
    }

    impl<'a> StepDisk<'a> {
        fn new(volume: &'a Volume) -> Self {
            Self {
                volume,
                failing: false,
                confidence: CapacityConfidence::Confident,
            }
        }

        fn failing(volume: &'a Volume) -> Self {
            Self {
                failing: true,
                ..Self::new(volume)
            }
        }

        /// Models what macOS reports: a figure that may include snapshot or
        /// purgeable space.
        fn snapshot_uncertain(volume: &'a Volume) -> Self {
            Self {
                confidence: CapacityConfidence::SnapshotUncertain,
                ..Self::new(volume)
            }
        }
    }

    impl DiskProvider for StepDisk<'_> {
        fn capacity_for(&self, path: &Path) -> Result<DiskCapacity, DiskError> {
            if self.failing {
                return Err(DiskError::PathNotFound(path.to_path_buf()));
            }
            DiskCapacity::new(10_000, self.volume.free.get())
        }

        fn capacity_confidence(&self, _path: &Path) -> CapacityConfidence {
            self.confidence
        }
    }

    /// Records each invocation and adds a fixed amount of free space when a
    /// cleanup command succeeds.
    /// Program, arguments, working directory, and timeout of one invocation.
    type Invocation = (PathBuf, Vec<String>, Option<PathBuf>, Duration);

    struct FakeRunner<'a> {
        volume: &'a Volume,
        gain: u64,
        exit_code: i32,
        error: bool,
        calls: RefCell<Vec<Invocation>>,
    }

    impl<'a> FakeRunner<'a> {
        fn new(volume: &'a Volume, gain: u64) -> Self {
            Self {
                volume,
                gain,
                exit_code: 0,
                error: false,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn failing(volume: &'a Volume) -> Self {
            Self {
                exit_code: 1,
                ..Self::new(volume, 0)
            }
        }

        fn erroring(volume: &'a Volume) -> Self {
            Self {
                error: true,
                ..Self::new(volume, 0)
            }
        }

        fn invocations(&self) -> Vec<(Vec<String>, Option<PathBuf>)> {
            self.calls
                .borrow()
                .iter()
                .map(|(_, args, cwd, _)| (args.clone(), cwd.clone()))
                .collect()
        }
    }

    impl CommandRunner for FakeRunner<'_> {
        fn run(&self, request: &CommandRequest<'_>) -> Result<CommandOutcome, CommandError> {
            self.calls.borrow_mut().push((
                request.program.to_path_buf(),
                request.args.iter().map(|arg| (*arg).to_owned()).collect(),
                request.working_directory.map(Path::to_path_buf),
                request.timeout,
            ));
            if self.error {
                return Err(CommandError::TimedOut {
                    program: request.program.to_path_buf(),
                    timeout: request.timeout,
                });
            }
            if self.exit_code == 0 {
                self.volume
                    .free
                    .set(self.volume.free.get().saturating_add(self.gain));
            }
            Ok(CommandOutcome {
                exit_code: Some(self.exit_code),
                stdout: String::new(),
                stderr: "injected failure".to_owned(),
                output_truncated: false,
            })
        }
    }

    /// Models another process filling the volume while this run proceeds.
    struct ShrinkingRunner<'a> {
        volume: &'a Volume,
        loss: u64,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl CommandRunner for ShrinkingRunner<'_> {
        fn run(&self, request: &CommandRequest<'_>) -> Result<CommandOutcome, CommandError> {
            self.calls
                .borrow_mut()
                .push(request.args.iter().map(|arg| (*arg).to_owned()).collect());
            self.volume
                .free
                .set(self.volume.free.get().saturating_sub(self.loss));
            Ok(CommandOutcome {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                output_truncated: false,
            })
        }
    }

    struct FakeDiscovery {
        identities: BTreeMap<PathBuf, PathIdentity>,
        files: BTreeSet<PathBuf>,
    }

    impl DiscoveryProvider for FakeDiscovery {
        fn entry_kind(&self, path: &Path) -> io::Result<EntryKind> {
            if self.files.contains(path) {
                Ok(EntryKind::File)
            } else {
                Err(io::Error::from(io::ErrorKind::NotFound))
            }
        }

        fn read_directory(&self, _path: &Path) -> io::Result<Vec<DirectoryEntry>> {
            Ok(Vec::new())
        }

        fn resolve_identity(&self, path: &Path) -> Result<PathIdentity, IdentityError> {
            self.identities
                .get(path)
                .cloned()
                .ok_or_else(|| IdentityError::NotAbsolute(path.to_path_buf()))
        }
    }

    struct NoMarkers;

    impl ProtectionProbe for NoMarkers {
        fn entry_exists(&self, _path: &Path) -> bool {
            false
        }
    }

    struct FixedLiveness(Liveness);

    impl LivenessProver for FixedLiveness {
        fn liveness(&self, _workspace: &Path) -> Liveness {
            self.0
        }
    }

    struct FixedGit(GitState);

    impl GitProver for FixedGit {
        fn worktree_state(&self, _workspace: &Path) -> GitState {
            self.0
        }
    }

    fn identity(path: &Path, file: &str) -> PathIdentity {
        PathIdentity::from_parts(
            CanonicalPath::from_verified(path.to_path_buf()).unwrap(),
            VolumeId::new(VOLUME).unwrap(),
            FileId::new(file).unwrap(),
        )
    }

    fn enrolment(path: &Path) -> PnpmEnrolment {
        PnpmEnrolment {
            identity: identity(path, "pnpm-file"),
            version: PnpmVersion::parse("11.2.3").unwrap(),
            enrolled_at: Timestamp::from_unix_millis(0),
        }
    }

    /// One assembled world: a state directory, a ledger, two workspaces, an
    /// enrolled pnpm, and injected providers.
    struct World {
        _dir: tempfile::TempDir,
        state_directory: PathBuf,
        state_path: PathBuf,
        workspaces: Vec<WorkspaceRecord>,
        roots: Vec<ApprovedRootRecord>,
        pnpm: PnpmEnrolment,
        config: Config,
        discovery: FakeDiscovery,
    }

    impl World {
        /// Registers the root and workspaces in a real ledger, long enough ago
        /// to satisfy the observation window, then reads the records back. The
        /// executor writes cooldowns to these rows, so they must be real rows.
        fn new(workspace_count: usize) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let state_directory = dir.path().to_path_buf();
            let state_path = state_directory.join("state.sqlite3");
            let root = abs("/approved");
            let pnpm_path = abs("/usr/local/bin/pnpm");

            let mut identities = BTreeMap::new();
            let mut files = BTreeSet::new();
            let mut names = Vec::new();
            for index in 0..workspace_count {
                let name = format!("workspace-{index}");
                let path = root.join(&name);
                identities.insert(path.clone(), identity(&path, &name));
                for marker in crate::planner::REQUIRED_MARKERS {
                    files.insert(path.join(marker));
                }
                names.push((name, path));
            }
            identities.insert(pnpm_path.clone(), identity(&pnpm_path, "pnpm-file"));

            // Registration happens at an old clock so the workspaces are past
            // the observation window by the time the run starts.
            let clock = FixedClock::new(NOW - 120 * DAY);
            let (roots, workspaces) = {
                let mut store =
                    SqliteStateStore::open_with_clock(&state_path, Box::new(clock.clone()))
                        .unwrap();
                let root_record = store
                    .add_approved_root(crate::state::NewApprovedRoot {
                        identity: identity(&root, "root-file"),
                    })
                    .unwrap();
                for (name, path) in &names {
                    store
                        .upsert_workspace_observation(crate::state::WorkspaceObservation {
                            identity: identity(path, name),
                            approved_root_id: root_record.id.clone(),
                            activity_at: Some(Timestamp::from_unix_millis(NOW - 90 * DAY)),
                            git_state: GitState::Unknown,
                            git_head_fingerprint: None,
                            git_index_fingerprint: None,
                        })
                        .unwrap();
                }
                store
                    .set_pnpm_enrolment(&enrolment(&pnpm_path).to_record())
                    .unwrap();
                (
                    store.list_approved_roots().unwrap(),
                    store.list_workspaces().unwrap(),
                )
            };

            Self {
                _dir: dir,
                state_directory,
                state_path,
                workspaces,
                roots,
                pnpm: enrolment(&pnpm_path),
                config: Config::new(MINIMUM, TARGET).unwrap(),
                discovery: FakeDiscovery { identities, files },
            }
        }

        fn store(&self) -> SqliteStateStore {
            SqliteStateStore::open_with_clock(&self.state_path, Box::new(FixedClock::new(NOW)))
                .unwrap()
        }
    }

    struct Injected<'a> {
        probe: NoMarkers,
        denied: DeniedRoots,
        liveness: FixedLiveness,
        git: FixedGit,
        world: &'a World,
    }

    impl<'a> Injected<'a> {
        fn new(world: &'a World, liveness: Liveness, git: GitState) -> Self {
            Self {
                probe: NoMarkers,
                denied: DeniedRoots::default(),
                liveness: FixedLiveness(liveness),
                git: FixedGit(git),
                world,
            }
        }

        fn planning(&self) -> PProviders<'_> {
            PProviders {
                discovery: &self.world.discovery,
                protection_probe: &self.probe,
                denied_roots: &self.denied,
                liveness: &self.liveness,
                git: &self.git,
            }
        }
    }

    fn build_plan(world: &World, planning: &PProviders<'_>) -> Plan {
        plan(
            &PInputs {
                workspaces: &world.workspaces,
                approved_roots: &world.roots,
                pnpm: Some(&world.pnpm),
                pressured_volume: &VolumeId::new(VOLUME).unwrap(),
                config: &world.config,
                now: Timestamp::from_unix_millis(NOW),
            },
            planning,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        world: &World,
        plan: &Plan,
        planning: &PProviders<'_>,
        disk: &dyn DiskProvider,
        runner: &dyn CommandRunner,
        state: &mut dyn StateStore,
    ) -> RunReport {
        let clock = FixedClock::new(NOW);
        execute(
            &ExecutionInputs {
                plan,
                workspaces: &world.workspaces,
                approved_roots: &world.roots,
                config: &world.config,
                volume_path: &world.state_directory,
                pressured_volume: &VolumeId::new(VOLUME).unwrap(),
                pnpm: Some(&world.pnpm),
                state_directory: &world.state_directory,
            },
            &ExecutionProviders {
                disk,
                runner,
                clock: &clock,
                planning,
            },
            state,
        )
        .unwrap()
    }

    fn journal(world: &World) -> Vec<RunRecord> {
        world.store().list_runs(50).unwrap()
    }

    #[test]
    fn a_healthy_volume_does_nothing_at_all() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume {
            free: Cell::new(MINIMUM),
        };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        assert_eq!(report.result, RunResult::OkNoPressure);
        assert!(report.actions.is_empty());
        assert!(runner.invocations().is_empty(), "nothing may be invoked");
        assert!(journal(&world).is_empty(), "a no-op run leaves no journal");
    }

    #[test]
    fn a_pressured_volume_stops_the_moment_the_target_is_reached() {
        let world = World::new(2);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        assert_eq!(plan.workspace_clean_count(), 2);

        let volume = Volume {
            free: Cell::new(500),
        };
        let disk = StepDisk::new(&volume);
        // The first cleanup alone carries the volume past the target.
        let runner = FakeRunner::new(&volume, TARGET);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        assert_eq!(report.result, RunResult::OkTargetRestored);
        assert!(report.free_after_bytes.unwrap() >= TARGET);
        assert_eq!(
            runner.invocations().len(),
            1,
            "later actions must not run once the target is reached"
        );
    }

    #[test]
    fn a_workspace_clean_uses_the_built_in_command_in_the_exact_workspace() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume {
            free: Cell::new(500),
        };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::new(&volume, 100);
        let mut store = world.store();

        run(&world, &plan, &planning, &disk, &runner, &mut store);

        let calls = runner.calls.borrow();
        let workspace_path = world.workspaces[0].canonical_path.as_path();
        let clean = calls
            .iter()
            .find(|(_, args, _, _)| args == &["pm".to_owned(), "clean".to_owned()])
            .expect("pnpm pm clean must be used");
        assert_eq!(clean.0, world.pnpm.identity.canonical.as_path());
        assert_eq!(clean.2.as_deref(), Some(workspace_path));
        assert_eq!(clean.3, CLEANUP_TIMEOUT);

        for (program, args, _, _) in calls.iter() {
            assert_eq!(program, world.pnpm.identity.canonical.as_path());
            assert_ne!(args, &["clean".to_owned()], "bare pnpm clean is forbidden");
            for forbidden in ["--lockfile", "-l", "--force"] {
                assert!(!args.iter().any(|arg| arg == forbidden));
            }
            for shell in ["sh", "bash", "cmd.exe", "powershell"] {
                assert!(!program.to_string_lossy().contains(shell));
            }
        }
    }

    #[test]
    fn one_store_prune_may_follow_a_workspace_clean_and_no_more() {
        let world = World::new(2);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        // Nothing recovers space, so the loop runs to exhaustion and the cap
        // on post-clean prunes is what limits them.
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        run(&world, &plan, &planning, &disk, &runner, &mut store);

        let prunes = runner
            .invocations()
            .iter()
            .filter(|(args, _)| args == &["store".to_owned(), "prune".to_owned()])
            .count();
        // One planned prune plus at most one post-clean exception per run.
        assert_eq!(prunes, 1 + MAX_POST_CLEAN_STORE_PRUNES);
    }

    #[test]
    fn a_failing_command_stops_the_run_without_trying_anything_else() {
        let world = World::new(2);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::failing(&volume);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        assert_eq!(report.result, RunResult::FailedCommand);
        assert_eq!(
            runner.invocations().len(),
            1,
            "a failed command must not be followed by another"
        );
        let states: Vec<_> = report.actions.iter().map(|action| action.state).collect();
        assert!(states.contains(&ActionState::Failed));
    }

    #[test]
    fn a_timed_out_command_stops_the_run_safely() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::erroring(&volume);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        assert_eq!(report.result, RunResult::FailedCommand);
        assert!(report.detail.unwrap().contains("did not finish"));
    }

    #[test]
    fn a_second_run_on_the_same_volume_is_refused_without_doing_work() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        let held = VolumeLock::acquire(&world.state_directory, VOLUME)
            .unwrap()
            .expect("the first lock must be granted");

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        assert_eq!(report.result, RunResult::AlreadyRunning);
        assert!(runner.invocations().is_empty());
        assert!(journal(&world).is_empty());

        drop(held);
        // Once released, the same volume can be locked again.
        assert!(
            VolumeLock::acquire(&world.state_directory, VOLUME)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_workspace_that_became_active_is_skipped_rather_than_cleaned() {
        let world = World::new(1);
        let planned = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let plan = build_plan(&world, &planned.planning());

        // Between planning and execution, something starts using it.
        let now_active = Injected::new(&world, Liveness::Active, GitState::Clean);
        let planning = now_active.planning();
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        let clean = report
            .actions
            .iter()
            .find(|action| action.action == "PNPM_WORKSPACE_CLEAN")
            .unwrap();
        assert_eq!(clean.state, ActionState::SkippedActive);
        assert!(
            !runner
                .invocations()
                .iter()
                .any(|(args, _)| args == &["pm".to_owned(), "clean".to_owned()]),
            "an active workspace must never be cleaned"
        );
    }

    #[test]
    fn a_workspace_whose_worktree_became_dirty_is_skipped() {
        let world = World::new(1);
        let planned = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let plan = build_plan(&world, &planned.planning());

        let now_dirty = Injected::new(&world, Liveness::Inactive, GitState::Dirty);
        let planning = now_dirty.planning();
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        let clean = report
            .actions
            .iter()
            .find(|action| action.action == "PNPM_WORKSPACE_CLEAN")
            .unwrap();
        assert_eq!(clean.state, ActionState::SkippedChanged);
    }

    /// Revalidation exists for exactly this case: the path still spells the
    /// same thing, but a different physical directory is behind it now.
    /// Identity carries the authority, never the spelling.
    #[test]
    fn a_workspace_replaced_after_planning_is_skipped_rather_than_cleaned() {
        let world = World::new(1);
        let planned = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let plan = build_plan(&world, &planned.planning());

        let path = world.workspaces[0].canonical_path.as_path().to_path_buf();
        let mut identities = world.discovery.identities.clone();
        identities.insert(path.clone(), identity(&path, "a-different-directory"));
        let replaced = FakeDiscovery {
            identities,
            files: world.discovery.files.clone(),
        };
        let planning = PProviders {
            discovery: &replaced,
            protection_probe: &planned.probe,
            denied_roots: &planned.denied,
            liveness: &planned.liveness,
            git: &planned.git,
        };

        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        let clean = report
            .actions
            .iter()
            .find(|action| action.action == "PNPM_WORKSPACE_CLEAN")
            .unwrap();
        assert_eq!(clean.state, ActionState::SkippedChanged);
        assert!(
            !runner
                .invocations()
                .iter()
                .any(|(args, _)| args == &["pm".to_owned(), "clean".to_owned()]),
            "a replaced directory must never be cleaned"
        );
    }

    /// The volume can move underneath a run: another process may fill it while
    /// this one works. That must not invent recovery, wrap a subtraction, or
    /// grant an action the plan did not contain.
    #[test]
    fn a_volume_that_shrinks_during_a_run_invents_no_recovery() {
        let world = World::new(2);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);

        let volume = Volume {
            free: Cell::new(500),
        };
        let disk = StepDisk::new(&volume);
        let runner = ShrinkingRunner {
            volume: &volume,
            loss: 100,
            calls: RefCell::new(Vec::new()),
        };
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        assert_eq!(report.result, RunResult::ShortfallSafeActionsExhausted);
        assert_eq!(
            report.measured_recovery_bytes(),
            Some(0),
            "a shrinking volume recovers nothing, never a wrapped figure"
        );
        for action in &report.actions {
            assert!(
                action.actual_bytes.is_none_or(|actual| actual == 0),
                "{} claimed {:?} bytes back while the volume was shrinking",
                action.action,
                action.actual_bytes
            );
        }

        let calls = runner.calls.borrow();
        assert!(
            calls.len() <= plan.actions.len(),
            "no action beyond the plan may run: {calls:?}"
        );
        assert!(
            calls.iter().all(|args| {
                args == &["pm".to_owned(), "clean".to_owned()]
                    || args == &["store".to_owned(), "prune".to_owned()]
            }),
            "only the compiled argument arrays may run: {calls:?}"
        );
    }

    #[test]
    fn an_expired_plan_executes_nothing() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let mut plan = build_plan(&world, &planning);
        // The plan describes a machine that was measured long ago.
        plan.expires_at_millis = NOW - 1;

        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        assert!(
            runner.invocations().is_empty(),
            "an expired plan runs nothing"
        );
        assert!(
            report
                .actions
                .iter()
                .all(|action| action.state == ActionState::SkippedChanged)
        );
    }

    #[test]
    fn a_failed_measurement_stops_before_any_action() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::failing(&volume);
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        assert_eq!(report.result, RunResult::FailedStorageMeasurement);
        assert!(runner.invocations().is_empty());
    }

    #[test]
    fn the_journal_records_every_action_with_measured_figures() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::new(&volume, 300);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);

        let runs = journal(&world);
        assert_eq!(runs.len(), 1);
        let record = &runs[0];
        assert!(!record.interrupted());
        assert!(record.unresolved_actions().is_empty());
        assert_eq!(record.result, Some(report.result));
        assert_eq!(record.plan_id, plan.plan_id);
        assert_eq!(record.policy_hash, plan.policy_hash);
        assert_eq!(record.free_before_bytes, 0);
        assert_eq!(record.actions.len(), report.actions.len());

        let succeeded: Vec<_> = record
            .actions
            .iter()
            .filter(|action| action.state == ActionState::Succeeded)
            .collect();
        assert!(!succeeded.is_empty());
        for action in succeeded {
            assert!(action.free_after_bytes.is_some());
            assert!(action.actual_bytes.is_some());
            assert!(action.finished_at_millis.is_some());
        }
    }

    #[test]
    fn a_successful_clean_starts_the_workspace_cooldown() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        let runner = FakeRunner::new(&volume, 10);
        let mut store = world.store();

        assert_eq!(world.workspaces[0].last_cleaned_at, None);
        run(&world, &plan, &planning, &disk, &runner, &mut store);

        let cleaned = world
            .store()
            .get_workspace(&world.workspaces[0].id)
            .unwrap()
            .expect("the workspace must still be in the ledger");
        assert_eq!(
            cleaned.last_cleaned_at,
            Some(Timestamp::from_unix_millis(NOW)),
            "a successful clean must start the cooldown"
        );
    }

    #[test]
    fn an_ambiguous_capacity_figure_refuses_workspace_cleans_but_not_the_rest() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        assert_eq!(plan.workspace_clean_count(), 1);

        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::snapshot_uncertain(&volume);
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);

        let clean = report
            .actions
            .iter()
            .find(|action| action.action == "PNPM_WORKSPACE_CLEAN")
            .expect("the clean must be journalled, not silently dropped");
        assert_eq!(clean.state, ActionState::SkippedUnknown);
        assert!(clean.detail.as_ref().unwrap().contains("snapshot"));
        assert!(
            !runner
                .invocations()
                .iter()
                .any(|(args, _)| args == &["pm".to_owned(), "clean".to_owned()]),
            "no workspace may be cleaned while free space is ambiguous"
        );

        // The delegated store prune and own-state cleanup do not depend on an
        // exact figure and still run.
        assert!(
            runner
                .invocations()
                .iter()
                .any(|(args, _)| args == &["store".to_owned(), "prune".to_owned()])
        );
        assert_eq!(report.result, RunResult::SkippedSnapshotCapacityUncertain);
    }

    #[test]
    fn nothing_in_the_executor_can_touch_a_snapshot() {
        // The only commands this product can issue are the compiled pnpm
        // argument arrays, so no snapshot command is reachable by construction.
        for action in [
            AllowedAction::CleanTerminalJanitorState,
            AllowedAction::PnpmStorePrune,
        ] {
            for argument in action.pnpm_args().unwrap_or(&[]) {
                for forbidden in ["tmutil", "snapshot", "thin", "diskutil", "apfs"] {
                    assert!(!argument.contains(forbidden));
                }
            }
        }
    }

    #[test]
    fn exhausting_every_safe_action_reports_a_shortfall_and_no_more_authority() {
        let world = World::new(1);
        let injected = Injected::new(&world, Liveness::Inactive, GitState::Clean);
        let planning = injected.planning();
        let plan = build_plan(&world, &planning);
        let volume = Volume { free: Cell::new(0) };
        let disk = StepDisk::new(&volume);
        // Every action succeeds but frees nothing.
        let runner = FakeRunner::new(&volume, 0);
        let mut store = world.store();

        let report = run(&world, &plan, &planning, &disk, &runner, &mut store);
        assert_eq!(report.result, RunResult::ShortfallSafeActionsExhausted);
        assert!(report.free_after_bytes.unwrap() < TARGET);
        // A shortfall must not have reached for anything beyond the plan.
        let planned_actions = plan.actions.len() + MAX_POST_CLEAN_STORE_PRUNES;
        assert!(report.actions.len() <= planned_actions);
    }
}
