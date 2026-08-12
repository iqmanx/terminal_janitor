# Vision

## Product

`terminal_janitor` is a native terminal storage governor that protects a user-defined amount of free disk space on developer machines.

It is not a universal cleaner. It is a deliberately conservative janitor that performs only narrowly authorised cleanup actions with strong proof of safety.

## User promise

> Set the amount of free space you want to preserve. `terminal_janitor` will attempt only proven, regenerable cleanup actions, stop when the target is restored, and refuse to touch uncertain data.

## Why it exists

Developer storage fills gradually across dependency installations, package stores, build artefacts, test caches, temporary outputs, and tool-owned data. Existing tools usually solve only one ecosystem, require manual invocation, or delete broad directories without proving ownership, liveness, recoverability, or cross-application dependencies.

`terminal_janitor` coordinates a small number of trusted cleanup operations against one device-level storage threshold.

## v0.1 product shape

```text
One Rust binary
├── Engine
├── CLI
├── small local ledger
└── native OS scheduler integration
```

No permanent daemon is required. The same binary runs briefly through systemd user timers, macOS LaunchAgents, or Windows Task Scheduler and exits.

## v0.1 target user

A developer using pnpm workspaces on Linux, macOS, or Windows who wants predictable storage headroom without manually searching for old dependency installations.

## Fixed operating model

The user configures:

- `minimum_free`: the trigger point;
- `target_free`: the recovery point;
- approved project roots;
- protected projects;
- whether native scheduling is enabled.

Behaviour:

```text
free >= minimum_free
    -> exit immediately

free < minimum_free
    -> attempt approved cleanup actions in strict order

free >= target_free
    -> stop immediately

safe actions exhausted
    -> report shortfall and stop
```

`target_free` must be greater than `minimum_free` so the tool does not repeatedly clean around one boundary.

## Core principles

### Less deletion, stronger proof

The product is judged by the safety and correctness of what it does, not by the number of gigabytes it claims it can delete.

### Unknown means protected

Uncertain ownership, uncertain process state, uncertain recoverability, shared application data, ambiguous platform capacity, and changed filesystem identity always result in a skip.

### Owner-controlled cleanup first

When the owning ecosystem provides a supported cleanup operation, `terminal_janitor` delegates to it instead of deleting its internal files directly.

### Storage pressure never expands authority

A nearly full disk may increase urgency. It cannot lower the proof standard or unlock broader deletion.

### Measured recovery

Estimated size is advisory. The engine remeasures actual free capacity after each action and stops when the configured target is reached.

### No interference

The product must not interfere with active projects, personal information, application state, shared stores, synchronised folders, credentials, databases, source code, or data another tool may still depend upon.

### Simplicity over architecture

The first complete product is Engine + CLI. MCP, Claude Code, Codex, GUI, plugin systems, cloud services, and broad ecosystem coverage are deferred.

## v0.1 automatic authority

Only compiled, typed actions may execute automatically:

```rust
enum AllowedAction {
    CleanTerminalJanitorState,
    PnpmStorePrune,
    PnpmWorkspaceClean { workspace_id: WorkspaceId },
}
```

There is no generic automatic `DeletePath`, configurable shell command, recursive pattern remover, or agent-selected filesystem target.

## What success means

The product succeeds when it:

- remains fast and invisible when storage is healthy;
- restores the configured reserve when fully proven cleanup is available;
- leaves active and uncertain data untouched;
- produces deterministic, understandable decisions;
- measures actual results;
- survives interruption safely;
- runs repeatedly without unbounded state growth;
- reports honestly when it cannot safely do enough.

Missing a questionable cleanup opportunity is acceptable.

Damaging important information is not.
