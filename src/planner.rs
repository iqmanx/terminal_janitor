//! Proof-driven planning.
//!
//! The planner turns ledger records into an immutable list of fully proven
//! actions. It executes nothing. Every candidate leaves this module in one of
//! exactly two states: eligible with a complete [`ProofBundle`], or skipped
//! with exactly one named gate and a reason.
//!
//! Gates are evaluated in a fixed order and the first failure stops evaluation,
//! so a skip always names the most fundamental thing that was wrong rather than
//! a list the reader has to weigh.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::adapters::AllowedAction;
use crate::adapters::pnpm::{MINIMUM_PNPM_MAJOR, PnpmEnrolment};
use crate::config::Config;
use crate::discovery::{DiscoveryProvider, EntryKind};
use crate::identity::VolumeId;
use crate::protection::{DeniedRoots, ProtectionProbe, ProtectionReason, evaluate};
use crate::state::{ApprovedRootRecord, GitState, Timestamp, WorkspaceRecord, WorkspaceStatus};

/// A workspace must be observed for a full day before it can be cleaned, so a
/// project registered moments ago can never be a candidate.
pub const REQUIRED_OBSERVATION_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// A workspace must show no relevant activity for a month.
pub const REQUIRED_INACTIVITY_MILLIS: i64 = 30 * 24 * 60 * 60 * 1000;

/// A cleaned workspace is left alone for a week, so a repeatedly pressured
/// volume cannot churn the same project.
pub const WORKSPACE_COOLDOWN_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;

/// A plan describes the machine as it was measured. After this it must be
/// rebuilt rather than trusted (`ACCEPTANCE.md` section G).
pub const PLAN_VALIDITY_MILLIS: i64 = 15 * 60 * 1000;

/// No automatic run may clean more than two workspaces (`SAFETY.md` section 9).
pub const MAX_AUTOMATIC_WORKSPACE_CLEANS: usize = 2;

/// The three files that make a pnpm workspace both recognisable and
/// regenerable. The lockfile is the recovery proof and is never removed.
pub const REQUIRED_MARKERS: &[&str] = &["package.json", "pnpm-workspace.yaml", "pnpm-lock.yaml"];

/// The mandatory proof bundle from `SAFETY.md` section 4.
///
/// Every field must be true before an action may be planned. There is no
/// score, weight, or override: a single false field removes the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ProofBundle {
    pub action_allowlisted: bool,
    pub owner_proven: bool,
    pub regenerability_proven: bool,
    pub reference_safety_proven: bool,
    pub inactivity_proven: bool,
    pub protection_checks_passed: bool,
    pub executable_identity_verified: bool,
    pub target_identity_verified: bool,
}

impl ProofBundle {
    pub fn complete(&self) -> bool {
        self.action_allowlisted
            && self.owner_proven
            && self.regenerability_proven
            && self.reference_safety_proven
            && self.inactivity_proven
            && self.protection_checks_passed
            && self.executable_identity_verified
            && self.target_identity_verified
    }
}

/// Whether anything is using a workspace right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Inactive,
    Active,
    /// Process information could not be obtained. Unknown is not "probably
    /// idle"; it skips.
    Unknown,
}

/// Injectable process liveness (`AGENTS.md` section 6).
pub trait LivenessProver {
    fn liveness(&self, workspace: &Path) -> Liveness;
}

/// The Day 3 production prover.
///
/// Process enumeration is Day 5 work. Until it exists, production reports
/// `Unknown` for every workspace, which means production plans contain no
/// workspace clean at all. That is deliberate: it keeps the Day 4 executor
/// unable to clean a workspace whose liveness has never been proven, instead
/// of leaving a gate that silently passes until Day 5 fills it in.
#[derive(Debug, Default)]
pub struct UnavailableLivenessProver;

impl LivenessProver for UnavailableLivenessProver {
    fn liveness(&self, _workspace: &Path) -> Liveness {
        Liveness::Unknown
    }
}

/// Injectable Git worktree state.
pub trait GitProver {
    fn worktree_state(&self, workspace: &Path) -> GitState;
}

/// Used when no Git executable could be located. Reports `Unknown`, which
/// skips, rather than assuming an unversioned directory is safe.
#[derive(Debug, Default)]
pub struct UnavailableGitProver;

impl GitProver for UnavailableGitProver {
    fn worktree_state(&self, _workspace: &Path) -> GitState {
        GitState::Unknown
    }
}

