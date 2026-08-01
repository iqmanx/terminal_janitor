# Seven-Day Implementation Plan

## Mandate

Build `terminal_janitor` 0.1.0 in seven calendar days.

The result must be a single native Rust Engine + CLI that maintains a user-configured free-storage threshold through narrowly authorised cleanup actions and is safe enough to leave scheduled automatically.

The objective is not universal cleaning. The objective is:

> Delete less, require stronger proof, stop at the configured target, and stop safely when proof is insufficient.

`SAFETY.md` and `ACCEPTANCE.md` are hard gates. The deadline never permits weakening them.

---

## Fixed product identity

```text
Product:       terminal_janitor
Repository:    terminal_janitor
Rust package:  terminal_janitor
Binary:        terminal_janitor
Initial release: 0.1.0
```

---

## Week-one scope

### Included

- one Rust crate and native executable;
- Linux, macOS, and Windows builds;
- `minimum_free` trigger and `target_free` recovery thresholds;
- approved-root and pnpm-workspace registration;
- small bundled SQLite ledger;
- proof-driven planning;
- typed allowlisted actions;
- `pnpm store prune`;
- `pnpm pm clean` for fully proven inactive workspaces;
- per-volume locking and execution journalling;
- actual free-space measurement after every action;
- native user-level scheduling;
- human-readable and JSON output;
- adversarial cross-platform tests;
- installers and uninstallers.

### Excluded

- MCP, Claude Code, Codex, GUI, dashboard, VPS, accounts, telemetry;
- generic filesystem or cache cleaning;
- arbitrary path deletion or commands;
- user-defined cleanup recipes;
- Docker, Cargo, Nix, Gradle, Maven, Xcode, Python virtual environments;
- generic `target`, `dist`, `build`, `node_modules`, or cache removal;
- Downloads or personal-file cleaning;
- automatic restore;
- trash-based automatic cleaning;
- AI or learned deletion decisions.

These are not stretch goals.

---

## Planned repository layout

```text
terminal_janitor/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── VISION.md
├── PLANS.md
├── SAFETY.md
├── ACCEPTANCE.md
├── AGENTS.md
├── RESEARCH.md
├── IMPLEMENTATION_STATUS.md
├── BOOTSTRAP_PROMPT.md
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── config.rs
│   ├── model.rs
│   ├── state.rs
│   ├── disk.rs
│   ├── activity.rs
│   ├── protection.rs
│   ├── planner.rs
│   ├── executor.rs
│   ├── journal.rs
│   ├── adapters/
│   │   ├── mod.rs
│   │   ├── own_state.rs
│   │   └── pnpm.rs
│   └── platform/
│       ├── mod.rs
│       ├── linux.rs
│       ├── macos.rs
│       └── windows.rs
├── tests/
│   ├── fixtures/
│   ├── config_tests.rs
│   ├── planner_tests.rs
│   ├── executor_tests.rs
│   ├── protection_tests.rs
│   ├── concurrency_tests.rs
│   └── autonomy_tests.rs
├── scripts/
│   ├── install.sh
│   ├── install.ps1
│   └── uninstall.sh
└── .github/workflows/
    ├── ci.yml
    └── release.yml
```

Begin as one crate. Split only when a concrete problem requires it.

---

## Planned CLI

```text
terminal_janitor init
terminal_janitor status
terminal_janitor scan
terminal_janitor check
terminal_janitor clean
terminal_janitor protect add <path>
terminal_janitor protect remove <path>
terminal_janitor protect list
terminal_janitor history
terminal_janitor enable
terminal_janitor disable
```

Global options:

```text
--json
--dry-run
--verbose
```

`check` is the non-interactive scheduler entry point. `clean` uses the same authority and must not unlock broader actions.

---

## Threshold algorithm

Configuration:

```text
minimum_free: cleanup trigger
target_free:  cleanup stopping point
```

Validation:

```text
minimum_free > 0
target_free > minimum_free
```

Execution:

```text
1. Load and validate configuration.
2. Measure free storage.
3. Exit when free >= minimum_free.
4. Acquire the per-volume lock.
5. Remeasure after acquiring the lock.
6. Calculate deficit = target_free - current_free.
7. Clean terminal_janitor's expired state.
8. Remeasure; stop at target.
9. Run pnpm store prune when cooldown permits.
10. Remeasure; stop at target.
11. Find fully proven inactive registered pnpm workspaces.
12. Sort oldest proven activity first with stable tie-breaking.
13. Revalidate one workspace.
14. Run pnpm pm clean directly without a shell.
15. Remeasure.
16. Permit one immediate pnpm store prune when still below target.
17. Remeasure; stop at target.
18. Repeat for at most one additional workspace.
19. Stop and report shortfall if still below target.
20. Commit the run journal and release the lock.
```

Maximum automatic workspace cleans per run: two.

---

## Workspace eligibility

Every condition must pass:

1. Workspace is inside a user-approved root.
2. Canonical path matches the registered identity.
3. Workspace is on the pressured volume.
4. `package.json` exists.
5. `pnpm-workspace.yaml` exists.
6. `pnpm-lock.yaml` exists.
7. Enrolled pnpm executable exists and identity matches.
8. Pnpm major version is 11 or newer.
9. Git worktree is clean.
10. No active process working directory lies inside the workspace.
11. No active process command references the workspace.
12. Workspace is not explicitly protected.
13. Workspace is not in a recognised cloud-sync root.
14. Workspace has been observed for at least 24 hours.
15. No relevant activity has been observed for at least 30 days.
16. Workspace and pnpm-store cooldowns permit the action.
17. Plan has not expired.
18. Volume lock is held.
19. All proof fields pass.
20. Every unknown result causes a skip.

Tests use injected time. Production contains no bypass for observation or inactivity requirements.

---

# Day 1 — Foundation and contracts

## Objective

Create a compiling cross-platform foundation with correct threshold configuration and read-only status.

## Implement

- Rust package and binary named `terminal_janitor`;
- `clap` CLI foundation;
- platform-appropriate config/state paths;
- human-size parser (`10GiB`, bytes);
- validated config model;
- cross-platform disk-capacity abstraction;
- `status` and `status --json`;
- GitHub Actions matrix: Ubuntu, macOS, Windows;
- documentation precedence and status recording.

Suggested config model:

```rust
struct Config {
    minimum_free_bytes: u64,
    target_free_bytes: u64,
    approved_roots: Vec<PathBuf>,
    check_interval_minutes: u32,
    workspace_min_observed_hours: u32,
    workspace_min_inactive_days: u32,
    store_prune_cooldown_days: u32,
}
```

Defaults:

```text
check interval:             60 minutes
minimum observation:       24 hours
minimum inactivity:        30 days
pnpm store cooldown:        7 days
```

## Tests

- size parsing and overflow;
- invalid threshold ordering;
- missing/corrupt config;
- disk-stat abstraction;
- JSON schema;
- spaces, Unicode, Windows and Unix path serialisation.

## Gate

- `terminal_janitor status` works on all CI operating systems;
- `terminal_janitor status --json` is stable;
- no cleanup code exists;
- format, clippy, and tests pass.

## Commit

```text
chore: establish terminal_janitor foundation
```

---

# Day 2 — State, registration, read-only discovery

## Objective

Build the local ledger and discover only pnpm workspace roots beneath user-approved locations.

## Implement

- bundled SQLite with explicit migrations;
- atomic config writes;
- `init`;
- approved-root registration;
- pnpm workspace discovery;
- canonical path and volume identity;
- project protection commands;
- `scan` read-only foundation;
- state and log caps;
- initial activity observations.

A valid registered workspace requires:

```text
package.json
pnpm-workspace.yaml
pnpm-lock.yaml
```

Record:

```text
first_observed_at
last_observed_at
last_activity_at
last_cleaned_at
protected
```

