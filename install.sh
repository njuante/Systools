#!/bin/sh
# SysTUI installer. Downloads the static release binary for this architecture,
# verifies its SHA256 (and, when cosign is present, its keyless Sigstore
# signature), then installs it to a bin directory.
#
#   curl -fsSL https://raw.githubusercontent.com/njuante/Systools/main/install.sh | sh
#
# Environment overrides:
#   SYSTUI_VERSION   release to install (default: latest)
#   SYSTUI_BINDIR    install dir (default: /usr/local/bin, ~/.local/bin if not writable)
#   SYSTUI_REPO      owner/repo (default: njuante/Systools)
set -eu

REPO="${SYSTUI_REPO:-njuante/Systools}"
VERSION="${SYSTUI_VERSION:-latest}"

err() { printf 'install: %s\n' "$1" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# Pick a downloader.
if have curl; then
    dl() { curl -fsSL "$1" -o "$2"; }
    dl_stdout() { curl -fsSL "$1"; }
elif have wget; then
    dl() { wget -qO "$2" "$1"; }
    dl_stdout() { wget -qO- "$1"; }
else
    err "need curl or wget"
fi

# Map architecture to the released target triple.
arch="$(uname -m)"
case "$arch" in
    x86_64|amd64)  target="x86_64-unknown-linux-musl" ;;
    aarch64|arm64) target="aarch64-unknown-linux-musl" ;;
    *) err "unsupported architecture: $arch" ;;
esac
[ "$(uname -s)" = "Linux" ] || err "SysTUI only supports Linux"

# Resolve the version tag.
if [ "$VERSION" = "latest" ]; then
    tag="$(dl_stdout "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' | head -n1 | cut -d'"' -f4)"
    [ -n "$tag" ] || err "could not determine latest release"
else
    case "$VERSION" in v*) tag="$VERSION" ;; *) tag="v$VERSION" ;; esac
fi
ver="${tag#v}"

base="https://github.com/$REPO/releases/download/$tag"
archive="systui-$ver-$target.tar.gz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
cd "$tmp"

printf 'install: downloading %s (%s)\n' "$archive" "$tag" >&2
dl "$base/$archive" "$archive"
dl "$base/SHA256SUMS" "SHA256SUMS"

# Verify the checksum.
if have sha256sum; then
    grep " $archive\$" SHA256SUMS | sha256sum -c - >/dev/null 2>&1 \
        || err "checksum verification failed for $archive"
elif have shasum; then
    grep " $archive\$" SHA256SUMS | shasum -a 256 -c - >/dev/null 2>&1 \
        || err "checksum verification failed for $archive"
else
    printf 'install: warning: no sha256 tool found, skipping checksum\n' >&2
fi

# Verify the Sigstore signature when cosign is available (best-effort).
if have cosign; then
    if dl "$base/SHA256SUMS.cosign.bundle" "SHA256SUMS.cosign.bundle" 2>/dev/null; then
        cosign verify-blob \
            --bundle SHA256SUMS.cosign.bundle \
            --certificate-identity-regexp "https://github.com/$REPO" \
            --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
            SHA256SUMS >/dev/null 2>&1 \
            && printf 'install: cosign signature verified\n' >&2 \
            || printf 'install: warning: cosign verification failed\n' >&2
    fi
fi

tar -xzf "$archive"

# Choose an install dir.
bindir="${SYSTUI_BINDIR:-/usr/local/bin}"
if [ ! -d "$bindir" ] || [ ! -w "$bindir" ]; then
    if [ "$(id -u)" -ne 0 ] && have sudo; then
        SUDO="sudo"
    elif [ "$(id -u)" -ne 0 ]; then
        bindir="$HOME/.local/bin"; mkdir -p "$bindir"; SUDO=""
    else
        SUDO=""
    fi
else
    SUDO=""
fi
SUDO="${SUDO:-}"

$SUDO install -Dm755 systui "$bindir/systui"
printf 'install: installed systui %s to %s/systui\n' "$ver" "$bindir" >&2

case ":$PATH:" in
    *":$bindir:"*) ;;
    *) printf 'install: note: %s is not on your PATH\n' "$bindir" >&2 ;;
esac
