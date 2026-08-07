//! Persistent metadata ledger for approved roots and registered workspaces
//! (Day 2A).
//!
//! This module owns persistence and identity invariants only. It stores
//! metadata — canonical paths, native volume/file identities, observation
//! timestamps, and protection flags. It never stores file contents, secrets,
//! environment values, or command history, and it exposes no raw SQL to the
//! rest of the crate. Discovery, registration workflows, planning, and
//! cleanup live in later slices and must go through this API.
//!
//! Fail-closed rules: a corrupt database, an unknown database, or a database
//! written by a newer schema version is rejected with an explicit error. The
//! existing file is never renamed, deleted, or recreated in response —
//! protection state must never disappear through automatic recovery.

use std::cell::Cell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::identity::{CanonicalPath, FileId, IdentityError, PathIdentity, VolumeId};
use crate::journal::{
    ActionId, ActionOutcome, ActionRecord, ActionState, MAX_DETAILED_RUNS, NewAction, NewRun,
    RunId, RunRecord, RunResult,
};

/// Normal maximum ledger size (SAFETY.md section 12).
pub const MAX_STATE_DB_BYTES: u64 = 20 * 1024 * 1024;

/// Current schema version; equals the number of entries in `MIGRATIONS`.
pub const SCHEMA_VERSION: u32 = 4;

/// Ordered, explicit migrations. Each entry upgrades the schema by exactly
/// one version and runs inside one transaction together with the version
/// bump, so a failure rolls the database back to its previous version.
const MIGRATIONS: &[&str] = &[
    // v1: schema metadata, approved roots, workspaces.
    "CREATE TABLE schema_meta (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        version INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE approved_roots (
        id TEXT PRIMARY KEY,
        canonical_path TEXT NOT NULL,
        volume_id TEXT NOT NULL,
        file_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        last_validated_at INTEGER NOT NULL,
        active INTEGER NOT NULL CHECK (active IN (0, 1))
    ) STRICT;
    CREATE UNIQUE INDEX approved_roots_by_path
        ON approved_roots (volume_id, canonical_path);
    CREATE INDEX approved_roots_by_file
        ON approved_roots (volume_id, file_id);
    CREATE TABLE workspaces (
        id TEXT PRIMARY KEY,
        canonical_path TEXT NOT NULL,
        approved_root_id TEXT NOT NULL REFERENCES approved_roots (id),
        volume_id TEXT NOT NULL,
        file_id TEXT NOT NULL,
        first_observed_at INTEGER NOT NULL,
        last_observed_at INTEGER NOT NULL,
        last_activity_at INTEGER NOT NULL,
        last_cleaned_at INTEGER,
        protected INTEGER NOT NULL CHECK (protected IN (0, 1)),
        status TEXT NOT NULL CHECK (status IN ('present', 'replaced'))
    ) STRICT;
    CREATE UNIQUE INDEX workspaces_present_by_path
        ON workspaces (volume_id, canonical_path) WHERE status = 'present';
    CREATE INDEX workspaces_by_file
        ON workspaces (volume_id, file_id);",
    // v2: retain absent workspaces and the bounded fingerprints needed to
    // detect Git HEAD/index movement without storing project contents.
    "ALTER TABLE workspaces RENAME TO workspaces_v1;
    CREATE TABLE workspaces (
        id TEXT PRIMARY KEY,
        canonical_path TEXT NOT NULL,
        approved_root_id TEXT NOT NULL REFERENCES approved_roots (id),
        volume_id TEXT NOT NULL,
        file_id TEXT NOT NULL,
        first_observed_at INTEGER NOT NULL,
        last_observed_at INTEGER NOT NULL,
        last_activity_at INTEGER NOT NULL,
        last_cleaned_at INTEGER,
        protected INTEGER NOT NULL CHECK (protected IN (0, 1)),
        status TEXT NOT NULL CHECK (status IN ('present', 'missing', 'replaced')),
        git_state TEXT NOT NULL CHECK (git_state IN ('unknown', 'clean', 'dirty')),
        git_head_fingerprint TEXT,
        git_index_fingerprint TEXT
    ) STRICT;
    INSERT INTO workspaces
        (id, canonical_path, approved_root_id, volume_id, file_id,
         first_observed_at, last_observed_at, last_activity_at,
         last_cleaned_at, protected, status, git_state,
         git_head_fingerprint, git_index_fingerprint)
    SELECT id, canonical_path, approved_root_id, volume_id, file_id,
           first_observed_at, last_observed_at, last_activity_at,
           last_cleaned_at, protected, status, 'unknown', NULL, NULL
    FROM workspaces_v1;
    DROP TABLE workspaces_v1;
    CREATE UNIQUE INDEX workspaces_live_by_path
        ON workspaces (volume_id, canonical_path)
        WHERE status IN ('present', 'missing');
    CREATE INDEX workspaces_by_file
        ON workspaces (volume_id, file_id);",
    // v3: the single enrolled pnpm executable. One row by construction, so a
    // second enrolment replaces the first rather than leaving two executables
    // with cleanup authority.
    "CREATE TABLE pnpm_enrolment (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        canonical_path TEXT NOT NULL,
        volume_id TEXT NOT NULL,
        file_id TEXT NOT NULL,
        version TEXT NOT NULL,
        enrolled_at INTEGER NOT NULL
    ) STRICT;",
    // v4: the run and action journal. An action row exists before the action
    // is attempted, so an interruption leaves a readable record rather than an
    // absent one. `result` and `finished_at` stay NULL until a run completes,
    // which is exactly how an interrupted run is recognised.
    "CREATE TABLE runs (
        id TEXT PRIMARY KEY,
        plan_id TEXT NOT NULL,
        policy_hash TEXT NOT NULL,
        volume_id TEXT NOT NULL,
        result TEXT,
        free_before_bytes INTEGER NOT NULL,
        free_after_bytes INTEGER,
        minimum_free_bytes INTEGER NOT NULL,
        target_free_bytes INTEGER NOT NULL,
        started_at INTEGER NOT NULL,
        finished_at INTEGER
    ) STRICT;
    CREATE INDEX runs_by_start ON runs (started_at DESC, id);
    CREATE TABLE run_actions (
        id INTEGER PRIMARY KEY,
        run_id TEXT NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
        sequence INTEGER NOT NULL,
        action TEXT NOT NULL,
        workspace_id TEXT,
        canonical_path TEXT,
        state TEXT NOT NULL CHECK (state IN (
            'PLANNED', 'VALIDATING', 'RUNNING', 'SUCCEEDED', 'FAILED',
            'SKIPPED_CHANGED', 'SKIPPED_ACTIVE', 'SKIPPED_PROTECTED',
            'SKIPPED_UNKNOWN'
        )),
        free_before_bytes INTEGER NOT NULL,
        free_after_bytes INTEGER,
        actual_bytes INTEGER,
        detail TEXT,
        recovery TEXT,
        started_at INTEGER NOT NULL,
        finished_at INTEGER
    ) STRICT;
    CREATE UNIQUE INDEX run_actions_by_sequence ON run_actions (run_id, sequence);",
];

/// Milliseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    pub fn from_unix_millis(millis: i64) -> Self {
        Self(millis)
    }

    pub fn as_unix_millis(self) -> i64 {
        self.0
    }
}

