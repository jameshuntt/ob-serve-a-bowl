#!/bin/sh
# Runs the crate's criterion benches on tag v0.2.1 and tag v0.2.2, one after the other,
# three rounds, then prints each case's median of the three against each other.
# WORK is where the two trees, their builds and the saved results go (default: a new temp dir).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)
work=${WORK:-$(mktemp -d)}
for v in 0.2.1 0.2.2; do
  rm -rf "$work/src-$v" && mkdir -p "$work/src-$v"
  git -C "$repo" archive "v$v" | tar -x -C "$work/src-$v"
done
for round in 1 2 3; do
  for v in 0.2.1 0.2.2; do
    (cd "$work/src-$v" && CARGO_TARGET_DIR="$work/target-$v" \
      cargo bench --all-features --locked --bench observable_runtime_benches -- --noplot)
    rm -rf "$work/r$round-$v" && cp -R "$work/target-$v/criterion" "$work/r$round-$v"
  done
done
python3 "$here/compare.py" "$work"
