#!/bin/sh
# terminal_janitor uninstaller for Linux and macOS.
#
# Order matters. The schedule is removed *before* the binary, because a
# schedule left pointing at a deleted binary is a per-user timer that fails
# every hour forever.
#
# Configuration and protection state are kept by default. They record which
# roots a user approved and which workspaces they protected, and deleting that
# silently would be the one irreversible thing an uninstaller could do. Pass
# --purge to remove them deliberately.
#
# Usage:
#   ./uninstall.sh                     remove the schedule and the binary
#   ./uninstall.sh --purge             also remove configuration and state
#   ./uninstall.sh --prefix "$HOME/bin"

set -eu

NAME="terminal_janitor"
PREFIX="${TERMINAL_JANITOR_PREFIX:-$HOME/.local/bin}"
PURGE=0

die() {
    printf 'uninstall: %s\n' "$1" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --purge)  PURGE=1; shift ;;
        --prefix) [ $# -ge 2 ] || die "--prefix needs a value"; PREFIX="$2"; shift 2 ;;
        -h|--help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option $1" ;;
    esac
done

binary="$PREFIX/$NAME"

if [ -x "$binary" ]; then
    # Disable is idempotent and succeeds when nothing is installed, so this is
    # safe whether or not the user ever enabled a schedule.
    printf 'uninstall: removing any installed schedule\n'
    "$binary" disable >/dev/null 2>&1 || \
        printf 'uninstall: the binary could not remove its schedule; check it by hand\n' >&2
    rm -f "$binary"
    printf 'uninstall: removed %s\n' "$binary"
else
    printf 'uninstall: no binary at %s\n' "$binary"
fi

if [ "$PURGE" -eq 1 ]; then
    case "$(uname -s)" in
        Darwin)
            config="$HOME/Library/Application Support/terminal_janitor"
            state="$HOME/Library/Application Support/terminal_janitor"
            ;;
        *)
            config="${XDG_CONFIG_HOME:-$HOME/.config}/terminal_janitor"
            state="${XDG_DATA_HOME:-$HOME/.local/share}/terminal_janitor"
            ;;
    esac
    for directory in "$config" "$state"; do
        if [ -d "$directory" ]; then
            rm -rf "$directory"
            printf 'uninstall: purged %s\n' "$directory"
        fi
    done
else
    printf 'uninstall: configuration and protection state were kept; --purge removes them\n'
fi