/// Injectable time source. Production uses [`SystemClock`]; tests inject a
/// deterministic clock instead of sleeping. The store itself enforces that
/// observation timestamps never move backwards, so even a misbehaving clock
/// cannot rewind recorded history.
pub trait Clock {
    fn now(&self) -> Timestamp;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => Timestamp(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)),
            Err(before_epoch) => Timestamp(
                i64::try_from(before_epoch.duration().as_millis())
                    .map(|millis| -millis)
                    .unwrap_or(i64::MIN),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApprovedRootId(String);

impl ApprovedRootId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Test-only construction; see [`WorkspaceId::for_tests`].
    #[cfg(test)]
    pub(crate) fn for_tests(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ApprovedRootId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Test-only construction. Production identities are only ever produced by
    /// the ledger, so no caller can name a workspace the ledger does not hold.
    #[cfg(test)]
    pub(crate) fn for_tests(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceStatus {
    /// The workspace was present at its canonical path when last observed.
    Present,
    /// The owning approved root was scanned successfully, but the workspace
    /// was not observed. Its identity and protection remain in the ledger.
    Missing,
    /// A different physical directory now occupies this record's canonical
    /// path; the record is retained (including its protection flag) but no
    /// longer carries live authority.
    Replaced,
}

impl WorkspaceStatus {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "present" => Some(Self::Present),
            "missing" => Some(Self::Missing),
            "replaced" => Some(Self::Replaced),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitState {
    Unknown,
    Clean,
    Dirty,
}

impl GitState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Clean => "clean",
            Self::Dirty => "dirty",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "unknown" => Some(Self::Unknown),
            "clean" => Some(Self::Clean),
            "dirty" => Some(Self::Dirty),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewApprovedRoot {
    pub identity: PathIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedRootRecord {
    pub id: ApprovedRootId,
    pub canonical_path: CanonicalPath,
    pub volume_id: VolumeId,
    pub file_id: FileId,
    pub created_at: Timestamp,
    pub last_validated_at: Timestamp,
    pub active: bool,
}

/// One observation of a workspace, produced by a later discovery slice. The
/// observation carries the physically resolved identity plus the newest
/// activity evidence the observer holds (for example a manifest mtime).
#[derive(Debug, Clone)]
pub struct WorkspaceObservation {
    pub identity: PathIdentity,
    pub approved_root_id: ApprovedRootId,
    /// Newest activity evidence, if any. When absent, a newly inserted
    /// record conservatively uses the observation time — treating an
    /// unknown-activity workspace as recently active delays cleanup, never
    /// accelerates it. Existing records never move backwards.
    pub activity_at: Option<Timestamp>,
    /// Day 2 records uncertainty explicitly. The production filesystem probe
    /// does not invoke Git, so worktree state is normally `Unknown` for now.
    pub git_state: GitState,
    /// Stable bounded fingerprints, never raw Git or project contents.
    pub git_head_fingerprint: Option<String>,
    pub git_index_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub id: WorkspaceId,
    pub canonical_path: CanonicalPath,
    pub approved_root_id: ApprovedRootId,
    pub volume_id: VolumeId,
    pub file_id: FileId,
    pub first_observed_at: Timestamp,
    pub last_observed_at: Timestamp,
    pub last_activity_at: Timestamp,
    pub last_cleaned_at: Option<Timestamp>,
    pub protected: bool,
    pub status: WorkspaceStatus,
    pub git_state: GitState,
    pub git_head_fingerprint: Option<String>,
    pub git_index_fingerprint: Option<String>,
}

/// The stored identity of the one enrolled pnpm executable.
///
/// The version is kept as the text pnpm reported. The ledger deliberately does
/// not parse it: persistence stores what was observed, and the pnpm adapter
/// owns the meaning of that text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PnpmEnrolmentRecord {
    pub identity: PathIdentity,
    pub version: String,
    pub enrolled_at: Timestamp,
}

#[derive(Debug, Clone)]
pub struct ValidatedRoot {
    pub id: ApprovedRootId,
    pub identity: PathIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceChangeKind {
    Registered,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceChange {
    pub kind: WorkspaceChangeKind,
    pub workspace: WorkspaceRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanStateResult {
    pub changes: Vec<WorkspaceChange>,
    pub missing: Vec<WorkspaceRecord>,
}

#[derive(Debug)]
pub enum StateError {
    /// The database file cannot be opened or its directory is unusable.
    Unavailable {
        path: PathBuf,
        message: String,
    },
    /// The database exists but is corrupt or not a terminal_janitor ledger.
    /// The file is left untouched; it is never deleted or recreated.
    Corrupt {
        path: PathBuf,
        message: String,
    },
    /// The database was written by a newer terminal_janitor.
    UnsupportedSchemaVersion {
        found: u32,
        supported: u32,
    },
    MigrationFailed {
        message: String,
    },
    TransactionFailed {
        message: String,
    },
    /// A state change would grow the ledger past its bound; the change was
    /// rolled back and the previous state preserved.
    SizeLimitExceeded {
        size_bytes: u64,
        limit_bytes: u64,
    },
    /// A stored or supplied path violates the canonical-identity invariants.
    InvalidPath {
        message: String,
    },
    VolumeIdentityUnavailable {
        message: String,
    },
    /// Two records would claim the same physical identity, or an identity
    /// conflicts with an existing record in a way that cannot be collapsed
    /// safely.
    ConflictingIdentity {
        message: String,
    },
    UnknownApprovedRoot {
        id: ApprovedRootId,
    },
    UnknownWorkspace {
        id: WorkspaceId,
    },
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { path, message } => {
                write!(
                    f,
                    "state database {} unavailable: {message}",
                    path.display()
                )
            }
            Self::Corrupt { path, message } => write!(
                f,
                "state database {} is corrupt or not a terminal_janitor ledger \
                 ({message}); it was left untouched and must be inspected manually",
                path.display()
            ),
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                f,
                "state database schema version {found} is newer than supported \
                 version {supported}; refusing to open it"
            ),
            Self::MigrationFailed { message } => {
                write!(f, "state migration failed and was rolled back: {message}")
            }
            Self::TransactionFailed { message } => {
                write!(f, "state transaction failed: {message}")
            }
            Self::SizeLimitExceeded {
                size_bytes,
                limit_bytes,
            } => write!(
                f,
                "state database size {size_bytes} bytes exceeds the {limit_bytes}-byte \
                 limit; the change was rejected and prior state preserved"
            ),
            Self::InvalidPath { message } => write!(f, "invalid state path: {message}"),
            Self::VolumeIdentityUnavailable { message } => {
                write!(f, "volume identity unavailable: {message}")
            }
            Self::ConflictingIdentity { message } => {
                write!(f, "conflicting identity: {message}")
            }
            Self::UnknownApprovedRoot { id } => {
                write!(f, "approved root {id} does not exist in the ledger")
            }
            Self::UnknownWorkspace { id } => {
                write!(f, "workspace {id} does not exist in the ledger")
            }
        }
    }
}

impl std::error::Error for StateError {}

impl From<IdentityError> for StateError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::VolumeIdentityUnavailable { .. } => Self::VolumeIdentityUnavailable {
                message: error.to_string(),
            },
            _ => Self::InvalidPath {
                message: error.to_string(),
            },
        }
    }
}

/// Narrow persistence API for Day 2B registration and discovery. Callers
/// never see SQL or the underlying connection.
pub trait StateStore {
    fn schema_version(&self) -> Result<u32, StateError>;
    fn add_approved_root(
        &mut self,
        root: NewApprovedRoot,
    ) -> Result<ApprovedRootRecord, StateError>;
    fn list_approved_roots(&self) -> Result<Vec<ApprovedRootRecord>, StateError>;
    fn upsert_workspace_observation(
        &mut self,
        observation: WorkspaceObservation,
    ) -> Result<WorkspaceRecord, StateError>;
    fn get_workspace(&self, id: &WorkspaceId) -> Result<Option<WorkspaceRecord>, StateError>;
    fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>, StateError>;
    fn set_protected(&mut self, id: &WorkspaceId, protected: bool) -> Result<(), StateError>;
    /// Replaces the enrolled pnpm executable. Enrolment is deliberately
    /// singular: exactly one executable may ever hold cleanup authority.
    fn set_pnpm_enrolment(&mut self, enrolment: &PnpmEnrolmentRecord) -> Result<(), StateError>;
    fn get_pnpm_enrolment(&self) -> Result<Option<PnpmEnrolmentRecord>, StateError>;

    /// Opens a run in the journal. Called before any action is attempted.
    fn begin_run(&mut self, run: &NewRun) -> Result<(), StateError>;
    /// Writes one action down before it is attempted, in state `PLANNED`.
    fn record_planned_action(&mut self, action: &NewAction) -> Result<ActionId, StateError>;
    /// Moves an action to a non-terminal state as execution proceeds.
    fn advance_action(&mut self, id: ActionId, state: ActionState) -> Result<(), StateError>;
    /// Closes an action with its outcome and measured figures.
    fn finish_action(&mut self, id: ActionId, outcome: &ActionOutcome) -> Result<(), StateError>;
    /// Closes a run. Until this is called the run reads as interrupted.
    fn finish_run(
        &mut self,
        run_id: &RunId,
        result: RunResult,
        free_after_bytes: Option<u64>,
        finished_at: Timestamp,
    ) -> Result<(), StateError>;
    /// Most recent runs first, newest `limit` runs, each with its actions.
    fn list_runs(&self, limit: usize) -> Result<Vec<RunRecord>, StateError>;
    /// Marks the workspace as cleaned so its cooldown starts.
    fn record_workspace_cleaned(
        &mut self,
        id: &WorkspaceId,
        cleaned_at: Timestamp,
    ) -> Result<(), StateError>;
}

/// Bundled-SQLite implementation of [`StateStore`].
pub struct SqliteStateStore {
    conn: Connection,
    db_path: PathBuf,
    clock: Box<dyn Clock>,
    max_db_bytes: u64,
}

#[derive(Debug)]
pub enum CoordinatedWriteError<E> {
    State(StateError),
    External(E),
    StateCommit(StateError),
    StateCommitAndRollback { state: StateError, rollback: E },
}

impl fmt::Debug for SqliteStateStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqliteStateStore")
            .field("db_path", &self.db_path)
            .field("max_db_bytes", &self.max_db_bytes)
            .finish_non_exhaustive()
    }
}

impl SqliteStateStore {
    /// Opens (creating if absent) the ledger at `path` with the system clock.
    pub fn open(path: &Path) -> Result<Self, StateError> {
        Self::open_inner(path, Box::new(SystemClock), MAX_STATE_DB_BYTES)
    }

    /// Opens the ledger with an injected clock (Day 2B workflows and tests).
    pub fn open_with_clock(path: &Path, clock: Box<dyn Clock>) -> Result<Self, StateError> {
        Self::open_inner(path, clock, MAX_STATE_DB_BYTES)
    }

