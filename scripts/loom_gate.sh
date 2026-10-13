#!/usr/bin/env bash
# loom checks the wake/epoch protocol's concurrent memory ordering: every thread
# interleaving and weak-memory reordering of the real atomics the runtime uses.
# Only the loom tests run under --cfg loom; the ordinary tests use std atomics
# and are not meaningful under the loom cfg.
set -euo pipefail
cd "$(dirname "$0")/.."
RUSTFLAGS="--cfg loom" cargo test --all-features --lib loom_wake_tests
RUSTFLAGS="--cfg loom" cargo test --all-features --test loom_atomic_wake_tests
