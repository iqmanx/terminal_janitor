//! Run and action journalling.
//!
//! Every automatic action is written down before it runs and moved through
//! explicit states as it proceeds (`SAFETY.md` section 10). The journal exists
//! so that a crash leaves a readable record rather than an ambiguous one: an
//! action interrupted in `RUNNING` is never later read as success, and never
//! replayed blindly.
//!
//! This module owns the vocabulary and the transition rules. Storage lives in
//! [`crate::state`], which owns all SQL.

use std::fmt;
use std::path::PathBuf;

use serde::Serialize;

use crate::state::Timestamp;

/// Detailed history is capped (`SAFETY.md` section 12).
pub const MAX_DETAILED_RUNS: usize = 200;

/// Identity of one journalled run.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(String);

impl RunId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identity of one journalled action within a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActionId(i64);

impl ActionId {
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    pub fn as_i64(self) -> i64 {
        self.0
    }
}

/// The explicit action states from `SAFETY.md` section 10.
///
/// The three terminal skip reasons are distinct on purpose: "we refused" and
/// "we could not tell" must never be read as the same outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActionState {
    /// Written down, not yet touched.
    Planned,
    /// Proof and identity are being revalidated immediately before execution.
    Validating,
    /// The command has been started.
    Running,
    Succeeded,
    Failed,
    /// The target moved, was replaced, or no longer matches its plan.
    SkippedChanged,
    /// Something is using the workspace now.
    SkippedActive,
    /// Protection refused the location.
    SkippedProtected,
    /// A required fact could not be established. Unknown skips.
    SkippedUnknown,
}

impl ActionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "PLANNED",
            Self::Validating => "VALIDATING",
            Self::Running => "RUNNING",
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::SkippedChanged => "SKIPPED_CHANGED",
            Self::SkippedActive => "SKIPPED_ACTIVE",
            Self::SkippedProtected => "SKIPPED_PROTECTED",
            Self::SkippedUnknown => "SKIPPED_UNKNOWN",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "PLANNED" => Self::Planned,
            "VALIDATING" => Self::Validating,
            "RUNNING" => Self::Running,
            "SUCCEEDED" => Self::Succeeded,
            "FAILED" => Self::Failed,
            "SKIPPED_CHANGED" => Self::SkippedChanged,
            "SKIPPED_ACTIVE" => Self::SkippedActive,
            "SKIPPED_PROTECTED" => Self::SkippedProtected,
            "SKIPPED_UNKNOWN" => Self::SkippedUnknown,
            _ => return None,
        })
    }

    /// True once the action can no longer change.
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Planned | Self::Validating | Self::Running)
    }

    /// True when the action definitely did not run to completion.
    pub fn is_skip(self) -> bool {
        matches!(
            self,
            Self::SkippedChanged
                | Self::SkippedActive
                | Self::SkippedProtected
                | Self::SkippedUnknown
        )
    }

    /// True when an interrupted run left this action's outcome unknowable.
    ///
    /// A crash between `RUNNING` and a terminal state means the command may or
    /// may not have done its work. That is not success and not failure; it is
    /// an unresolved action, and the reader must be told so.
    pub fn is_unresolved(self) -> bool {
        matches!(self, Self::Validating | Self::Running)
    }
}

impl fmt::Display for ActionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The complete result vocabulary from `ACCEPTANCE.md` section N.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunResult {
    /// Free space was at or above `minimum_free`; nothing was scanned or done.
    OkNoPressure,
    /// `target_free` was reached.
    OkTargetRestored,
    /// Space was recovered safely, but the target was not reached and more
    /// proven actions remain for a later run.
    OkPartialSafeReclaim,
    /// Every authorised action was exhausted and the target was not reached.
    ShortfallSafeActionsExhausted,
    /// Activity or liveness could not be established for any candidate.
    SkippedActivityUncertain,
    /// Every candidate was refused by protection.
    SkippedProtected,
    /// macOS snapshot or purgeable capacity is ambiguous (Day 5 scope; the
    /// variant exists so the vocabulary is complete and testable).
    SkippedSnapshotCapacityUncertain,
    FailedConfiguration,
    FailedCommand,
    FailedStorageMeasurement,
    FailedState,
    /// Another run holds this volume's lock.
    AlreadyRunning,
}