    fn open_inner(
        path: &Path,
        clock: Box<dyn Clock>,
        max_db_bytes: u64,
    ) -> Result<Self, StateError> {
        let conn = Connection::open(path).map_err(|error| open_error(path, &error))?;
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(|error| open_error(path, &error))?;

        // Integrity first: a damaged ledger must be rejected before any
        // schema inspection could misread it.
        let check: String = conn
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(|error| open_error(path, &error))?;
        if check != "ok" {
            return Err(StateError::Corrupt {
                path: path.to_path_buf(),
                message: format!("integrity check reported: {check}"),
            });
        }

        let mut store = Self {
            conn,
            db_path: path.to_path_buf(),
            clock,
            max_db_bytes,
        };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&mut self) -> Result<(), StateError> {
        let has_meta: bool = self
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_meta'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count > 0)
            .map_err(|error| self.op_error(&error))?;

        let from_version = if has_meta {
            let version: Option<i64> = self
                .conn
                .query_row("SELECT version FROM schema_meta WHERE id = 1", [], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(|error| self.op_error(&error))?;
            let Some(version) = version else {
                return Err(StateError::Corrupt {
                    path: self.db_path.clone(),
                    message: "schema_meta exists but holds no version row".to_owned(),
                });
            };
            u32::try_from(version).map_err(|_| StateError::Corrupt {
                path: self.db_path.clone(),
                message: format!("schema_meta holds invalid version {version}"),
            })?
        } else {
            // No schema metadata. Only a completely empty database may be
            // initialised; anything else is not a terminal_janitor ledger
            // and must not be modified.
            let object_count: i64 = self
                .conn
                .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
                .map_err(|error| self.op_error(&error))?;
            if object_count != 0 {
                return Err(StateError::Corrupt {
                    path: self.db_path.clone(),
                    message: "database contains objects but no terminal_janitor schema metadata"
                        .to_owned(),
                });
            }
            0
        };

        if from_version > SCHEMA_VERSION {
            return Err(StateError::UnsupportedSchemaVersion {
                found: from_version,
                supported: SCHEMA_VERSION,
            });
        }
        if from_version < SCHEMA_VERSION {
            apply_migrations(&mut self.conn, from_version, MIGRATIONS)?;
        }
        Ok(())
    }

    fn op_error(&self, error: &rusqlite::Error) -> StateError {
        if is_corruption(error) {
            StateError::Corrupt {
                path: self.db_path.clone(),
                message: error.to_string(),
            }
        } else {
            StateError::TransactionFailed {
                message: error.to_string(),
            }
        }
    }

    fn database_size(conn: &Connection) -> Result<u64, rusqlite::Error> {
        let page_count: u64 = conn.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let page_size: u64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
        Ok(page_count.saturating_mul(page_size))
    }

    /// Runs one state-changing transaction with the size bound checked both
    /// before starting and before committing. A bound violation rolls the
    /// transaction back, so the previous state always survives intact.
    fn mutate<T>(
        &mut self,
        operation: impl FnOnce(&Transaction<'_>, Timestamp) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        let size_before = Self::database_size(&self.conn).map_err(|error| self.op_error(&error))?;
        if size_before > self.max_db_bytes {
            return Err(StateError::SizeLimitExceeded {
                size_bytes: size_before,
                limit_bytes: self.max_db_bytes,
            });
        }

        let now = self.clock.now();
        let db_path = self.db_path.clone();
        let max_db_bytes = self.max_db_bytes;
        let tx = self
            .conn
            .transaction()
            .map_err(|error| map_op_error(&db_path, &error))?;
        let value = operation(&tx, now)?;
        let size_after =
            Self::database_size(&tx).map_err(|error| map_op_error(&db_path, &error))?;
        if size_after > max_db_bytes {
            // Dropping the transaction rolls it back.
            drop(tx);
            return Err(StateError::SizeLimitExceeded {
                size_bytes: size_after,
                limit_bytes: max_db_bytes,
            });
        }
        tx.commit()
            .map_err(|error| map_op_error(&db_path, &error))?;
        Ok(value)
    }

    /// Registers a complete root set in one SQLite transaction while an
    /// external atomic writer persists the matching configuration before the
    /// database commit. A failed writer rolls SQLite back; a failed database
    /// commit invokes the supplied compensating config rollback.
    pub fn register_approved_roots_coordinated<E>(
        &mut self,
        identities: &[PathIdentity],
        persist_config: impl FnOnce(&[ApprovedRootRecord]) -> Result<(), E>,
        rollback_config: impl FnOnce() -> Result<(), E>,
    ) -> Result<Vec<ApprovedRootRecord>, CoordinatedWriteError<E>> {
        let size_before = Self::database_size(&self.conn)
            .map_err(|error| CoordinatedWriteError::State(self.op_error(&error)))?;
        if size_before > self.max_db_bytes {
            return Err(CoordinatedWriteError::State(
                StateError::SizeLimitExceeded {
                    size_bytes: size_before,
                    limit_bytes: self.max_db_bytes,
                },
            ));
        }

        let now = self.clock.now();
        let db_path = self.db_path.clone();
        let max_db_bytes = self.max_db_bytes;
        let tx = self
            .conn
            .transaction()
            .map_err(|error| CoordinatedWriteError::State(map_op_error(&db_path, &error)))?;
        let mut records = Vec::with_capacity(identities.len());
        for identity in identities {
            records.push(
                add_root_tx(&tx, &db_path, identity, now).map_err(CoordinatedWriteError::State)?,
            );
        }
        records.sort_by(|a, b| a.canonical_path.cmp(&b.canonical_path));
        records.dedup_by(|a, b| a.id == b.id);

        let size_after = Self::database_size(&tx)
            .map_err(|error| CoordinatedWriteError::State(map_op_error(&db_path, &error)))?;
        if size_after > max_db_bytes {
            return Err(CoordinatedWriteError::State(
                StateError::SizeLimitExceeded {
                    size_bytes: size_after,
                    limit_bytes: max_db_bytes,
                },
            ));
        }
        persist_config(&records).map_err(CoordinatedWriteError::External)?;
        if let Err(error) = tx.commit() {
            let state = map_op_error(&db_path, &error);
            return match rollback_config() {
                Ok(()) => Err(CoordinatedWriteError::StateCommit(state)),
                Err(rollback) => {
                    Err(CoordinatedWriteError::StateCommitAndRollback { state, rollback })
                }
            };
        }
        Ok(records)
    }

    /// Applies one complete scan atomically. Only roots whose live identity
    /// was revalidated participate; missing rows are retained, never deleted.
    pub fn apply_scan(
        &mut self,
        validated_roots: &[ValidatedRoot],
        observations: &[WorkspaceObservation],
        uncertain_prefixes: &[PathBuf],
    ) -> Result<ScanStateResult, StateError> {
        let db_path = self.db_path.clone();
        self.mutate(|tx, now| {
            for root in validated_roots {
                let raw = tx
                    .query_row(
                        &format!("SELECT {ROOT_COLUMNS} FROM approved_roots WHERE id = ?1"),
                        params![root.id.as_str()],
                        RawRoot::from_row,
                    )
                    .optional()
                    .map_err(|error| map_op_error(&db_path, &error))?
                    .ok_or_else(|| StateError::UnknownApprovedRoot {
                        id: root.id.clone(),
                    })?;
                if raw.active == 0
                    || raw.canonical_path != root.identity.canonical.as_str()
                    || raw.volume_id != root.identity.volume.as_str()
                    || raw.file_id != root.identity.file.as_str()
                {
                    return Err(StateError::ConflictingIdentity {
                        message: format!(
                            "approved root {} changed identity during scan",
                            root.identity.canonical
                        ),
                    });
                }
                tx.execute(
                    "UPDATE approved_roots SET last_validated_at = ?1 WHERE id = ?2",
                    params![
                        max_timestamp(raw.last_validated_at, now.0),
                        root.id.as_str()
                    ],
                )
                .map_err(|error| map_op_error(&db_path, &error))?;
            }

            let mut changes = Vec::with_capacity(observations.len());
            let mut seen = std::collections::BTreeSet::new();
            for observation in observations {
                let (workspace, kind) = upsert_workspace_tx(tx, &db_path, observation, now)?;
                seen.insert(workspace.id.as_str().to_owned());
                changes.push(WorkspaceChange { kind, workspace });
            }

            let validated_ids: std::collections::BTreeSet<_> = validated_roots
                .iter()
                .map(|root| root.id.as_str().to_owned())
                .collect();
            let mut statement = tx
                .prepare(&format!(
                    "SELECT {WORKSPACE_COLUMNS} FROM workspaces
                     WHERE status IN ('present', 'missing')
                     ORDER BY volume_id, canonical_path, id"
                ))
                .map_err(|error| map_op_error(&db_path, &error))?;
            let existing = statement
                .query_map([], RawWorkspace::from_row)
                .map_err(|error| map_op_error(&db_path, &error))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| map_op_error(&db_path, &error))?;
            drop(statement);

            let mut missing = Vec::new();
            for raw in existing {
                let uncertain = uncertain_prefixes
                    .iter()
                    .any(|prefix| Path::new(&raw.canonical_path).starts_with(prefix));
                if validated_ids.contains(&raw.approved_root_id)
                    && !seen.contains(&raw.id)
                    && !uncertain
                {
                    tx.execute(
                        "UPDATE workspaces SET status = 'missing' WHERE id = ?1",
                        params![raw.id],
                    )
                    .map_err(|error| map_op_error(&db_path, &error))?;
                    let mut record = raw.into_record(&db_path)?;
                    record.status = WorkspaceStatus::Missing;
                    missing.push(record);
                }
            }
            changes.sort_by(|a, b| a.workspace.canonical_path.cmp(&b.workspace.canonical_path));
            missing.sort_by(|a, b| a.canonical_path.cmp(&b.canonical_path));
            Ok(ScanStateResult { changes, missing })
        })
    }
}

fn is_corruption(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(ffi_error, _)
            if matches!(
                ffi_error.code,
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
            )
    )
}

fn open_error(path: &Path, error: &rusqlite::Error) -> StateError {
    if is_corruption(error) {
        StateError::Corrupt {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    } else {
        StateError::Unavailable {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }
}

fn map_op_error(path: &Path, error: &rusqlite::Error) -> StateError {
    if is_corruption(error) {
        StateError::Corrupt {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    } else {
        StateError::TransactionFailed {
            message: error.to_string(),
        }
    }
}

/// Applies every migration after `from_version`, together with its version
/// bump, inside a single transaction. Any failure rolls the whole upgrade
/// back and leaves the database at its previous version.
fn apply_migrations(
    conn: &mut Connection,
    from_version: u32,
    migrations: &[&str],
) -> Result<(), StateError> {
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| StateError::TransactionFailed {
            message: error.to_string(),
        })?;
    for (index, migration) in migrations.iter().enumerate().skip(from_version as usize) {
        let target_version = index as u32 + 1;
        tx.execute_batch(migration)
            .map_err(|error| StateError::MigrationFailed {
                message: format!("migration to version {target_version} failed: {error}"),
            })?;
        tx.execute(
            "INSERT INTO schema_meta (id, version) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET version = excluded.version",
            params![target_version],
        )
        .map_err(|error| StateError::MigrationFailed {
            message: format!("recording schema version {target_version} failed: {error}"),
        })?;
    }
    tx.commit().map_err(|error| StateError::MigrationFailed {
        message: error.to_string(),
    })
}

fn generate_id(tx: &Transaction<'_>, path: &Path) -> Result<String, StateError> {
    tx.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
        .map_err(|error| map_op_error(path, &error))
}

fn max_timestamp(a: i64, b: i64) -> i64 {
    a.max(b)
}

struct RawRoot {
    id: String,
    canonical_path: String,
    volume_id: String,
    file_id: String,
    created_at: i64,
    last_validated_at: i64,
    active: i64,
}

impl RawRoot {
    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            canonical_path: row.get(1)?,
            volume_id: row.get(2)?,
            file_id: row.get(3)?,
            created_at: row.get(4)?,
            last_validated_at: row.get(5)?,
            active: row.get(6)?,
        })
    }

    fn into_record(self) -> Result<ApprovedRootRecord, StateError> {
        Ok(ApprovedRootRecord {
            id: ApprovedRootId(self.id),
            canonical_path: CanonicalPath::from_verified(PathBuf::from(self.canonical_path))?,
            volume_id: VolumeId::new(self.volume_id)?,
            file_id: FileId::new(self.file_id)?,
            created_at: Timestamp(self.created_at),
            last_validated_at: Timestamp(self.last_validated_at),
            active: self.active != 0,
        })
    }
}

