# terminal_janitor

`terminal_janitor` is a small native terminal storage governor for developer machines.

Its first product goal is deliberately narrow:

> Maintain a user-configured minimum amount of free storage by performing fewer cleanup actions with stronger proof, and stop safely when proof is insufficient.

The v0.1 product is a single Rust binary containing the Engine and CLI. It runs on Linux, macOS, and Windows, uses the operating system's native user scheduler, and requires no VPS, account, cloud database, GUI, MCP server, or permanent daemon.

## Locked seven-day scope

The first release supports:

- configurable `minimum_free` and `target_free` thresholds;
- a fast no-pressure exit;
- read-only discovery beneath user-approved project roots;
- a small local activity and execution ledger;
- mechanical ownership, reference, liveness, and protection gates;
- typed, compiled cleanup actions only;
- owner-controlled pnpm cleanup through `pnpm pm clean` and `pnpm store prune`;
- automatic scheduling without administrator privileges;
- post-action storage measurement and immediate stop at the target;
- fail-closed shortfall reporting when safe actions are exhausted;
- human-readable and JSON output.

The first release does **not** support generic cache cleaning, arbitrary path deletion, user-defined cleanup recipes, Docker cleanup, Cargo cleanup, Python virtual-environment removal, Downloads cleaning, GUI, MCP, Claude Code, Codex, cloud services, or AI-directed deletion.

## Core invariant

> Storage pressure changes urgency. It never expands what `terminal_janitor` is authorised to touch.

Unknown ownership, uncertain activity, unverified recoverability, shared data, active work, protected paths, and ambiguous platform state always result in a skip.

## Governing documents

Read these before implementing:

1. [`VISION.md`](VISION.md) — product purpose and boundaries.
2. [`PLANS.md`](PLANS.md) — the approved seven-day execution plan.
3. [`SAFETY.md`](SAFETY.md) — non-negotiable safety contract.
4. [`ACCEPTANCE.md`](ACCEPTANCE.md) — mandatory release gates.
5. [`AGENTS.md`](AGENTS.md) — instructions for autonomous coding agents.
6. [`RESEARCH.md`](RESEARCH.md) — prior art, feasibility findings, and deferred architecture.
7. [`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md) — current repository state and next action.
8. [`BOOTSTRAP_PROMPT.md`](BOOTSTRAP_PROMPT.md) — the exact starting prompt for an implementation agent.

When documents appear to conflict, use this precedence:

```text
SAFETY.md
ACCEPTANCE.md
PLANS.md
AGENTS.md
VISION.md
RESEARCH.md
README.md
```

No document may weaken `SAFETY.md` or `ACCEPTANCE.md`.

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

Global output options:

```text
--json
--dry-run
--verbose
```

## Development loop

Every implementation stage follows:

```text
Define -> implement -> test -> analyse -> improve -> retest -> document -> commit
```

Complexity is introduced only when a demonstrated failure requires it. Optional features are cut before any safety gate is weakened.

## Current state

The repository contains the approved product and implementation documents. No implementation has been completed yet. See [`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md) for the exact next action.
