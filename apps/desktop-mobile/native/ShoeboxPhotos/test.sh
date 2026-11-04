#!/bin/sh
# Run the ShoeboxPhotos unit tests (P1-03). `swift test` where SwiftPM works (Xcode, CI); otherwise
# with `swiftc` directly against the Command Line Tools' Testing framework, since some Command
# Line Tools installs carry a stale manifest API that SwiftPM cannot compile a manifest against.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
if swift test --package-path "$here" 2>/dev/null; then
  exit 0
fi
echo "swift test did not run; compiling the tests with swiftc" >&2
clt=$(xcode-select -p)
fw=$clt/Library/Developer/Frameworks
out=${TMPDIR:-/tmp}/shoebox-photos-tests
rm -rf "$out"
mkdir -p "$out"
printf 'import Testing\n@main struct Runner { static func main() async { await Testing.__swiftPMEntryPoint() as Never } }\n' > "$out/main.swift"
swiftc -swift-version 5 -parse-as-library -target "$(uname -m)-apple-macos14" -module-name ShoeboxPhotos \
  -enable-testing -emit-library -static -emit-module -emit-module-path "$out/ShoeboxPhotos.swiftmodule" \
  -o "$out/libShoeboxPhotos.a" "$here"/Sources/ShoeboxPhotos/*.swift
swiftc -swift-version 5 -parse-as-library -target "$(uname -m)-apple-macos14" -I "$out" -L "$out" -lShoeboxPhotos \
  -F "$fw" -plugin-path "$clt/usr/lib/swift/host/plugins/testing" \
  -Xlinker -rpath -Xlinker "$fw" -Xlinker -rpath -Xlinker "$clt/Library/Developer/usr/lib" \
  -framework Testing -framework Photos -framework AVFoundation \
  -o "$out/tests" "$here"/Tests/ShoeboxPhotosTests/*.swift "$out/main.swift"
"$out/tests"
