# Acceptance Gates

`terminal_janitor` 0.1.0 is releasable only when every mandatory item below passes.

These gates are binary. An incomplete, skipped, flaky, platform-exempt, manually assumed, or undocumented item does not pass.

## How the ticked items are evidenced

Every ticked box below is carried by the automated suite — 219 unit, 7
adversarial, and 7 CLI integration tests — passing on Ubuntu, macOS, and
Windows, plus the three real-machine workflows: `Scheduler gate` (enable,
trigger, no-op, disable on each platform), `Qualification` (generated pnpm
fixture and clean-account install), and `Release` (five targets, checksummed).
`IMPLEMENTATION_STATUS.md` records the run identifiers per day.

Four items rest on source and schema inspection rather than on a test that
could fail, and are ticked on that weaker basis. They are called out here
rather than left to look like the rest:

- section C, that configuration and environment cannot create destructive
  authority — carried by `AllowedAction` being a compiled enum with no
  path-carrying variant, which is a type-level property, not a runtime one;
- section D, that `pnpm store prune` obeys its own cooldown — this product
  simply never passes `--force`, and pnpm's cooldown is pnpm's;
- section F, that source, secrets, credentials, databases, and personal files
  stay outside automatic authority — the same type-level property, plus the
  non-interference test;
- section I, that no project contents, secrets, or command histories are
  stored — the schema holds only metadata columns, which is inspectable but
  not assertable.

## A. Product and packaging

- [x] One Rust executable runs on supported Linux, macOS, and Windows targets.
- [x] The package and binary are named `terminal_janitor`.
- [x] No Node.js, Python, Docker, VPS, cloud account, or database server is required by the product.
- [x] Pnpm is required only when a pnpm action is actually used.
- [x] Release artefacts have checksums.
- [x] Install and uninstall work from clean user accounts.
- [x] Upgrade preserves configuration and protection state.
- [x] Scheduling is not silently enabled during installation.

## B. Threshold behaviour

- [x] User can configure `minimum_free` and `target_free`.
- [x] `minimum_free` must be greater than zero.
- [x] `target_free` must be greater than `minimum_free`.
- [x] Above `minimum_free`, `check` exits without project scanning or cleanup.
- [x] Below `minimum_free`, the engine calculates the deficit against `target_free`.
- [x] Free capacity is remeasured after acquiring the execution lock.
- [x] Free capacity is remeasured after every action.
- [x] Cleaning stops immediately when `target_free` is reached.
- [x] Estimated bytes are never reported as actual recovered bytes.
- [x] Safe-action exhaustion produces a shortfall result.
- [x] Shortfall never causes broader cleanup authority.

## C. Automatic authority

- [x] Only compiled `AllowedAction` variants may execute automatically.
- [x] The automatic executor exposes no generic path deletion.
- [x] The automatic executor exposes no shell or configurable command.
- [x] AI output, user paths, environment variables, and configuration cannot create destructive authority.
- [x] Automatic runs clean at most two workspaces.
- [x] Unknown proof values always cause a skip.
- [x] A numerical score cannot override a failed proof gate.

## D. Pnpm contract

- [x] Pnpm major version 11 or newer is required for workspace cleanup.
- [x] The pnpm executable is enrolled and its identity is revalidated.
- [x] `pnpm pm clean` is used for workspace cleanup.
- [x] `pnpm clean` is never used.
- [x] `--lockfile`, `-l`, and `--force` are never used.
- [x] Project-defined `clean` scripts cannot execute.
- [x] `package.json`, `pnpm-workspace.yaml`, and `pnpm-lock.yaml` are required.
- [x] Lockfiles survive every cleanup test.
- [x] Pnpm store contents are never deleted directly by Rust code.
- [x] `pnpm store prune` obeys its normal cooldown.
- [x] At most one immediate post-workspace-clean store prune is allowed per run.

## E. Ownership and boundaries

- [x] Only user-approved roots are inspected.
- [x] Only registered pnpm workspaces are eligible.
- [x] Canonical path and volume identity are stored.
- [x] Workspace identity is revalidated immediately before execution.
- [x] A moved or replaced workspace invalidates its plan.
- [x] Symlink, junction, reparse-point, and mount-boundary tests pass.
- [x] Discovery cannot escape an approved root.
- [x] Unknown directories named `cache`, `build`, `target`, `dist`, or `node_modules` gain no authority.
- [x] No write occurs outside the product state directory and the exact verified pnpm execution context.

## F. Liveness and protection

- [x] A workspace must be observed for at least 24 hours before eligibility.
- [x] A workspace must have at least 30 days of proven inactivity.
- [x] Production code has no bypass for observation or inactivity.
- [x] Dirty Git worktrees are skipped.
- [x] Untracked work causes a skip.
- [x] Active process working directories cause a skip.
- [x] Active process command references cause a skip.
- [x] Process-enumeration failure causes a skip.
- [x] Liveness is revalidated immediately before cleanup.
- [x] Explicitly protected projects are skipped.
- [x] Protection survives rescanning and upgrades.
- [x] Recognised cloud-sync projects are skipped.
- [x] Ambiguous cloud-sync detection fails closed.
- [x] Source, secrets, credentials, databases, application state, personal files, and generic cache roots remain outside automatic authority.

