# Implementation Status

## Repository

```text
Repository:     iqmanx/terminal_janitor
Default branch: main
Product:        terminal_janitor
Target version: 0.1.0
Current phase:  Days 1, 2A, 2B, and 3 complete; the Day 2 and Day 3 gates
                both passed on Ubuntu, macOS, and Windows; Day 4 not started
```

## Governing state

The owner has approved the seven-day Engine + CLI plan.

Locked decisions:

- Rust native binary;
- one crate initially;
- Linux, macOS, and Windows targets;
- user-configured `minimum_free` and `target_free`;
- Engine + CLI only for 0.1.0;
- no permanent daemon;
- native user-level scheduling;
- pnpm-first automatic authority;
- fewer cleanup actions with stronger proof;
- unknown or uncertain state always skips;
- safe shortfall is an accepted outcome;
- no generic path deletion;
- no MCP, Claude Code, Codex, GUI, cloud service, or broad ecosystem support in 0.1.0.

## Documents present

- [x] `README.md`
- [x] `VISION.md`
- [x] `SAFETY.md`
- [x] `PLANS.md`
- [x] `ACCEPTANCE.md`
- [x] `AGENTS.md`
- [x] `RESEARCH.md`
- [x] `IMPLEMENTATION_STATUS.md`
- [x] `BOOTSTRAP_PROMPT.md`

## Implementation progress

### Day 0 — Product and execution contract

Status: **complete**

Delivered:

- fixed product identity;
- fixed scope and exclusions;
- safety precedence;
- seven-day daily plan;
- release acceptance checklist;
- agent operating rules;
- constrained research reference;
- implementation bootstrap prompt.

Evidence:

- governing documents are committed to `main`;
- no code or destructive implementation exists yet.

### Day 1 — Foundation and contracts

Status: **complete**

`PLANS.md`/`BOOTSTRAP_PROMPT.md` describe Day 1 as a single unit. The owner's
Day 1A/1B split further divides it: Day 1A delivered the read-only systems
foundation; Day 1B connects it to strict threshold configuration and a
read-only `status` CLI. Both implementation slices are present. GitHub Actions
run `30749467277` passed format, Clippy, and tests on Ubuntu, macOS, and Windows,
so the full Day 1 gate is complete.

#### Day 1A — Architecture & systems foundation

Status: **complete**

Commit: `62ec505` — `chore: establish systems foundation`

Files changed:

```text
Cargo.toml
Cargo.lock
.gitignore
src/lib.rs
src/main.rs
src/model.rs
src/disk.rs
src/platform/mod.rs
src/platform/linux.rs
src/platform/macos.rs
src/platform/windows.rs
.github/workflows/ci.yml
IMPLEMENTATION_STATUS.md
```

Architecture:

- `src/lib.rs` exposes `pub mod model` (platform-neutral `DiskCapacity` +
  `DiskError`) and `pub mod disk` (`DiskProvider` trait, `SystemDiskProvider`,
  `FakeDiskProvider`) as the crate's library API; `platform` is a private
  module reached only through `disk`. `src/main.rs` is a thin binary over
  the library — this split exists so `FakeDiskProvider` and `DiskError`
  variants are real public API (not binary-only dead code under
  `-D warnings`) and so Day 1B / later integration tests in `tests/` can
  depend on the crate as a library, per PLANS.md's planned `tests/` layout.
- `DiskCapacity::new(total, available)` is the only constructor; it enforces
  `available_bytes <= total_bytes` and derives `used_bytes`, so the
  invariant `used_bytes = total_bytes - available_bytes` cannot drift.
- `platform::capacity_for` dispatches to `linux`/`macos`/`windows` submodules
  behind `#[cfg(target_os = ...)]`. All three currently delegate to one
  shared `measure()` helper (same underlying cross-platform syscall
  wrapper); the per-OS files exist as a dedicated seam for later platform
  divergence (e.g. macOS APFS/snapshot capacity ambiguity — ACCEPTANCE.md
  section K — is explicitly deferred, not attempted here). Unrecognised
  target OSes return `DiskError::UnsupportedPlatform` rather than guessing.
- `main.rs` is a harmless read-only placeholder (prints the capacity of the
  current directory) — not the product CLI. No `clap`, subcommands, config,
  or cleanup logic exists.

Dependencies chosen and why:

- `fs4 = "1"` (default features only) — cross-platform `total_space`/
  `available_space` for an arbitrary path (`statvfs` via `rustix` on
  Unix, `GetDiskFreeSpaceExW` via `windows-sys` on Windows). No shelling
  out to `df`/`wmic`/PowerShell, no output parsing, no unsafe code written
  in this crate. Default features pull in no async runtime.
- `tempfile = "3"` (dev-dependency only) — isolated, auto-cleaned temp
  directories for the spaces/Unicode-path filesystem tests, so tests don't
  touch the real project tree or leak directories.

Commands run:

```text
cargo build
cargo build --release
cargo run
cargo fmt
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
git diff --check
git status --short
```

Automated test evidence recorded at the Day 1A handover, before Day 1B CI:

- `cargo fmt --check`: pass (after one `cargo fmt` pass).
- `cargo clippy --all-targets --all-features -- -D warnings`: pass, zero
  warnings.
- `cargo test --all-targets`: 11/11 passed, 0 failed, 0 ignored.
  - `model::tests`: valid capacity, zero available, available == total,
    zero total, available > total is rejected as `Inconsistent`.
  - `disk::tests`: `FakeDiskProvider` returns injected values and errors
    `PathNotFound` for unregistered paths; `SystemDiskProvider` rejects a
    guaranteed-nonexistent path, measures a temp directory whose name
    contains spaces, measures a temp directory whose name contains Unicode
    (skipped gracefully if the host filesystem rejects the name), and
    returns a deterministic `total_bytes` across repeated calls on a
    stable volume.
