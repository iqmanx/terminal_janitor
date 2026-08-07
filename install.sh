#!/bin/sh
# terminal_janitor installer for Linux and macOS.
#
# Three rules this script will not break:
#
#   1. It never asks for administrator rights. Everything lands under the
#      invoking user's own home directory.
#   2. It never enables scheduling. Automatic operation is opt-in and the user
#      turns it on themselves with `terminal_janitor enable`.
#   3. It never touches configuration or state, so upgrading over an existing
#      install preserves thresholds, approved roots, and protection.
#
# Usage:
#   ./install.sh                       install the latest release
#   ./install.sh --version v0.1.0      install an exact release
#   ./install.sh --from ./path/to/bin  install an already-built binary
#   ./install.sh --prefix "$HOME/bin"  choose the install directory

set -eu

REPO="iqmanx/terminal_janitor"
NAME="terminal_janitor"
PREFIX="${TERMINAL_JANITOR_PREFIX:-$HOME/.local/bin}"
VERSION="latest"
FROM=""

die() {
    printf 'install: %s\n' "$1" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift 2 ;;
        --from)    [ $# -ge 2 ] || die "--from needs a value";    FROM="$2";    shift 2 ;;
        --prefix)  [ $# -ge 2 ] || die "--prefix needs a value";  PREFIX="$2";  shift 2 ;;
        -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option $1" ;;
    esac
done

# Resolve the artefact for this machine. An unrecognised platform is an error,
# never a guess at a binary that would not run.
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Linux)  os_part="unknown-linux-gnu" ;;
    Darwin) os_part="apple-darwin" ;;
    *) die "unsupported operating system: $os" ;;
esac
case "$arch" in
    x86_64|amd64)  arch_part="x86_64" ;;
    arm64|aarch64) arch_part="aarch64" ;;
    *) die "unsupported architecture: $arch" ;;
esac
target="${arch_part}-${os_part}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ -n "$FROM" ]; then
    [ -f "$FROM" ] || die "no such file: $FROM"
    cp "$FROM" "$tmp/$NAME"
else
    command -v curl >/dev/null 2>&1 || die "curl is required to download a release"

    if [ "$VERSION" = "latest" ]; then
        base="https://github.com/$REPO/releases/latest/download"
    else
        base="https://github.com/$REPO/releases/download/$VERSION"
    fi
    archive="${NAME}-${target}.tar.gz"

    printf 'install: downloading %s\n' "$archive"
    curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
        --output "$tmp/$archive" "$base/$archive" \
        || die "could not download $base/$archive"
    curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
        --output "$tmp/SHA256SUMS" "$base/SHA256SUMS" \
        || die "could not download $base/SHA256SUMS"

    # A release artefact is verified before it is ever unpacked.
    if command -v sha256sum >/dev/null 2>&1; then
        expected="$(awk -v f="$archive" '$2 == f || $2 == "*"f {print $1}' "$tmp/SHA256SUMS")"
        actual="$(sha256sum "$tmp/$archive" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        expected="$(awk -v f="$archive" '$2 == f || $2 == "*"f {print $1}' "$tmp/SHA256SUMS")"
        actual="$(shasum -a 256 "$tmp/$archive" | awk '{print $1}')"
    else
        die "neither sha256sum nor shasum is available; refusing to install unverified"
    fi
    [ -n "$expected" ] || die "$archive is not listed in SHA256SUMS"
    [ "$expected" = "$actual" ] || die "checksum mismatch for $archive"
    printf 'install: checksum verified\n'

    tar -xzf "$tmp/$archive" -C "$tmp" || die "could not unpack $archive"
    [ -f "$tmp/$NAME" ] || die "$archive did not contain $NAME"
fi

mkdir -p "$PREFIX"
# Replace atomically so a running or scheduled binary is never half-written.
cp "$tmp/$NAME" "$PREFIX/.$NAME.new"
chmod 755 "$PREFIX/.$NAME.new"
mv -f "$PREFIX/.$NAME.new" "$PREFIX/$NAME"

installed="$("$PREFIX/$NAME" --version 2>/dev/null || true)"
[ -n "$installed" ] || die "the installed binary did not run"

printf '\ninstall: %s -> %s\n' "$installed" "$PREFIX/$NAME"

case ":$PATH:" in
    *":$PREFIX:"*) ;;
    *) printf 'install: add %s to your PATH to use it by name\n' "$PREFIX" ;;
esac

cat <<'NOTE'

Scheduling is NOT enabled. Nothing runs automatically until you ask:

    terminal_janitor init --root /path/to/your/projects
    terminal_janitor enable      # hourly, per-user, no administrator rights
    terminal_janitor disable     # stops it again

Your configuration and protection state were not touched by this install.
NOTE