impl RunResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OkNoPressure => "OK_NO_PRESSURE",
            Self::OkTargetRestored => "OK_TARGET_RESTORED",
            Self::OkPartialSafeReclaim => "OK_PARTIAL_SAFE_RECLAIM",
            Self::ShortfallSafeActionsExhausted => "SHORTFALL_SAFE_ACTIONS_EXHAUSTED",
            Self::SkippedActivityUncertain => "SKIPPED_ACTIVITY_UNCERTAIN",
            Self::SkippedProtected => "SKIPPED_PROTECTED",
            Self::SkippedSnapshotCapacityUncertain => "SKIPPED_SNAPSHOT_CAPACITY_UNCERTAIN",
            Self::FailedConfiguration => "FAILED_CONFIGURATION",
            Self::FailedCommand => "FAILED_COMMAND",
            Self::FailedStorageMeasurement => "FAILED_STORAGE_MEASUREMENT",
            Self::FailedState => "FAILED_STATE",
            Self::AlreadyRunning => "ALREADY_RUNNING",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "OK_NO_PRESSURE" => Self::OkNoPressure,
            "OK_TARGET_RESTORED" => Self::OkTargetRestored,
            "OK_PARTIAL_SAFE_RECLAIM" => Self::OkPartialSafeReclaim,
            "SHORTFALL_SAFE_ACTIONS_EXHAUSTED" => Self::ShortfallSafeActionsExhausted,
            "SKIPPED_ACTIVITY_UNCERTAIN" => Self::SkippedActivityUncertain,
            "SKIPPED_PROTECTED" => Self::SkippedProtected,
            "SKIPPED_SNAPSHOT_CAPACITY_UNCERTAIN" => Self::SkippedSnapshotCapacityUncertain,
            "FAILED_CONFIGURATION" => Self::FailedConfiguration,
            "FAILED_COMMAND" => Self::FailedCommand,
            "FAILED_STORAGE_MEASUREMENT" => Self::FailedStorageMeasurement,
            "FAILED_STATE" => Self::FailedState,
            "ALREADY_RUNNING" => Self::AlreadyRunning,
            _ => return None,
        })
    }

    /// Process exit code for this result. Success results exit zero so a
    /// scheduler does not treat a healthy machine as a failure.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::OkNoPressure
            | Self::OkTargetRestored
            | Self::OkPartialSafeReclaim
            | Self::SkippedActivityUncertain
            | Self::SkippedProtected
            | Self::SkippedSnapshotCapacityUncertain => 0,
            // A shortfall is a correct, safe outcome, but it is one the owner
            // should be able to notice from a scheduler.
            Self::ShortfallSafeActionsExhausted => 1,
            Self::FailedConfiguration => 2,
            Self::FailedStorageMeasurement => 3,
            Self::FailedState => 5,
            Self::FailedCommand => 8,
            Self::AlreadyRunning => 9,
        }
    }
}

impl fmt::Display for RunResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A run about to be journalled.
#[derive(Debug, Clone)]
pub struct NewRun {
    pub run_id: RunId,
    pub plan_id: String,
    pub policy_hash: String,
    pub volume_id: String,
    pub free_before_bytes: u64,
    pub minimum_free_bytes: u64,
    pub target_free_bytes: u64,
    pub started_at: Timestamp,
}

/// An action about to be journalled, written before anything is attempted.
#[derive(Debug, Clone)]
pub struct NewAction {
    pub run_id: RunId,
    pub sequence: i64,
    pub action: &'static str,
    pub workspace_id: Option<String>,
    pub canonical_path: Option<PathBuf>,
    pub free_before_bytes: u64,
    pub started_at: Timestamp,
}