- `cargo run`: printed a real `total`/`available`/`used` triple for this
  machine's volume; manually confirmed `used == total - available`.
- `git diff --check`: no whitespace errors.

Manual evidence:

- Ran on this development machine only (Linux, aarch64, Ubuntu 26.04 under
  proot). Not yet run on macOS or Windows, and GitHub Actions CI has not
  executed (no push performed this session).

Historical Day 1A limitations (the CI limitation is superseded below):

- CI (`.github/workflows/ci.yml`) is written and matrix-configured for
  `ubuntu-latest`/`macos-latest`/`windows-latest` but has not actually run
  yet — it needs a push/PR to execute. Ubuntu/macOS/Windows CI passing is
  therefore not yet evidenced, only asserted by local build success plus
  code review.
- macOS APFS purgeable-space / Time Machine snapshot ambiguity
  (ACCEPTANCE.md section K) is explicitly unresolved: `platform::macos`
  currently reports the raw `statvfs`-equivalent figures with no snapshot
  awareness. This is in scope for a later day, not Day 1A.
- No CLI, configuration, or `status` command exists yet — by design; that
  is Day 1B.

Acceptance items with evidence so far (from `ACCEPTANCE.md`; all others
remain unaddressed until later days):

- None of `ACCEPTANCE.md`'s checkboxes are claimed complete yet. Day 1A is
  purely a prerequisite systems layer; the first acceptance-relevant items
  (e.g. CI passing on all three OSes) require Day 1B's CLI and an actual
  CI run.

Blockers: none. Day 1A's own gate (compiling single-crate systems
foundation, cross-platform capacity API, no shell-out, no filesystem walk
or mutation, tested, CI defined) is met.

Exact next action: implement **Day 1B — CLI, Configuration & Boilerplate**
(clap-based CLI foundation, validated `Config` model, human-size parsing,
platform-appropriate config/state directories, `status` and
`status --json`) so that the full Day 1 gate in `PLANS.md` /
`BOOTSTRAP_PROMPT.md` can be evaluated. Do not begin Day 2 work first.

#### Day 1B — CLI, Configuration & Boilerplate

Status: **complete**

Date: 2026-08-02

Commits:

- `f072a1f` — `feat: add status CLI and threshold configuration`
- `c740ae5` — `fix: validate disk measurement paths`
- `19b3801` — `test: isolate CLI volume fixtures`
- `5f87fb3` — `test: preserve Windows known-folder resolution`

Files changed:

```text
Cargo.toml
Cargo.lock
src/lib.rs
src/main.rs
src/cli.rs
src/config.rs
src/status.rs
src/platform/mod.rs
tests/cli_tests.rs
README.md
IMPLEMENTATION_STATUS.md
```

Delivered:

- `terminal_janitor status`, `status --json`, `--help`, and `--version`;
- no placeholder commands for later days;
- `Config { minimum_free_bytes, target_free_bytes }` with strict ordering;
- deterministic case-sensitive `B`/`KiB`/`MiB`/`GiB`/`TiB` parsing with
  rejection of decimals, signs, malformed values, unknown units, and overflow;
- `directories::ProjectDirs` config resolution and read-only 10 GiB / 15 GiB
  defaults when the file is absent;
- fail-closed invalid/corrupt existing configuration with
  `FAILED_CONFIGURATION` and exit code 2;
- pure healthy/pressure status calculation over Day 1A `DiskCapacity`;
- separate human and stable JSON renderers;
- `PRESSURE_DETECTED` without cleanup or false restoration claims;
- storage-measurement failures reported as `FAILED_STORAGE_MEASUREMENT` with
  exit code 3;
- injected fake capacity and path tests, plus actual-binary integration tests.

Dependencies added:

```text
clap = "4" (derive)
directories = "6"
serde = "1" (derive)
serde_json = "1"
toml = "0.9"
```

