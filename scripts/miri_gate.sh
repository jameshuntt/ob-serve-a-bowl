#!/usr/bin/env bash
# Miri checks the test suite and every example for undefined behavior, data races included.
# The ObserverHandle tests use real threads, so they also run across 8 thread schedules.
# Some tests read the clock and write temp files, which Miri allows only without isolation.
set -euo pipefail
cd "$(dirname "$0")/.."
export MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-disable-isolation"
cargo +nightly miri test --all-features --locked
MIRIFLAGS="$MIRIFLAGS -Zmiri-many-seeds=0..8" cargo +nightly miri test --all-features --locked --test observer_handle_tests
for e in examples/*.rs; do
  cargo +nightly miri run --all-features --locked --example "$(basename "$e" .rs)"
done
