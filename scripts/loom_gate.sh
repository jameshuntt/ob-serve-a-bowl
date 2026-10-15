#!/usr/bin/env bash
# loom checks the wake/epoch publication between the runtime and ObserverHandles on other
# threads: every thread interleaving and weak-memory reordering of the real code
# (src/cells.rs, and the whole runtime in tests/loom_atomic_wake_tests.rs).
# Only the loom tests run under --cfg loom; the ordinary tests use std atomics
# and are not meaningful under the loom cfg.
set -euo pipefail
cd "$(dirname "$0")/.."
RUSTFLAGS="--cfg loom -D warnings" cargo test --all-features --locked --lib loom_wake_tests
RUSTFLAGS="--cfg loom -D warnings" cargo test --all-features --locked --test loom_atomic_wake_tests
