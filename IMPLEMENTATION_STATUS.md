# Implementation Status

## Repository

```text
Repository:     iqmanx/terminal_janitor
Default branch: main
Product:        terminal_janitor
Target version: 0.1.0
Current phase:  Day 1 complete; Day 2 not started
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

### Days 2–7

Status: **not started**

Follow `PLANS.md` strictly. Do not begin a later day until the current day's mandatory gate passes.

## Current repository contents

Day 1 now contains the systems, configuration, status, and CLI foundation.
Still intentionally absent (Day 2 and later scope):

```text
src/state.rs
src/activity.rs
src/protection.rs
src/planner.rs
src/executor.rs
src/journal.rs
src/adapters/
scripts/
```

Their absence is not a defect at this point in the plan. Present after Day 1B:

```text
Cargo.toml
Cargo.lock
src/lib.rs
src/main.rs
src/cli.rs
src/config.rs
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
2. Verify the repository and branch state.
3. Begin **Day 2 — State, Registration & Read-Only Discovery** with bundled
   SQLite, explicit migrations, atomic configuration writes, approved-root
   registration, pnpm workspace discovery, canonical path/volume identity,
   protection commands, and read-only `scan`.
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
