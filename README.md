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

## Current CLI

```text
terminal_janitor status
terminal_janitor status --json
```

`status` measures the volume containing the current working directory. It reads
at most one small configuration file, calculates `Healthy` or `Pressure`,
prints the result, and exits. It does not scan projects, open a database, invoke
another command, or perform cleanup. Pressure is reported only.

## Threshold configuration

Running `status` never creates or changes configuration. If no configuration
file exists, it uses clearly labelled conservative defaults:

```text
minimum_free = 10 GiB
target_free  = 15 GiB
```

The exact `config.toml` path is resolved by the maintained `directories` crate:

- Linux: `$XDG_CONFIG_HOME/terminal_janitor/config.toml`, or
  `$HOME/.config/terminal_janitor/config.toml` when `XDG_CONFIG_HOME` is unset.
- macOS: `$HOME/Library/Application Support/terminal_janitor/config.toml`.
- Windows:
  `{FOLDERID_RoamingAppData}\terminal_janitor\config\config.toml` (normally
  beneath `%APPDATA%`).

The file format is strict TOML:

```toml
minimum_free = "10GiB"
target_free = "15GiB"
```

Sizes must be unsigned whole numbers followed immediately by one of the
case-sensitive binary units `B`, `KiB`, `MiB`, `GiB`, or `TiB`. Decimal values,
signs, whitespace, unknown units, overflow, zero `minimum_free`, and a
`target_free` less than or equal to `minimum_free` are rejected. `GB` is not
accepted or interpreted as `GiB`. An invalid existing file fails closed with
`FAILED_CONFIGURATION`; it never falls back to defaults.

## Status JSON

`terminal_janitor status --json` writes only a stable JSON object:

```json
{
  "result": "OK_NO_PRESSURE",
  "state": "healthy",
  "total_bytes": 274877906944,
  "used_bytes": 216036854579,
  "available_bytes": 58841052365,
  "minimum_free_bytes": 10737418240,
  "target_free_bytes": 16106127360,
  "config_source": "defaults"
}
```

Pressure uses `PRESSURE_DETECTED`; Day 1 never claims that a target was
restored. Configuration failures exit 2, and storage-measurement failures exit
3. JSON-mode failures remain valid JSON.

## Development loop

Every implementation stage follows:

```text
Define -> implement -> test -> analyse -> improve -> retest -> document -> commit
```

Complexity is introduced only when a demonstrated failure requires it. Optional features are cut before any safety gate is weakened.

## Current state

Day 1's read-only systems, configuration, and status CLI implementation is
present. Cross-platform CI evidence is still required before the complete Day
1 gate can be claimed. See [`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md)
for the exact handover state.