/// The single gate that refused a candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "gate", content = "detail", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GateFailure {
    /// G1 — the workspace is not inside any active approved root.
    NotInsideApprovedRoot,
    /// G1 — the ledger no longer considers this record live.
    WorkspaceNotPresent { status: &'static str },
    /// Filesystem identity — the path is gone, unreadable, or now a different
    /// physical directory.
    TargetIdentityChanged { detail: String },
    /// The workspace is not on the volume under pressure, so cleaning it would
    /// not help and is not authorised by this run.
    DifferentVolume,
    /// G4 — protection refused the location.
    Protected(ProtectionReason),
    /// G1/G2 — a required pnpm marker is missing, so ownership and
    /// regenerability cannot both be proven.
    MissingMarker { marker: &'static str },
    /// No pnpm executable has been enrolled.
    PnpmNotEnrolled,
    /// The enrolled pnpm is no longer the file that was enrolled.
    PnpmIdentityChanged { detail: String },
    /// The enrolled pnpm is too old for `pnpm pm clean`.
    PnpmBelowMinimumVersion { found: String, required_major: u64 },
    /// G3 — Git could not answer, so uncommitted work cannot be ruled out.
    GitStateUnknown,
    /// G3 — the worktree holds modified or untracked work.
    GitWorktreeDirty,
    /// G3 — the workspace has not been watched long enough to trust its history.
    ObservationWindowNotMet {
        observed_for_millis: i64,
        required_millis: i64,
    },
    /// G3 — the workspace was active too recently.
    RecentActivity {
        inactive_for_millis: i64,
        required_millis: i64,
    },
    /// The workspace was cleaned recently enough that cleaning again is churn.
    CooldownActive {
        since_last_clean_millis: i64,
        required_millis: i64,
    },
    /// G3 — something is using the workspace now.
    WorkspaceActive,
    /// G3 — liveness could not be established. Unknown skips.
    LivenessUnknown,
    /// Two workspaces are already planned; the rest wait for the next run.
    AutomaticWorkspaceCapReached { cap: usize },
}

impl GateFailure {
    /// Stable machine-readable gate name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotInsideApprovedRoot => "NOT_INSIDE_APPROVED_ROOT",
            Self::WorkspaceNotPresent { .. } => "WORKSPACE_NOT_PRESENT",
            Self::TargetIdentityChanged { .. } => "TARGET_IDENTITY_CHANGED",
            Self::DifferentVolume => "DIFFERENT_VOLUME",
            Self::Protected(_) => "PROTECTED",
            Self::MissingMarker { .. } => "MISSING_MARKER",
            Self::PnpmNotEnrolled => "PNPM_NOT_ENROLLED",
            Self::PnpmIdentityChanged { .. } => "PNPM_IDENTITY_CHANGED",
            Self::PnpmBelowMinimumVersion { .. } => "PNPM_BELOW_MINIMUM_VERSION",
            Self::GitStateUnknown => "GIT_STATE_UNKNOWN",
            Self::GitWorktreeDirty => "GIT_WORKTREE_DIRTY",
            Self::ObservationWindowNotMet { .. } => "OBSERVATION_WINDOW_NOT_MET",
            Self::RecentActivity { .. } => "RECENT_ACTIVITY",
            Self::CooldownActive { .. } => "COOLDOWN_ACTIVE",
            Self::WorkspaceActive => "WORKSPACE_ACTIVE",
            Self::LivenessUnknown => "LIVENESS_UNKNOWN",
            Self::AutomaticWorkspaceCapReached { .. } => "AUTOMATIC_WORKSPACE_CAP_REACHED",
        }
    }
}

impl fmt::Display for GateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInsideApprovedRoot => {
                write!(f, "the workspace is not inside an active approved root")
            }
            Self::WorkspaceNotPresent { status } => {
                write!(f, "the ledger records this workspace as {status}")
            }
            Self::TargetIdentityChanged { detail } => {
                write!(f, "the workspace identity changed: {detail}")
            }
            Self::DifferentVolume => write!(f, "the workspace is not on the pressured volume"),
            Self::Protected(reason) => reason.fmt(f),
            Self::MissingMarker { marker } => write!(f, "{marker} is missing"),
            Self::PnpmNotEnrolled => write!(f, "no pnpm executable is enrolled"),
            Self::PnpmIdentityChanged { detail } => {
                write!(f, "the enrolled pnpm executable changed: {detail}")
            }
            Self::PnpmBelowMinimumVersion {
                found,
                required_major,
            } => write!(
                f,
                "enrolled pnpm {found} is below the required major version {required_major}"
            ),
            Self::GitStateUnknown => write!(f, "Git worktree state could not be established"),
            Self::GitWorktreeDirty => {
                write!(f, "the Git worktree has modified or untracked work")
            }
            Self::ObservationWindowNotMet {
                observed_for_millis,
                required_millis,
            } => write!(
                f,
                "observed for {} of the required {} hours",
                observed_for_millis / 3_600_000,
                required_millis / 3_600_000
            ),
            Self::RecentActivity {
                inactive_for_millis,
                required_millis,
            } => write!(
                f,
                "inactive for {} of the required {} days",
                inactive_for_millis / 86_400_000,
                required_millis / 86_400_000
            ),
            Self::CooldownActive {
                since_last_clean_millis,
                required_millis,
            } => write!(
                f,
                "cleaned {} days ago; the cooldown is {} days",
                since_last_clean_millis / 86_400_000,
                required_millis / 86_400_000
            ),
            Self::WorkspaceActive => write!(f, "a process is using the workspace"),
            Self::LivenessUnknown => {
                write!(f, "process liveness could not be established")
            }
            Self::AutomaticWorkspaceCapReached { cap } => write!(
                f,
                "an automatic run cleans at most {cap} workspaces; this one waits"
            ),
        }
    }
}

/// One action that passed every gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedAction {
    pub action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub proof: ProofBundle,
    #[serde(skip)]
    pub typed_action: AllowedAction,
}

/// One candidate that did not pass.
///
/// `gate` and `reason` are the stable JSON surface: an exact machine-readable
/// gate name and one human sentence. The typed `failure` is for Rust callers —
/// the Day 4 executor matches on it — and is not serialised, so adding a field
/// to a variant can never change the published output shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedCandidate {
    pub workspace_id: String,
    pub path: PathBuf,
    pub gate: &'static str,
    pub reason: String,
    #[serde(skip)]
    pub failure: GateFailure,
}

/// An immutable plan.
///
/// The identity fields exist so a later executor can prove it is running the
/// plan that was built, under the policy that built it, before it expired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub plan_id: String,
    pub policy_hash: String,
    pub created_at_millis: i64,
    pub expires_at_millis: i64,
    pub actions: Vec<PlannedAction>,
    pub skipped: Vec<SkippedCandidate>,
}

impl Plan {
    pub fn is_expired(&self, now: Timestamp) -> bool {
        now.as_unix_millis() >= self.expires_at_millis
    }