const ROOT_COLUMNS: &str =
    "id, canonical_path, volume_id, file_id, created_at, last_validated_at, active";

struct RawWorkspace {
    id: String,
    canonical_path: String,
    approved_root_id: String,
    volume_id: String,
    file_id: String,
    first_observed_at: i64,
    last_observed_at: i64,
    last_activity_at: i64,
    last_cleaned_at: Option<i64>,
    protected: i64,
    status: String,
    git_state: String,
    git_head_fingerprint: Option<String>,
    git_index_fingerprint: Option<String>,
}

impl RawWorkspace {
    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            canonical_path: row.get(1)?,
            approved_root_id: row.get(2)?,
            volume_id: row.get(3)?,
            file_id: row.get(4)?,
            first_observed_at: row.get(5)?,
            last_observed_at: row.get(6)?,
            last_activity_at: row.get(7)?,
            last_cleaned_at: row.get(8)?,
            protected: row.get(9)?,
            status: row.get(10)?,
            git_state: row.get(11)?,
            git_head_fingerprint: row.get(12)?,
            git_index_fingerprint: row.get(13)?,
        })
    }

    fn into_record(self, db_path: &Path) -> Result<WorkspaceRecord, StateError> {
        let status = WorkspaceStatus::parse(&self.status).ok_or_else(|| StateError::Corrupt {
            path: db_path.to_path_buf(),
            message: format!("workspace row holds unknown status {:?}", self.status),
        })?;
        let git_state = GitState::parse(&self.git_state).ok_or_else(|| StateError::Corrupt {
            path: db_path.to_path_buf(),
            message: format!("workspace row holds unknown Git state {:?}", self.git_state),
        })?;
        Ok(WorkspaceRecord {
            id: WorkspaceId(self.id),
            canonical_path: CanonicalPath::from_verified(PathBuf::from(self.canonical_path))?,
            approved_root_id: ApprovedRootId(self.approved_root_id),
            volume_id: VolumeId::new(self.volume_id)?,
            file_id: FileId::new(self.file_id)?,
            first_observed_at: Timestamp(self.first_observed_at),
            last_observed_at: Timestamp(self.last_observed_at),
            last_activity_at: Timestamp(self.last_activity_at),
            last_cleaned_at: self.last_cleaned_at.map(Timestamp),
            protected: self.protected != 0,
            status,
            git_state,
            git_head_fingerprint: self.git_head_fingerprint,
            git_index_fingerprint: self.git_index_fingerprint,
        })
    }
}

const WORKSPACE_COLUMNS: &str = "id, canonical_path, approved_root_id, volume_id, file_id, \
     first_observed_at, last_observed_at, last_activity_at, last_cleaned_at, protected, status, \
     git_state, git_head_fingerprint, git_index_fingerprint";

impl StateStore for SqliteStateStore {
    fn schema_version(&self) -> Result<u32, StateError> {
        let version: i64 = self
            .conn
            .query_row("SELECT version FROM schema_meta WHERE id = 1", [], |row| {
                row.get(0)
            })
            .map_err(|error| self.op_error(&error))?;
        u32::try_from(version).map_err(|_| StateError::Corrupt {
            path: self.db_path.clone(),
            message: format!("schema_meta holds invalid version {version}"),
        })
    }

    fn add_approved_root(
        &mut self,
        root: NewApprovedRoot,
    ) -> Result<ApprovedRootRecord, StateError> {
        let db_path = self.db_path.clone();
        self.mutate(|tx, now| add_root_tx(tx, &db_path, &root.identity, now))
    }

    fn list_approved_roots(&self) -> Result<Vec<ApprovedRootRecord>, StateError> {
        let mut statement = self
            .conn
            .prepare(&format!(
                "SELECT {ROOT_COLUMNS} FROM approved_roots
                 ORDER BY volume_id, canonical_path"
            ))
            .map_err(|error| self.op_error(&error))?;
        let raw_roots = statement
            .query_map([], RawRoot::from_row)
            .map_err(|error| self.op_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| self.op_error(&error))?;
        raw_roots.into_iter().map(RawRoot::into_record).collect()
    }

    fn upsert_workspace_observation(
        &mut self,
        observation: WorkspaceObservation,
    ) -> Result<WorkspaceRecord, StateError> {
        let db_path = self.db_path.clone();
        self.mutate(|tx, now| upsert_workspace_tx(tx, &db_path, &observation, now).map(|x| x.0))
    }

    fn get_workspace(&self, id: &WorkspaceId) -> Result<Option<WorkspaceRecord>, StateError> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?1"),
                params![id.as_str()],
                RawWorkspace::from_row,
            )
            .optional()
            .map_err(|error| self.op_error(&error))?;
        raw.map(|raw| raw.into_record(&self.db_path)).transpose()
    }

    fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>, StateError> {
        let mut statement = self
            .conn
            .prepare(&format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces
                 ORDER BY volume_id, canonical_path, id"
            ))
            .map_err(|error| self.op_error(&error))?;
        let raw_workspaces = statement
            .query_map([], RawWorkspace::from_row)
            .map_err(|error| self.op_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| self.op_error(&error))?;
        raw_workspaces
            .into_iter()
            .map(|raw| raw.into_record(&self.db_path))
            .collect()
    }

    fn set_protected(&mut self, id: &WorkspaceId, protected: bool) -> Result<(), StateError> {
        let db_path = self.db_path.clone();
        let id = id.clone();
        self.mutate(|tx, _now| {
            let updated = tx
                .execute(
                    "UPDATE workspaces SET protected = ?1 WHERE id = ?2",
                    params![i64::from(protected), id.as_str()],
                )
                .map_err(|error| map_op_error(&db_path, &error))?;
            if updated == 0 {
                return Err(StateError::UnknownWorkspace { id: id.clone() });
            }
            Ok(())
        })
    }

    fn set_pnpm_enrolment(&mut self, enrolment: &PnpmEnrolmentRecord) -> Result<(), StateError> {
        let db_path = self.db_path.clone();
        let enrolment = enrolment.clone();
        self.mutate(|tx, _now| {
            tx.execute(
                "INSERT INTO pnpm_enrolment
                    (id, canonical_path, volume_id, file_id, version, enrolled_at)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (id) DO UPDATE SET
                    canonical_path = excluded.canonical_path,
                    volume_id = excluded.volume_id,
                    file_id = excluded.file_id,
                    version = excluded.version,
                    enrolled_at = excluded.enrolled_at",
                params![
                    enrolment.identity.canonical.as_str(),
                    enrolment.identity.volume.as_str(),
                    enrolment.identity.file.as_str(),
                    enrolment.version,
                    enrolment.enrolled_at.0,
                ],
            )
            .map_err(|error| map_op_error(&db_path, &error))?;
            Ok(())
        })
    }

    fn get_pnpm_enrolment(&self) -> Result<Option<PnpmEnrolmentRecord>, StateError> {
        let raw: Option<(String, String, String, String, i64)> = self
            .conn
            .query_row(
                "SELECT canonical_path, volume_id, file_id, version, enrolled_at
                 FROM pnpm_enrolment WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| self.op_error(&error))?;
        let Some((canonical_path, volume_id, file_id, version, enrolled_at)) = raw else {
            return Ok(None);
        };
        // A row that cannot be turned back into a real identity is corruption,
        // not an absent enrolment; reporting `None` would silently drop the
        // executable identity that authority depends on.
        let corrupt = |error: IdentityError| StateError::Corrupt {
            path: self.db_path.clone(),
            message: format!("pnpm_enrolment holds an unusable identity: {error}"),
        };
        Ok(Some(PnpmEnrolmentRecord {
            identity: PathIdentity::from_parts(
                CanonicalPath::from_verified(PathBuf::from(canonical_path)).map_err(corrupt)?,
                VolumeId::new(volume_id).map_err(corrupt)?,
                FileId::new(file_id).map_err(corrupt)?,
            ),
            version,
            enrolled_at: Timestamp(enrolled_at),
        }))
    }

    fn begin_run(&mut self, run: &NewRun) -> Result<(), StateError> {
        let db_path = self.db_path.clone();
        let run = run.clone();
        self.mutate(|tx, _now| {
            tx.execute(
                "INSERT INTO runs
                    (id, plan_id, policy_hash, volume_id, result,
                     free_before_bytes, free_after_bytes,
                     minimum_free_bytes, target_free_bytes,
                     started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, ?6, ?7, ?8, NULL)",
                params![
                    run.run_id.as_str(),
                    run.plan_id,
                    run.policy_hash,
                    run.volume_id,
                    bytes_to_i64(run.free_before_bytes),
                    bytes_to_i64(run.minimum_free_bytes),
                    bytes_to_i64(run.target_free_bytes),
                    run.started_at.0,
                ],
            )
            .map_err(|error| map_op_error(&db_path, &error))?;
            prune_runs_tx(tx, &db_path)
        })
    }

    fn record_planned_action(&mut self, action: &NewAction) -> Result<ActionId, StateError> {
        let db_path = self.db_path.clone();
        let action = action.clone();
        self.mutate(|tx, _now| {
            tx.execute(
                "INSERT INTO run_actions
                    (run_id, sequence, action, workspace_id, canonical_path, state,
                     free_before_bytes, free_after_bytes, actual_bytes,
                     detail, recovery, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'PLANNED', ?6, NULL, NULL,
                         NULL, NULL, ?7, NULL)",
                params![
                    action.run_id.as_str(),
                    action.sequence,
                    action.action,
                    action.workspace_id,
                    action
                        .canonical_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned()),
                    bytes_to_i64(action.free_before_bytes),
                    action.started_at.0,
                ],
            )
            .map_err(|error| map_op_error(&db_path, &error))?;
            Ok(ActionId::new(tx.last_insert_rowid()))
        })
    }

    fn advance_action(&mut self, id: ActionId, state: ActionState) -> Result<(), StateError> {
        let db_path = self.db_path.clone();
        self.mutate(|tx, _now| {
            let updated = tx
                .execute(
                    "UPDATE run_actions SET state = ?1 WHERE id = ?2",
                    params![state.as_str(), id.as_i64()],
                )
                .map_err(|error| map_op_error(&db_path, &error))?;
            if updated == 0 {
                return Err(StateError::Corrupt {
                    path: db_path.clone(),
                    message: format!("journalled action {} disappeared", id.as_i64()),
                });
            }
            Ok(())
        })
    }

    fn finish_action(&mut self, id: ActionId, outcome: &ActionOutcome) -> Result<(), StateError> {
        let db_path = self.db_path.clone();
        let outcome = outcome.clone();
        self.mutate(|tx, _now| {
            let updated = tx
                .execute(
                    "UPDATE run_actions
                     SET state = ?1, free_after_bytes = ?2, actual_bytes = ?3,
                         detail = ?4, recovery = ?5, finished_at = ?6
                     WHERE id = ?7",
                    params![
                        outcome.state.as_str(),
                        outcome.free_after_bytes.map(bytes_to_i64),
                        outcome.actual_bytes.map(bytes_to_i64),
                        outcome.detail,
                        outcome.recovery,
                        outcome.finished_at.0,
                        id.as_i64(),
                    ],
                )
                .map_err(|error| map_op_error(&db_path, &error))?;
            if updated == 0 {
                return Err(StateError::Corrupt {
                    path: db_path.clone(),
                    message: format!("journalled action {} disappeared", id.as_i64()),
                });
            }
            Ok(())
        })
    }

    fn finish_run(
        &mut self,
        run_id: &RunId,
        result: RunResult,
        free_after_bytes: Option<u64>,
        finished_at: Timestamp,
    ) -> Result<(), StateError> {
        let db_path = self.db_path.clone();
        let run_id = run_id.clone();
        self.mutate(|tx, _now| {
            tx.execute(
                "UPDATE runs SET result = ?1, free_after_bytes = ?2, finished_at = ?3
                 WHERE id = ?4",
                params![
                    result.as_str(),
                    free_after_bytes.map(bytes_to_i64),
                    finished_at.0,
                    run_id.as_str(),
                ],
            )
            .map_err(|error| map_op_error(&db_path, &error))?;
            Ok(())
        })
    }

    fn list_runs(&self, limit: usize) -> Result<Vec<RunRecord>, StateError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id, plan_id, policy_hash, volume_id, result,
                        free_before_bytes, free_after_bytes,
                        minimum_free_bytes, target_free_bytes,
                        started_at, finished_at
                 FROM runs ORDER BY started_at DESC, id DESC LIMIT ?1",
            )
            .map_err(|error| self.op_error(&error))?;
        let raw_runs = statement
            .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                ))
            })
            .map_err(|error| self.op_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| self.op_error(&error))?;

        let mut runs = Vec::with_capacity(raw_runs.len());
        for raw in raw_runs {
            let (
                run_id,
                plan_id,
                policy_hash,
                volume_id,
                result,
                free_before,
                free_after,
                minimum_free,
                target_free,
                started_at,
                finished_at,
            ) = raw;
            // An unreadable result is corruption. Reporting it as an
            // interrupted run would quietly turn a damaged ledger into a
            // plausible history.
            let result = match result {
                Some(text) => Some(RunResult::parse(&text).ok_or_else(|| StateError::Corrupt {
                    path: self.db_path.clone(),
                    message: format!("run {run_id} holds unknown result {text}"),
                })?),
                None => None,
            };
            runs.push(RunRecord {
                actions: self.actions_for_run(&run_id)?,
                run_id,
                plan_id,
                policy_hash,
                volume_id,
                result,
                free_before_bytes: i64_to_bytes(free_before),
                free_after_bytes: free_after.map(i64_to_bytes),
                minimum_free_bytes: i64_to_bytes(minimum_free),
                target_free_bytes: i64_to_bytes(target_free),
                started_at_millis: started_at,
                finished_at_millis: finished_at,
            });
        }
        Ok(runs)
    }

    fn record_workspace_cleaned(
        &mut self,
        id: &WorkspaceId,
        cleaned_at: Timestamp,
    ) -> Result<(), StateError> {
        let db_path = self.db_path.clone();
        let id = id.clone();
        self.mutate(|tx, _now| {
            let updated = tx
                .execute(
                    "UPDATE workspaces SET last_cleaned_at = ?1 WHERE id = ?2",
                    params![cleaned_at.0, id.as_str()],
                )
                .map_err(|error| map_op_error(&db_path, &error))?;
            if updated == 0 {
                return Err(StateError::UnknownWorkspace { id: id.clone() });
            }
            Ok(())
        })
    }
}

