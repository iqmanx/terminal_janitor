# Agent Operating Contract

This file governs any autonomous coding agent working in `terminal_janitor`.

## 1. Read order

Before inspecting or changing code, read these files in order:

1. `SAFETY.md`
2. `ACCEPTANCE.md`
3. `PLANS.md`
4. `AGENTS.md`
5. `VISION.md`
6. `RESEARCH.md`
7. `IMPLEMENTATION_STATUS.md`

Then inspect the repository and verify that `IMPLEMENTATION_STATUS.md` matches reality.

Document precedence is:

```text
SAFETY.md
ACCEPTANCE.md
PLANS.md
AGENTS.md
VISION.md
RESEARCH.md
README.md
```

A lower document may explain a higher document. It may not weaken or contradict it.

## 2. Mission

Build `terminal_janitor` 0.1.0 as a single native Rust Engine + CLI in the approved seven-day sequence.

The product must maintain a user-configured free-storage threshold through fewer cleanup actions with stronger proof.

It is not a universal cleaner.

## 3. Starting rule

Do not begin from assumptions.

At the start of every session:

1. Read the governing documents.
2. Run `git status --short --branch`.
3. Inspect the current tree and recent commits.
4. Run the checks appropriate to the current stage.
5. Compare the result with `IMPLEMENTATION_STATUS.md`.
6. Update stale status documentation before beginning later-stage work.

Preserve unrelated and untracked user files. Never delete, stage, rewrite, or commit them unless the owner explicitly includes them in scope.

## 4. Work loop

Every implementation stage follows:

```text
Define -> implement -> test -> analyse -> improve -> retest -> document -> commit
```

For each day:

1. Restate the day's objective and release gate internally.
2. Implement the smallest design that can satisfy the gate.
3. Add tests with the implementation, not afterwards.
4. Run the complete relevant test set.
5. Analyse failures for root cause.
6. Improve the design without broadening authority.
7. Rerun tests.
8. Update `IMPLEMENTATION_STATUS.md` with evidence.
9. Commit only the files belonging to that completed unit.

Do not start the next day while the current day's mandatory gate is incomplete.

## 5. Safety rules for implementation

The agent must not:

- introduce a generic `DeletePath(PathBuf)` action;
- expose arbitrary shell or command execution;
- invoke Bash, sh, PowerShell, or `cmd.exe` for cleanup;
- allow configuration or model output to create destructive authority;
- weaken `UNKNOWN -> SKIP`;
- bypass observation, inactivity, identity, process, Git, or protection gates;
- delete pnpm stores directly;
- call `pnpm clean`;
- pass `--lockfile`, `-l`, or `--force`;
- automatically clean more than two workspaces per run;
- touch real user projects during destructive tests;
- treat estimated bytes as actual recovery;
- escalate into uncertain data when the target cannot be restored;
- add optional ecosystems before all mandatory v0.1 gates pass.

The automatic action boundary is fixed:

```rust
enum AllowedAction {
    CleanTerminalJanitorState,
    PnpmStorePrune,
    PnpmWorkspaceClean { workspace_id: WorkspaceId },
}
```

Any proposed change to this enum requires explicit owner approval and corresponding updates to `SAFETY.md`, `ACCEPTANCE.md`, and `PLANS.md` before implementation.

## 6. Test isolation

All destructive tests must operate within generated, disposable fixtures.

Use dependency injection for:

```rust
trait DiskProvider;
trait ProcessProvider;
trait Clock;
trait CommandRunner;
```

Use fake executables and controlled temporary directories for command tests.

Never simulate safety by turning off production gates globally. Time-based tests must use an injected clock. Filesystem tests must create the relevant symlink, junction, mount, replacement, permission, and path conditions in isolated fixtures.

## 7. Cross-platform rule

Linux, macOS, and Windows are first-release targets.

Platform behaviour must be behind a narrow platform abstraction. A platform-specific failure must fail closed.

Do not mark a mandatory platform test ignored merely to make CI green. When the implementation cannot meet a platform gate, record the blocker and stop the release.

## 8. Simplicity rule

Begin as one Rust crate.

Do not add:

- crate workspaces;
- plugin systems;
- generic recipe engines;
- remote services;
- UI frameworks;
- MCP or coding-agent wrappers;
- telemetry;
- broad filesystem indexing;
- machine learning;
- direct artefact deletion for unsupported ecosystems.

Complexity is allowed only when a demonstrated correctness, safety, portability, or testing problem requires it.

When behind schedule, cut optional work. Never cut proof.

## 9. Required checks

Before every implementation commit, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Run any additional stage-specific integration and platform tests required by `PLANS.md` and `ACCEPTANCE.md`.

A commit message should match the approved daily unit where possible.

## 10. Documentation and evidence

`IMPLEMENTATION_STATUS.md` is the handover record.

After each completed gate, record:

- date and stage;
- commit SHA;
- files materially changed;
- commands run;
- test results;
- manual evidence;
- remaining limitations;
- exact next action.

Do not claim a test passed when it was not run. Distinguish local, CI, simulated, and real-machine evidence.

Do not mark an acceptance checkbox complete without supporting evidence.

## 11. Stop conditions

Stop implementation and record the blocker when:

- a mandatory safety condition cannot be proved;
- an operating system cannot meet the documented behaviour;
- pnpm behaves differently from the assumed official contract;
- non-interference cannot be tested;
- filesystem or executable identity cannot be revalidated;
- process enumeration would fail open;
- scheduler installation requires elevation;
- macOS capacity ambiguity cannot fail closed;
- a safety test would need weakening to pass;
- arbitrary path deletion appears necessary;
- release qualification remains incomplete at the deadline.

A safe partial product is preferable to an unsafe complete-looking product.

## 12. Completion rule

Do not tag or describe 0.1.0 as released until every applicable item in `ACCEPTANCE.md` has evidence.

The correct final behaviour is:

> Restore the configured target when fully proven actions are available; otherwise stop safely, report the remaining shortfall, and leave uncertain data untouched.