    /// The number of workspace cleans, which the cap applies to.
    pub fn workspace_clean_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|action| {
                matches!(
                    action.typed_action,
                    AllowedAction::PnpmWorkspaceClean { .. }
                )
            })
            .count()
    }
}

/// Everything planning reads. Grouping it keeps the gate order in one readable
/// function rather than spread across a long argument list.
pub struct PlanningInputs<'a> {
    pub workspaces: &'a [WorkspaceRecord],
    pub approved_roots: &'a [ApprovedRootRecord],
    pub pnpm: Option<&'a PnpmEnrolment>,
    pub pressured_volume: &'a VolumeId,
    pub config: &'a Config,
    pub now: Timestamp,
}

/// The injected observers planning uses.
pub struct PlanningProviders<'a> {
    pub discovery: &'a dyn DiscoveryProvider,
    pub protection_probe: &'a dyn ProtectionProbe,
    pub denied_roots: &'a DeniedRoots,
    pub liveness: &'a dyn LivenessProver,
    pub git: &'a dyn GitProver,
}

/// Builds a plan. Nothing here writes to the filesystem or runs a cleanup
/// command; the only commands involved are the read-only Git status queries the
/// injected [`GitProver`] performs.
pub fn plan(inputs: &PlanningInputs<'_>, providers: &PlanningProviders<'_>) -> Plan {
    let mut candidates: Vec<(&WorkspaceRecord, Result<ProofBundle, GateFailure>)> = inputs
        .workspaces
        .iter()
        .map(|workspace| (workspace, evaluate_workspace(workspace, inputs, providers)))
        .collect();

    // Oldest proven activity first, then the workspace we have watched longest,
    // then canonical path. Every key is stored data, so identical state always
    // produces one identical order.
    candidates.sort_by(|(left, _), (right, _)| {
        left.last_activity_at
            .cmp(&right.last_activity_at)
            .then(left.first_observed_at.cmp(&right.first_observed_at))
            .then(left.canonical_path.cmp(&right.canonical_path))
    });

    let mut actions = Vec::new();
    let mut skipped = Vec::new();

    // Own state first: it is the smallest authority the product has and it
    // never touches a user project.
    actions.push(PlannedAction {
        action: AllowedAction::CleanTerminalJanitorState.as_str(),
        workspace_id: None,
        path: None,
        proof: own_state_proof(),
        typed_action: AllowedAction::CleanTerminalJanitorState,
    });

    // Delegated store reclamation, which deletes nothing directly.
    if let Some(pnpm) = inputs.pnpm
        && pnpm_gate(pnpm, providers).is_ok()
    {
        actions.push(PlannedAction {
            action: AllowedAction::PnpmStorePrune.as_str(),
            workspace_id: None,
            path: None,
            proof: store_prune_proof(),
            typed_action: AllowedAction::PnpmStorePrune,
        });
    }

    let mut planned_cleans = 0;
    for (workspace, outcome) in candidates {
        let failure = match outcome {
            Ok(proof) if planned_cleans < MAX_AUTOMATIC_WORKSPACE_CLEANS => {
                planned_cleans += 1;
                actions.push(PlannedAction {
                    action: AllowedAction::PnpmWorkspaceClean {
                        workspace_id: workspace.id.clone(),
                    }
                    .as_str(),
                    workspace_id: Some(workspace.id.as_str().to_owned()),
                    path: Some(workspace.canonical_path.as_path().to_path_buf()),
                    proof,
                    typed_action: AllowedAction::PnpmWorkspaceClean {
                        workspace_id: workspace.id.clone(),
                    },
                });
                continue;
            }
            Ok(_) => GateFailure::AutomaticWorkspaceCapReached {
                cap: MAX_AUTOMATIC_WORKSPACE_CLEANS,
            },
            Err(failure) => failure,
        };
        skipped.push(SkippedCandidate {
            workspace_id: workspace.id.as_str().to_owned(),
            path: workspace.canonical_path.as_path().to_path_buf(),
            gate: failure.as_str(),
            reason: failure.to_string(),
            failure,
        });
    }

    let policy_hash = policy_hash(inputs);
    let created_at_millis = inputs.now.as_unix_millis();
    let plan_id = plan_id(&policy_hash, created_at_millis, &actions);

    Plan {
        plan_id,
        policy_hash,
        created_at_millis,
        expires_at_millis: created_at_millis.saturating_add(PLAN_VALIDITY_MILLIS),
        actions,
        skipped,
    }
}

/// `terminal_janitor`'s own expired state: our directory, our data, nothing
/// referenced by another tool, and nothing a user could lose.
fn own_state_proof() -> ProofBundle {
    ProofBundle {
        action_allowlisted: true,
        owner_proven: true,
        regenerability_proven: true,
        reference_safety_proven: true,
        inactivity_proven: true,
        protection_checks_passed: true,
        executable_identity_verified: true,
        target_identity_verified: true,
    }
}

/// A store prune is delegated to pnpm, which is the only component that knows
/// what its store still references.
fn store_prune_proof() -> ProofBundle {
    ProofBundle {
        action_allowlisted: true,
        owner_proven: true,
        regenerability_proven: true,
        reference_safety_proven: true,
        inactivity_proven: true,
        protection_checks_passed: true,
        executable_identity_verified: true,
        target_identity_verified: true,
    }
}

