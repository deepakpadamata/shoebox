#!/bin/sh
# Install shoebox-keeper: downloads the release for this machine, verifies the SHA256 checksum
# and the minisign signature of the checksum file, and installs the binary.
#
#   curl -fsSL <release url>/install-keeper.sh | sh
#   SHOEBOX_VERSION=keeper-v0.1.0 sh install-keeper.sh      # a specific release
#   SHOEBOX_PREFIX=$HOME/.local/bin sh install-keeper.sh    # where to put it
#
# Environment (mainly for testing): SHOEBOX_BASE_URL overrides the download location
# (a directory or URL holding the release files).
set -eu

# The release signing key (public). Replace after generating the key pair; see the release workflow.
MINISIGN_PUBKEY="${SHOEBOX_MINISIGN_PUBKEY:-RWQ_REPLACE_WITH_THE_PROJECT_PUBLIC_KEY}"
REPO="${SHOEBOX_REPO:-OWNER/shoebox}"
VERSION="${SHOEBOX_VERSION:-latest}"
PREFIX="${SHOEBOX_PREFIX:-/usr/local/bin}"

die() { echo "install-keeper: $*" >&2; exit 1; }

os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
  Linux/x86_64) asset=linux-x86_64 ;;
  Linux/aarch64|Linux/arm64) asset=linux-aarch64 ;;
  Darwin/*) asset=macos-universal ;;
  *) die "unsupported platform $os/$arch" ;;
esac

if [ -n "${SHOEBOX_BASE_URL:-}" ]; then
  base="$SHOEBOX_BASE_URL"
elif [ "$VERSION" = latest ]; then
  base="https://github.com/$REPO/releases/latest/download"
else
  base="https://github.com/$REPO/releases/download/$VERSION"
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  die "need sha256sum or shasum"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fetch() { # url dest
  case "$1" in
    http://*|https://*) curl -fsSL "$1" -o "$2" ;;
    file://*) cp "${1#file://}" "$2" ;;
    *) cp "$1" "$2" ;;
  esac
}

tarball="shoebox-keeper-$asset.tar.gz"
fetch "$base/$tarball" "$tmp/$tarball" || die "could not download $tarball"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "could not download SHA256SUMS"
fetch "$base/SHA256SUMS.minisig" "$tmp/SHA256SUMS.minisig" || die "could not download SHA256SUMS.minisig"

# 1. The signature proves the checksum file is the project's.
if command -v minisign >/dev/null 2>&1; then
  minisign -Vm "$tmp/SHA256SUMS" -P "$MINISIGN_PUBKEY" -x "$tmp/SHA256SUMS.minisig" >/dev/null \
    || die "signature check FAILED: refusing to install"
else
  die "minisign is not installed (apt install minisign / brew install minisign); it is needed to verify the download"
fi

# 2. The checksum proves the tarball is the one that was signed.
want=$(grep " $tarball\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
[ -n "$want" ] || die "$tarball is not listed in SHA256SUMS"
got=$(sha256 "$tmp/$tarball")
[ "$want" = "$got" ] || die "checksum mismatch for $tarball: refusing to install"

tar -xzf "$tmp/$tarball" -C "$tmp" shoebox-keeper
mkdir -p "$PREFIX" 2>/dev/null || true
if [ -w "$PREFIX" ]; then
  install -m 0755 "$tmp/shoebox-keeper" "$PREFIX/shoebox-keeper"
else
  echo "$PREFIX is not writable; using sudo"
  sudo install -m 0755 "$tmp/shoebox-keeper" "$PREFIX/shoebox-keeper"
fi
echo "Installed $PREFIX/shoebox-keeper"
"$PREFIX/shoebox-keeper" --version || true
echo "Next: shoebox-keeper run   (then, from another terminal: shoebox-keeper pair <ticket>)"