Commands run locally:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
terminal_janitor --help
terminal_janitor --version
terminal_janitor status
terminal_janitor status --json
git diff --check
git status --short
```

Automated test evidence:

- pre-edit baseline: format pass, Clippy pass, 11/11 tests pass;
- Day 1B local suite: 34/34 tests pass, 0 failed, 0 ignored;
- [GitHub Actions run `30749467277`](https://github.com/iqmanx/terminal_janitor/actions/runs/30749467277):
  success on Ubuntu, macOS, and Windows;
- each CI job passed `cargo fmt --check`, strict Clippy, and all applicable
  tests; Linux ran 34 tests and macOS/Windows ran 33 tests because the
  invalid-XDG-config process test is Linux-specific;
- the first Windows run exposed that `GetDiskFreeSpaceExW` can accept a
  nonexistent child path; the provider now validates the requested path before
  measuring, preserving the Day 1A exact-path contract on every platform;
- size/config tests cover valid `10GiB`, byte boundaries, zero, unknown and
  wrong-case units, negative and decimal values, malformed values, overflow,
  equal/below target, corrupt TOML, absent defaults, invalid-file fail-closed,
  unknown fields, spaces, and Unicode;
- status tests cover exactly-at/above threshold, one-byte-below and zero
  availability, used-space derivation, human truthfulness, JSON values, and
  stable field names;
- CLI tests cover help, version, human and JSON output, real JSON parsing,
  invalid-config exit 2, injected disk-failure exit 3, and paths containing
  spaces and Unicode.

Manual evidence:

- Linux aarch64 Ubuntu 26.04 under proot: live `status` and `status --json`
  succeeded using defaults and reported pressure truthfully;
- live JSON parsed with `JSON.parse` as an eight-field object;
- observed warm debug runtime was 0.05 seconds for both human and JSON status;
- source inspection confirms the status path performs no filesystem walk,
  SQLite access, pnpm/Git/process inspection, scheduler operation, shell
  invocation, or user-data mutation.

Known limitations:

- Local manual CLI smoke evidence is Linux/aarch64; macOS and Windows evidence
  comes from the successful CI jobs.
- Status measures the volume containing the current working directory.
- Day 1 does not create configuration; users may place strict TOML at the
  documented conventional path until Day 2 adds `init`.
- macOS APFS snapshot/purgeable-capacity ambiguity remains unresolved as
  recorded in Day 1A; Day 1 only reports raw capacity and performs no cleanup.

Acceptance items supported:

- configurable and validated threshold model;
- `status` performs no project walk or cleanup;
- stable valid JSON and machine-readable Day 1 results;
- local format, Clippy, and test checks pass;
- Linux live CLI smoke and JSON-parser validation pass.
- Ubuntu, macOS, and Windows CI pass for the final Day 1 commit.

Blockers: none for Day 1.

Exact next action: begin **Day 2 — State, Registration & Read-Only Discovery**.
Do not add later-day cleanup, execution, or scheduling behavior.

### Day 2 — State, registration, read-only discovery

`PLANS.md` describes Day 2 as one unit. The owner's Day 2A/2B split further
divides it: Day 2A delivered the persistent, fail-closed state and identity
foundation; Day 2B connects it to `init`, approved-root registration,
read-only pnpm workspace discovery, protection commands, and `scan`. Day 2B's
implementation, local gate, and cross-platform CI all pass. The complete Day 2
gate is closed by run `31163641988` on commit `cd01c1d`, which passes format,
strict Clippy, and the full test suite on Ubuntu, macOS, and Windows with no
ignored test.

#### Day 2A — State Ledger & Identity Safety

Status: **complete**

Date: 2026-08-02

Commits:

- `7c0b3f0` — `feat: add state ledger and identity safety`
- `190a7aa` — `fix: open directories for Windows identity queries`

Files changed:

```text
Cargo.toml
Cargo.lock
src/lib.rs
src/config.rs
src/identity.rs      (new)
src/state.rs         (new)
src/platform/mod.rs
src/platform/linux.rs
src/platform/macos.rs
src/platform/windows.rs
README.md
IMPLEMENTATION_STATUS.md
```

Dependencies added and why:

- `rusqlite = "0.37"` with the `bundled` feature — the metadata ledger.
  Bundling compiles SQLite into the binary, so users never need a system
  SQLite library or database server (ACCEPTANCE.md section A).
- `tempfile = "3"` promoted from dev-dependency to dependency — provides the
  same-directory temporary file plus atomic `persist` (rename on Unix,
  `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` on Windows) used by
  `config::save_config_at`.
- `dunce = "1"` — canonicalisation that strips Windows verbatim `\\?\`
  prefixes when safe, so one directory always canonicalises to one stored
  spelling per platform; a passthrough to `std::fs::canonicalize` on Unix.
- `windows-sys = "0.61"` (Windows target only; already a transitive
  dependency of `fs4` there) with `Win32_Foundation` and
  `Win32_Storage_FileSystem` — `GetFileInformationByHandle` for native
  volume-serial and file-index identity without any shell command.

Architecture:

- `src/identity.rs` — `CanonicalPath` (absolute, symlink-resolved,
  UTF-8-lossless; relative and non-representable paths are explicit errors),
  `VolumeId` and `FileId` (opaque non-empty native identities), and
  `PathIdentity` combining all three. `PathIdentity::resolve` canonicalises
  and captures identity from the real filesystem; `from_parts` assembles
  verified parts for ledger loads and mocked-volume tests.
- `src/platform` gained `file_identity(path)`: `st_dev`/`st_ino` from native
  metadata on Linux/macOS; volume serial number plus 64-bit file index from
  `GetFileInformationByHandle` on Windows (via std `File::open`, which
  already applies `FILE_FLAG_BACKUP_SEMANTICS` for directories). Failure is
  an explicit error — identity is never guessed. No shell, parsing of tool
  output, or elevation anywhere; capacity measurement remains a separate
  function.
- `src/state.rs` — `StateStore` trait (schema version, approved roots,
  workspace observations, protection) implemented by `SqliteStateStore`.
  The `rusqlite::Connection` is a private field; no raw SQL crosses the API.
  Explicit ordered migrations run inside one immediate transaction together
  with the `schema_meta` version bump; failure rolls everything back.
  Opening runs `PRAGMA quick_check` first, rejects corrupt files, rejects
  non-empty databases lacking terminal_janitor schema metadata, and rejects
  newer schema versions — always leaving the file byte-for-byte untouched
  (never renamed, deleted, or recreated). Schema v1: `schema_meta`,
  `approved_roots` (unique per volume+canonical path), `workspaces` (partial
  unique index per volume+canonical path where status is `present`), STRICT
  tables, indexes only on the identity lookups the API performs.
- Identity semantics: duplicate root approvals and repeated workspace
  observations collapse into their existing records; an ASCII-case-variant
  spelling of the same physical directory (same volume+file identity)
  collapses rather than duplicating authority; the same physical directory
  under an unrelated path is a `ConflictingIdentity` error for roots and a
  fresh workspace identity for observations; a different physical directory
  appearing at a recorded path marks the old record `replaced` (its
  protection flag is retained, never erased) and registers a brand-new
  `WorkspaceId` with fresh history. The same path string on another volume
  is always a distinct record.
- Time: an injectable `Clock` (`SystemClock` in production, `FixedClock`
  for tests) feeds every mutation; the store enforces that
  `first_observed_at` never changes and `last_observed_at` /
  `last_activity_at` never move backwards, even under a misbehaving clock.
  `last_cleaned_at` exists, is nullable, and nothing in Day 2 writes it.
  Absent activity evidence on insert conservatively uses the observation
  time (delays future cleanup eligibility; never accelerates it).
- Bounds: 20 MiB normal maximum (SAFETY.md section 12) checked before each
  state-changing transaction and again before commit; a violation rolls the
  transaction back with `SizeLimitExceeded` and prior state stays readable.
  Repeated identical observations update one row.
- `config::save_config_at` — validates the complete configuration first
  (thresholds re-checked; roots must be absolute, UTF-8, sorted,
  de-duplicated), serialises, writes to a same-directory temporary file,
  flushes and syncs, then replaces the destination atomically; any failure
  preserves the previous valid file and removes the temporary file. Parent
  directories are created user-only (0o700) on Unix. `Config` gained
  `approved_roots` (defaults to empty; Day 1 files stay readable);
  threshold validation is unchanged. No command writes configuration as a
  side effect — the writer has no callers yet outside tests, by design.

Commands run:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo run -- status --json
git diff --check
git status --short
```

