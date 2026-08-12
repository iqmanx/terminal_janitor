# terminal_janitor

`terminal_janitor` is a small native terminal storage governor for developer machines.

Its first product goal is deliberately narrow:

> Maintain a user-configured minimum amount of free storage by performing fewer cleanup actions with stronger proof, and stop safely when proof is insufficient.

The v0.1 product is a single Rust binary containing the Engine and CLI. It runs on Linux, macOS, and Windows, uses the operating system's native user scheduler, and requires no VPS, account, cloud database, GUI, MCP server, or permanent daemon.

## Scope

`terminal_janitor` supports:

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

It does **not** support generic cache cleaning, arbitrary path deletion, user-defined cleanup recipes, Docker cleanup, Cargo cleanup, Python virtual-environment removal, Downloads cleaning, GUI, MCP, Claude Code, Codex, cloud services, or AI-directed deletion.

## Core invariant

> Storage pressure changes urgency. It never expands what `terminal_janitor` is authorised to touch.

Unknown ownership, uncertain activity, unverified recoverability, shared data, active work, protected paths, and ambiguous platform state always result in a skip.

## Installing

Everything below happens inside your own account. Nothing asks for
administrator rights, and nothing is scheduled until you ask for it.

Linux and macOS:

```sh
curl -fsSL https://raw.githubusercontent.com/iqmanx/terminal_janitor/main/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/iqmanx/terminal_janitor/main/install.ps1 | iex
```

That downloads the release archive for your platform, verifies it against
`SHA256SUMS`, and places one binary in `~/.local/bin`
(`%LOCALAPPDATA%\Programs\terminal_janitor` on Windows). A missing or
mismatched checksum refuses the install rather than continuing.

Piping a script into a shell deserves suspicion, so: read it first if you like,
because it is the same script either way.

```sh
curl -fsSL https://raw.githubusercontent.com/iqmanx/terminal_janitor/main/install.sh | less
```

It never asks for root, never writes outside your home directory, never enables
scheduling, and never touches configuration or protection state.

If you have the repository cloned, run it from there instead, and pass options
directly:

```sh
./install.sh                          # latest release into ~/.local/bin
./install.sh --version v0.1.0         # an exact release
./install.sh --prefix "$HOME/bin"     # somewhere else
./install.sh --from ./path/to/binary  # install a binary you already built
```

The one-liner takes the same options after `-s --`:

```sh
curl -fsSL https://raw.githubusercontent.com/iqmanx/terminal_janitor/main/install.sh | sh -s -- --prefix "$HOME/bin"
```

### With npm or pnpm

Anyone this product is for already has a JavaScript package manager installed,
so there is one:

```sh
npm install -g terminal_janitor
pnpm add -g terminal_janitor
```

The published package is a wrapper with no logic in it. It selects the native
binary from the one platform package matching your `os` and `cpu`, and passes
arguments, stdio, and exit codes straight through. Nothing is downloaded during
installation and no install script runs, so this works under `--ignore-scripts`
and against a registry mirror. Every package carries an npm provenance
attestation tying it to the workflow run and commit that built it.

The binaries are the same artefacts the installers download, repackaged by the
release workflow — not a second build.

Run `terminal_janitor disable` **before** `npm rm -g terminal_janitor`. Removing
the package deletes the binary the schedule points at, and npm has no reliable
way to run the product's own uninstall step first. `uninstall.sh` gets this
right on your behalf; a package manager cannot.

Removing it again:

```sh
./uninstall.sh            # removes the schedule, then the binary
./uninstall.sh --purge    # also removes configuration and protection state
```

```powershell
.\uninstall.ps1
.\uninstall.ps1 -Purge
```

Uninstall removes the schedule *before* the binary, because a schedule left
pointing at a deleted binary would fail every hour forever. Configuration and
protection state are kept unless you pass `--purge`: they record which roots
you approved and which workspaces you protected, and deleting that silently
would be the one irreversible thing an uninstaller could do. Upgrading by
re-running the installer never touches either.

Nothing runs automatically until you turn it on:

```sh
terminal_janitor init --root /path/to/your/projects
terminal_janitor enable      # hourly, per-user
terminal_janitor disable     # stops it again
```

## Commands

