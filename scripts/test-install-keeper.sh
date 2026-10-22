#!/bin/sh
# Tests the install script's control flow with a fake `minisign` (accepts a signature file whose
# content is "good"). It does NOT test real signature verification.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
mkdir -p "$t/rel" "$t/bin" "$t/prefix" "$t/pkg"

case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) asset=linux-x86_64 ;; Linux/aarch64|Linux/arm64) asset=linux-aarch64 ;; Darwin/*) asset=macos-universal ;;
  *) echo "unsupported platform for this test"; exit 0 ;;
esac
printf '#!/bin/sh\necho shoebox-keeper test build\n' > "$t/pkg/shoebox-keeper"; chmod +x "$t/pkg/shoebox-keeper"
tar -C "$t/pkg" -czf "$t/rel/shoebox-keeper-$asset.tar.gz" shoebox-keeper
if command -v sha256sum >/dev/null 2>&1; then sum=$(sha256sum "$t/rel/shoebox-keeper-$asset.tar.gz" | cut -d' ' -f1)
else sum=$(shasum -a 256 "$t/rel/shoebox-keeper-$asset.tar.gz" | cut -d' ' -f1); fi
echo "$sum  shoebox-keeper-$asset.tar.gz" > "$t/rel/SHA256SUMS"
echo good > "$t/rel/SHA256SUMS.minisig"
cat > "$t/bin/minisign" <<'SH'
#!/bin/sh
# minisign -Vm FILE -P KEY -x SIG
while [ $# -gt 0 ]; do case "$1" in -x) sig="$2"; shift 2;; *) shift;; esac; done
[ "$(cat "$sig")" = good ]
SH
chmod +x "$t/bin/minisign"

run() { PATH="$t/bin:$PATH" SHOEBOX_BASE_URL="$t/rel" SHOEBOX_PREFIX="$t/prefix" sh "$here/install-keeper.sh" 2>&1; }

# 1. Happy path installs a runnable binary.
run >/dev/null; "$t/prefix/shoebox-keeper" | grep -q "test build"; rm "$t/prefix/shoebox-keeper"

# 2. A bad signature is refused and nothing is installed.
echo bad > "$t/rel/SHA256SUMS.minisig"
if run >/dev/null; then echo "FAIL: bad signature accepted"; exit 1; fi
[ ! -e "$t/prefix/shoebox-keeper" ]; echo good > "$t/rel/SHA256SUMS.minisig"

# 3. A tampered tarball (checksum mismatch) is refused.
printf 'tampered' >> "$t/rel/shoebox-keeper-$asset.tar.gz"
if run | grep -q "checksum mismatch"; then :; else echo "FAIL: tampering not detected"; exit 1; fi
[ ! -e "$t/prefix/shoebox-keeper" ]

# 4. Without minisign installed the script refuses rather than skipping verification.
rm "$t/bin/minisign"
if PATH="/usr/bin:/bin" SHOEBOX_BASE_URL="$t/rel" SHOEBOX_PREFIX="$t/prefix" sh "$here/install-keeper.sh" 2>&1 | grep -q "minisign is not installed"; then :; else echo "FAIL: ran without minisign"; exit 1; fi
echo "install-keeper tests passed"
