# patched-loom-gate

`run.sh` takes a clean copy of one commit, applies one patch from this folder, and runs each `cargo test` line of that commit's `scripts/loom_gate.sh`. Every line runs even if an earlier one fails. Each run builds in a new temp dir and deletes it at the end, because a reused target dir can keep the build of an unpatched copy with the same version.

Usage, from anywhere in the repo: `sh proofs/patched-loom-gate/run.sh <commit or tag> <patch>`

What each patch does, and what the run prints:

- `0.1.0-runtime-orderings-relaxed.patch`, on `1a472db` (0.1.0): every `Acquire`, `Release` and `AcqRel` in the runtime's own code (22 of them, in `src/lib.rs` above the loom module, `src/cold.rs` and `src/tagged_payload.rs`) becomes `Relaxed`. The tests are left alone. Prints `test result: ok. 3 passed` and `test result: ok. 2 passed`: the loom tests can't see the runtime's orderings.
- `0.2.1-wake-and-delivery-relaxed.patch`, on `v0.2.1`: the key store, the wake bump and the delivery bump in `src/lib.rs` (lines 1075, 1076 and 1115) become `Relaxed`. Prints `test result: ok. 3 passed` and `test result: ok. 2 passed`.
- `0.2.2-wake-bump-relaxed.patch`, on `v0.2.2`: the wake bump in `ObserverCells::publish_wake` (`src/cells.rs` line 66) becomes `Relaxed`. `a_reader_sees_the_key_of_its_epoch_or_a_later_one` fails with `ObservedWake { epoch: 1, key: None }`, and `a_handle_reads_a_publish_on_another_thread` fails with `left: None` against `right: Some(WakeKey64 { .. })`. The other 3 pass.
- `0.2.2-delivery-bump-relaxed.patch`, on `v0.2.2`: the delivery bump in `ObserverCells::bump_delivery` (`src/cells.rs` line 73) becomes `Relaxed`. `delivery_never_runs_ahead_of_wakes` fails with `1 deliveries but 0 wakes`. The other 4 pass.
- `0df7915-with-the-0.2.1-loom-gate.patch`, on `0df7915`: `scripts/loom_gate.sh` becomes the one `f77ef14` ships (`-D warnings --locked`). The `--lib` line passes 3. The `--test` line stops building the library with 4 errors: unused imports `super::CacheLineAtomicU64`, `crate::sync::atomic::Ordering` and `Arc` and `thread`, and `constant WAKE_KEY is never used`.
- `0.2.1-without-the-loom-allows.patch`, on `v0.2.1`: the two `#[allow(unused_imports)]` on the loom-side `Arc` and `thread` in `src/sync.rs` come out. The `--lib` line passes 3. The `--test` line stops with `unused import: loom::sync::Arc` and `unused import: loom::thread`.