Automated test evidence (local Linux aarch64, Ubuntu 26.04 under proot):

- pre-edit baseline: format pass, Clippy pass, 34/34 tests pass;
- Day 2A suite: 74/74 tests pass, 0 failed, 0 ignored
  (69 unit + 5 CLI integration);
- migrations: fresh database receives schema v1; three consecutive reopens
  are idempotent and preserve data; an injected failing migration rolls back
  to zero objects; schema version 999 is rejected as unsupported with all
  rows and the version left untouched; a garbage file and a foreign SQLite
  database are rejected as corrupt and left byte-for-byte identical;
  protection survives reopen and the migration check;
- identity: duplicate roots collapse to one record with stable `created_at`;
  ASCII-case-variant aliases of the same physical directory collapse for
  roots and workspaces (protection retained); the same physical directory
  under an unrelated path is a conflict; a replaced directory at a root path
  is a conflict; replaced workspaces get fresh identity while the old
  record keeps `protected` and becomes `replaced`; moved directories start
  fresh; the same path string on a second mocked volume stays distinct for
  roots and workspaces; relative paths are rejected as identities (resolve
  and from_verified); nonexistent paths error; symlink aliases resolve to
  one identity (Unix); spaces and Unicode round-trip losslessly; unknown
  root and unknown workspace references fail closed;
- timestamps: `first_observed_at` stable, `last_observed_at` and
  `last_activity_at` non-decreasing under a backwards-running clock and
  stale evidence, `last_cleaned_at` stays null;
- bounds: 100 repeated observations keep exactly one row; an oversized
  change is rejected with rollback and the prior state remains readable and
  reopenable; a store already over budget refuses writes but still reads;
- atomic config: saved files are read back identically by the Day 1 loader
  (including spaces/Unicode roots); Day 1 files without `approved_roots`
  stay readable; duplicates collapse deterministically; relative roots are
  rejected at set, load, and save; invalid new configurations leave the
  previous file byte-identical; a read-only directory (Unix) fails the
  temporary write with the previous file intact; successful replacement
  leaves no temporary files; parents are created 0o700 on Unix;
  `format_size` round-trips through the strict parser including `u64::MAX`;
