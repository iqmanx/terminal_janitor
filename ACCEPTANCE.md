# Acceptance Gates

`terminal_janitor` 0.1.0 is releasable only when every mandatory item below passes.

These gates are binary. An incomplete, skipped, flaky, platform-exempt, manually assumed, or undocumented item does not pass.

## A. Product and packaging

- [ ] One Rust executable runs on supported Linux, macOS, and Windows targets.
- [ ] The package and binary are named `terminal_janitor`.
- [ ] No Node.js, Python, Docker, VPS, cloud account, or database server is required by the product.
- [ ] Pnpm is required only when a pnpm action is actually used.
- [ ] Release artefacts have checksums.
- [ ] Install and uninstall work from clean user accounts.
- [ ] Upgrade preserves configuration and protection state.
- [ ] Scheduling is not silently enabled during installation.

## B. Threshold behaviour

- [ ] User can configure `minimum_free` and `target_free`.
- [ ] `minimum_free` must be greater than zero.
- [ ] `target_free` must be greater than `minimum_free`.
- [ ] Above `minimum_free`, `check` exits without project scanning or cleanup.
- [ ] Below `minimum_free`, the engine calculates the deficit against `target_free`.
- [ ] Free capacity is remeasured after acquiring the execution lock.
- [ ] Free capacity is remeasured after every action.
- [ ] Cleaning stops immediately when `target_free` is reached.
- [ ] Estimated bytes are never reported as actual recovered bytes.
- [ ] Safe-action exhaustion produces a shortfall result.
- [ ] Shortfall never causes broader cleanup authority.

## C. Automatic authority

- [ ] Only compiled `AllowedAction` variants may execute automatically.
- [ ] The automatic executor exposes no generic path deletion.
- [ ] The automatic executor exposes no shell or configurable command.
- [ ] AI output, user paths, environment variables, and configuration cannot create destructive authority.
- [ ] Automatic runs clean at most two workspaces.
- [ ] Unknown proof values always cause a skip.
- [ ] A numerical score cannot override a failed proof gate.

## D. Pnpm contract

- [ ] Pnpm major version 11 or newer is required for workspace cleanup.
- [ ] The pnpm executable is enrolled and its identity is revalidated.
- [ ] `pnpm pm clean` is used for workspace cleanup.
- [ ] `pnpm clean` is never used.
- [ ] `--lockfile`, `-l`, and `--force` are never used.
- [ ] Project-defined `clean` scripts cannot execute.
- [ ] `package.json`, `pnpm-workspace.yaml`, and `pnpm-lock.yaml` are required.
- [ ] Lockfiles survive every cleanup test.
- [ ] Pnpm store contents are never deleted directly by Rust code.
- [ ] `pnpm store prune` obeys its normal cooldown.
- [ ] At most one immediate post-workspace-clean store prune is allowed per run.

## E. Ownership and boundaries

- [ ] Only user-approved roots are inspected.
- [ ] Only registered pnpm workspaces are eligible.
- [ ] Canonical path and volume identity are stored.
- [ ] Workspace identity is revalidated immediately before execution.
- [ ] A moved or replaced workspace invalidates its plan.
- [ ] Symlink, junction, reparse-point, and mount-boundary tests pass.
- [ ] Discovery cannot escape an approved root.
- [ ] Unknown directories named `cache`, `build`, `target`, `dist`, or `node_modules` gain no authority.
- [ ] No write occurs outside the product state directory and the exact verified pnpm execution context.

## F. Liveness and protection

- [ ] A workspace must be observed for at least 24 hours before eligibility.
- [ ] A workspace must have at least 30 days of proven inactivity.
- [ ] Production code has no bypass for observation or inactivity.
- [ ] Dirty Git worktrees are skipped.
- [ ] Untracked work causes a skip.
- [ ] Active process working directories cause a skip.
- [ ] Active process command references cause a skip.
- [ ] Process-enumeration failure causes a skip.
- [ ] Liveness is revalidated immediately before cleanup.
- [ ] Explicitly protected projects are skipped.
- [ ] Protection survives rescanning and upgrades.
- [ ] Recognised cloud-sync projects are skipped.
- [ ] Ambiguous cloud-sync detection fails closed.
- [ ] Source, secrets, credentials, databases, application state, personal files, and generic cache roots remain outside automatic authority.

