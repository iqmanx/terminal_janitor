# terminal_janitor

A small native terminal storage governor for developer machines.

> Maintain a user-configured minimum amount of free storage by performing fewer
> cleanup actions with stronger proof, and stop safely when proof is
> insufficient.

```sh
npm install -g terminal_janitor
# or
pnpm add -g terminal_janitor
```

This package is a thin wrapper. It contains no logic: it selects the native
binary from the matching platform package and hands control to it, passing
arguments, stdio, and exit codes straight through. Nothing is downloaded at
install time — npm resolves the one platform package whose `os` and `cpu` match
your machine.

Nothing runs automatically until you ask for it:

```sh
terminal_janitor init --root /path/to/your/projects
terminal_janitor status
terminal_janitor enable      # hourly, per-user, no administrator rights
terminal_janitor disable     # stops it again
```

**Run `terminal_janitor disable` before `npm rm -g terminal_janitor`.** Removing
the package deletes the binary the schedule points at, and npm cannot run the
tool's own uninstall step for you.

Full documentation, the safety contract, and the other install methods are at
<https://github.com/iqmanx/terminal_janitor>.