fn evaluate_workspace(
    workspace: &WorkspaceRecord,
    inputs: &PlanningInputs<'_>,
    providers: &PlanningProviders<'_>,
) -> Result<ProofBundle, GateFailure> {
    let mut proof = ProofBundle {
        // The action is a compiled variant of `AllowedAction`; no caller can
        // widen it.
        action_allowlisted: true,
        ..ProofBundle::default()
    };
    let path = workspace.canonical_path.as_path();

    if workspace.status != WorkspaceStatus::Present {
        return Err(GateFailure::WorkspaceNotPresent {
            status: match workspace.status {
                WorkspaceStatus::Present => "present",
                WorkspaceStatus::Missing => "missing",
                WorkspaceStatus::Replaced => "replaced",
            },
        });
    }

    let root = inputs
        .approved_roots
        .iter()
        .filter(|root| root.active && path.starts_with(root.canonical_path.as_path()))
        .max_by_key(|root| root.canonical_path.as_str().len())
        .ok_or(GateFailure::NotInsideApprovedRoot)?;
    proof.owner_proven = true;

    // Identity is revalidated against the live filesystem, not trusted from the
    // ledger. A moved or replaced directory invalidates the candidate.
    let identity = providers
        .discovery
        .resolve_identity(path)
        .map_err(|error| GateFailure::TargetIdentityChanged {
            detail: error.to_string(),
        })?;
    if identity.canonical != workspace.canonical_path
        || identity.volume != workspace.volume_id
        || identity.file != workspace.file_id
    {
        return Err(GateFailure::TargetIdentityChanged {
            detail: "canonical path, volume, or file identity no longer matches the ledger"
                .to_owned(),
        });
    }
    proof.target_identity_verified = true;

    if &identity.volume != inputs.pressured_volume {
        return Err(GateFailure::DifferentVolume);
    }

    if let Some(reason) = evaluate(
        path,
        root.canonical_path.as_path(),
        workspace.protected,
        providers.denied_roots,
        providers.protection_probe,
    ) {
        return Err(GateFailure::Protected(reason));
    }
    proof.protection_checks_passed = true;

    for marker in REQUIRED_MARKERS {
        match providers.discovery.entry_kind(&path.join(marker)) {
            Ok(EntryKind::File) => {}
            _ => return Err(GateFailure::MissingMarker { marker }),
        }
    }
    // All three markers are present, so pnpm can rebuild what it removes and
    // the lockfile remains the recovery proof.
    proof.regenerability_proven = true;
    // Workspace cleanup is delegated to pnpm, which never lets this product
    // delete shared store content directly.
    proof.reference_safety_proven = true;

    let pnpm = inputs.pnpm.ok_or(GateFailure::PnpmNotEnrolled)?;
    pnpm_gate(pnpm, providers)?;
    proof.executable_identity_verified = true;

    match providers.git.worktree_state(path) {
        GitState::Clean => {}
        GitState::Dirty => return Err(GateFailure::GitWorktreeDirty),
        GitState::Unknown => return Err(GateFailure::GitStateUnknown),
    }

    let observed_for = inputs
        .now
        .as_unix_millis()
        .saturating_sub(workspace.first_observed_at.as_unix_millis());
    if observed_for < REQUIRED_OBSERVATION_MILLIS {
        return Err(GateFailure::ObservationWindowNotMet {
            observed_for_millis: observed_for,
            required_millis: REQUIRED_OBSERVATION_MILLIS,
        });
    }

    let inactive_for = inputs
        .now
        .as_unix_millis()
        .saturating_sub(workspace.last_activity_at.as_unix_millis());
    if inactive_for < REQUIRED_INACTIVITY_MILLIS {
        return Err(GateFailure::RecentActivity {
            inactive_for_millis: inactive_for,
            required_millis: REQUIRED_INACTIVITY_MILLIS,
        });
    }

    if let Some(last_cleaned) = workspace.last_cleaned_at {
        let since = inputs
            .now
            .as_unix_millis()
            .saturating_sub(last_cleaned.as_unix_millis());
        if since < WORKSPACE_COOLDOWN_MILLIS {
            return Err(GateFailure::CooldownActive {
                since_last_clean_millis: since,
                required_millis: WORKSPACE_COOLDOWN_MILLIS,
            });
        }
    }

    match providers.liveness.liveness(path) {
        Liveness::Inactive => {}
        Liveness::Active => return Err(GateFailure::WorkspaceActive),
        Liveness::Unknown => return Err(GateFailure::LivenessUnknown),
    }
    proof.inactivity_proven = true;

    debug_assert!(proof.complete(), "a planned action must carry full proof");
    Ok(proof)
}

fn pnpm_gate(pnpm: &PnpmEnrolment, providers: &PlanningProviders<'_>) -> Result<(), GateFailure> {
    if !pnpm.version.meets_minimum() {
        return Err(GateFailure::PnpmBelowMinimumVersion {
            found: pnpm.version.as_str().to_owned(),
            required_major: MINIMUM_PNPM_MAJOR,
        });
    }
    let live = providers
        .discovery
        .resolve_identity(pnpm.identity.canonical.as_path())
        .map_err(|error| GateFailure::PnpmIdentityChanged {
            detail: error.to_string(),
        })?;
    if live != pnpm.identity {
        return Err(GateFailure::PnpmIdentityChanged {
            detail: "the enrolled executable is no longer the same file".to_owned(),
        });
    }
    Ok(())
}