## G. Planning and journalling

- [ ] Every action has a complete `ProofBundle`.
- [ ] Plans have immutable IDs, expiry, and policy/config identity.
- [ ] Identical state produces deterministic plan ordering.
- [ ] Expired plans cannot execute.
- [ ] Every action is journalled before execution.
- [ ] Action states distinguish planned, validating, running, success, failure, and skip reasons.
- [ ] Interrupted runs remain readable.
- [ ] Uncertain interrupted actions are not blindly replayed.
- [ ] History records free-before, free-after, expected bytes, actual bytes, proof, result, and recovery instruction.

## H. Command execution

- [ ] Cleanup commands are invoked directly with argument arrays.
- [ ] Bash, sh, cmd.exe, and PowerShell are not used for cleanup execution.
- [ ] The exact enrolled executable path is used.
- [ ] The exact verified workspace is used as the working directory.
- [ ] Command timeout is enforced.
- [ ] Captured stdout and stderr are bounded.
- [ ] Non-zero exit stops safely.
- [ ] Timeout stops safely.
- [ ] Ambiguous command failure does not continue into broader cleanup.

## I. Concurrency and state

- [ ] Only one run may operate on a volume at once.
- [ ] A second run returns `ALREADY_RUNNING` without destructive work.
- [ ] SQLite migrations are explicit and tested.
- [ ] Corrupt state fails safely rather than silently resetting protection.
- [ ] Atomic configuration writes are used.
- [ ] Database normal maximum is 20 MiB.
- [ ] Logs are capped at 10 MiB.
- [ ] Detailed history is capped at 200 runs.
- [ ] The product does not store project contents, secrets, or command histories.

## J. Scheduling

- [ ] Linux uses a user-level systemd timer.
- [ ] macOS uses a user LaunchAgent.
- [ ] Windows uses a per-user Task Scheduler task.
- [ ] No scheduler requires administrator privileges.
- [ ] Scheduler invokes the exact installed binary path.
- [ ] Scheduler runs `terminal_janitor check` without a shell pipeline.
- [ ] Enable is idempotent.
- [ ] Disable is idempotent.
- [ ] A scheduled no-pressure run exits quickly.
- [ ] Scheduler installation and removal are tested on all supported platforms.

## K. macOS capacity safety

- [ ] macOS capacity handling distinguishes or safely handles snapshot/purgeable ambiguity.
- [ ] Unresolved snapshot capacity uncertainty blocks workspace cleanup.
- [ ] The result is reported as `SKIPPED_SNAPSHOT_CAPACITY_UNCERTAIN`.
- [ ] `terminal_janitor` never deletes or thins Time Machine snapshots.

## L. CLI and explanations

- [ ] All planned CLI commands exist or are explicitly removed from both plan and docs before release.
- [ ] `status` does not perform a project walk.
- [ ] `scan` is read-only.
- [ ] `check` never prompts.
- [ ] `clean` has no broader authority than `check`.
- [ ] `--json` output is valid and stable.
- [ ] Every skipped candidate names its failed gate and reason.
- [ ] Result states are machine-readable.
- [ ] Human output clearly states what was touched and what was protected.

## M. Mandatory test matrix

- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --all-targets` passes.
- [ ] Ubuntu CI passes.
- [ ] macOS CI passes.
- [ ] Windows CI passes.
- [ ] No mandatory safety test is ignored.
- [ ] Symlink-loop test passes.
- [ ] Windows junction/reparse test passes.
- [ ] Mount-boundary test passes.
- [ ] Dirty/untracked project test passes.
- [ ] Active-process test passes.
- [ ] Process-enumeration failure test passes.
- [ ] Replaced-target test passes.
- [ ] Replaced-pnpm test passes.
- [ ] Project-defined clean-script test passes.
- [ ] Concurrent-run test passes.
- [ ] Timeout and killed-child tests pass.
- [ ] Corrupt-state test passes.
- [ ] Target-reached stop test passes.
- [ ] Safe-shortfall test passes.
- [ ] Two-workspace cap test passes.
- [ ] Non-interference test passes.
- [ ] At least 1,000 simulated scheduler cycles pass.
- [ ] Real-machine generated-fixture smoke test passes on Linux, macOS, and Windows.

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
