#!/bin/sh
# Build the PhotoKit probe as a signed app bundle and run one command (P1-01).
# macOS asks for Photos access only on behalf of a bundle with a usage string, so the probe is
# launched with `open`, not run from the shell. Output goes to the terminal when it finishes.
#
#   tools/photokit-probe/run.sh auth
#   tools/photokit-probe/run.sh enumerate
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=${PROBE_OUT:-$here/../../target/photokit-probe}
app=$out/PhotoKitProbe.app
mkdir -p "$app/Contents/MacOS"
if [ ! -x "$app/Contents/MacOS/probe" ] || [ "$here/probe.swift" -nt "$app/Contents/MacOS/probe" ]; then
  swiftc -O -target arm64-apple-macos13 -o "$app/Contents/MacOS/probe" "$here/probe.swift"
  cp "$here/Info.plist" "$app/Contents/Info.plist"
  # Ad hoc signing: every rebuild is a new app to macOS, so run `auth` again after one.
  codesign --force -s - "$app" 2>/dev/null
fi
rm -f "$out/stdout" "$out/stderr"
open -W -n --stdout "$out/stdout" --stderr "$out/stderr" "$app" --args "$@"
cat "$out/stdout"
[ -s "$out/stderr" ] && cat "$out/stderr" >&2 || true
