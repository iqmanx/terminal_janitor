# Implementation Status

## Repository

```text
Repository:     iqmanx/terminal_janitor
Default branch: main
Product:        terminal_janitor
Target version: 0.1.0
Current phase:  Documentation foundation complete; implementation not started
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

Status: **not started**

Required next deliverables:

- initialise Rust package and binary named `terminal_janitor`;
- add the CLI foundation;
- add configuration and size parsing;
- add platform-appropriate directories;
- add cross-platform disk-capacity abstraction;
- implement read-only `status` and `status --json`;
- add Ubuntu, macOS, and Windows CI;
- add tests;
- update this status file with evidence.

Day 1 gate:

- `terminal_janitor status` works on all CI operating systems;
- JSON output is stable;
- no cleanup code exists;
- format, clippy, and tests pass.

### Days 2–7

Status: **not started**

Follow `PLANS.md` strictly. Do not begin a later day until the current day's mandatory gate passes.

## Current repository contents

At this handover point, the repository is intentionally documentation-only.

Expected absent files include:

```text
Cargo.toml
Cargo.lock
src/
tests/
scripts/
.github/workflows/
```

Their absence is not a defect before Day 1 begins.

## Exact next action

An implementation agent should:

1. Read `SAFETY.md`, `ACCEPTANCE.md`, `PLANS.md`, `AGENTS.md`, `VISION.md`, `RESEARCH.md`, and this file.
2. Verify the repository and branch state.
3. Start Day 1 only.
4. Create the smallest compiling Rust foundation needed for the Day 1 gate.
5. Add tests and cross-platform CI with the implementation.
6. Run all required checks.
7. Update this file with actual commands, results, commit SHA, limitations, and the next action.
8. Commit using:

```text
chore: establish terminal_janitor foundation
```

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
