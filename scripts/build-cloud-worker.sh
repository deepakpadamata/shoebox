#!/usr/bin/env bash
# Build Shoebox Cloud for Cloudflare (apps/cloud-worker): the viewer page into apps/web-viewer/dist (unless
# SKIP_VIEWER=1 and it is there already) and the Worker into apps/cloud-worker/build. wrangler runs this.
# Needs what scripts/build-viewer.sh needs, and installs worker-build 0.8.7 into target/tools the first time.
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "${SKIP_VIEWER:-}" != 1 ] || [ ! -f apps/web-viewer/dist/index.html ]; then
  scripts/build-viewer.sh
fi
if [ ! -x target/tools/bin/worker-build ]; then
  cargo install worker-build --version 0.8.7 --locked --root target/tools
fi
cd apps/cloud-worker
PATH="$PWD/../../target/tools/bin:$PATH" worker-build --release
ls -l build/index_bg.wasm