```text
terminal_janitor init --root <path> [--root <path> ...] [--pnpm <path>]
terminal_janitor scan
terminal_janitor scan --json
terminal_janitor scan --dry-run
terminal_janitor protect add <path>
terminal_janitor protect remove <path>
terminal_janitor protect list
terminal_janitor status
terminal_janitor status --json
terminal_janitor check
terminal_janitor clean
terminal_janitor history [--limit <n>]
terminal_janitor enable
terminal_janitor disable
```

`--json` and `--dry-run` are global. `--dry-run` reads and reports without
writing state, so a rehearsal never advances the ledger's observation history.

`init` requires at least one explicit `--root`; it never guesses the home
directory or a conventional projects directory. Optional `--minimum-free` and
`--target-free` values use the strict size syntax below. Inputs must exist and
be directories. Each root is canonicalised, bound to its native volume and
file identity, and written to configuration and state as one coordinated
transaction. Filesystem roots/whole system drives and root paths whose final
component is a symbolic link are rejected. Repeating the same registration is
idempotent. `init` neither scans nor enables scheduling.

`init` also enrols one pnpm executable. `--pnpm <path>` names it explicitly;
without that flag `PATH` is searched once. What is stored is the canonical
path plus the executable's native volume and file identity, so later runs
prove the same file is still there instead of trusting `PATH` again. The only
command enrolment runs is `pnpm --version`, and pnpm major version 11 is the
minimum. An `npm`-installed Windows wrapper (`pnpm.cmd`, `pnpm.bat`,
`pnpm.ps1`) is refused, because running one would place `cmd.exe` or
PowerShell between this product and pnpm; enrol the pnpm executable itself.
A machine with no usable pnpm still initialises — the outcome is reported and
no pnpm action can be planned until one is enrolled.

`scan` is explicitly invoked and entirely read-only with respect to projects.
It scans only registered approved roots, invokes neither pnpm nor another
package tool, executes no package script or shell, and performs no cleanup. A
pnpm workspace is recognised only when these three regular files occur in the
same directory:

```text
package.json
pnpm-workspace.yaml
pnpm-lock.yaml
```

Discovery does not recurse into `.git` or `node_modules`, never follows a
symbolic directory link, caps traversal at 64 directory levels, and retains at
most 1,000 diagnostics per scan. These exclusions exist only for traversal
safety/performance; they grant no deletion authority. Nested roots are
validated separately but their shared subtree is walked once, and a workspace
is bound deterministically to its most specific approved root. A contained
mount remains allowed and keeps its own `VolumeId`. Permission failures,
identity changes, disappearing paths, partial workspace markers, and
unavailable roots are reported with stable exact reasons rather than treated
as absence.

`scan` also builds a plan and prints it. Planning decides; it never executes.
Every registered workspace ends in one of exactly two states: eligible with a
complete proof bundle, or skipped with one exact gate name and one reason.
Gates run in a fixed order — registration status, approved-root ownership,
live identity revalidation, pressured volume, protection, the three pnpm
markers, pnpm enrolment and version, Git worktree cleanliness, the 24-hour
observation window, the 30-day inactivity window, the 7-day cooldown, then
process liveness — and the first refusal is the one reported. Plans carry an
immutable plan ID, a policy hash, and a 15-minute expiry, and no plan may
contain more than two workspace cleans.

Worktree state comes from Git through a fixed read-only argument array
(`git --no-optional-locks status --porcelain=v1 --untracked-files=all`), run
directly with no shell. Empty output is clean; any line is dirty; a missing
Git, a non-zero exit, a timeout, or truncated output is `unknown`, and unknown
skips.

Process liveness is now proven rather than assumed. A process whose working
directory is inside the workspace, or whose command names it, makes the
workspace active. A failed enumeration is unknown. So is an unreadable working
directory when the process belongs to this user, or when it is a package
manager or Node whoever owns it — those are exactly the processes that would be
working in a workspace. Another user's unreadable daemon does not block a
clean, because a per-user tool cannot inspect it and treating that as
uncertainty would mean never cleaning anything.

`enable` installs an hourly per-user schedule and `disable` removes it. Linux
uses a systemd user timer, macOS a LaunchAgent, Windows a per-user scheduled
task. Each names the canonicalised installed binary and runs `check` as a bare
argument: no shell, no pipeline, and no administrator rights are ever
requested. Both commands are idempotent, including disabling something that was
never installed.