Conservative activity signals:

- manifests and lockfile modification;
- Git HEAD and index movement;
- Git worktree status;
- terminal_janitor observations.

Do not use filesystem access time as authoritative proof.

## Tests

- discovery cannot escape approved roots;
- symlinked roots fail safely;
- duplicate identities collapse;
- moved workspaces do not inherit stale authority;
- separate mounted volumes stay distinct;
- protection survives rescans;
- corrupt state fails safely and does not silently erase protection.

## Gate

`scan` lists registered workspaces and exact exclusion reasons without mutating projects or invoking pnpm.

## Commit

```text
feat: add safe workspace registration and state
```

---

# Day 3 — Pnpm adapter, proof gates, planning

## Objective

Produce complete, non-executable cleanup plans with mechanical proof for every decision.

## Implement

- pnpm executable enrolment during `init`;
- canonical executable path and version;
- minimum pnpm major version 11;
- typed `AllowedAction`;
- `ProofBundle`;
- ownership, reference, liveness, and protection gates;
- cloud-sync protection;
- Git cleanliness;
- observation and inactivity requirements;
- cooldowns;
- immutable plan ID, expiry, and policy hash;
- `scan --json` explanations;
- `--dry-run`.

Order eligible workspaces by:

```text
oldest proven activity
then oldest first observation
then canonical path
```

Do not build inode accounting or broad filesystem scoring in v0.1.

## Tests

- pnpm below 11 rejected;
- missing lockfile/workspace manifest rejected;
- dirty Git rejected;
- protected and cloud-sync locations rejected;
- new and recent workspaces rejected;
- unknown Git/activity result fails closed;
- identical state produces deterministic plans;
- expired plans cannot execute.

## Gate

Every candidate is either eligible or contains one exact failed gate and reason in JSON. No destructive command can run.

## Commit

```text
feat: add proof-driven pnpm planning
```

---

# Day 4 — Verified executor and threshold loop

## Objective

Execute only immutable, fully proven plans against generated fixtures.

## Implement

- per-volume lock;
- run and action journal;
- direct command execution with exact argument arrays;
- no shell;
- command timeout and bounded output;
- pre-action proof and identity revalidation;
- free-space measurement before/after every action;
- stop-at-target;
- shortfall result;
- two-workspace automatic cap;
- own-state cleanup;
- pnpm-store cooldown and one immediate post-clean exception.

Injectable test interfaces:

```rust
trait DiskProvider
trait ProcessProvider
trait Clock
trait CommandRunner
```

Action states:

```text
PLANNED
VALIDATING
RUNNING
SUCCEEDED
FAILED
SKIPPED_CHANGED
SKIPPED_ACTIVE
SKIPPED_PROTECTED
SKIPPED_UNKNOWN
```

## Tests

- shell never invoked;
- exact executable, args, and working directory;
- `pnpm pm clean` used;
- `pnpm clean`, `--lockfile`, `-l`, and `--force` absent;
- project `clean` scripts cannot execute;
- non-zero exit and timeout stop safely;
- remeasurement after every action;
- target stops later actions;
- shortfall never expands authority;
- concurrency rejected;
- changed target invalidates action;
- interrupted journal remains readable.

## Gate

A pressured simulated volume executes only eligible actions, stops at target, leaves ineligible fixtures untouched, and records a complete journal.

## Commit

```text
feat: add verified autonomous executor
```

---

# Day 5 — Activity protection and native scheduling

## Objective

Complete automatic operation on Linux, macOS, and Windows.

## Implement

Activity detection for processes whose:

- working directory is inside a registered workspace;
- command arguments reference it;
- executable is pnpm or Node operating against it.

Any process-enumeration failure protects the workspace. Revalidate immediately before cleanup.

Implement idempotent:

```text
terminal_janitor enable
terminal_janitor disable
```

Schedulers:

```text
Linux:   systemd user timer
macOS:   user LaunchAgent
Windows: per-user Task Scheduler task
```

