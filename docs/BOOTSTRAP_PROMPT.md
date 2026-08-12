# Implementation Agent Bootstrap Prompt

Use the prompt below to start an implementation agent in this repository.

---

You are implementing `terminal_janitor` 0.1.0 in `iqmanx/terminal_janitor`.

Your task is to follow the approved seven-day plan exactly and build a native Rust Engine + CLI that maintains a user-configured free-storage threshold through fewer cleanup actions with stronger proof.

## Mandatory first actions

Before changing anything:

1. Read these files in this order:
   - `SAFETY.md`
   - `ACCEPTANCE.md`
   - `PLANS.md`
   - `AGENTS.md`
   - `VISION.md`
   - `RESEARCH.md`
   - `IMPLEMENTATION_STATUS.md`
2. Run `git status --short --branch`.
3. Inspect the repository tree and recent commits.
4. Confirm that `IMPLEMENTATION_STATUS.md` matches the repository.
5. Preserve all unrelated and untracked files.

Do not begin with later-stage features.

## Current task

Start **Day 1 only** from `PLANS.md`.

Deliver:

- a Rust package and binary named `terminal_janitor`;
- one crate, not a workspace;
- CLI foundation;
- validated configuration model;
- human-readable byte-size parsing;
- platform-appropriate configuration and state paths;
- cross-platform disk-capacity abstraction;
- read-only `terminal_janitor status`;
- stable `terminal_janitor status --json`;
- GitHub Actions CI on Ubuntu, macOS, and Windows;
- unit and integration tests required by the Day 1 gate.

Do not add cleanup execution on Day 1.

## Non-negotiable safety boundary

The implementation must obey `SAFETY.md`.

In particular:

- unknown always means skip;
- storage pressure never expands authority;
- no generic `DeletePath(PathBuf)`;
- no arbitrary command or shell execution;
- no configurable destructive recipes;
- no broad cache scanning;
- no MCP, Claude Code, Codex, GUI, daemon, cloud service, telemetry, or extra ecosystem support;
- do not weaken a safety or acceptance condition to meet the deadline.

The only planned automatic action variants for the complete 0.1.0 product are:

```rust
enum AllowedAction {
    CleanTerminalJanitorState,
    PnpmStorePrune,
    PnpmWorkspaceClean { workspace_id: WorkspaceId },
}
```

Do not implement those actions during Day 1 unless a non-executable type is genuinely required for compilation. No destructive path is permitted yet.

## Development method

Follow:

```text
Define -> implement -> test -> analyse -> improve -> retest -> document -> commit
```

Use the smallest design that satisfies the current gate.

Add tests with implementation.

Before committing, run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Run the CI matrix and record the result when available.

## Day 1 completion gate

Day 1 is complete only when:

- `terminal_janitor status` works on Linux, macOS, and Windows CI;
- `terminal_janitor status --json` has deterministic valid output;
- configuration validation rejects invalid threshold ordering;
- path and size parsing tests pass;
- no cleanup implementation exists;
- formatting, clippy, and all tests pass;
- `IMPLEMENTATION_STATUS.md` contains evidence and the exact next action.

If the gate does not pass, do not start Day 2.

## Commit

When the Day 1 gate passes, commit the intended files with:

```text
chore: establish terminal_janitor foundation
```

Do not push incomplete or unrelated changes as a completed Day 1 result.

## Required final report

Report:

- current branch and commit;
- files changed;
- architecture implemented;
- exact commands run;
- test and CI results;
- Day 1 acceptance evidence;
- known limitations;
- whether the Day 1 gate passed;
- exact next action.

If blocked, state the blocker precisely and stop rather than bypassing a governing rule.

---

For later sessions, replace “Day 1” with the exact current day recorded in `IMPLEMENTATION_STATUS.md`, but retain the same read order, safety boundary, evidence requirements, and stop rules.
