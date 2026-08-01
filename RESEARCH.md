# Research Reference

This document preserves the research and design reasoning behind `terminal_janitor`.

It is not an implementation plan. `SAFETY.md`, `ACCEPTANCE.md`, and `PLANS.md` take precedence.

Research ideas marked **deferred** must not enter 0.1.0 unless the owner explicitly changes the governing documents first.

## 1. Problem

Developer machines accumulate storage across package stores, dependency installations, compiler caches, build outputs, test caches, logs, temporary downloads, SDKs, container data, and application state.

Existing tools usually do one of four things:

1. show where storage is used;
2. clean one ecosystem;
3. ask the user to delete directories manually;
4. remove broad cache locations without proving ownership or downstream dependencies.

The missing product is a local coordinator that protects one device-level storage reserve while respecting each supported tool's ownership and recovery contract.

The first release deliberately solves only a narrow, high-proof subset.

## 2. Prior art

### Interactive scanners

Tools such as `npkill`, `kondo`, `ncdu`, `gdu`, `dua`, and similar utilities help find large directories or selected build artefacts.

Useful lesson:

- discovery and display can be fast and convenient;
- size alone is not permission to delete;
- interactive scanning does not provide unattended threshold governance.

### Per-ecosystem garbage collectors

Mature developer tools often understand their own stores better than a generic cleaner can.

Relevant examples include:

- pnpm store pruning and workspace cleaning;
- Cargo cache tracking and garbage collection;
- uv cache pruning;
- Go cache cleaning;
- Docker BuildKit garbage-collection policies;
- Nix reachability-based garbage collection;
- ccache size limits;
- journald and temporary-file retention policies.

Useful lesson:

> Prefer owner-controlled pruning and native budgets over guessed recursive deletion.

### Operating-system automation

Linux systemd user timers, macOS LaunchAgents, and Windows Task Scheduler can run a short-lived user process periodically.

Useful lesson:

- a permanent daemon is unnecessary for the first product;
- installation can remain local and require no VPS;
- scheduled execution must use the exact binary path and fail closed.

### OS storage cleaners

Windows Storage Sense and macOS storage-management features demonstrate threshold or capacity-aware cleanup, but they do not understand developer workspaces and cross-tool recovery contracts.

Useful lesson:

- global storage pressure is a valid trigger;
- platform-managed or purgeable capacity can make raw free-space measurements ambiguous;
- the product must not duplicate or fight operating-system storage management.

## 3. Why the original threshold script is insufficient

A script such as:

```bash
rm -rf ~/.cache/*
pnpm store prune
apt clean
```

is unsuitable for unattended product behaviour because it lacks:

- ownership proof;
- reference safety;
- active-project detection;
- protection boundaries;
- target identity revalidation;
- threshold hysteresis;
- concurrency control;
- execution journalling;
- actual post-action capacity measurement;
- cross-platform scheduling;
- safe shortfall behaviour.

Generic cache roots can contain information that other applications or workflows still depend upon.

A directory name is a discovery hint, not deletion authority.

## 4. Core research conclusions

### 4.1 Less deletion, stronger proof

The strongest first product is not the one that claims the most reclaimable space. It is the one with the smallest destructive authority that can still maintain useful storage headroom.

### 4.2 Unknown means protected

Missing process data, unclear ownership, changed paths, uncertain cloud-sync status, ambiguous filesystem identity, and unclear recovery all result in a skip.

### 4.3 Shared stores are delegate-only

Content-addressed and cross-project stores may use hard links, references, deduplication, or internal metadata.

Directly deleting their files can:

- recover less space than expected;
- break another project;
- corrupt the owning tool's state;
- behave differently across filesystems and operating systems.

The owner tool must perform the pruning.

For 0.1.0, Rust code never directly deletes the pnpm store.

### 4.4 Apparent size is not actual recovery

Directory size can differ from actual recovered capacity because of hard links, sparse files, compression, copy-on-write, snapshots, and filesystem accounting.

The first release avoids complicated cross-platform reclaim estimation.

It treats estimates as advisory and measures the volume after every action. The measured free-space change is authoritative.

### 4.5 Filesystem access time is weak evidence

Access time is often disabled, delayed, or modified by filesystem policies.

The product therefore keeps a small observation ledger and combines:

- first and last observation;
- relevant manifest and lockfile changes;
- Git state;
- process activity;
- explicit protection;
- previous cleanup history.

### 4.6 Trash does not solve threshold pressure

Moving generated data to Trash or Recycle Bin usually keeps it on the same volume and therefore does not restore the desired reserve.

Automatic cleanup must rely on regenerability and recorded recovery instructions rather than pretending permanent removal is reversible.

### 4.7 Storage pressure must not expand authority

A nearly full device must never cause the engine to lower its proof standard or begin touching unknown data.

When the approved actions are insufficient, a shortfall report is the correct result.

## 5. Why Rust is the core language

