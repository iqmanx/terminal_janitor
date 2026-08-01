# Implementation Status

## Repository

```text
Repository:     iqmanx/terminal_janitor
Default branch: main
Product:        terminal_janitor
Target version: 0.1.0
Current phase:  Day 1A (systems foundation) complete; Day 1B (CLI/config) not started
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

Status: **in progress (Day 1A complete; Day 1B not started)**

`PLANS.md`/`BOOTSTRAP_PROMPT.md` describe Day 1 as a single unit. The owner's
Day 1A/1B task split further divides it: Day 1A delivers a read-only,
CLI-free systems foundation (disk-capacity model + provider); Day 1B (not
yet started) will add `clap`, configuration, size parsing, platform paths,
and `status`/`status --json`. This does not change or weaken the Day 1 gate
in `PLANS.md`/`BOOTSTRAP_PROMPT.md` — that gate still requires a working
`status` command, which remains unmet until Day 1B completes. Day 1 as a
whole is not being marked complete.

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

Automated test evidence (local, this machine — CI has not run yet, see
limitations below):

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

Known limitations:

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

### Days 2–7

Status: **not started**

Follow `PLANS.md` strictly. Do not begin a later day until the current day's mandatory gate passes.

## Current repository contents

Day 1A added a compiling systems-only foundation. Still intentionally absent
(Day 1B and later scope):

```text
src/cli.rs
src/config.rs
src/state.rs
src/activity.rs
src/protection.rs
src/planner.rs
src/executor.rs
src/journal.rs
src/adapters/
tests/
scripts/
```

Their absence is not a defect at this point in the plan. Present as of Day 1A:

```text
Cargo.toml
Cargo.lock
src/lib.rs
src/main.rs
src/model.rs
src/disk.rs
src/platform/
.github/workflows/ci.yml
```

## Exact next action

An implementation agent should:

1. Read `SAFETY.md`, `ACCEPTANCE.md`, `PLANS.md`, `AGENTS.md`, `VISION.md`, `RESEARCH.md`, and this file.
2. Verify the repository and branch state.
3. Continue Day 1: implement **Day 1B — CLI, Configuration & Boilerplate**
   (clap, `Config` model, size parsing, config/state paths, `status` and
   `status --json`) on top of the Day 1A systems foundation.
4. Add tests and confirm cross-platform CI actually runs (push/PR) with the
   implementation.
5. Run all required checks.
6. Update this file with actual commands, results, commit SHA, limitations,
   and the next action.
7. Only once the full Day 1 gate in `PLANS.md`/`BOOTSTRAP_PROMPT.md` passes
   (including a working `status` command on all three CI operating
   systems), mark Day 1 complete and move to Day 2.

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
Reason: implementation and acceptance evidence do not yet exist
```

Do not tag or publish 0.1.0 until every applicable gate in `ACCEPTANCE.md` is supported by recorded evidence.