Default schedule:

```text
hourly terminal_janitor check
```

No daemon and no administrator privileges.

macOS rule: when Time Machine/APFS capacity creates unresolved ambiguity, skip automatic workspace cleaning and report `SKIPPED_SNAPSHOT_CAPACITY_UNCERTAIN`. Never delete or thin snapshots.

## Tests

- active process protects project;
- process-enumeration failure protects project;
- exact binary path in schedule;
- no shell pipeline;
- enable/disable idempotent;
- upgrade updates safely;
- no elevation;
- no-pressure scheduled run exits quickly;
- macOS uncertainty fails closed.

## Gate

On all three platforms, enable, manually trigger, verify no-op above threshold, verify fixture execution below threshold, and disable cleanly.

## Commit

```text
feat: add activity protection and native scheduling
```

---

# Day 6 — Adversarial hardening

## Objective

Try to break the safety model. Add no product features.

## Filesystem cases

- symlink loops;
- Windows junctions/reparse points;
- moved or replaced workspace after planning;
- mount inside approved root;
- permission holes;
- broken links;
- spaces, Unicode, deep paths;
- case differences on insensitive filesystems.

## Project cases

- dirty and untracked work;
- missing required files;
- recent activity;
- active Node/pnpm processes;
- protected/cloud-sync projects;
- newly observed projects;
- replaced pnpm executable;
- project-defined clean script.

## Execution cases

- concurrent runs;
- killed/timed-out child;
- huge or partial output;
- SQLite failure/corruption;
- disk-stat failure;
- capacity changing during run;
- target reached after first action;
- safe actions exhausted;
- two-workspace cap.

## Non-interference assertion

No write may occur outside:

- terminal_janitor's own state directory;
- the enrolled pnpm process running in the exact verified workspace.

The Rust engine itself must not recursively delete project paths.

## Fuzz/property tests

- config and size parsers;
- path canonicalisation;
- proof-state combinations;
- plan serialisation;
- journal transitions.

## Gate

All mandatory tests pass on Ubuntu, macOS, and Windows. No ignored safety test and no policy weakening.

## Commit

```text
test: harden terminal_janitor safety boundaries
```

---

# Day 7 — Release qualification

## Objective

Ship 0.1.0 only when every safety and autonomy gate passes.

## Implement and verify

- `install.sh`, `install.ps1`, `uninstall.sh`;
- checksummed release artefacts;
- Linux x86_64/aarch64;
- macOS x86_64/aarch64;
- Windows x86_64;
- preserve config on upgrade;
- scheduling remains opt-in through `enable`;
- complete README, safety, acceptance, and status docs;
- at least 1,000 simulated scheduler cycles;
- clean-account install/uninstall smoke tests;
- generated pnpm fixture smoke test on each OS;
- source and lockfile survival checks;
- scheduler enable/disable checks.

Termux compatibility may be advertised only after a real aarch64 smoke test.

## Gate

Every item in `ACCEPTANCE.md` passes. Otherwise do not release.

## Commit

```text
release: terminal_janitor 0.1.0
```

---

## Required checks before every implementation commit

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Run the relevant cross-platform CI before marking a daily gate complete.

---

## Optional work

Only after all mandatory gates pass, an `UvCachePrune` typed adapter may be considered.

It must be compiled, directly invoked without a shell, executable-identity checked, cooldown controlled, and fully tested. Remove it immediately if it threatens the schedule or safety.

Nothing else enters v0.1.

---

## Stop rules

Stop and document rather than improvise when:

- a safety condition cannot be implemented reliably;
- platform behaviour materially diverges;
- pnpm behaviour conflicts with its official contract;
- implementation would require arbitrary deletion;
- non-interference cannot be proven;
- activity detection fails open;
- identity cannot be revalidated;
- scheduling requires elevation;
- macOS available capacity remains ambiguous;
- a test would require weakening policy;
- schedule pressure threatens a release gate.

Cut optional scope first. Never cut proof.