- CI: the first run for `7c0b3f0`
  ([`30753921516`](https://github.com/iqmanx/terminal_janitor/actions/runs/30753921516))
  passed Ubuntu and macOS completely (format, strict Clippy, all tests —
  including Windows-targeted Clippy passing on the real Windows toolchain)
  but failed three Windows identity tests: `File::open` cannot open a
  directory on Windows without `FILE_FLAG_BACKUP_SEMANTICS`, so
  `file_identity` returned the explicit `VolumeIdentityUnavailable` error —
  the fail-closed path, not a wrong identity. `190a7aa` opens directory
  handles with a zero access mode plus `FILE_FLAG_BACKUP_SEMANTICS`;
  [GitHub Actions run `30754968125`](https://github.com/iqmanx/terminal_janitor/actions/runs/30754968125)
  then passed on Ubuntu, macOS, and Windows (74/74 tests on each OS-applicable
  set, 0 ignored).

Manual evidence:

- `cargo run -- status --json` still produces the stable Day 1 JSON object,
  exits without opening the ledger, and truthfully reported pressure on this
  machine;
- source inspection confirms: no shell command anywhere in the new code; no
  project-tree scan (identity resolution touches only the exact path it is
  given); no pnpm invocation; no user-project mutation (the only writes are
  the ledger file and atomic config replacement); corrupt state cannot be
  silently erased (no code path deletes or recreates the database); state
  growth is bounded.

Known limitations:

- Volume identity is best-available native metadata: `st_dev` can change
  across reboots for some filesystem types, and the Windows volume serial is
  32-bit. Day 4's pre-execution revalidation must re-resolve identity from
  the live filesystem rather than trusting stored identity alone (already
  required by SAFETY.md section 8).
- File identity (`st_ino`/file index) can be reused by the OS after
  deletion; the replaced/moved rules treat identity conservatively (fresh
  history, no inherited authority), and later gates revalidate against the
  live filesystem before any action.
- Case-alias collapse uses ASCII case-insensitive comparison; exotic
  Unicode case aliases on case-insensitive filesystems would create
  distinct records — duplicate metadata, not duplicate cleanup authority,
  since Day 3+ proof gates revalidate physical identity.
- Paths not representable in UTF-8 are rejected as identities (explicit
  error, never lossy storage).
- `schema_version`, `list_*`, and `get_workspace` are read paths and do not
  re-run `quick_check` per call; corruption arising mid-session surfaces as
  explicit SQLite errors mapped to `Corrupt`/`TransactionFailed`.

Acceptance items supported (evidence recorded above; the full checkboxes
remain open until their CLI workflows and CI runs exist):

- I: SQLite migrations are explicit and tested; corrupt state fails safely
  rather than silently resetting protection; atomic configuration writes
  exist and are tested; database normal maximum 20 MiB enforced; no project
  contents, secrets, or command histories are stored;
- E: canonical path and volume identity are stored for roots and
  workspaces;
- F (partial): protection state persists across reopen and updates;
- A (partial): no database server or system SQLite is required.

Blockers: none for Day 2A.

Exact next action: **Day 2B — Registration, Discovery & CLI Workflows**
(`init`, approved-root registration writing config atomically, read-only
pnpm workspace discovery beneath approved roots, protection commands, and
`scan`), strictly on top of the Day 2A state API. Do not begin Day 3
planning/proof behaviour.

#### Day 2B — Registration, Discovery & CLI Workflows

Status: **complete; complete Day 2 gate passed on Ubuntu, macOS, and Windows**

Date: 2026-08-02 (implementation), 2026-08-07 (cross-platform fix and gate)

Commits:

- `3f1c50b` — `feat: add workspace registration and read-only discovery`
- `cd01c1d` — `fix: canonicalise the Git observation root before containment checks`

Files changed:

```text
src/activity.rs       (new)
src/discovery.rs      (new)
src/workflows.rs      (new)
src/cli.rs
src/lib.rs
src/main.rs
src/state.rs
tests/cli_tests.rs
README.md
IMPLEMENTATION_STATUS.md
```

Dependencies added and why:

- None. Day 2B uses the existing standard library, serde, directories,
  rusqlite, dunce, and tempfile dependencies.

Delivered:

- `init --root <path>` with repeatable explicit roots, optional strict
  thresholds, canonical/volume/file identity, filesystem-root and symlink-root
  rejection, deterministic deduplication, no inferred home/project roots, no
  scheduling, and no automatic scan;
- coordinated atomic config plus transactional root registration: config
  writer failure rolls back all root rows and removes a newly-created empty
  state file; state failure leaves the old config unchanged; a rare state
  commit failure after config replacement invokes a compensating atomic config
  rollback;
- schema v2 migration adding retained `missing` workspace status and bounded
  Git state/fingerprints while preserving v1 protection data;
- bounded discovery only beneath approved canonical roots, with no symlink
  following, no `.git`/`node_modules` recursion, depth limit 64, diagnostic cap
  1,000, canonical containment checks at every visited directory, deterministic
  nested-root ownership, duplicate physical-identity collapse, and contained
  mounted-volume identity preservation;
- exact stable exclusion reasons for partial markers, permissions,
  disappearing/unsupported paths, symlinks, identity failures, changed or
  unavailable roots, outside-root paths, and duplicates;
- valid pnpm workspace recognition only when `package.json`,
  `pnpm-workspace.yaml`, and `pnpm-lock.yaml` are regular files in one directory;
- conservative activity from marker mtimes, bounded `.git/HEAD`/loose-ref/
  `packed-refs` fingerprints, and Git index mtime/length; access time is never
  read; worktree state is explicitly `unknown` because Day 2 invokes no Git;
- one-transaction scan observation upsert/missing marking with stable first
  observation, forward-only last observation/activity, unchanged
  `last_cleaned_at`, and protection never cleared; uncertain permission/
  canonicalisation/volume/depth prefixes do not falsely mark known workspaces
  missing;
- moved/replaced workspaces receive new IDs and no inherited protection or
  observation authority; absent workspaces and their protection remain stored;
- exact registered-workspace `protect add`, `protect remove`, and `protect list`
  with idempotent updates, persistent state, and no identity bypass;
- stable human and JSON output for init, scan, protection, and the unchanged
  read-only status command; no later-day CLI placeholders.

Commands run locally:

```text
git status --short --branch
git log --oneline -15
git diff --check
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
terminal_janitor --help
terminal_janitor --version
terminal_janitor status
terminal_janitor status --json
terminal_janitor init --root <generated-fixture-root>
terminal_janitor init --root <generated-fixture-root> --json
terminal_janitor scan
terminal_janitor scan --json
terminal_janitor protect add <generated-fixture-workspace>
terminal_janitor protect list
terminal_janitor protect list --json
terminal_janitor protect remove <generated-fixture-workspace>
```

Automated test evidence (local Linux aarch64, Ubuntu 26.04 under proot):

- pre-edit Day 2A baseline: format pass, strict Clippy pass, 74/74 tests pass;
- Day 2B final local suite: 101/101 tests pass, 0 failed, 0 ignored (95 unit +
  6 actual-binary integration tests);
- init tests cover explicit-root requirement, canonical registration, missing/
  file/filesystem-root/symlink rejection, duplicate idempotency, threshold
  preservation/validation, injected config-write rollback, injected state
  conflict preserving config, and explicit no-schedule/no-scan results;
- discovery tests cover all marker combinations, approved-root containment,
  symlink escape/loop non-following, nested/overlapping roots, injected child
  volume identity, permission/disappearing paths, Unicode/spaces, deterministic
  ordering, and repeated discovery;
- activity/state tests cover marker and lock observations, HEAD/index
  fingerprints, explicit unknown Git state, stable first observation,
  forward-only activity, v1→v2 protection-preserving migration, bounded rows,
  moved/replaced/missing identity behaviour, and protection across rescan/reopen;
- CLI instrumentation places fake `pnpm`, `npm`, `node`, `git`, `sh`, and
  `bash` executables first on `PATH`; scan invokes none, and before/after marker
  contents plus modification times are identical;
- all tested JSON is parsed with `serde_json`; live JSON evidence below was
  parsed by Python's standard `json` parser.

Manual evidence:

- generated Linux fixture with spaces and Unicode completed human `status`,
  `init`, `scan`, protection add/list/remove, plus JSON status/init/scan/list;
- every live JSON stream parsed successfully with a real JSON parser;
- repeated init returned the same canonical approved root and repeated scan
  updated the existing workspace instead of appending a row;
- SHA-256 snapshots of all three project markers matched before and after all
  workflows; no cleanup was performed;
- source inspection confirms production contains no `std::process::Command`,
  shell, pnpm invocation, package-script execution, planner, `ProofBundle`,
  `AllowedAction`, cleanup plan, or project write API; discovery/activity expose
  reads only, while writes are limited to atomic config, SQLite state, private
  state directories, and exact rollback of newly written product-state files;
- schema inspection confirms rows scale with approved roots and workspaces,
  not visited directories; traversal retains only a bounded stack, a per-root
  visited identity set for the current scan, workspace observations, and at
  most 1,000 diagnostics.

Cross-platform gate — 2026-08-07:

The first CI run for `3f1c50b` (run `30788532065`) passed Ubuntu and failed
macOS and Windows. Format and strict Clippy passed on all three; `cargo test`
failed three `src/activity.rs` tests on both platforms:

```text
activity::tests::head_content_change_changes_fingerprint
activity::tests::git_index_change_changes_bounded_fingerprint
activity::tests::marker_and_git_observations_are_read_only_and_git_stays_unknown
```

Root cause (one defect, three failures): `read_optional_bounded` compared a
canonicalised child path against an uncanonicalised `.git` root, so a `.git`
child appeared to fall outside its own approved root whenever an ancestor was
a link or an alias. macOS reaches temporary and user directories through
`/var -> /private/var`; Windows supplies 8.3 short names such as `RUNNER~1`.
Both platforms therefore discarded every HEAD and index fingerprint. Linux
temporary directories are already canonical, which is why Linux passed and the
defect was invisible locally.

Fix (`cd01c1d`): canonicalise the `.git` root once, after the existing symlink
rejection and before any containment check, then compare canonical against
canonical. The symlink guard still runs first, so a symlinked `.git` is still
refused rather than resolved; `git_symlink_is_not_followed` continues to pass.

Regression test `activity::tests::observations_survive_a_linked_ancestor_directory`
observes one workspace directly and again through a symlinked ancestor and
requires identical snapshots. It reproduces the macOS and Windows failure on
Linux: with the fix reverted it fails locally with
`assertion failed: linked.git_head_fingerprint.is_some()`; with the fix it
passes. It is `#[cfg(unix)]`, so it compiles on Linux and macOS only; the
Windows short-name case is covered by the same production code path and by
Windows CI on the three original tests.

Local evidence for `cd01c1d` (Linux aarch64, Ubuntu 26.04 under proot):

```text
cargo fmt --check                                        pass
cargo clippy --all-targets --all-features -- -D warnings  pass
cargo test --all-targets                                  102 passed; 0 failed; 0 ignored
```

CI evidence for `cd01c1d` — run `31163641988`,
`https://github.com/iqmanx/terminal_janitor/actions/runs/31163641988`:

```text
test (ubuntu-latest)   success   96 unit + 0 + 6 integration; 0 failed; 0 ignored
test (macos-latest)    success   96 unit + 0 + 4 integration; 0 failed; 0 ignored
test (windows-latest)  success   91 unit + 0 + 4 integration; 0 failed; 0 ignored
```

All three jobs ran `cargo fmt --check`, strict Clippy, and `cargo test
--all-targets`, and all three passed. No test is ignored on any platform. The
lower macOS and Windows counts are `#[cfg(unix)]` and Unix-only-fixture tests
that are not compiled on Windows, not tests that were skipped at runtime.

Known limitations:

- Day 2 deliberately reports Git worktree state as `unknown`; it observes HEAD
  and index movement without invoking Git. Day 3 may strengthen this behind the
  separable activity probe without changing registration identity.
- Paths not representable in UTF-8 fail closed, matching Day 2A.
- A final-component symlink is rejected for approved-root registration;
  symlinks in ancestor spelling are resolved to the exact canonical root, and
  no directory symlink encountered below it is followed.
- Local manual evidence remains Linux/aarch64 only. macOS and Windows evidence
  is CI evidence, not real-machine manual evidence; the real-machine
  generated-fixture smoke test in ACCEPTANCE.md section M stays open.
- The three-platform fingerprint defect proves that Linux-only local runs
  cannot qualify a filesystem-facing change. Path-containment work must be
  proven with a linked-ancestor fixture, not only a plain temporary directory.

Acceptance items supported:

- E (Day 2 scope): only approved roots are inspected; canonical path and volume
  identity are stored; discovery cannot escape; names grant no authority; no
  project write occurs;
- F (Day 2 scope): explicit protection persists across scans/reopen/migration;
- I (Day 2 scope): explicit migration, corruption failure, atomic config, 20 MiB
  database bound, and metadata-only state;
- L (Day 2 scope): status remains no-walk/read-only, scan is read-only, JSON is
  valid/stable, exclusions are exact, and human output states no cleanup;
- D (discovery prerequisite only): all three pnpm markers are required and
  instrumentation proves pnpm/project scripts are not invoked;
- M (Day 2 scope): `cargo fmt --check`, strict Clippy, and `cargo test
  --all-targets` pass, and Ubuntu, macOS, and Windows CI all pass with no
  ignored test. The remaining section M items belong to later days.

Blockers: none. The complete Day 2 gate is closed.

Exact next action: **Day 3 — Pnpm Adapter, Proof Gates & Planning**.

### Day 3 — Pnpm adapter, proof gates, planning

Status: **complete; Day 3 gate passed on Ubuntu, macOS, and Windows**

Date: 2026-08-07

Started immediately after the complete Day 2 gate closed on run `31163641988`.

Commits:

- `b244281` — `feat: add proof-driven pnpm planning`
- `fae3f7c` — `fix: make Day 3 fixtures portable and collapse the PATH lookup match`

Files changed:

```text
src/adapters/mod.rs   (new)
src/adapters/pnpm.rs  (new)
src/adapters/git.rs   (new)
src/planner.rs        (new)
src/protection.rs     (new)
src/state.rs
src/workflows.rs
src/cli.rs
src/main.rs
src/lib.rs
tests/cli_tests.rs
README.md
IMPLEMENTATION_STATUS.md
```

Dependencies added and why:

- None. Day 3 uses the existing dependencies plus the standard library's
  `std::process` and `std::thread`.

Delivered:

- typed `AllowedAction` with exactly the three `SAFETY.md` variants, fixed
  compiled argument arrays (`pm clean`, `store prune`), and no variant carrying
  a caller-supplied path or command string;
- `CommandRunner` with a direct, shell-free system implementation: argument
  arrays only, stdin closed, per-stream bounded capture on separate reader
  threads so a full pipe cannot deadlock the timeout, an enforced timeout that
  kills and reaps the child, and truncated or non-UTF-8 output treated as
  ambiguous;
- pnpm enrolment during `init` with `--pnpm <path>` or a single `PATH` search,
  canonical executable identity (path, volume, file), `pnpm --version` as the
  only command run, strict version parsing that rejects empty, multi-line,
  prefixed, or partial output, and a minimum major version of 11;
- refusal of `.cmd`, `.bat`, `.ps1`, `.psm1`, `.vbs`, and `.js` wrappers,
  because executing one would place `cmd.exe` or PowerShell between the product
  and pnpm;
- schema v3 storing exactly one pnpm enrolment, replacing rather than
  accumulating, with a corrupt row reported as corruption instead of absence;
- read-only Git worktree proof through
  `git --no-optional-locks status --porcelain=v1 --untracked-files=all`, where
  empty output is clean, any line is dirty, and a missing Git, non-zero exit,
  timeout, or truncated output is `unknown`;
- location protection: explicit protection, a denylist built from the
  platform's own user directories rather than guessed English names, recognised
  cloud-sync providers, and unidentifiable sync names refused as ambiguous;
  provider markers are probed only at or below an approved root, so no
  unapproved directory is ever inspected;
- `ProofBundle` with all eight `SAFETY.md` fields and no score, weight, or
  override; an action reaches a plan only when every field is true;
- ordered gates returning exactly one failure each: registration status,
  approved-root ownership, live identity revalidation, pressured volume,
  protection, the three pnpm markers, pnpm enrolment/version/identity, Git
  cleanliness, the 24-hour observation window, the 30-day inactivity window,
  the 7-day workspace cooldown, and process liveness;
- deterministic ordering by oldest proven activity, then oldest first
  observation, then canonical path, with a two-workspace automatic cap and the
  remainder reported as `AUTOMATIC_WORKSPACE_CAP_REACHED`;
- immutable plan identity: an FNV-1a policy hash over thresholds, approved
  roots, every window and cap, the minimum pnpm major, the action names, and
  the enrolled executable identity and version; a plan ID over the policy hash,
  the creation moment, and the exact action list; and a 15-minute expiry;
- `scan` explanations — one plan per volume holding registered workspaces,
  ordered by volume identity, in both human and JSON output;
- a global `--dry-run` that reads and reports without writing the ledger.

Design decisions worth carrying forward:

- **Production plans no workspace clean yet, by design.** Process enumeration
  is Day 5 scope in `PLANS.md`. Rather than leave the liveness gate absent or
  silently passing, Day 3 ships `UnavailableLivenessProver`, which answers
  `Unknown` for every workspace. Unknown skips, so a real machine reports
  `LIVENESS_UNKNOWN` and plans nothing destructive. This keeps the Day 4
  executor structurally unable to clean a workspace whose liveness was never
  proven. Tests inject a prover to exercise the eligible path.
- **Git is resolved, not enrolled.** Pnpm is the delegate that deletes, so its
  identity is stored and revalidated. Git only answers a question, and every
  failure mode already collapses to `unknown`, so it is located at plan time
  and never granted stored authority.
- **The pnpm-store cooldown is Day 4.** `PLANS.md` assigns "pnpm-store cooldown
  and one immediate post-clean exception" to Day 4. Day 3 implements the
  workspace cooldown, which the existing `last_cleaned_at` column supports, and
  adds no state for the store cooldown.
- **Enrolment never fails registration.** `ACCEPTANCE.md` section A requires
  pnpm only when a pnpm action is used, so a missing, old, or wrapper-only pnpm
  is reported in the init result and leaves approved roots registered.

Commands run locally:

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Automated test evidence (local Linux aarch64, Ubuntu 26.04 under proot):

- 159/159 tests pass, 0 failed, 0 ignored (153 unit + 6 actual-binary
  integration tests), up from 102 at the close of Day 2;
- adapter tests cover the fixed argument arrays, the absence of a bare
  `pnpm clean` and of `--lockfile`, `-l`, and `--force`, version parsing and
  rejection, enrolment identity, wrapper refusal, directory and missing-file
  refusal, and every Git failure mode collapsing to `unknown`;
- protection tests cover explicit protection precedence, confirmed and
  name-only provider detection, ambiguous names failing closed, denied roots,
  a sibling that merely shares a name prefix, and the filesystem root never
  becoming a boundary;
- planner tests cover a complete proof bundle, each gate refusing in isolation,
  each missing marker, dirty and unknown Git, active and unknown liveness, the
  production prover planning nothing, ordering and both tie-breaks, the
  two-workspace cap, deterministic identical plans, policy-hash and plan-ID
  change on policy or time change, and expiry at and after the boundary;
- the CLI integration test replaces `pnpm`, `npm`, `node`, `git`, `sh`, and
  `bash` with logging scripts and asserts token by token that only
  `pnpm --version` and `git --no-optional-locks status …` ever run, that no
  `clean`, `--lockfile`, `-l`, or `--force` argument is ever passed, that a new
  workspace is refused with exactly `OBSERVATION_WINDOW_NOT_MET`, and that
  `--dry-run` leaves the ledger file byte-identical.

Cross-platform gate:

The first Day 3 run (`31166832906`) failed on all three platforms during
strict Clippy, before any test executed: CI stable is 1.97 and raises
`clippy::collapsible_match` on the pnpm `PATH` lookup, which the local 1.93
toolchain does not. `fae3f7c` collapses that lookup to the same single
question the Git lookup already asks, and also makes the planner and
protection fixtures portable — they wrote POSIX paths such as `/approved/a`,
which Windows does not treat as absolute, so they would have failed
`CanonicalPath::from_verified` and been silently dropped from the denylist
once Clippy stopped masking them.

CI evidence for `fae3f7c` — run `31167346665`,
`https://github.com/iqmanx/terminal_janitor/actions/runs/31167346665`:

```text
test (ubuntu-latest)   success   153 unit + 0 + 6 integration; 0 failed; 0 ignored
test (macos-latest)    success   153 unit + 0 + 4 integration; 0 failed; 0 ignored
test (windows-latest)  success   148 unit + 0 + 4 integration; 0 failed; 0 ignored
```

All three jobs ran `cargo fmt --check`, strict Clippy, and `cargo test
--all-targets`, and all three passed with no ignored test. The lower macOS and
Windows counts are `#[cfg(unix)]` and Unix-only-fixture tests that are not
compiled there, not tests skipped at runtime.

Known limitations:

- No workspace clean is planned on a real machine until Day 5 supplies process
  liveness. This is the intended fail-closed behaviour, not a defect.
- Cloud-sync detection above an approved root is name-based only, because
  inspecting an unapproved parent is not permitted. A provider marker outside
  the approved root is never read.
- Local evidence is Linux/aarch64. Windows wrapper refusal is proven by unit
  tests that construct the wrapper names on any platform; no real Windows
  `pnpm.cmd` has been exercised on a Windows machine.
- The local toolchain is cargo 1.93.1 while CI uses stable, which is currently
  1.97. A clean local `cargo clippy -D warnings` therefore does not guarantee a
  clean CI Clippy: the first Day 3 CI run (`31166832906`) failed on all three
  platforms on `clippy::collapsible_match`, a lint the local toolchain does not
  raise. Strict Clippy must be treated as a CI gate, not a local one, until the
  local toolchain matches.

Blockers: none.

Exact next action: begin **Day 4 — Verified Executor and Threshold Loop**.

### Days 4–7

Status: **not started**

Follow `PLANS.md` strictly. Do not begin a later day until the current day's mandatory gate passes.

## Current repository contents

Days 1, 2A, 2B, and 3 contain the systems, configuration, status/CLI, identity,
state ledger, registration, discovery, activity-observation, protection,
pnpm/Git adapters, and proof-driven planning foundation. Still intentionally
absent (Day 4 and later scope):

```text
src/executor.rs
src/journal.rs
scripts/
```

Their absence is not a defect at this point in the plan. Present after Day 3:

```text
Cargo.toml
Cargo.lock
src/lib.rs
src/main.rs
src/cli.rs
src/config.rs
src/activity.rs
src/discovery.rs
src/identity.rs
src/state.rs
src/status.rs
src/workflows.rs
src/model.rs
src/disk.rs
src/planner.rs
src/protection.rs
src/adapters/mod.rs
src/adapters/pnpm.rs
src/adapters/git.rs
src/platform/
tests/cli_tests.rs
.github/workflows/ci.yml
```

## Exact next action

The next implementation agent should:

1. Read `SAFETY.md`, `ACCEPTANCE.md`, `PLANS.md`, `AGENTS.md`, `VISION.md`, `RESEARCH.md`, and this file.
2. Begin **Day 4 — Verified Executor and Threshold Loop**: per-volume lock, run and action journal,
   direct execution with exact argument arrays, command timeout and bounded
   output, pre-action proof and identity revalidation, free-space measurement
   before and after every action, stop-at-target, shortfall, the two-workspace
   cap, own-state cleanup, and the pnpm-store cooldown with its one immediate
   post-clean exception.
3. Do not weaken the Day 3 liveness gate to make an executor demonstration
   work. `UnavailableLivenessProver` must keep answering `Unknown` until Day 5
   supplies real process enumeration; a Day 4 demonstration uses an injected
   prover and generated fixtures.
4. Prove every path-containment change with a linked-ancestor fixture, not only
   a plain temporary directory, and require Ubuntu, macOS, and Windows CI
   before claiming any gate.

## Required status-update format

After each day, replace or extend the relevant section with:

```text
Status:
Commit:
Files changed:
Commands run:
Automated test evidence:
Manual evidence:
Known limitations:
Acceptance items supported:
Blockers:
Exact next action:
```

Do not state that a platform, test, or acceptance gate passed without evidence.

## Release status

```text
0.1.0 release authorised: NO
Reason: Days 3–7 remain outstanding
```

Do not tag or publish 0.1.0 until every applicable gate in `ACCEPTANCE.md` is supported by recorded evidence.