impl SqliteStateStore {
    fn actions_for_run(&self, run_id: &str) -> Result<Vec<ActionRecord>, StateError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT sequence, action, workspace_id, canonical_path, state,
                        free_before_bytes, free_after_bytes, actual_bytes,
                        detail, recovery, started_at, finished_at
                 FROM run_actions WHERE run_id = ?1 ORDER BY sequence",
            )
            .map_err(|error| self.op_error(&error))?;
        let raw_actions = statement
            .query_map(params![run_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                ))
            })
            .map_err(|error| self.op_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| self.op_error(&error))?;

        raw_actions
            .into_iter()
            .map(|raw| {
                let state = ActionState::parse(&raw.4).ok_or_else(|| StateError::Corrupt {
                    path: self.db_path.clone(),
                    message: format!("run {run_id} holds unknown action state {}", raw.4),
                })?;
                Ok(ActionRecord {
                    sequence: raw.0,
                    action: raw.1,
                    workspace_id: raw.2,
                    path: raw.3.map(PathBuf::from),
                    state,
                    free_before_bytes: i64_to_bytes(raw.5),
                    free_after_bytes: raw.6.map(i64_to_bytes),
                    actual_bytes: raw.7.map(i64_to_bytes),
                    detail: raw.8,
                    recovery: raw.9,
                    started_at_millis: raw.10,
                    finished_at_millis: raw.11,
                })
            })
            .collect()
    }
}

/// Keeps detailed history within its documented bound (`SAFETY.md` section 12).
/// Actions vanish with their run through the cascade.
fn prune_runs_tx(tx: &Transaction<'_>, db_path: &Path) -> Result<(), StateError> {
    tx.execute(
        "DELETE FROM runs WHERE id NOT IN (
             SELECT id FROM runs ORDER BY started_at DESC, id DESC LIMIT ?1
         )",
        params![i64::try_from(MAX_DETAILED_RUNS).unwrap_or(i64::MAX)],
    )
    .map_err(|error| map_op_error(db_path, &error))?;
    Ok(())
}

/// SQLite integers are signed. Byte counts are stored saturated rather than
/// wrapped, so a preposterous value can never read back as negative free space.
fn bytes_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn i64_to_bytes(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn add_root_tx(
    tx: &Transaction<'_>,
    db_path: &Path,
    identity: &PathIdentity,
    now: Timestamp,
) -> Result<ApprovedRootRecord, StateError> {
    let exact = tx
        .query_row(
            &format!(
                "SELECT {ROOT_COLUMNS} FROM approved_roots
                 WHERE volume_id = ?1 AND canonical_path = ?2 AND active = 1"
            ),
            params![identity.volume.as_str(), identity.canonical.as_str()],
            RawRoot::from_row,
        )
        .optional()
        .map_err(|error| map_op_error(db_path, &error))?;
    if let Some(raw) = exact {
        if raw.file_id != identity.file.as_str() {
            return Err(StateError::ConflictingIdentity {
                message: format!(
                    "approved root {} exists but points at a different physical directory; re-approval is required",
                    identity.canonical
                ),
            });
        }
        tx.execute(
            "UPDATE approved_roots SET last_validated_at = ?1 WHERE id = ?2",
            params![max_timestamp(raw.last_validated_at, now.0), raw.id],
        )
        .map_err(|error| map_op_error(db_path, &error))?;
        let mut record = raw.into_record()?;
        record.last_validated_at = Timestamp(max_timestamp(record.last_validated_at.0, now.0));
        return Ok(record);
    }

    let same_file = tx
        .query_row(
            &format!(
                "SELECT {ROOT_COLUMNS} FROM approved_roots
                 WHERE volume_id = ?1 AND file_id = ?2 AND active = 1"
            ),
            params![identity.volume.as_str(), identity.file.as_str()],
            RawRoot::from_row,
        )
        .optional()
        .map_err(|error| map_op_error(db_path, &error))?;
    if let Some(raw) = same_file {
        if raw
            .canonical_path
            .eq_ignore_ascii_case(identity.canonical.as_str())
        {
            tx.execute(
                "UPDATE approved_roots SET last_validated_at = ?1 WHERE id = ?2",
                params![max_timestamp(raw.last_validated_at, now.0), raw.id],
            )
            .map_err(|error| map_op_error(db_path, &error))?;
            let mut record = raw.into_record()?;
            record.last_validated_at = Timestamp(max_timestamp(record.last_validated_at.0, now.0));
            return Ok(record);
        }
        return Err(StateError::ConflictingIdentity {
            message: format!(
                "the physical directory of {} is already approved as {}; one directory must not hold two authorities",
                identity.canonical, raw.canonical_path
            ),
        });
    }

    let id = generate_id(tx, db_path)?;
    tx.execute(
        "INSERT INTO approved_roots
         (id, canonical_path, volume_id, file_id, created_at, last_validated_at, active)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, 1)",
        params![
            id,
            identity.canonical.as_str(),
            identity.volume.as_str(),
            identity.file.as_str(),
            now.0,
        ],
    )
    .map_err(|error| map_op_error(db_path, &error))?;
    Ok(ApprovedRootRecord {
        id: ApprovedRootId(id),
        canonical_path: identity.canonical.clone(),
        volume_id: identity.volume.clone(),
        file_id: identity.file.clone(),
        created_at: now,
        last_validated_at: now,
        active: true,
    })
}

