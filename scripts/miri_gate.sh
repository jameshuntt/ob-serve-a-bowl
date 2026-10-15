#!/usr/bin/env bash
# Miri checks the test suite and every example for undefined behavior, single-threaded.
# Some tests read the clock and write temp files, which Miri allows only without isolation.
set -euo pipefail
cd "$(dirname "$0")/.."
export MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-disable-isolation"
cargo +nightly miri test --all-features --locked
for e in examples/*.rs; do
  cargo +nightly miri run --all-features --locked --example "$(basename "$e" .rs)"
done