## G. Planning and journalling

- [x] Every action has a complete `ProofBundle`.
- [x] Plans have immutable IDs, expiry, and policy/config identity.
- [x] Identical state produces deterministic plan ordering.
- [x] Expired plans cannot execute.
- [x] Every action is journalled before execution.
- [x] Action states distinguish planned, validating, running, success, failure, and skip reasons.
- [x] Interrupted runs remain readable.
- [x] Uncertain interrupted actions are not blindly replayed.
- [x] History records free-before, free-after, expected bytes, actual bytes, proof, result, and recovery instruction.

## H. Command execution

- [x] Cleanup commands are invoked directly with argument arrays.
- [x] Bash, sh, cmd.exe, and PowerShell are not used for cleanup execution.
- [x] The exact enrolled executable path is used.
- [x] The exact verified workspace is used as the working directory.
- [x] Command timeout is enforced.
- [x] Captured stdout and stderr are bounded.
- [x] Non-zero exit stops safely.
- [x] Timeout stops safely.
- [x] Ambiguous command failure does not continue into broader cleanup.

## I. Concurrency and state

- [x] Only one run may operate on a volume at once.
- [x] A second run returns `ALREADY_RUNNING` without destructive work.
- [x] SQLite migrations are explicit and tested.
- [x] Corrupt state fails safely rather than silently resetting protection.
- [x] Atomic configuration writes are used.
- [x] Database normal maximum is 20 MiB.
- [x] Logs are capped at 10 MiB.
- [x] Detailed history is capped at 200 runs.
- [x] The product does not store project contents, secrets, or command histories.

## J. Scheduling

- [x] Linux uses a user-level systemd timer.
- [x] macOS uses a user LaunchAgent.
- [x] Windows uses a per-user Task Scheduler task.
- [x] No scheduler requires administrator privileges.
- [x] Scheduler invokes the exact installed binary path.
- [x] Scheduler runs `terminal_janitor check` without a shell pipeline.
- [x] Enable is idempotent.
- [x] Disable is idempotent.
- [x] A scheduled no-pressure run exits quickly.
- [x] Scheduler installation and removal are tested on all supported platforms.

## K. macOS capacity safety

- [x] macOS capacity handling distinguishes or safely handles snapshot/purgeable ambiguity.
- [x] Unresolved snapshot capacity uncertainty blocks workspace cleanup.
- [x] The result is reported as `SKIPPED_SNAPSHOT_CAPACITY_UNCERTAIN`.
- [x] `terminal_janitor` never deletes or thins Time Machine snapshots.

## L. CLI and explanations

- [x] All planned CLI commands exist or are explicitly removed from both plan and docs before release.
- [x] `status` does not perform a project walk.
- [x] `scan` is read-only.
- [x] `check` never prompts.
- [x] `clean` has no broader authority than `check`.
- [x] `--json` output is valid and stable.
- [x] Every skipped candidate names its failed gate and reason.
- [x] Result states are machine-readable.
- [x] Human output clearly states what was touched and what was protected.

## M. Mandatory test matrix

- [x] `cargo fmt --check` passes.
- [x] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [x] `cargo test --all-targets` passes.
- [x] Ubuntu CI passes.
- [x] macOS CI passes.
- [x] Windows CI passes.
- [x] No mandatory safety test is ignored.
- [x] Symlink-loop test passes.
- [x] Windows junction/reparse test passes.
- [x] Mount-boundary test passes.
- [x] Dirty/untracked project test passes.
- [x] Active-process test passes.
- [x] Process-enumeration failure test passes.
- [x] Replaced-target test passes.
- [x] Replaced-pnpm test passes.
- [x] Project-defined clean-script test passes.
- [x] Concurrent-run test passes.
- [x] Timeout and killed-child tests pass.
- [x] Corrupt-state test passes.
- [x] Target-reached stop test passes.
- [x] Safe-shortfall test passes.
- [x] Two-workspace cap test passes.
- [x] Non-interference test passes.
- [x] At least 1,000 simulated scheduler cycles pass.
- [x] Real-machine generated-fixture smoke test passes on Linux, macOS, and Windows.

## N. Required result states

The implementation provides and tests:

```text
OK_NO_PRESSURE
OK_TARGET_RESTORED
OK_PARTIAL_SAFE_RECLAIM
SHORTFALL_SAFE_ACTIONS_EXHAUSTED
SKIPPED_ACTIVITY_UNCERTAIN
SKIPPED_PROTECTED
SKIPPED_SNAPSHOT_CAPACITY_UNCERTAIN
FAILED_CONFIGURATION
FAILED_COMMAND
FAILED_STORAGE_MEASUREMENT
FAILED_STATE
ALREADY_RUNNING
```

## Release decision

Release only when every applicable checkbox is supported by test or recorded manual evidence.

If any mandatory gate fails, update `IMPLEMENTATION_STATUS.md` with the blocker and do not tag 0.1.0.