fn upsert_workspace_tx(
    tx: &Transaction<'_>,
    db_path: &Path,
    observation: &WorkspaceObservation,
    now: Timestamp,
) -> Result<(WorkspaceRecord, WorkspaceChangeKind), StateError> {
    let identity = &observation.identity;
    let root_exists: bool = tx
        .query_row(
            "SELECT count(*) FROM approved_roots WHERE id = ?1 AND active = 1",
            params![observation.approved_root_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|error| map_op_error(db_path, &error))?;
    if !root_exists {
        return Err(StateError::UnknownApprovedRoot {
            id: observation.approved_root_id.clone(),
        });
    }

    let exact = tx
        .query_row(
            &format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces
                 WHERE volume_id = ?1 AND canonical_path = ?2
                   AND status IN ('present', 'missing')"
            ),
            params![identity.volume.as_str(), identity.canonical.as_str()],
            RawWorkspace::from_row,
        )
        .optional()
        .map_err(|error| map_op_error(db_path, &error))?;
    if let Some(raw) = exact {
        if raw.file_id == identity.file.as_str() {
            return update_observed(tx, db_path, raw, observation, now)
                .map(|record| (record, WorkspaceChangeKind::Updated));
        }
        tx.execute(
            "UPDATE workspaces SET status = 'replaced' WHERE id = ?1",
            params![raw.id],
        )
        .map_err(|error| map_op_error(db_path, &error))?;
        return insert_workspace(tx, db_path, observation, now)
            .map(|record| (record, WorkspaceChangeKind::Registered));
    }

    let case_variant = tx
        .query_row(
            &format!(
                "SELECT {WORKSPACE_COLUMNS} FROM workspaces
                 WHERE volume_id = ?1 AND file_id = ?2
                   AND status IN ('present', 'missing')"
            ),
            params![identity.volume.as_str(), identity.file.as_str()],
            RawWorkspace::from_row,
        )
        .optional()
        .map_err(|error| map_op_error(db_path, &error))?;
    if let Some(raw) = case_variant
        && raw
            .canonical_path
            .eq_ignore_ascii_case(identity.canonical.as_str())
    {
        return update_observed(tx, db_path, raw, observation, now)
            .map(|record| (record, WorkspaceChangeKind::Updated));
    }

    insert_workspace(tx, db_path, observation, now)
        .map(|record| (record, WorkspaceChangeKind::Registered))
}

/// Updates an existing record for a repeated observation. `first_observed_at`
/// is untouched, `last_observed_at` and `last_activity_at` never move
/// backwards, and `protected`, `last_cleaned_at`, and `status` are preserved.
fn update_observed(
    tx: &Transaction<'_>,
    db_path: &Path,
    raw: RawWorkspace,
    observation: &WorkspaceObservation,
    now: Timestamp,
) -> Result<WorkspaceRecord, StateError> {
    let last_observed_at = max_timestamp(raw.last_observed_at, now.0);
    let fingerprint_moved = fingerprint_changed(
        raw.git_head_fingerprint.as_deref(),
        observation.git_head_fingerprint.as_deref(),
    ) || fingerprint_changed(
        raw.git_index_fingerprint.as_deref(),
        observation.git_index_fingerprint.as_deref(),
    );
    let evidence_activity = observation
        .activity_at
        .map(|activity| max_timestamp(raw.last_activity_at, activity.0))
        .unwrap_or(raw.last_activity_at);
    let last_activity_at = if fingerprint_moved {
        max_timestamp(evidence_activity, now.0)
    } else {
        evidence_activity
    };
    tx.execute(
        "UPDATE workspaces
         SET last_observed_at = ?1,
             last_activity_at = ?2,
             approved_root_id = ?3,
             status = 'present',
             git_state = ?4,
             git_head_fingerprint = ?5,
             git_index_fingerprint = ?6
         WHERE id = ?7",
        params![
            last_observed_at,
            last_activity_at,
            observation.approved_root_id.as_str(),
            observation.git_state.as_str(),
            observation.git_head_fingerprint,
            observation.git_index_fingerprint,
            raw.id,
        ],
    )
    .map_err(|error| map_op_error(db_path, &error))?;
    let mut record = raw.into_record(db_path)?;
    record.last_observed_at = Timestamp(last_observed_at);
    record.last_activity_at = Timestamp(last_activity_at);
    record.approved_root_id = observation.approved_root_id.clone();
    record.status = WorkspaceStatus::Present;
    record.git_state = observation.git_state;
    record.git_head_fingerprint = observation.git_head_fingerprint.clone();
    record.git_index_fingerprint = observation.git_index_fingerprint.clone();
    Ok(record)
}

fn fingerprint_changed(previous: Option<&str>, current: Option<&str>) -> bool {
    matches!((previous, current), (Some(previous), Some(current)) if previous != current)
}

fn insert_workspace(
    tx: &Transaction<'_>,
    db_path: &Path,
    observation: &WorkspaceObservation,
    now: Timestamp,
) -> Result<WorkspaceRecord, StateError> {
    let identity = &observation.identity;
    let id = generate_id(tx, db_path)?;
    let last_activity_at = observation.activity_at.map(|t| t.0).unwrap_or(now.0);
    tx.execute(
        "INSERT INTO workspaces
         (id, canonical_path, approved_root_id, volume_id, file_id,
          first_observed_at, last_observed_at, last_activity_at,
          last_cleaned_at, protected, status, git_state,
          git_head_fingerprint, git_index_fingerprint)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, NULL, 0, 'present', ?8, ?9, ?10)",
        params![
            id,
            identity.canonical.as_str(),
            observation.approved_root_id.as_str(),
            identity.volume.as_str(),
            identity.file.as_str(),
            now.0,
            last_activity_at,
            observation.git_state.as_str(),
            observation.git_head_fingerprint,
            observation.git_index_fingerprint,
        ],
    )
    .map_err(|error| map_op_error(db_path, &error))?;
    Ok(WorkspaceRecord {
        id: WorkspaceId(id),
        canonical_path: identity.canonical.clone(),
        approved_root_id: observation.approved_root_id.clone(),
        volume_id: identity.volume.clone(),
        file_id: identity.file.clone(),
        first_observed_at: now,
        last_observed_at: now,
        last_activity_at: Timestamp(last_activity_at),
        last_cleaned_at: None,
        protected: false,
        status: WorkspaceStatus::Present,
        git_state: observation.git_state,
        git_head_fingerprint: observation.git_head_fingerprint.clone(),
        git_index_fingerprint: observation.git_index_fingerprint.clone(),
    })
}

/// Deterministic test clock; also used by Day 2B tests via
/// [`SqliteStateStore::open_with_clock`].
#[derive(Debug, Clone)]
pub struct FixedClock {
    millis: std::rc::Rc<Cell<i64>>,
}

impl FixedClock {
    pub fn new(millis: i64) -> Self {
        Self {
            millis: std::rc::Rc::new(Cell::new(millis)),
        }
    }

    pub fn set(&self, millis: i64) {
        self.millis.set(millis);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.millis.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("ledger.sqlite3")
    }

    fn open_at(path: &Path, clock: &FixedClock) -> SqliteStateStore {
        SqliteStateStore::open_with_clock(path, Box::new(clock.clone())).unwrap()
    }

    fn identity(path: &str, volume: &str, file: &str) -> PathIdentity {
        PathIdentity::from_parts(
            CanonicalPath::from_verified(PathBuf::from(path)).unwrap(),
            VolumeId::new(volume).unwrap(),
            FileId::new(file).unwrap(),
        )
    }

    #[cfg(unix)]
    const ROOT_A: &str = "/projects/root a";
    #[cfg(windows)]
    const ROOT_A: &str = "C:\\projects\\root a";
    #[cfg(unix)]
    const WS_A: &str = "/projects/root a/wörk space";
    #[cfg(windows)]
    const WS_A: &str = "C:\\projects\\root a\\wörk space";

    fn add_root(
        store: &mut SqliteStateStore,
        path: &str,
        volume: &str,
        file: &str,
    ) -> ApprovedRootRecord {
        store
            .add_approved_root(NewApprovedRoot {
                identity: identity(path, volume, file),
            })
            .unwrap()
    }

    fn observe(
        store: &mut SqliteStateStore,
        root: &ApprovedRootId,
        path: &str,
        volume: &str,
        file: &str,
        activity: Option<i64>,
    ) -> Result<WorkspaceRecord, StateError> {
        store.upsert_workspace_observation(WorkspaceObservation {
            identity: identity(path, volume, file),
            approved_root_id: root.clone(),
            activity_at: activity.map(Timestamp::from_unix_millis),
            git_state: GitState::Unknown,
            git_head_fingerprint: None,
            git_index_fingerprint: None,
        })
    }

    // ---- migration behaviour -------------------------------------------