/// Identity of the policy that produced a plan.
///
/// Anything that could change which actions are legal is folded in, so a plan
/// built under one policy can be recognised as stale after the policy changes.
fn policy_hash(inputs: &PlanningInputs<'_>) -> String {
    let mut hasher = Fnv1a::new();
    hasher.write(b"terminal_janitor.policy.v1");
    hasher.write_u64(inputs.config.minimum_free_bytes);
    hasher.write_u64(inputs.config.target_free_bytes);
    for root in &inputs.config.approved_roots {
        hasher.write(root.as_os_str().as_encoded_bytes());
    }
    hasher.write_i64(REQUIRED_OBSERVATION_MILLIS);
    hasher.write_i64(REQUIRED_INACTIVITY_MILLIS);
    hasher.write_i64(WORKSPACE_COOLDOWN_MILLIS);
    hasher.write_i64(PLAN_VALIDITY_MILLIS);
    hasher.write_u64(MAX_AUTOMATIC_WORKSPACE_CLEANS as u64);
    hasher.write_u64(MINIMUM_PNPM_MAJOR);
    for action in [
        AllowedAction::CleanTerminalJanitorState.as_str(),
        AllowedAction::PnpmStorePrune.as_str(),
        "PNPM_WORKSPACE_CLEAN",
    ] {
        hasher.write(action.as_bytes());
    }
    match inputs.pnpm {
        Some(pnpm) => {
            hasher.write(pnpm.identity.canonical.as_str().as_bytes());
            hasher.write(pnpm.identity.volume.as_str().as_bytes());
            hasher.write(pnpm.identity.file.as_str().as_bytes());
            hasher.write(pnpm.version.as_str().as_bytes());
        }
        None => hasher.write(b"no-pnpm-enrolled"),
    }
    format!("policy-fnv1a64:{:016x}", hasher.finish())
}

/// Immutable plan identity: the policy, the moment, and the exact action list.
fn plan_id(policy_hash: &str, created_at_millis: i64, actions: &[PlannedAction]) -> String {
    let mut hasher = Fnv1a::new();
    hasher.write(policy_hash.as_bytes());
    hasher.write_i64(created_at_millis);
    for action in actions {
        hasher.write(action.action.as_bytes());
        if let Some(id) = &action.workspace_id {
            hasher.write(id.as_bytes());
        }
    }
    format!("plan-fnv1a64:{:016x}", hasher.finish())
}