**macOS cannot currently clean a workspace.** On APFS the reported free space
can include purgeable space and space held by Time Machine local snapshots, and
this product has no snapshot-aware measurement. Unresolved ambiguity must block
workspace cleaning rather than be guessed at, so macOS runs report
`SKIPPED_SNAPSHOT_CAPACITY_UNCERTAIN` and perform own-state cleanup and store
pruning only. `terminal_janitor` never deletes or thins a snapshot.

The observation ledger records marker modification time plus bounded Git HEAD
and index fingerprints. It does not use filesystem access time. The stored
`git_state` on a workspace record remains `unknown`, because the ledger holds
only what read-only filesystem observation established; worktree cleanliness is
proven at planning time and is never persisted as authority. Activity may move forward but never backwards; first observation and
protection remain stable. Missing workspaces remain in state, including their
protection, and a moved workspace receives a new identity and fresh history.

Protection commands accept only the exact live root of a registered workspace.
Add and remove are idempotent, and protection persists across scans, process
restarts, and schema migration. `protect list --json`, `protect add ... --json`,
and `protect remove ... --json` are also supported through the global JSON
option.

`check` is the non-interactive threshold run and the scheduler's entry point.
`clean` is the same run asked for by a person: it shares every line of
`check`'s authority and unlocks nothing extra. Both follow one fixed order —
measure free space; exit above `minimum_free` without scanning, locking, or
journalling; take an exclusive per-volume lock; remeasure under the lock;
then execute proven actions one at a time, measuring the volume after each and
stopping the moment `target_free` is reached.

Before each action runs, every gate is asked again. Planning proved the
workspace was eligible; revalidation proves it still is, because liveness,
identity, protection, and Git state can all change in between. A refusal at
that point is journalled as exactly one of `SKIPPED_CHANGED`,
`SKIPPED_ACTIVE`, `SKIPPED_PROTECTED`, or `SKIPPED_UNKNOWN`. A non-zero exit,
a timeout, or truncated output stops the whole run; it never escalates into a
broader attempt.

Only one run may operate on a volume at once. A second run returns
`ALREADY_RUNNING` without measuring further or touching anything. The lock is
an advisory file lock rather than a file's existence, so a killed process
releases it and a leftover file blocks nothing.

Every action is written to the journal before it is attempted and moved
through explicit states as it proceeds, so an interrupted run stays readable:
an action caught in `VALIDATING` or `RUNNING` is reported as unresolved, never
as success, and is never replayed blindly. `history` shows journalled runs
newest first, with free-space figures, per-action states, and the recovery
instruction for anything removed. Detailed history is capped at 200 runs.

