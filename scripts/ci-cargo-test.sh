#!/usr/bin/env bash
# `cargo test <args>` for CI: on failure, each failing test and its panic line becomes an error
# annotation, which shows on the run's summary page (and in the API) without opening the log.
set -o pipefail
log=$(mktemp)
if cargo test "$@" 2>&1 | tee "$log"; then
  exit 0
fi
# The line after "panicked at" is the panic message (an assert's values, the report).
grep -E -A1 '^---- .* stdout ----$|panicked at|^test .* FAILED$|^error(\[E[0-9]+\])?: ' "$log" | grep -v '^--$' | head -60 |
  sed -e 's/%/%25/g' -e 's/^/::error title=cargo test::/'
exit 1
