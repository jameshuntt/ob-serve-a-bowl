#!/bin/sh
# Runs each `cargo test` line of scripts/loom_gate.sh on a clean copy of one commit after applying
# one patch from this folder. Every line runs even if an earlier one fails, so every result shows.
# Usage, from anywhere in the repo:  sh proofs/patched-loom-gate/run.sh <commit or tag> <patch>
# Each run builds in a new temp dir, deleted at the end: a reused target dir can keep a build
# from the unpatched copy, since both have the same version.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)
patch="$here/$(basename "$2")"
work=$(mktemp -d "${TMPDIR:-/tmp}/patched-loom-gate.XXXXXX")
trap 'rm -rf "$work"' EXIT
git -C "$repo" archive "$1" | tar -x -C "$work"
(cd "$work" && patch -p1 < "$patch")
cd "$work"
grep '^RUSTFLAGS=.* cargo test' scripts/loom_gate.sh | while read -r line; do
  echo "+ $line"
  CARGO_TARGET_DIR="$work/target" sh -c "$line" || echo "exit $?"
done
