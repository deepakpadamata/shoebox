#!/usr/bin/env bash
# Build the web-link viewer page into apps/web-viewer/dist (W1-06): the page's own files and the WebAssembly
# build of shoebox-core's blob and manifest code. Needs the wasm32 target (`rustup target add
# wasm32-unknown-unknown`) and wasm-bindgen-cli 0.2.129 (`cargo install wasm-bindgen-cli --version 0.2.129
# --locked --root target/tools`), found in $WASM_BINDGEN, target/tools/bin or PATH.
set -euo pipefail
cd "$(dirname "$0")/.."
BINDGEN="${WASM_BINDGEN:-}"
if [ -z "$BINDGEN" ]; then
  if [ -x target/tools/bin/wasm-bindgen ]; then BINDGEN=target/tools/bin/wasm-bindgen; else BINDGEN="$(command -v wasm-bindgen)"; fi
fi
cargo build -p shoebox-web --release --target wasm32-unknown-unknown
rm -rf apps/web-viewer/dist
mkdir -p apps/web-viewer/dist
"$BINDGEN" --target web --no-typescript --out-dir apps/web-viewer/dist/pkg target/wasm32-unknown-unknown/release/shoebox_web.wasm
cp apps/web-viewer/index.html apps/web-viewer/style.css apps/web-viewer/viewer.js apps/web-viewer/zip.js apps/web-viewer/dist/
du -sh apps/web-viewer/dist/pkg/*.wasm
