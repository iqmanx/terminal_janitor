# Implementation Status

## Repository

```text
Repository:     iqmanx/terminal_janitor
Default branch: main
Product:        terminal_janitor
Target version: 0.1.0
Current phase:  Day 1 complete; Day 2A complete; Day 2B not started
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
read-only pnpm workspace discovery, protection commands, and `scan`. Only
Day 2A is complete; the full Day 2 gate (`scan` lists registered workspaces
with exact exclusion reasons) remains open until Day 2B.

#### Day 2A — State Ledger & Identity Safety

Status: **complete**

Date: 2026-08-02

Commit: `feat: add state ledger and identity safety` (single focused commit;
SHA is the commit carrying this section — verify with `git log --oneline -1`)

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
  `format_size` round-trips through the strict parser including `u64::MAX`.

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

- CI has not run for this commit (no push was authorised this session);
  macOS and Windows evidence for the new platform identity code and the
  case/Windows-path test variants therefore comes from code review, not CI.
  Run the three-OS matrix before treating the full Day 2 gate as satisfied.
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
planning/proof behaviour, and run three-OS CI for this commit first.

### Days 2B–7

Status: **not started**

Follow `PLANS.md` strictly. Do not begin a later day until the current day's mandatory gate passes.

## Current repository contents

Days 1 and 2A contain the systems, configuration, status/CLI, identity, and
state-ledger foundation. Still intentionally absent (Day 2B and later scope):

```text
src/activity.rs
src/protection.rs
src/planner.rs
src/executor.rs
src/journal.rs
src/adapters/
scripts/
```

Their absence is not a defect at this point in the plan. Present after Day 2A:

```text
Cargo.toml
Cargo.lock
src/lib.rs
src/main.rs
src/cli.rs
src/config.rs
src/identity.rs
src/state.rs
src/status.rs
src/model.rs
src/disk.rs
src/platform/
tests/cli_tests.rs
.github/workflows/ci.yml
```

## Exact next action

The next implementation agent should:

1. Read `SAFETY.md`, `ACCEPTANCE.md`, `PLANS.md`, `AGENTS.md`, `VISION.md`, `RESEARCH.md`, and this file.
2. Verify the repository and branch state, and run three-OS CI for the Day 2A
   commit if it has not run yet.
3. Begin **Day 2B — Registration, Discovery & CLI Workflows**: `init`,
   approved-root registration (writing configuration through
   `config::save_config_at`), read-only pnpm workspace discovery beneath
   approved roots recording observations through the `state::StateStore`
   API, protection commands, and read-only `scan`.
4. Keep all project discovery read-only and do not invoke pnpm or begin Day 3
   proof/planning behavior.

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
Reason: Days 2–7 implementation and acceptance evidence remain outstanding
```

Do not tag or publish 0.1.0 until every applicable gate in `ACCEPTANCE.md` is supported by recorded evidence.
