# Safety Contract

This document is the highest-authority repository document.

No implementation, configuration, optimisation, agent instruction, deadline, test shortcut, or future feature may weaken it.

## 1. Governing invariant

> Storage pressure changes urgency. It never expands what `terminal_janitor` is authorised to touch.

When safe actions cannot restore the configured threshold, the correct result is a shortfall report.

## 2. Fail-closed rule

Every unavailable, failed, inconsistent, ambiguous, or unknown safety check means:

```text
UNKNOWN -> SKIP
```

The engine must never infer permission from missing evidence.

## 3. Automatic action boundary

The automatic executor may accept only compiled, typed actions:

```rust
enum AllowedAction {
    CleanTerminalJanitorState,
    PnpmStorePrune,
    PnpmWorkspaceClean { workspace_id: WorkspaceId },
}
```

The following automatic interfaces are prohibited:

```rust
DeletePath(PathBuf)
RunShell(String)
RunConfiguredCommand(String)
RemoveMatchingGlob(String)
```

No user configuration, plugin, environment variable, model output, or CLI argument may create a new destructive action.

## 4. Mandatory proof bundle

Every automatic action must produce and pass:

```rust
struct ProofBundle {
    action_allowlisted: bool,
    owner_proven: bool,
    regenerability_proven: bool,
    reference_safety_proven: bool,
    inactivity_proven: bool,
    protection_checks_passed: bool,
    executable_identity_verified: bool,
    target_identity_verified: bool,
}
```

All fields must be true. Scoring, disk pressure, age, size, or user convenience cannot compensate for a failed field.

## 5. Four gates

### G1: Ownership

The engine must prove that the candidate belongs to an explicitly supported tool or registered workspace.

A name such as `cache`, `temp`, `build`, `target`, `dist`, or `node_modules` is never sufficient proof by itself.

### G2: Reference safety

The engine must not remove bytes another consumer may still reference.

Shared stores are delegate-only. The owning ecosystem's supported garbage collector must perform the operation.

For v0.1, pnpm's store is never deleted directly.

### G3: Liveness

The engine must prove the workspace is inactive.

A workspace is skipped when:

- a process has a working directory inside it;
- a process command references it;
- pnpm or Node is operating against it;
- relevant activity is recent;
- process enumeration fails;
- liveness information is otherwise uncertain.

Liveness is revalidated immediately before execution.

### G4: Protection

The engine must not operate on:

- dirty Git worktrees;
- protected projects;
- cloud-synchronised projects;
- source or personal data;
- credentials or secrets;
- databases;
- application state;
- unknown paths;
- paths outside approved roots;
- projects without required recovery evidence.

## 6. Permanent automatic denylist

The following remain outside automatic authority:

- source code;
- `.git` directories;
- uncommitted or untracked work;
- `.env*` files;
- SSH, GPG, keychain, token, credential, and password data;
- databases and container volumes;
- personal documents, Downloads, Desktop, photos, videos, and backups;
- generic `~/.cache`, `~/Library/Caches`, Application Support, `%LOCALAPPDATA%`, `%APPDATA%`, and `%PROGRAMDATA%` roots;
- local models, SDKs, toolchains, emulator images, and archives;
- arbitrary paths supplied by an AI agent or user;
- anything whose ownership or recovery contract is uncertain.

Inspection of an approved, narrow child does not grant permission over its parent or siblings.

## 7. Pnpm execution rules

Workspace cleanup must execute:

```text
pnpm pm clean
```

It must never execute:

```text
pnpm clean
```

It must never pass:

```text
--lockfile
-l
--force
```

Requirements:

- pnpm major version 11 or newer;
- `package.json`, `pnpm-workspace.yaml`, and `pnpm-lock.yaml` exist;
- the enrolled pnpm executable identity is verified;
- the command is invoked directly with an argument array;
- no shell is involved;
- the exact verified workspace is the working directory;
- non-zero exit, timeout, or ambiguous output stops the run safely.

## 8. Filesystem identity and boundaries

Before execution, the engine must revalidate:

- canonical workspace path;
- volume identity;
- target identity;
- approved-root containment;
- symlink, junction, and reparse-point state;
- mount boundary;
- plan expiry;
- policy version;
- current activity and protection state.

A changed or moved target invalidates the action.

The engine must not cross a volume, mount, junction, symlink, or reparse boundary unintentionally.

## 9. Threshold execution

The engine must:

1. measure free space;
2. exit above `minimum_free`;
3. acquire a per-volume lock;
4. remeasure after the lock;
5. execute one fully proven action at a time;
6. remeasure actual free space after every action;
7. stop immediately at `target_free`;
8. execute no more than two workspace cleans per automatic run;
9. report shortfall when authorised actions are exhausted.

Estimated bytes may guide display, but only measured free-space change is authoritative.

## 10. Concurrency and journalling

Only one run may operate on a volume at once.

Every action must be journalled before execution and transition through explicit states:

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

A crash or interruption must not cause unfinished work to be marked successful or replayed blindly.

## 11. Scheduling

Automatic scheduling must:

- run as the current user;
- require no administrator privileges;
- use systemd user timers, macOS LaunchAgents, or Windows Task Scheduler;
- invoke the exact installed binary path;
- invoke `terminal_janitor check` without a shell pipeline;
- be idempotently installable and removable.

## 12. State minimisation

The ledger stores metadata only. It must not store project contents, command histories, secrets, or arbitrary internal filenames.

Limits:

```text
Database: 20 MiB normal maximum
Logs:     10 MiB maximum
Runs:     200 detailed records maximum
```

The product must govern its own storage.

## 13. Agent and deadline rule

An implementation agent must remove optional scope before weakening this safety contract.

A seven-day deadline is not authority to bypass a gate.

If a mandatory safety condition cannot be implemented and tested, the release must stop.