/// How one action finished.
#[derive(Debug, Clone)]
pub struct ActionOutcome {
    pub state: ActionState,
    pub free_after_bytes: Option<u64>,
    /// Measured free-space change. Only a measurement may fill this in; an
    /// estimate never becomes a recovery figure (`SAFETY.md` section 9).
    pub actual_bytes: Option<u64>,
    pub detail: Option<String>,
    /// What the owner would do to restore this, when that is meaningful.
    pub recovery: Option<String>,
    pub finished_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionRecord {
    pub sequence: i64,
    pub action: String,
    pub workspace_id: Option<String>,
    pub path: Option<PathBuf>,
    pub state: ActionState,
    pub free_before_bytes: u64,
    pub free_after_bytes: Option<u64>,
    pub actual_bytes: Option<u64>,
    pub detail: Option<String>,
    pub recovery: Option<String>,
    pub started_at_millis: i64,
    pub finished_at_millis: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunRecord {
    pub run_id: String,
    pub plan_id: String,
    pub policy_hash: String,
    pub volume_id: String,
    pub result: Option<RunResult>,
    pub free_before_bytes: u64,
    pub free_after_bytes: Option<u64>,
    pub minimum_free_bytes: u64,
    pub target_free_bytes: u64,
    pub started_at_millis: i64,
    pub finished_at_millis: Option<i64>,
    pub actions: Vec<ActionRecord>,
}

impl RunRecord {
    /// True when this run never reached a result — it was interrupted.
    pub fn interrupted(&self) -> bool {
        self.result.is_none() || self.finished_at_millis.is_none()
    }

    /// Actions whose outcome the interruption left unknowable. These must not
    /// be replayed blindly and must not be reported as either success or
    /// failure (`SAFETY.md` section 10).
    pub fn unresolved_actions(&self) -> Vec<&ActionRecord> {
        self.actions
            .iter()
            .filter(|action| action.state.is_unresolved())
            .collect()
    }

    /// Measured free-space change across the run, when both ends were
    /// measured. Estimates never contribute.
    pub fn measured_recovery_bytes(&self) -> Option<u64> {
        let after = self.free_after_bytes?;
        Some(after.saturating_sub(self.free_before_bytes))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// The complete vocabularies. Every exhaustive test below walks these, so
    /// a variant added without a name, a classification, or an exit code fails
    /// a test rather than reaching the ledger unexamined.
    const ALL_STATES: [ActionState; 9] = [
        ActionState::Planned,
        ActionState::Validating,
        ActionState::Running,
        ActionState::Succeeded,
        ActionState::Failed,
        ActionState::SkippedChanged,
        ActionState::SkippedActive,
        ActionState::SkippedProtected,
        ActionState::SkippedUnknown,
    ];

    const ALL_RESULTS: [RunResult; 12] = [
        RunResult::OkNoPressure,
        RunResult::OkTargetRestored,
        RunResult::OkPartialSafeReclaim,
        RunResult::ShortfallSafeActionsExhausted,
        RunResult::SkippedActivityUncertain,
        RunResult::SkippedProtected,
        RunResult::SkippedSnapshotCapacityUncertain,
        RunResult::FailedConfiguration,
        RunResult::FailedCommand,
        RunResult::FailedStorageMeasurement,
        RunResult::FailedState,
        RunResult::AlreadyRunning,
    ];

    #[test]
    fn every_action_state_round_trips() {
        for state in ALL_STATES {
            assert_eq!(ActionState::parse(state.as_str()), Some(state));
        }
        assert_eq!(ActionState::parse("NONSENSE"), None);
    }

    #[test]
    fn every_result_round_trips_and_has_an_exit_code() {
        for result in ALL_RESULTS {
            assert_eq!(RunResult::parse(result.as_str()), Some(result));
            let _ = result.exit_code();
        }
        assert_eq!(RunResult::parse("NONSENSE"), None);
    }

    /// The ledger stores `as_str`; every JSON reader sees the serde name. If
    /// the two ever drift, the recorded history and the reported history
    /// describe the same run differently, and neither is obviously wrong.
    #[test]
    fn the_stored_name_and_the_json_name_are_the_same_name() {
        for state in ALL_STATES {
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::Value::String(state.as_str().to_owned()),
                "{state} is journalled and serialised under different names"
            );
        }
        for result in ALL_RESULTS {
            assert_eq!(
                serde_json::to_value(result).unwrap(),
                serde_json::Value::String(result.as_str().to_owned()),
                "{result} is journalled and serialised under different names"
            );
        }
    }

    /// Exhaustive over the state vocabulary. Classification must be total and
    /// must never let one state be read as both finished and still in flight.
    #[test]
    fn action_state_classification_is_total_and_consistent() {
        for state in ALL_STATES {
            assert!(
                !(state.is_terminal() && state.is_unresolved()),
                "{state} is both finished and in flight"
            );
            assert!(
                !state.is_skip() || state.is_terminal(),
                "{state} is a skip, so it is a finished action"
            );
            assert!(!state.as_str().is_empty());
        }

        let unresolved: Vec<_> = ALL_STATES
            .into_iter()
            .filter(|state| state.is_unresolved())
            .collect();
        assert_eq!(
            unresolved,
            vec![ActionState::Validating, ActionState::Running],
            "only a started action can be left unresolvable by a crash"
        );
        // PLANNED is deliberately neither: it was written down and never begun.
        assert!(!ActionState::Planned.is_terminal());
        assert!(!ActionState::Planned.is_unresolved());
    }

    /// A scheduler sees only the exit code. Two outcomes it must react to
    /// differently may not arrive under the same number.
    #[test]
    fn every_non_zero_result_has_its_own_exit_code() {
        let mut seen: BTreeMap<u8, RunResult> = BTreeMap::new();
        for result in ALL_RESULTS {
            let code = result.exit_code();
            if code == 0 {
                continue;
            }
            if let Some(other) = seen.insert(code, result) {
                panic!("{result} and {other} both exit {code}");
            }
        }
        assert!(
            ALL_RESULTS
                .into_iter()
                .any(|result| result.exit_code() == 0),
            "a healthy machine must exit zero"
        );
    }

    #[test]
    fn a_healthy_machine_exits_zero_and_a_blocked_run_does_not() {
        assert_eq!(RunResult::OkNoPressure.exit_code(), 0);
        assert_eq!(RunResult::OkTargetRestored.exit_code(), 0);
        assert_eq!(RunResult::SkippedProtected.exit_code(), 0);
        assert_ne!(RunResult::ShortfallSafeActionsExhausted.exit_code(), 0);
        assert_ne!(RunResult::AlreadyRunning.exit_code(), 0);
        assert_ne!(RunResult::FailedCommand.exit_code(), 0);
    }

    #[test]
    fn in_flight_states_are_unresolved_and_terminal_states_are_not() {
        assert!(ActionState::Validating.is_unresolved());
        assert!(ActionState::Running.is_unresolved());
        assert!(!ActionState::Planned.is_unresolved());
        assert!(!ActionState::Succeeded.is_unresolved());
        assert!(!ActionState::Failed.is_unresolved());

        assert!(!ActionState::Planned.is_terminal());
        assert!(ActionState::Succeeded.is_terminal());
        assert!(ActionState::SkippedUnknown.is_terminal());
        assert!(ActionState::SkippedUnknown.is_skip());
        assert!(!ActionState::Failed.is_skip());
    }

    fn action(state: ActionState) -> ActionRecord {
        ActionRecord {
            sequence: 1,
            action: "PNPM_WORKSPACE_CLEAN".to_owned(),
            workspace_id: Some("workspace-a".to_owned()),
            path: Some(PathBuf::from("workspace")),
            state,
            free_before_bytes: 10,
            free_after_bytes: None,
            actual_bytes: None,
            detail: None,
            recovery: None,
            started_at_millis: 1,
            finished_at_millis: None,
        }
    }

    fn run(result: Option<RunResult>, states: &[ActionState]) -> RunRecord {
        RunRecord {
            run_id: "run-1".to_owned(),
            plan_id: "plan-1".to_owned(),
            policy_hash: "policy-1".to_owned(),
            volume_id: "vol-1".to_owned(),
            result,
            free_before_bytes: 10,
            free_after_bytes: result.map(|_| 30),
            minimum_free_bytes: 20,
            target_free_bytes: 40,
            started_at_millis: 1,
            finished_at_millis: result.map(|_| 9),
            actions: states.iter().copied().map(action).collect(),
        }
    }

    #[test]
    fn an_interrupted_run_reports_its_unresolved_actions() {
        let crashed = run(None, &[ActionState::Succeeded, ActionState::Running]);
        assert!(crashed.interrupted());
        let unresolved = crashed.unresolved_actions();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].state, ActionState::Running);
    }

    #[test]
    fn a_finished_run_has_nothing_unresolved() {
        let finished = run(
            Some(RunResult::OkTargetRestored),
            &[ActionState::Succeeded, ActionState::SkippedProtected],
        );
        assert!(!finished.interrupted());
        assert!(finished.unresolved_actions().is_empty());
        assert_eq!(finished.measured_recovery_bytes(), Some(20));
    }

    #[test]
    fn an_unmeasured_run_reports_no_recovery_rather_than_zero() {
        let crashed = run(None, &[ActionState::Running]);
        assert_eq!(crashed.measured_recovery_bytes(), None);
    }
}
