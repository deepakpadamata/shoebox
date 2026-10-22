#!/usr/bin/env bash
# Delete stale build leftovers. Cargo keeps every old hash-named test binary in target/debug/deps
# (each links the whole iroh stack, ~100-200 MB), so `target/` grows by gigabytes per day of work.
# Usage: scripts/prune-target.sh [minutes]   (default 25: keep what was built in the last 25 min)
set -euo pipefail
cd "$(dirname "$0")/.."
mins="${1:-25}"
rm -rf target/debug/incremental
find target/debug/deps -type f ! -name '*.*' -mmin +"$mins" -delete 2>/dev/null || true
du -sh target