Only measured free-space change is ever reported as recovery. Estimated bytes
appear nowhere in a result.

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
approved_roots = []
```

Sizes must be unsigned whole numbers followed immediately by one of the
case-sensitive binary units `B`, `KiB`, `MiB`, `GiB`, or `TiB`. Decimal values,
signs, whitespace, unknown units, overflow, zero `minimum_free`, and a
`target_free` less than or equal to `minimum_free` are rejected. `GB` is not
accepted or interpreted as `GiB`. An invalid existing file fails closed with
`FAILED_CONFIGURATION`; it never falls back to defaults.

`approved_roots` is optional and defaults to an empty list, so configuration
files that predate it remain valid unchanged. When present it must contain
absolute paths; entries are sorted and de-duplicated deterministically, and a
relative entry fails closed. `init` is the supported way to populate it. No
current command performs cleanup based on this list.

## Scan JSON

JSON uses stable field and enum names. A successful scan has this shape (the
workspace summary objects include identity, protection, status, Git state, and
observation timestamps):

```json
{
  "result": "SCAN_COMPLETE",
  "approved_roots": 1,
  "roots_scanned": 1,
  "registered": [],
  "updated": [],
  "excluded": [],
  "unavailable_roots": [],
  "missing": [],
  "protected_workspaces": 0,
  "cleanup_performed": false,
  "dry_run": false,
  "plans": [
    {
      "plan_id": "plan-fnv1a64:…",
      "policy_hash": "policy-fnv1a64:…",
      "created_at_millis": 0,
      "expires_at_millis": 900000,
      "actions": [
        {
          "action": "CLEAN_TERMINAL_JANITOR_STATE",
          "proof": {
            "action_allowlisted": true,
            "owner_proven": true,
            "regenerability_proven": true,
            "reference_safety_proven": true,
            "inactivity_proven": true,
            "protection_checks_passed": true,
            "executable_identity_verified": true,
            "target_identity_verified": true
          }
        }
      ],
      "skipped": [
        {
          "workspace_id": "…",
          "path": "/approved/projects/api",
          "gate": "OBSERVATION_WINDOW_NOT_MET",
          "reason": "observed for 0 of the required 24 hours"
        }
      ]
    }
  ]
}
```

One plan is emitted per volume holding registered workspaces, ordered by
volume identity. Stable gate names are `NOT_INSIDE_APPROVED_ROOT`,
`WORKSPACE_NOT_PRESENT`, `TARGET_IDENTITY_CHANGED`, `DIFFERENT_VOLUME`,
`PROTECTED`, `MISSING_MARKER`, `PNPM_NOT_ENROLLED`, `PNPM_IDENTITY_CHANGED`,
`PNPM_BELOW_MINIMUM_VERSION`, `GIT_STATE_UNKNOWN`, `GIT_WORKTREE_DIRTY`,
`OBSERVATION_WINDOW_NOT_MET`, `RECENT_ACTIVITY`, `COOLDOWN_ACTIVE`,
`WORKSPACE_ACTIVE`, `LIVENESS_UNKNOWN`, and
`AUTOMATIC_WORKSPACE_CAP_REACHED`. Action names are
`CLEAN_TERMINAL_JANITOR_STATE`, `PNPM_STORE_PRUNE`, and
`PNPM_WORKSPACE_CLEAN`; an action appears only with all eight proof fields
true.

When relevant exclusions exist, `result` is
`SCAN_COMPLETE_WITH_EXCLUSIONS`. Stable reasons include
`outside_approved_root`, `root_unavailable`, `root_identity_changed`,
`symlink_not_followed`, `permission_denied`, `missing_package_json`,
`missing_pnpm_workspace`, `missing_pnpm_lockfile`,
`canonicalisation_failed`, `volume_identity_unavailable`,
`duplicate_identity`, `path_changed_during_scan`, and `unsupported_path`.

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

Pressure uses `PRESSURE_DETECTED`; `status` never claims that a target was
restored. Configuration failures exit 2, and storage-measurement failures exit
3. JSON-mode failures remain valid JSON.

## Release status

**0.1.1 is released.** Checksummed artefacts for all five targets are attached
to the GitHub release, and the npm channel carries the same binaries with a
provenance attestation tying each package to the workflow run and commit that
built it.

Every safety gate is exercised by CI on Ubuntu, macOS, and Windows, and the
safety model is additionally attacked from outside the crate, through the real
binary and the real filesystem. Scheduling was verified on real machines rather
than in CI alone: the schedule was enabled, triggered, observed firing the
binary, and disabled on Linux, macOS, and Windows. Release qualification covers
the per-user installers, a thousand simulated scheduler cycles, and
clean-account install and uninstall on all three platforms.

Two things remain honestly short of proof and are recorded rather than glossed:
no test yet downloads a published artefact and checks it against the published
`SHA256SUMS`, which the installers do but CI does not; and a scheduled run
cannot reach an actual workspace clean in CI, because the 24-hour observation
window is a real safety gate and a CI fixture is seconds old.

## Project documents

The design record, kept in [`docs/`](docs):

- [`SAFETY.md`](SAFETY.md) — the non-negotiable safety contract.
- [`docs/ACCEPTANCE.md`](docs/ACCEPTANCE.md) — the release gates.
- [`docs/VISION.md`](docs/VISION.md) — product purpose and boundaries.
- [`docs/PLANS.md`](docs/PLANS.md) — the seven-day execution plan.
- [`docs/RESEARCH.md`](docs/RESEARCH.md) — prior art and deferred architecture.
- [`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md) — the CI run
  identifiers behind each gate.
- [`docs/AGENTS.md`](docs/AGENTS.md) and
  [`docs/BOOTSTRAP_PROMPT.md`](docs/BOOTSTRAP_PROMPT.md) — how the project is
  built by coding agents.

Where they conflict, `SAFETY.md` and `docs/ACCEPTANCE.md` win, and no document
may weaken either.
