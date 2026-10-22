#!/usr/bin/env bash
# M1 demo: three "devices" on one machine, using only the shoebox-dev CLI.
#
#   laptop  imports photos and stays online (`run`)
#   pi      a Keeper: joins with the laptop's ticket and ends up with every original
#   phone   a Viewer: gets the library but only fetches an original on demand
#
# Usage:  docs/notes/demo-m1.sh            (builds the CLI first)
# Env:    BIN=<path to shoebox-dev>        skip the build and use this binary
#         DEMO_DIR=<dir>                   keep state here (default: a temp dir)
#         DEMO_REAL=1                      use the relay + mDNS instead of --local-only
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${BIN:-}"
if [ -z "$BIN" ]; then
  (cd "$ROOT" && cargo build -q -p shoebox-dev)
  BIN="$ROOT/target/debug/shoebox-dev"
fi
DEMO_DIR="${DEMO_DIR:-$(mktemp -d)}"
LOCAL="--local-only"
[ "${DEMO_REAL:-0}" = "1" ] && LOCAL=""
SAMPLES="$ROOT/fixtures/samples"

dev() { local name="$1"; shift; "$BIN" --dir "$DEMO_DIR/$name" $LOCAL "$@"; }
step() { printf '\n== %s\n' "$*"; }

DAEMON_PID=""
cleanup() { [ -n "$DAEMON_PID" ] && kill "$DAEMON_PID" 2>/dev/null || true; }
trap cleanup EXIT

step "1. The laptop creates a library and imports some photos"
dev laptop init --name laptop --role keeper
dev laptop import "$SAMPLES/photo-gps.jpg" "$SAMPLES/IMG_0001.jpg" "$SAMPLES/screenshot.png"
dev laptop ls
dev laptop status

step "2. The laptop goes online and prints a ticket"
dev laptop run > "$DEMO_DIR/laptop.log" 2>&1 &
DAEMON_PID=$!
for _ in $(seq 1 120); do grep -q '^ticket: ' "$DEMO_DIR/laptop.log" 2>/dev/null && break; sleep 0.5; done
TICKET="$(grep '^ticket: ' "$DEMO_DIR/laptop.log" | head -1 | cut -d' ' -f2)"
[ -n "$TICKET" ] || { echo "the laptop did not come online"; cat "$DEMO_DIR/laptop.log"; exit 1; }
VIEWER_TICKET="$(grep '^ticket-viewer: ' "$DEMO_DIR/laptop.log" | head -1 | cut -d' ' -f2)"
echo "ticket: ${TICKET:0:40}..."

step "3. The Pi (a keeper) joins: it gets the library's metadata immediately"
dev pi join "$TICKET" --name pi --role keeper
dev pi ls

step "4. The Pi fetches an original from the laptop; the copy count becomes 2"
ID="$(dev pi --json ls | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
dev pi fetch "${ID:0:12}" --kind original
dev pi holdings "${ID:0:12}"
dev pi peers

step "5. A phone-like viewer joins and sees the same library; it only fetches on demand"
dev phone join "$VIEWER_TICKET" --name phone --role viewer
dev phone ls
dev phone fetch "${ID:0:12}" --kind preview
dev phone ls

step "6. Everything is below the redundancy target of 2 except what the Pi now holds"
dev pi status

echo
echo "demo complete (state in $DEMO_DIR)"