/// Fixed FNV-1a, identical on every platform. These are identity tokens for
/// plans and policies, not security hashes.
struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
        // A length terminator stops two different field sequences from
        // concatenating to the same byte string.
        self.0 ^= bytes.len() as u64;
        self.0 = self.0.wrapping_mul(0x100000001b3);
    }

    fn write_u64(&mut self, value: u64) {
        self.write(&value.to_be_bytes());
    }

    fn write_i64(&mut self, value: i64) {
        self.write(&value.to_be_bytes());
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::pnpm::PnpmVersion;
    use crate::discovery::DirectoryEntry;
    use crate::identity::{CanonicalPath, FileId, PathIdentity};
    use crate::protection::ProtectionProbe;
    use crate::state::{ApprovedRootId, WorkspaceId};
    use std::collections::{BTreeMap, BTreeSet};
    use std::io;

    const DAY: i64 = 86_400_000;
    const NOW: i64 = 1_800_000_000_000;
    const ROOT: &str = "/approved";

    /// Test fixtures are written in one POSIX spelling and mapped to a single
    /// absolute platform path. Windows rejects a rootless path as an identity,
    /// so the same fixture must gain a drive prefix there.
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
    const VOLUME: &str = "unix-dev:66";

    struct FakeProvider {
        identities: BTreeMap<PathBuf, PathIdentity>,
        files: BTreeSet<PathBuf>,
    }

    impl DiscoveryProvider for FakeProvider {
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

        fn resolve_identity(&self, path: &Path) -> Result<PathIdentity, IdentityErrorAlias> {
            self.identities
                .get(path)
                .cloned()
                .ok_or_else(|| IdentityErrorAlias::NotAbsolute(path.to_path_buf()))
        }
    }

    type IdentityErrorAlias = crate::identity::IdentityError;

    #[derive(Default)]
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

    fn identity(path: &str, file_id: &str) -> PathIdentity {
        PathIdentity::from_parts(
            CanonicalPath::from_verified(abs(path)).unwrap(),
            VolumeId::new(VOLUME).unwrap(),
            FileId::new(file_id).unwrap(),
        )
    }

    fn approved_root() -> ApprovedRootRecord {
        ApprovedRootRecord {
            id: ApprovedRootId::for_tests("root-1"),
            canonical_path: CanonicalPath::from_verified(abs(ROOT)).unwrap(),
            volume_id: VolumeId::new(VOLUME).unwrap(),
            file_id: FileId::new("root-file").unwrap(),
            created_at: Timestamp::from_unix_millis(0),
            last_validated_at: Timestamp::from_unix_millis(NOW),
            active: true,
        }
    }

    /// A workspace that passes every gate, so each test can break exactly one
    /// thing and see exactly one gate refuse it.
    fn eligible_workspace(id: &str, path: &str) -> WorkspaceRecord {
        WorkspaceRecord {
            id: WorkspaceId::for_tests(id),
            canonical_path: CanonicalPath::from_verified(abs(path)).unwrap(),
            approved_root_id: ApprovedRootId::for_tests("root-1"),
            volume_id: VolumeId::new(VOLUME).unwrap(),
            file_id: FileId::new(id).unwrap(),
            first_observed_at: Timestamp::from_unix_millis(NOW - 120 * DAY),
            last_observed_at: Timestamp::from_unix_millis(NOW),
            last_activity_at: Timestamp::from_unix_millis(NOW - 90 * DAY),
            last_cleaned_at: None,
            protected: false,
            status: WorkspaceStatus::Present,
            git_state: GitState::Clean,
            git_head_fingerprint: None,
            git_index_fingerprint: None,
        }
    }

    fn enrolment() -> PnpmEnrolment {
        PnpmEnrolment {
            identity: identity("/usr/local/bin/pnpm", "pnpm-file"),
            version: PnpmVersion::parse("11.2.3").unwrap(),
            enrolled_at: Timestamp::from_unix_millis(0),
        }
    }

    fn provider(workspaces: &[&WorkspaceRecord], pnpm: Option<&PnpmEnrolment>) -> FakeProvider {
        let mut identities = BTreeMap::new();
        let mut files = BTreeSet::new();
        for workspace in workspaces {
            let path = workspace.canonical_path.as_path();
            identities.insert(
                path.to_path_buf(),
                PathIdentity::from_parts(
                    workspace.canonical_path.clone(),
                    workspace.volume_id.clone(),
                    workspace.file_id.clone(),
                ),
            );
            for marker in REQUIRED_MARKERS {
                files.insert(path.join(marker));
            }
        }
        if let Some(pnpm) = pnpm {
            identities.insert(
                pnpm.identity.canonical.as_path().to_path_buf(),
                pnpm.identity.clone(),
            );
        }
        FakeProvider { identities, files }
    }

    struct Fixture {
        workspaces: Vec<WorkspaceRecord>,
        roots: Vec<ApprovedRootRecord>,
        pnpm: Option<PnpmEnrolment>,
        config: Config,
        denied: DeniedRoots,
        liveness: Liveness,
        git: GitState,
        now: i64,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                workspaces: vec![eligible_workspace("workspace-a", "/approved/a")],
                roots: vec![approved_root()],
                pnpm: Some(enrolment()),
                config: Config::new(10 * 1024 * 1024 * 1024, 15 * 1024 * 1024 * 1024).unwrap(),
                denied: DeniedRoots::default(),
                liveness: Liveness::Inactive,
                git: GitState::Clean,
                now: NOW,
            }
        }

        fn build(&self) -> Plan {
            let borrowed: Vec<&WorkspaceRecord> = self.workspaces.iter().collect();
            let discovery = provider(&borrowed, self.pnpm.as_ref());
            let liveness = FixedLiveness(self.liveness);
            let git = FixedGit(self.git);
            let probe = NoMarkers;
            plan(
                &PlanningInputs {
                    workspaces: &self.workspaces,
                    approved_roots: &self.roots,
                    pnpm: self.pnpm.as_ref(),
                    pressured_volume: &VolumeId::new(VOLUME).unwrap(),
                    config: &self.config,
                    now: Timestamp::from_unix_millis(self.now),
                },
                &PlanningProviders {
                    discovery: &discovery,
                    protection_probe: &probe,
                    denied_roots: &self.denied,
                    liveness: &liveness,
                    git: &git,
                },
            )
        }
    }

    fn single_gate(plan: &Plan) -> &'static str {
        assert_eq!(
            plan.skipped.len(),
            1,
            "exactly one candidate must be judged"
        );
        assert_eq!(plan.workspace_clean_count(), 0);
        plan.skipped[0].gate
    }

    #[test]
    fn a_fully_proven_workspace_is_planned_with_a_complete_proof_bundle() {
        let plan = Fixture::new().build();
        assert_eq!(plan.workspace_clean_count(), 1);
        assert!(plan.skipped.is_empty());
        let clean = plan
            .actions
            .iter()
            .find(|action| action.action == "PNPM_WORKSPACE_CLEAN")
            .unwrap();
        assert!(clean.proof.complete());
        assert_eq!(clean.path.as_deref(), Some(abs("/approved/a").as_path()));
    }

    #[test]
    fn own_state_cleanup_and_store_prune_are_planned_without_a_workspace() {
        let plan = Fixture::new().build();
        let names: Vec<_> = plan.actions.iter().map(|action| action.action).collect();
        assert_eq!(names[0], "CLEAN_TERMINAL_JANITOR_STATE");
        assert_eq!(names[1], "PNPM_STORE_PRUNE");
        assert!(plan.actions.iter().all(|action| action.proof.complete()));
    }

    #[test]
    fn pnpm_below_eleven_refuses_every_workspace_and_the_store_prune() {
        let mut fixture = Fixture::new();
        fixture.pnpm = Some(PnpmEnrolment {
            version: PnpmVersion::parse("10.9.0").unwrap(),
            ..enrolment()
        });
        let plan = fixture.build();
        assert_eq!(single_gate(&plan), "PNPM_BELOW_MINIMUM_VERSION");
        assert!(
            !plan
                .actions
                .iter()
                .any(|action| action.action == "PNPM_STORE_PRUNE")
        );
    }

    #[test]
    fn an_unenrolled_pnpm_refuses_every_workspace() {
        let mut fixture = Fixture::new();
        fixture.pnpm = None;
        assert_eq!(single_gate(&fixture.build()), "PNPM_NOT_ENROLLED");
    }

    #[test]
    fn each_missing_marker_refuses_the_workspace() {
        for marker in REQUIRED_MARKERS {
            let fixture = Fixture::new();
            let borrowed: Vec<&WorkspaceRecord> = fixture.workspaces.iter().collect();
            let mut discovery = provider(&borrowed, fixture.pnpm.as_ref());
            discovery.files.remove(&abs("/approved/a").join(marker));
            let liveness = FixedLiveness(Liveness::Inactive);
            let git = FixedGit(GitState::Clean);
            let probe = NoMarkers;
            let plan = plan(
                &PlanningInputs {
                    workspaces: &fixture.workspaces,
                    approved_roots: &fixture.roots,
                    pnpm: fixture.pnpm.as_ref(),
                    pressured_volume: &VolumeId::new(VOLUME).unwrap(),
                    config: &fixture.config,
                    now: Timestamp::from_unix_millis(NOW),
                },
                &PlanningProviders {
                    discovery: &discovery,
                    protection_probe: &probe,
                    denied_roots: &fixture.denied,
                    liveness: &liveness,
                    git: &git,
                },
            );
            assert_eq!(single_gate(&plan), "MISSING_MARKER", "missing {marker}");
        }
    }

    #[test]
    fn a_dirty_or_unknown_worktree_refuses_the_workspace() {
        let mut fixture = Fixture::new();
        fixture.git = GitState::Dirty;
        assert_eq!(single_gate(&fixture.build()), "GIT_WORKTREE_DIRTY");

        fixture.git = GitState::Unknown;
        assert_eq!(single_gate(&fixture.build()), "GIT_STATE_UNKNOWN");
    }

    #[test]
    fn an_active_or_unknown_liveness_result_refuses_the_workspace() {
        let mut fixture = Fixture::new();
        fixture.liveness = Liveness::Active;
        assert_eq!(single_gate(&fixture.build()), "WORKSPACE_ACTIVE");

        fixture.liveness = Liveness::Unknown;
        assert_eq!(single_gate(&fixture.build()), "LIVENESS_UNKNOWN");
    }

    #[test]
    fn the_day_three_production_liveness_prover_plans_no_workspace_clean() {
        let fixture = Fixture::new();
        let borrowed: Vec<&WorkspaceRecord> = fixture.workspaces.iter().collect();
        let discovery = provider(&borrowed, fixture.pnpm.as_ref());
        let probe = NoMarkers;
        let git = FixedGit(GitState::Clean);
        let plan = plan(
            &PlanningInputs {
                workspaces: &fixture.workspaces,
                approved_roots: &fixture.roots,
                pnpm: fixture.pnpm.as_ref(),
                pressured_volume: &VolumeId::new(VOLUME).unwrap(),
                config: &fixture.config,
                now: Timestamp::from_unix_millis(NOW),
            },
            &PlanningProviders {
                discovery: &discovery,
                protection_probe: &probe,
                denied_roots: &fixture.denied,
                liveness: &UnavailableLivenessProver,
                git: &git,
            },
        );
        assert_eq!(plan.workspace_clean_count(), 0);
        assert_eq!(plan.skipped[0].gate, "LIVENESS_UNKNOWN");
    }

    #[test]
    fn a_protected_workspace_and_a_cloud_synchronised_location_are_refused() {
        let mut fixture = Fixture::new();
        fixture.workspaces[0].protected = true;
        assert_eq!(single_gate(&fixture.build()), "PROTECTED");

        let mut cloud = Fixture::new();
        cloud.workspaces = vec![eligible_workspace("workspace-a", "/approved/Dropbox/a")];
        assert_eq!(single_gate(&cloud.build()), "PROTECTED");
    }

    #[test]
    fn a_denied_root_refuses_the_workspace() {
        let mut fixture = Fixture::new();
        fixture.denied = DeniedRoots::from_paths(vec![(abs("/approved/a"), "downloads")]);
        assert_eq!(single_gate(&fixture.build()), "PROTECTED");
    }

    #[test]
    fn a_new_workspace_and_a_recently_active_workspace_are_refused() {
        let mut fixture = Fixture::new();
        fixture.workspaces[0].first_observed_at = Timestamp::from_unix_millis(NOW - 3_600_000);
        assert_eq!(single_gate(&fixture.build()), "OBSERVATION_WINDOW_NOT_MET");

        let mut recent = Fixture::new();
        recent.workspaces[0].last_activity_at = Timestamp::from_unix_millis(NOW - 5 * DAY);
        assert_eq!(single_gate(&recent.build()), "RECENT_ACTIVITY");
    }

    #[test]
    fn a_recently_cleaned_workspace_is_inside_its_cooldown() {
        let mut fixture = Fixture::new();
        fixture.workspaces[0].last_cleaned_at = Timestamp::from_unix_millis(NOW - DAY).into();
        assert_eq!(single_gate(&fixture.build()), "COOLDOWN_ACTIVE");
    }

    #[test]
    fn a_missing_replaced_or_moved_workspace_is_refused() {
        let mut missing = Fixture::new();
        missing.workspaces[0].status = WorkspaceStatus::Missing;
        assert_eq!(single_gate(&missing.build()), "WORKSPACE_NOT_PRESENT");

        let mut replaced = Fixture::new();
        replaced.workspaces[0].status = WorkspaceStatus::Replaced;
        assert_eq!(single_gate(&replaced.build()), "WORKSPACE_NOT_PRESENT");

        // The ledger says one file identity; the filesystem now reports another.
        let mut moved = Fixture::new();
        moved.workspaces[0].file_id = FileId::new("a-different-inode").unwrap();
        let borrowed: Vec<&WorkspaceRecord> = moved.workspaces.iter().collect();
        let mut discovery = provider(&borrowed, moved.pnpm.as_ref());
        discovery
            .identities
            .insert(abs("/approved/a"), identity("/approved/a", "a"));
        let liveness = FixedLiveness(Liveness::Inactive);
        let git = FixedGit(GitState::Clean);
        let probe = NoMarkers;
        let built = plan(
            &PlanningInputs {
                workspaces: &moved.workspaces,
                approved_roots: &moved.roots,
                pnpm: moved.pnpm.as_ref(),
                pressured_volume: &VolumeId::new(VOLUME).unwrap(),
                config: &moved.config,
                now: Timestamp::from_unix_millis(NOW),
            },
            &PlanningProviders {
                discovery: &discovery,
                protection_probe: &probe,
                denied_roots: &moved.denied,
                liveness: &liveness,
                git: &git,
            },
        );
        assert_eq!(single_gate(&built), "TARGET_IDENTITY_CHANGED");
    }

    #[test]
    fn a_workspace_outside_every_approved_root_is_refused() {
        let mut fixture = Fixture::new();
        fixture.workspaces = vec![eligible_workspace("workspace-a", "/elsewhere/a")];
        assert_eq!(single_gate(&fixture.build()), "NOT_INSIDE_APPROVED_ROOT");
    }

    #[test]
    fn an_inactive_approved_root_grants_no_authority() {
        let mut fixture = Fixture::new();
        fixture.roots[0].active = false;
        assert_eq!(single_gate(&fixture.build()), "NOT_INSIDE_APPROVED_ROOT");
    }

    #[test]
    fn a_workspace_on_another_volume_is_refused() {
        let mut fixture = Fixture::new();
        fixture.workspaces[0].volume_id = VolumeId::new("unix-dev:99").unwrap();
        assert_eq!(single_gate(&fixture.build()), "DIFFERENT_VOLUME");
    }

    #[test]
    fn ordering_is_oldest_activity_then_oldest_observation_then_path() {
        let mut fixture = Fixture::new();
        let mut newest = eligible_workspace("workspace-c", "/approved/c");
        newest.last_activity_at = Timestamp::from_unix_millis(NOW - 40 * DAY);
        let mut oldest = eligible_workspace("workspace-b", "/approved/b");
        oldest.last_activity_at = Timestamp::from_unix_millis(NOW - 200 * DAY);
        fixture.workspaces = vec![newest, oldest];

        let plan = fixture.build();
        let cleaned: Vec<_> = plan
            .actions
            .iter()
            .filter_map(|action| action.path.as_deref())
            .collect();
        assert_eq!(cleaned, vec![abs("/approved/b"), abs("/approved/c")]);
    }

    #[test]
    fn ties_break_on_first_observation_then_canonical_path() {
        let mut fixture = Fixture::new();
        let mut second = eligible_workspace("workspace-b", "/approved/b");
        second.first_observed_at = Timestamp::from_unix_millis(NOW - 200 * DAY);
        let mut third = eligible_workspace("workspace-c", "/approved/c");
        third.first_observed_at = Timestamp::from_unix_millis(NOW - 200 * DAY);
        fixture.workspaces = vec![third, second];

        let plan = fixture.build();
        let cleaned: Vec<_> = plan
            .actions
            .iter()
            .filter_map(|action| action.path.as_deref())
            .collect();
        assert_eq!(cleaned, vec![abs("/approved/b"), abs("/approved/c")]);
    }

    #[test]
    fn no_run_plans_more_than_two_workspace_cleans() {
        let mut fixture = Fixture::new();
        fixture.workspaces = ["a", "b", "c", "d"]
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let mut workspace = eligible_workspace(name, &format!("/approved/{name}"));
                workspace.last_activity_at =
                    Timestamp::from_unix_millis(NOW - (200 - index as i64) * DAY);
                workspace
            })
            .collect();

        let plan = fixture.build();
        assert_eq!(plan.workspace_clean_count(), MAX_AUTOMATIC_WORKSPACE_CLEANS);
        assert_eq!(plan.skipped.len(), 2);
        assert!(
            plan.skipped
                .iter()
                .all(|skipped| skipped.gate == "AUTOMATIC_WORKSPACE_CAP_REACHED")
        );
    }

    #[test]
    fn identical_state_produces_an_identical_plan() {
        let first = Fixture::new().build();
        let second = Fixture::new().build();
        assert_eq!(first, second);
        assert_eq!(first.plan_id, second.plan_id);
        assert_eq!(first.policy_hash, second.policy_hash);
    }

    #[test]
    fn a_changed_policy_changes_the_policy_hash_and_the_plan_id() {
        let baseline = Fixture::new().build();

        let mut thresholds = Fixture::new();
        thresholds.config = Config::new(1024, 4096).unwrap();
        let changed = thresholds.build();
        assert_ne!(baseline.policy_hash, changed.policy_hash);
        assert_ne!(baseline.plan_id, changed.plan_id);

        let mut other_pnpm = Fixture::new();
        other_pnpm.pnpm = Some(PnpmEnrolment {
            version: PnpmVersion::parse("11.9.9").unwrap(),
            ..enrolment()
        });
        assert_ne!(baseline.policy_hash, other_pnpm.build().policy_hash);
    }

    #[test]
    fn a_different_moment_produces_a_different_plan_id() {
        let baseline = Fixture::new().build();
        let mut later = Fixture::new();
        later.now = NOW + 1;
        let later = later.build();
        assert_eq!(baseline.policy_hash, later.policy_hash);
        assert_ne!(baseline.plan_id, later.plan_id);
    }

    #[test]
    fn a_plan_expires_and_reports_it() {
        let plan = Fixture::new().build();
        assert_eq!(
            plan.expires_at_millis,
            plan.created_at_millis + PLAN_VALIDITY_MILLIS
        );
        assert!(!plan.is_expired(Timestamp::from_unix_millis(NOW)));
        assert!(!plan.is_expired(Timestamp::from_unix_millis(plan.expires_at_millis - 1)));
        assert!(plan.is_expired(Timestamp::from_unix_millis(plan.expires_at_millis)));
        assert!(plan.is_expired(Timestamp::from_unix_millis(plan.expires_at_millis + DAY)));
    }

    #[test]
    fn every_skip_names_exactly_one_gate_with_a_readable_reason() {
        let mut fixture = Fixture::new();
        fixture.git = GitState::Dirty;
        let plan = fixture.build();
        let skipped = &plan.skipped[0];
        assert_eq!(skipped.gate, "GIT_WORKTREE_DIRTY");
        assert!(!skipped.reason.is_empty());
        assert_eq!(skipped.workspace_id, "workspace-a");
        assert_eq!(skipped.path, abs("/approved/a"));
    }

    #[test]
    fn a_partial_proof_can_never_reach_the_action_list() {
        let plan = Fixture::new().build();
        for action in &plan.actions {
            assert!(
                action.proof.complete(),
                "{} was planned with an incomplete proof",
                action.action
            );
        }
    }

    #[test]
    fn an_incomplete_bundle_is_never_complete() {
        let mut proof = own_state_proof();
        assert!(proof.complete());
        proof.inactivity_proven = false;
        assert!(!proof.complete());
        assert!(!ProofBundle::default().complete());
    }
}