    #[test]
    fn fresh_database_receives_current_schema() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let store = open_at(&temp_db(&dir), &clock);
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(store.list_approved_roots().unwrap().is_empty());
        assert!(store.list_workspaces().unwrap().is_empty());
    }

    #[test]
    fn reopening_is_idempotent_and_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        let clock = FixedClock::new(1_000);
        let root_id = {
            let mut store = open_at(&db, &clock);
            add_root(&mut store, ROOT_A, "vol-1", "file-1").id
        };
        for _ in 0..3 {
            let store = open_at(&db, &clock);
            assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
            let roots = store.list_approved_roots().unwrap();
            assert_eq!(roots.len(), 1);
            assert_eq!(roots[0].id, root_id);
        }
    }

    #[test]
    fn failed_migration_rolls_back_completely() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = Connection::open(temp_db(&dir)).unwrap();
        let bad: &[&str] = &["CREATE TABLE schema_meta (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                version INTEGER NOT NULL
            ) STRICT;
            CREATE TABLE will_roll_back (id TEXT PRIMARY KEY) STRICT;
            THIS IS NOT VALID SQL;"];
        let error = apply_migrations(&mut conn, 0, bad).unwrap_err();
        assert!(matches!(error, StateError::MigrationFailed { .. }));
        let object_count: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            object_count, 0,
            "failed migration must leave nothing behind"
        );
    }

    #[test]
    fn future_schema_version_is_rejected_without_modification() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        let clock = FixedClock::new(1_000);
        {
            let mut store = open_at(&db, &clock);
            add_root(&mut store, ROOT_A, "vol-1", "file-1");
        }
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute("UPDATE schema_meta SET version = 999", [])
                .unwrap();
        }
        let error = SqliteStateStore::open_with_clock(&db, Box::new(clock.clone())).unwrap_err();
        assert!(matches!(
            error,
            StateError::UnsupportedSchemaVersion {
                found: 999,
                supported: SCHEMA_VERSION
            }
        ));
        // The newer-versioned data is preserved, not downgraded or erased.
        let conn = Connection::open(&db).unwrap();
        let roots: i64 = conn
            .query_row("SELECT count(*) FROM approved_roots", [], |row| row.get(0))
            .unwrap();
        assert_eq!(roots, 1);
        let version: i64 = conn
            .query_row("SELECT version FROM schema_meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 999);
    }

    #[test]
    fn corrupt_database_is_rejected_and_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        let garbage = b"this is not a sqlite database at all............";
        std::fs::write(&db, garbage).unwrap();

        let error =
            SqliteStateStore::open_with_clock(&db, Box::new(FixedClock::new(0))).unwrap_err();
        assert!(matches!(error, StateError::Corrupt { .. }), "{error:?}");
        assert_eq!(
            std::fs::read(&db).unwrap(),
            garbage,
            "corrupt state must be left byte-for-byte untouched"
        );
    }

    #[test]
    fn foreign_sqlite_database_is_rejected_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch("CREATE TABLE somebody_elses (x INTEGER)")
                .unwrap();
        }
        let before = std::fs::read(&db).unwrap();
        let error =
            SqliteStateStore::open_with_clock(&db, Box::new(FixedClock::new(0))).unwrap_err();
        assert!(matches!(error, StateError::Corrupt { .. }));
        assert_eq!(std::fs::read(&db).unwrap(), before);
    }

    #[test]
    fn protection_survives_reopen_and_migration_check() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        let clock = FixedClock::new(1_000);
        let ws_id = {
            let mut store = open_at(&db, &clock);
            let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");
            let ws = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", None).unwrap();
            store.set_protected(&ws.id, true).unwrap();
            ws.id
        };
        let store = open_at(&db, &clock);
        let ws = store.get_workspace(&ws_id).unwrap().unwrap();
        assert!(ws.protected);
    }

    #[test]
    fn migration_from_v1_preserves_protection_and_reaches_the_current_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        {
            let mut conn = Connection::open(&db).unwrap();
            apply_migrations(&mut conn, 0, &MIGRATIONS[..1]).unwrap();
            conn.execute(
                "INSERT INTO approved_roots
                 (id, canonical_path, volume_id, file_id, created_at, last_validated_at, active)
                 VALUES ('root', ?1, 'vol-1', 'file-1', 1, 1, 1)",
                params![ROOT_A],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workspaces
                 (id, canonical_path, approved_root_id, volume_id, file_id,
                  first_observed_at, last_observed_at, last_activity_at,
                  last_cleaned_at, protected, status)
                 VALUES ('workspace', ?1, 'root', 'vol-1', 'file-2',
                         1, 2, 2, NULL, 1, 'present')",
                params![WS_A],
            )
            .unwrap();
        }
        let store = open_at(&db, &FixedClock::new(3));
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let workspaces = store.list_workspaces().unwrap();
        assert_eq!(workspaces.len(), 1);
        assert!(workspaces[0].protected);
        assert_eq!(workspaces[0].git_state, GitState::Unknown);
        assert_eq!(workspaces[0].status, WorkspaceStatus::Present);
        // v3 adds enrolment storage. Upgrading must not invent an enrolment:
        // an upgraded ledger has no pnpm until one is explicitly enrolled.
        assert_eq!(store.get_pnpm_enrolment().unwrap(), None);
    }

    #[test]
    fn pnpm_enrolment_round_trips_and_a_second_enrolment_replaces_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&db, &clock);
        assert_eq!(store.get_pnpm_enrolment().unwrap(), None);

        let first = PnpmEnrolmentRecord {
            identity: PathIdentity::from_parts(
                CanonicalPath::from_verified(PathBuf::from(WS_A)).unwrap(),
                VolumeId::new("vol-1").unwrap(),
                FileId::new("file-1").unwrap(),
            ),
            version: "11.2.3".to_owned(),
            enrolled_at: Timestamp(1_000),
        };
        store.set_pnpm_enrolment(&first).unwrap();
        assert_eq!(store.get_pnpm_enrolment().unwrap(), Some(first));

        let second = PnpmEnrolmentRecord {
            identity: PathIdentity::from_parts(
                CanonicalPath::from_verified(PathBuf::from(ROOT_A)).unwrap(),
                VolumeId::new("vol-2").unwrap(),
                FileId::new("file-2").unwrap(),
            ),
            version: "12.0.0".to_owned(),
            enrolled_at: Timestamp(2_000),
        };
        store.set_pnpm_enrolment(&second).unwrap();
        assert_eq!(store.get_pnpm_enrolment().unwrap(), Some(second));

        // Exactly one executable may hold cleanup authority.
        let count: i64 = store
            .conn
            .query_row("SELECT count(*) FROM pnpm_enrolment", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn a_corrupt_enrolment_row_is_an_error_rather_than_an_absent_enrolment() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        let clock = FixedClock::new(1_000);
        {
            let store = open_at(&db, &clock);
            store
                .conn
                .execute(
                    "INSERT INTO pnpm_enrolment
                     (id, canonical_path, volume_id, file_id, version, enrolled_at)
                     VALUES (1, 'not-absolute', 'vol-1', 'file-1', '11.2.3', 1)",
                    [],
                )
                .unwrap();
        }
        let store = open_at(&db, &clock);
        assert!(matches!(
            store.get_pnpm_enrolment(),
            Err(StateError::Corrupt { .. })
        ));
    }

    // ---- identity behaviour --------------------------------------------

    #[test]
    fn duplicate_approved_roots_collapse_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let first = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        clock.set(2_000);
        let second = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        assert_eq!(first.id, second.id);
        assert_eq!(second.created_at.as_unix_millis(), 1_000);
        assert_eq!(second.last_validated_at.as_unix_millis(), 2_000);
        assert_eq!(store.list_approved_roots().unwrap().len(), 1);
    }

    #[test]
    fn case_variant_alias_of_same_directory_collapses() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        #[cfg(unix)]
        let (spelling_a, spelling_b) = ("/Projects/App", "/projects/app");
        #[cfg(windows)]
        let (spelling_a, spelling_b) = ("C:\\Projects\\App", "C:\\projects\\app");
        let first = add_root(&mut store, spelling_a, "vol-1", "file-1");
        let second = add_root(&mut store, spelling_b, "vol-1", "file-1");
        assert_eq!(first.id, second.id, "one physical directory, one authority");
        assert_eq!(store.list_approved_roots().unwrap().len(), 1);
    }

    #[test]
    fn same_directory_under_unrelated_path_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        add_root(&mut store, ROOT_A, "vol-1", "file-1");
        #[cfg(unix)]
        let other = "/mnt/elsewhere";
        #[cfg(windows)]
        let other = "C:\\mnt\\elsewhere";
        let error = store
            .add_approved_root(NewApprovedRoot {
                identity: identity(other, "vol-1", "file-1"),
            })
            .unwrap_err();
        assert!(matches!(error, StateError::ConflictingIdentity { .. }));
    }

    #[test]
    fn replaced_directory_at_approved_root_path_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let error = store
            .add_approved_root(NewApprovedRoot {
                identity: identity(ROOT_A, "vol-1", "file-CHANGED"),
            })
            .unwrap_err();
        assert!(matches!(error, StateError::ConflictingIdentity { .. }));
    }

    #[test]
    fn same_path_on_different_volume_is_a_distinct_root() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let first = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let second = add_root(&mut store, ROOT_A, "vol-2", "file-1");
        assert_ne!(first.id, second.id);
        assert_eq!(store.list_approved_roots().unwrap().len(), 2);
    }

    #[test]
    fn duplicate_workspace_observations_update_one_record() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let first = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", None).unwrap();
        for step in 0..100 {
            clock.set(2_000 + step);
            let updated = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", None).unwrap();
            assert_eq!(updated.id, first.id);
        }
        assert_eq!(
            store.list_workspaces().unwrap().len(),
            1,
            "repeated observations must never append rows"
        );
    }

    #[test]
    fn first_observed_stays_and_observed_activity_only_move_forward() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(5_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let ws = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", Some(4_000)).unwrap();
        assert_eq!(ws.first_observed_at.as_unix_millis(), 5_000);
        assert_eq!(ws.last_observed_at.as_unix_millis(), 5_000);
        assert_eq!(ws.last_activity_at.as_unix_millis(), 4_000);

        // A clock that runs backwards and stale activity evidence must not
        // rewind anything.
        clock.set(3_000);
        let ws = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", Some(2_000)).unwrap();
        assert_eq!(ws.first_observed_at.as_unix_millis(), 5_000);
        assert_eq!(ws.last_observed_at.as_unix_millis(), 5_000);
        assert_eq!(ws.last_activity_at.as_unix_millis(), 4_000);

        clock.set(9_000);
        let ws = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", Some(8_000)).unwrap();
        assert_eq!(ws.first_observed_at.as_unix_millis(), 5_000);
        assert_eq!(ws.last_observed_at.as_unix_millis(), 9_000);
        assert_eq!(ws.last_activity_at.as_unix_millis(), 8_000);
        assert_eq!(ws.last_cleaned_at, None);
    }

    #[test]
    fn protection_survives_observation_updates() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let ws = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", None).unwrap();
        store.set_protected(&ws.id, true).unwrap();
        clock.set(2_000);
        let updated = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", Some(1_500)).unwrap();
        assert!(updated.protected, "observation must not clear protection");
        assert!(store.get_workspace(&ws.id).unwrap().unwrap().protected);
    }

    #[test]
    fn coordinated_config_failure_rolls_back_all_root_rows() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let identities = vec![
            identity(ROOT_A, "vol-1", "file-1"),
            identity(WS_A, "vol-1", "file-2"),
        ];
        let result = store.register_approved_roots_coordinated(
            &identities,
            |_| Err::<(), _>("injected config failure"),
            || Ok(()),
        );
        assert!(matches!(result, Err(CoordinatedWriteError::External(_))));
        assert!(store.list_approved_roots().unwrap().is_empty());
    }

    #[test]
    fn scan_marks_missing_without_erasing_protection_and_restores_on_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let observation = WorkspaceObservation {
            identity: identity(WS_A, "vol-1", "file-2"),
            approved_root_id: root.id.clone(),
            activity_at: Some(Timestamp::from_unix_millis(900)),
            git_state: GitState::Unknown,
            git_head_fingerprint: Some("head-a".to_owned()),
            git_index_fingerprint: Some("index-a".to_owned()),
        };
        let first = store
            .apply_scan(
                &[ValidatedRoot {
                    id: root.id.clone(),
                    identity: identity(ROOT_A, "vol-1", "file-1"),
                }],
                std::slice::from_ref(&observation),
                &[],
            )
            .unwrap();
        let workspace = &first.changes[0].workspace;
        store.set_protected(&workspace.id, true).unwrap();

        let uncertain = store
            .apply_scan(
                &[ValidatedRoot {
                    id: root.id.clone(),
                    identity: identity(ROOT_A, "vol-1", "file-1"),
                }],
                &[],
                &[PathBuf::from(WS_A)],
            )
            .unwrap();
        assert!(uncertain.missing.is_empty());
        assert_eq!(
            store.get_workspace(&workspace.id).unwrap().unwrap().status,
            WorkspaceStatus::Present
        );

        clock.set(2_000);
        let missing = store
            .apply_scan(
                &[ValidatedRoot {
                    id: root.id.clone(),
                    identity: identity(ROOT_A, "vol-1", "file-1"),
                }],
                &[],
                &[],
            )
            .unwrap();
        assert_eq!(missing.missing.len(), 1);
        assert_eq!(missing.missing[0].status, WorkspaceStatus::Missing);
        assert!(missing.missing[0].protected);

        clock.set(3_000);
        let restored = store
            .apply_scan(
                &[ValidatedRoot {
                    id: root.id,
                    identity: identity(ROOT_A, "vol-1", "file-1"),
                }],
                &[observation],
                &[],
            )
            .unwrap();
        assert_eq!(restored.changes[0].kind, WorkspaceChangeKind::Updated);
        assert_eq!(
            restored.changes[0].workspace.status,
            WorkspaceStatus::Present
        );
        assert!(restored.changes[0].workspace.protected);
    }

    #[test]
    fn git_fingerprint_movement_advances_activity_but_never_backwards() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(5_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let first = store
            .upsert_workspace_observation(WorkspaceObservation {
                identity: identity(WS_A, "vol-1", "file-2"),
                approved_root_id: root.id.clone(),
                activity_at: Some(Timestamp::from_unix_millis(4_000)),
                git_state: GitState::Unknown,
                git_head_fingerprint: Some("head-a".to_owned()),
                git_index_fingerprint: Some("index-a".to_owned()),
            })
            .unwrap();
        assert_eq!(first.last_activity_at.as_unix_millis(), 4_000);

        clock.set(8_000);
        let changed = store
            .upsert_workspace_observation(WorkspaceObservation {
                identity: identity(WS_A, "vol-1", "file-2"),
                approved_root_id: root.id,
                activity_at: Some(Timestamp::from_unix_millis(3_000)),
                git_state: GitState::Unknown,
                git_head_fingerprint: Some("head-b".to_owned()),
                git_index_fingerprint: Some("index-a".to_owned()),
            })
            .unwrap();
        assert_eq!(changed.first_observed_at, first.first_observed_at);
        assert_eq!(changed.last_activity_at.as_unix_millis(), 8_000);
        assert_eq!(changed.git_state, GitState::Unknown);
    }

    #[test]
    fn replaced_workspace_does_not_inherit_identity_or_history() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let original = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", None).unwrap();
        store.set_protected(&original.id, true).unwrap();

        // A different physical directory appears at the same canonical path.
        clock.set(9_000);
        let replacement = observe(&mut store, &root.id, WS_A, "vol-1", "file-NEW", None).unwrap();
        assert_ne!(replacement.id, original.id);
        assert_eq!(replacement.first_observed_at.as_unix_millis(), 9_000);
        assert!(
            !replacement.protected,
            "new directory earns no inherited state"
        );

        let old = store.get_workspace(&original.id).unwrap().unwrap();
        assert_eq!(old.status, WorkspaceStatus::Replaced);
        assert!(old.protected, "the old record's protection is never erased");
        assert_eq!(old.first_observed_at.as_unix_millis(), 1_000);
    }

    #[test]
    fn moved_directory_starts_fresh_at_its_new_path() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let original = observe(&mut store, &root.id, WS_A, "vol-1", "file-2", None).unwrap();

        #[cfg(unix)]
        let new_path = "/projects/root a/renamed";
        #[cfg(windows)]
        let new_path = "C:\\projects\\root a\\renamed";
        clock.set(9_000);
        let moved = observe(&mut store, &root.id, new_path, "vol-1", "file-2", None).unwrap();
        assert_ne!(moved.id, original.id, "a move must not transfer authority");
        assert_eq!(moved.first_observed_at.as_unix_millis(), 9_000);
    }

    #[test]
    fn case_variant_workspace_observation_updates_same_record() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        #[cfg(unix)]
        let (spelling_a, spelling_b) = ("/projects/root a/App", "/projects/root a/app");
        #[cfg(windows)]
        let (spelling_a, spelling_b) = ("C:\\projects\\root a\\App", "C:\\projects\\root a\\app");
        let first = observe(&mut store, &root.id, spelling_a, "vol-1", "file-2", None).unwrap();
        store.set_protected(&first.id, true).unwrap();
        clock.set(2_000);
        let second = observe(&mut store, &root.id, spelling_b, "vol-1", "file-2", None).unwrap();
        assert_eq!(first.id, second.id);
        assert!(second.protected);
        assert_eq!(store.list_workspaces().unwrap().len(), 1);
    }

    #[test]
    fn same_workspace_path_on_different_volume_is_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let root_one = add_root(&mut store, ROOT_A, "vol-1", "file-1");
        let root_two = add_root(&mut store, ROOT_A, "vol-2", "file-1");
        let ws_one = observe(&mut store, &root_one.id, WS_A, "vol-1", "file-2", None).unwrap();
        let ws_two = observe(&mut store, &root_two.id, WS_A, "vol-2", "file-2", None).unwrap();
        assert_ne!(
            ws_one.id, ws_two.id,
            "the same path string on another volume must not inherit authority"
        );
        assert_eq!(store.list_workspaces().unwrap().len(), 2);
    }

    #[test]
    fn observation_for_unknown_root_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let error = observe(
            &mut store,
            &ApprovedRootId("missing".to_owned()),
            WS_A,
            "vol-1",
            "file-2",
            None,
        )
        .unwrap_err();
        assert!(matches!(error, StateError::UnknownApprovedRoot { .. }));
    }

    #[test]
    fn set_protected_on_unknown_workspace_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let clock = FixedClock::new(1_000);
        let mut store = open_at(&temp_db(&dir), &clock);
        let error = store
            .set_protected(&WorkspaceId("missing".to_owned()), true)
            .unwrap_err();
        assert!(matches!(error, StateError::UnknownWorkspace { .. }));
    }

    // ---- state bounds ---------------------------------------------------

    #[test]
    fn oversized_change_is_rejected_and_prior_state_survives() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        let clock = FixedClock::new(1_000);
        let mut store = SqliteStateStore::open_inner(
            &db,
            Box::new(clock.clone()),
            96 * 1024, // tight test-only bound, far below the 20 MiB product bound
        )
        .unwrap();
        let root = add_root(&mut store, ROOT_A, "vol-1", "file-1");

        // A pathological identity large enough to burst the tight bound.
        #[cfg(unix)]
        let huge_path = format!("/projects/{}", "x".repeat(512 * 1024));
        #[cfg(windows)]
        let huge_path = format!("C:\\projects\\{}", "x".repeat(512 * 1024));
        let error = observe(&mut store, &root.id, &huge_path, "vol-1", "file-2", None).unwrap_err();
        assert!(
            matches!(error, StateError::SizeLimitExceeded { .. }),
            "{error:?}"
        );

        // The rejected transaction must leave the prior state fully usable.
        assert_eq!(store.list_approved_roots().unwrap().len(), 1);
        assert!(store.list_workspaces().unwrap().is_empty());
        let reopened = open_at(&db, &clock);
        assert_eq!(reopened.list_approved_roots().unwrap().len(), 1);
    }

    #[test]
    fn writes_are_refused_once_the_database_is_over_budget() {
        let dir = tempfile::tempdir().unwrap();
        let db = temp_db(&dir);
        let clock = FixedClock::new(1_000);
        {
            let mut store = open_at(&db, &clock);
            add_root(&mut store, ROOT_A, "vol-1", "file-1");
        }
        // Reopen with a bound smaller than the file already is: reads still
        // work, further growth is refused.
        let mut store = SqliteStateStore::open_inner(&db, Box::new(clock.clone()), 1).unwrap();
        assert_eq!(store.list_approved_roots().unwrap().len(), 1);
        let error = store
            .add_approved_root(NewApprovedRoot {
                identity: identity(WS_A, "vol-1", "file-9"),
            })
            .unwrap_err();
        assert!(matches!(error, StateError::SizeLimitExceeded { .. }));
    }
}