The product performs cross-platform filesystem inspection, process checks, direct command execution, state management, locking, and potentially destructive operations.

Rust is selected for:

- native standalone binaries;
- no Node.js or Python runtime requirement;
- strong memory and concurrency safety;
- precise platform abstractions;
- direct argument-array process execution;
- suitability for long-lived system utilities;
- one implementation shared by CLI and future integrations.

TypeScript may later be used for optional SDKs, editor integrations, or dashboards, but it is not the authoritative cleanup engine.

Go remains a reasonable alternative in the abstract, but the language decision is locked for this repository.

## 6. Why pnpm is the first ecosystem

Pnpm provides owner-controlled operations that allow `terminal_janitor` to remain narrow:

```text
pnpm store prune
pnpm pm clean
```

The explicit `pm` form prevents a project-defined `clean` script from being selected instead of pnpm's built-in command.

The workspace lockfile remains the recovery proof and must never be removed.

The seven-day product therefore targets developers using pnpm workspaces rather than attempting broad ecosystem coverage.

## 7. Fixed 0.1.0 model

```text
One Rust crate
One native binary
Engine + CLI
Small SQLite metadata ledger
Native user scheduler
Compiled actions only
Pnpm-first
No generic delete primitive
```

Automatic actions are limited to:

```rust
enum AllowedAction {
    CleanTerminalJanitorState,
    PnpmStorePrune,
    PnpmWorkspaceClean { workspace_id: WorkspaceId },
}
```

## 8. Deferred architecture

The following ideas may be useful after 0.1.0 proves its safety model.

They are not authorised for the first release.

### Additional owner-controlled adapters

Possible future adapters:

- uv cache pruning;
- Cargo-native cache GC;
- Go cache operations;
- Docker BuildKit GC;
- ccache and sccache budgets;
- Gradle- or Maven-owned cleanup;
- platform-specific build caches.

Each requires its own owner, reference, liveness, protection, recovery, and platform tests.

### Native budget broker

A future version may configure supported tools not to grow beyond a budget, reducing the need for reactive cleanup.

The preferred order would be:

```text
1. prevention through native limits;
2. owner-controlled garbage collection;
3. narrowly proven project artefact reclamation;
4. safe shortfall reporting.
```

### MCP, Claude Code, and Codex

Future agent integrations should be thin interfaces over the tested engine and stable JSON output.

They may expose operations such as:

```text
status
scan
explain
check
protect
history
```

They must never expose arbitrary path deletion or shell execution.

### Broader restore support

A future release may record or assist with deterministic recovery commands.

Automatic restoration is deferred because it may download data, execute lifecycle scripts, consume large storage, or alter active projects.

### Richer activity tracking

Future versions may integrate opt-in editor or shell session leases. These should strengthen protection, not create deletion permission by themselves.

### More advanced capacity modelling

Future versions may improve:

- hard-link and allocated-block accounting;
- filesystem compression awareness;
- APFS purgeable-capacity handling;
- copy-on-write and snapshot awareness;
- prediction of regeneration cost.

These features must not replace actual post-action capacity measurement.

## 9. Rejected for the first product

The following were considered and rejected for 0.1.0:

- twenty or more ecosystem recipes;
- user-overridable destructive recipes;
- direct generic removal of `node_modules`, `target`, `.venv`, `dist`, or `build`;
- generic cleaning of cache roots;
- arbitrary plugin commands;
- Docker image or volume deletion;
- duplicate personal-file cleaning;
- automatic Time Machine snapshot thinning;
- Trash as the automatic recovery mechanism;
- AI-selected filesystem actions;
- a permanent daemon;
- cloud coordination;
- a GUI.

They would either violate the seven-day constraint, weaken proof, or increase interference risk.

## 10. Source starting points

Use official documentation and primary code where possible when implementation details need verification:

- pnpm CLI documentation: `https://pnpm.io/cli`
- Cargo cache-cleaning design: `https://blog.rust-lang.org/2023/12/11/cargo-cache-cleaning/`
- Docker BuildKit GC: `https://docs.docker.com/build/cache/garbage-collection/`
- uv cache documentation: `https://docs.astral.sh/uv/concepts/cache/`
- systemd timers: `https://www.freedesktop.org/software/systemd/man/latest/systemd.timer.html`
- Apple LaunchAgent documentation and filesystem guidance: `https://developer.apple.com/documentation/`
- Windows Task Scheduler and storage APIs: `https://learn.microsoft.com/windows/`
- Model Context Protocol: `https://modelcontextprotocol.io/`

Before relying on a current command contract or library version, verify it against the official source.

## 11. Research decision

The feasibility conclusion is:

> A safe autonomous product is feasible in seven days only when its destructive authority is deliberately narrow, owner-controlled, mechanically proven, cross-platform tested, and allowed to report shortfall.

The repository plan embodies that conclusion. Broad cleanup remains future work.
