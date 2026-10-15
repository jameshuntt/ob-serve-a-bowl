# Changelog

Every link to code points at exact lines at an exact commit, and each one says what it shows. "Before" is the code as it was, "After" is the change, "Proof" is the test or the run that shows it, "In 0.1.0" is what the same check gives on the first release, and "Introduced" is the commit the problem came in with. Programs that aren't part of the crate are in [`proofs/`](proofs/), one folder each, with a README that gives the command and what it prints. Dates are when crates.io got the version, in UTC.

## 0.2.2 (2026-10-15 03:44 UTC)

From tag `v0.2.2`, commit [`c520dd9`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/c520dd95be69a55593519a2226e9968b0725b62c). Published by the CI `publish` job after `test`, `msrv`, `miri` and `loom` passed ([run 38677659061](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38677659061)).

### Added

#### `ObserverHandle`: read an observer's wakes from any thread

Up to 0.2.1 an observer's four counters (`state`, `wake_epoch`, `delivery_epoch`, `last_key`) sat inline in its `ObserverEntry`, inside the runtime's observer table. Cold observers had the same four inline in `ColdObserverEntry`. The only readers were `observer_epoch`, `observer_delivery_epoch` and `observer_last_key` for hot observers, and `cold_observer_epoch`, `cold_observer_delivery_epoch` and `cold_observer_last_key` for cold ones. All six borrow the runtime. Nothing read `state` outside the publish loops. `publish` takes `&mut self`, so no other thread could read an observer while the runtime was publishing, even though the counters were atomics and the key was already stored with `Release` for "external readers". So the loom tests checked a read during a publish that no safe caller could make.

Now each observer's counters live in one shared `Arc<ObserverCells>`. `ObservableWakeRuntime::observer_handle(id)` and `ColdObservableRuntime::cold_observer_handle(id)` hand out an `ObserverHandle` holding a clone of that `Arc`. It's `Clone + Send + Sync` ([`handles_are_cloneable_send_and_sync`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L52-L56)). `observe()` returns `ObservedWake { epoch, key }`, reading the epoch first and then the key, so `key` is the key of wake `epoch` or of a later one, never an earlier one. The handle holds its own block, so it never reads a different observer that later reuses the slot.

- Before: the four counters inline in the hot entry, [`src/lib.rs` L553-L560 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L553-L560); the same four in the cold entry, [`src/cold.rs` L201-L209 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L201-L209); the hot readers, which borrow the runtime, [`src/lib.rs` L857-L874 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L857-L874); the cold readers, [`src/cold.rs` L381-L398 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L381-L398); the key stored with `Release` for external readers, [`src/lib.rs` L1069-L1076 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1069-L1076).
- After: `ObserverCells`, `ObservedWake` and `ObserverHandle`, [`src/cells.rs` L31-L213 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L31-L213); `observer_handle`, [`src/lib.rs` L859-L863 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L859-L863); `cold_observer_handle`, [`src/cold.rs` L382-L386 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cold.rs#L382-L386).
- Proof: [`a_reader_thread_follows_every_wake_in_order`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L14-L50) publishes 500 wakes while a reader thread checks every `observe()`: the epoch never goes back, the key is never from an earlier wake, and `delivery_epoch` never passes `wake_epoch`. [`a_handle_outlives_removal_and_stops_moving`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L58-L77) removes the observer, registers a new one into the same slot and publishes; the old handle still reads epoch 1 with the old key. [`a_new_handle_reports_no_wake_yet`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L79-L91) reads `epoch: 0, key: None` before any wake. [`a_cold_observer_has_a_handle_too`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L123-L154) gives a cold observer's handle to a reader thread, which waits for `wake_epoch` to leave 0; after one `publish_stateful` it reads `ObservedWake { epoch: 1, key: Some(cold_key) }`. Under loom, [`a_handle_reads_a_publish_on_another_thread`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/loom_atomic_wake_tests.rs#L17-L42) and [`an_old_handle_never_sees_the_observer_that_reuses_its_slot`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/loom_atomic_wake_tests.rs#L44-L63) run the real `ObservableWakeRuntime` on one thread against a handle on another. Under Miri the handle tests also run across 8 thread schedules, [`scripts/miri_gate.sh` L9 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/scripts/miri_gate.sh#L9-L9). Run locally on `c520dd9`: `cargo test --all-features` passes 98, `scripts/loom_gate.sh` passes 5, `scripts/miri_gate.sh` exits 0 with seeds 0 through 7.
- In 0.1.0: `cargo build` in [`proofs/no-reader-while-publishing`](proofs/no-reader-while-publishing/) (one thread calls `publish`, another calls `observer_epoch`, on the same runtime) gave ``error[E0596]: cannot borrow `*shared` as mutable, as it is behind a `&` reference`` and ``error[E0501]: cannot borrow `*shared` as immutable because previous closure requires unique access``.

#### Handles see `Inactive` and `Removed`; dropping a runtime marks its observers removed

`ObserverCells::new(config.active)` starts an observer registered with `.inactive()` as `ObserverState::Inactive`. Before, every entry's `state` started as `Active` whatever the config said. Nothing outside the runtime could read `state` then, so it never showed, but a handle would have shown it. A handle can also outlive its runtime, so `ObservableWakeRuntime` and `ColdObservableRuntime` now implement `Drop` and mark every observer they still hold `Removed`. Without that, a handle on a dropped runtime would keep reading `Active`.

- Before: the hot entry starting `state` at `Active`, [`src/lib.rs` L562-L572 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L562-L572); the cold entry doing the same, [`src/cold.rs` L211-L222 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L211-L222).
- After: `ObserverCells::new` taking `active`, [`src/cells.rs` L44-L53 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L44-L53); `Drop` for `ObservableWakeRuntime`, [`src/lib.rs` L738-L745 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L738-L745); `Drop` for `ColdObservableRuntime`, [`src/cold.rs` L280-L288 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cold.rs#L280-L288).
- Proof: [`an_inactive_observer_reads_as_inactive`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L156-L166), [`dropping_the_runtime_marks_its_observers_removed`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L168-L177), [`cold_handles_report_inactive_and_removed_on_drop`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L188-L199).

### Changed

#### Hot, cold and tagged publishing go through one counter block

The same four atomics and the same "store the key, then bump the epoch" steps were written out three times: `CacheLineAtomicU64` and `ObserverEntry` in `lib.rs`, `ColdAtomicU64` and `ColdObserverEntry` in `cold.rs`, and the tagged-payload publish writing the hot entry's atomics by hand. The copies had already drifted apart: the `state` check loaded with `Acquire` in the hot and tagged loops and with `Relaxed` in the cold ones. Now all three call `ObserverCells::publish_wake` and `bump_delivery`, so the order and the orderings are written once. `CacheLineAtomicU64` moved to `src/cells.rs` and `ColdAtomicU64` is gone.

The per-profile delivery is still three copies: `deliver` in `lib.rs`, `make_cold_delivery` in `cold.rs`, and the `match profile` in `publish_tagged`. For `CrossbeamQueue` they still differ. The hot copy sends. The tagged copy reports `CrossbeamTaggedPayloadUnsupported`. The cold copy reports `CrossbeamSenderMissing` even when the hot runtime has a sender, in 0.1.0 and in 0.2.2.

- Before: `CacheLineAtomicU64` and the hot `ObserverEntry`, [`src/lib.rs` L538-L572 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L538-L572); the hot key store and epoch bump in `make_event`, [`src/lib.rs` L1069-L1076 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1069-L1076); `ColdAtomicU64` and `ColdObserverEntry`, [`src/cold.rs` L187-L222 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L187-L222); the cold copy in `make_cold_delivery`, [`src/cold.rs` L715-L753 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L715-L753); the tagged copy in `publish_tagged`, [`src/tagged_payload.rs` L541-L548 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/tagged_payload.rs#L541-L548).
- After: `publish_wake` and `bump_delivery`, [`src/cells.rs` L55-L74 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L55-L74); the hot caller in `make_event`, [`src/lib.rs` L1052 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1052-L1052); the cold caller in `make_cold_delivery`, [`src/cold.rs` L694-L728 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cold.rs#L694-L728); the tagged caller in `publish_tagged`, [`src/tagged_payload.rs` L537 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/tagged_payload.rs#L537-L537).
- Proof: the loom tests below run these exact functions, and the full suite (98 tests) passes.
- In 0.1.0: `grep -n 'state.0.load' src/lib.rs src/cold.rs src/tagged_payload.rs` gave `Ordering::Acquire` at `src/lib.rs:886`, `src/lib.rs:944` and `src/tagged_payload.rs:523`, and `Ordering::Relaxed` at `src/cold.rs:546` and `src/cold.rs:628`.
- Not fixed: the hot `CrossbeamQueue` arm calling `deliver_crossbeam`, [`src/lib.rs` L1085 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1085-L1085); the tagged arm, [`src/tagged_payload.rs` L570-L574 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/tagged_payload.rs#L570-L574); the cold arm returning `CrossbeamSenderMissing` without looking for a sender, [`src/cold.rs` L729-L731 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cold.rs#L729-L731). `cargo run` in [`proofs/cold-crossbeam-sender`](proofs/cold-crossbeam-sender/) prints the same three lines after `0.1.0` and after `0.2.2`: `hot:  crossbeam_delivered 1 delivery 1`, `cold: cold_delivered 0 errors [CrossbeamSenderMissing] delivery 0`, `events in the channel: 1`.

#### Registering an observer allocates its counter block

`register` and `register_cold` now make one heap allocation per observer, for its `Arc<ObserverCells>`. Up to 0.2.1 the counters were part of the table entry. The block has to be separate because a handle keeps reading it after the observer is removed and its slot is reused.

- Before: the counters built inline in the hot entry, [`src/lib.rs` L562-L572 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L562-L572).
- After: the `Arc::new` in `ObserverCells::new`, [`src/cells.rs` L45-L53 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L45-L53); the hot entry calling it, [`src/lib.rs` L546-L553 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L546-L553); the cold entry calling it, [`src/cold.rs` L193-L200 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cold.rs#L193-L200).

#### Wake and delivery bumps are `Release`, not `AcqRel`

The runtime is the only writer of an observer's counters, so a bump has nothing to acquire. The pairing that matters is the `Release` bump against the handle's `Acquire` load.

- Before: the hot wake bump with `AcqRel`, [`src/lib.rs` L1076 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1076-L1076); the hot delivery bump with `AcqRel`, [`src/lib.rs` L1115 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1115-L1115); the cold wake and delivery bumps with `AcqRel`, [`src/cold.rs` L719-L751 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L719-L751).
- After: both bumps with `Release` in `ObserverCells`, [`src/cells.rs` L61-L74 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L61-L74).
- Proof: loom passes with `Release`, and fails when either bump goes down to `Relaxed` (next entry).

#### The loom tests run the real code

In 0.2.1 the loom tests never ran the runtime's code. `tests/loom_atomic_wake_tests.rs` made its own `loom::sync::atomic::AtomicU64`s and repeated the runtime's orderings by hand. The module in `src/lib.rs` used the real `CacheLineAtomicU64` type, but the orderings were typed into the test. A wrong ordering in `make_event` couldn't fail either of them. With the key store, the wake bump and the delivery bump in 0.2.1 all set to `Relaxed` ([the key store and wake bump](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1075-L1076), [the delivery bump](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1115-L1115)), all 5 loom tests still pass: `sh proofs/patched-loom-gate/run.sh v0.2.1 0.2.1-wake-and-delivery-relaxed.patch` prints `test result: ok. 3 passed` and `test result: ok. 2 passed`.

Now the module in `src/lib.rs` calls `ObserverCells::publish_wake`, `bump_delivery`, `mark_removed`, `observe`, `delivery_epoch` and `state`, and `tests/loom_atomic_wake_tests.rs` runs `ObservableWakeRuntime::publish` against an `ObserverHandle` on another loom thread. 0.2.1's five loom tests are gone. The five that replace them are three in `src/lib.rs` on `ObserverCells` and two in `tests/loom_atomic_wake_tests.rs` on the whole runtime.

- Before: the integration tests on their own `AtomicU64`s, [`tests/loom_atomic_wake_tests.rs` L1-L66 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/tests/loom_atomic_wake_tests.rs#L1-L66); the module in `src/lib.rs` with the orderings typed into the test, [`src/lib.rs` L1577-L1675 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1577-L1675).
- After: the module calling `ObserverCells`, [`src/lib.rs` L1555-L1643 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1555-L1643); the runtime against a handle on another loom thread, [`tests/loom_atomic_wake_tests.rs` L1-L63 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/loom_atomic_wake_tests.rs#L1-L63).
- Proof: with the wake bump in `publish_wake` set to `Relaxed` ([the wake bump](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L66-L66), patch `0.2.2-wake-bump-relaxed.patch`), [`a_reader_sees_the_key_of_its_epoch_or_a_later_one`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1575-L1599) fails with `ObservedWake { epoch: 1, key: None }`, and [`a_handle_reads_a_publish_on_another_thread`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/loom_atomic_wake_tests.rs#L17-L42) fails with `left: None` and `right: Some(WakeKey64 { .. })`. Both are a reader that saw epoch 1 and no key. With `bump_delivery` set to `Relaxed` ([the delivery bump](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L73-L73), patch `0.2.2-delivery-bump-relaxed.patch`), [`delivery_never_runs_ahead_of_wakes`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1601-L1621) fails with `1 deliveries but 0 wakes`. Both patches run with [`proofs/patched-loom-gate`](proofs/patched-loom-gate/). Unchanged, all 5 pass.
- In 0.1.0: `sh proofs/patched-loom-gate/run.sh 1a472db 0.1.0-runtime-orderings-relaxed.patch`, which sets all 22 orderings in the runtime's own code to `Relaxed` and leaves the tests alone, gave `test result: ok. 3 passed` and `test result: ok. 2 passed`.
- Introduced: 0.1.0, [`1a472db`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/1a472db9907b09585bd98d61262bc8bf08291713): the integration tests on their own atomics, [`tests/loom_atomic_wake_tests.rs` L1-L66 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/tests/loom_atomic_wake_tests.rs#L1-L66); the module with the orderings typed in, [`src/lib.rs` L1590-L1676 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L1590-L1676).

#### Publish loops stop loading `state`

Every hot, cold and tagged publish loaded each observer's `state` and skipped it unless it was `Active`. That check could never skip anything. `remove` and `remove_cold` store `Removed` and take the entry out of the table in the same call. An `.inactive()` observer was already turned away by `ObserverConfig::matches` in the hot and tagged loops, and by `ColdObserverConfig::key_matches` in the cold ones. So any entry still in the table read `Active`, and the check was one atomic load per observer per publish for nothing. Now only handles read `state`.

- Before: the check in `publish`, [`src/lib.rs` L886-L888 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L886-L888); in `publish_indexed`, [`src/lib.rs` L943-L945 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L943-L945); in `evaluate_cold`, [`src/cold.rs` L542-L548 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L542-L548); in `evaluate_cold_indexed`, [`src/cold.rs` L627-L629 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L627-L629); in `publish_tagged`, [`src/tagged_payload.rs` L523-L525 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/tagged_payload.rs#L523-L525). Why it never fired: `remove` storing `Removed` and emptying the slot, [`src/lib.rs` L849-L851 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L849-L851); `remove_cold` doing the same, [`src/cold.rs` L373-L375 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L373-L375); `ObserverConfig::matches` refusing an inactive observer, [`src/lib.rs` L525-L528 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L525-L528); `ColdObserverConfig::key_matches` doing the same, [`src/cold.rs` L169-L172 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L169-L172).
- After: the comment on `ObserverCells` saying why the loops don't read `state`, [`src/cells.rs` L31-L36 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L31-L36); `publish` going from the slot straight to `config.matches`, [`src/lib.rs` L865-L878 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L865-L878).
- Proof: [`inactive_observer_config_does_not_match`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1205-L1211) and [`an_inactive_observer_reads_as_inactive`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L156-L166) (an inactive observer is never woken), [`a_handle_outlives_removal_and_stops_moving`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L58-L77) (a removed observer's counts stop moving).

#### `scripts/miri_gate.sh` runs the handle tests across 8 schedules

The handle tests use real threads, and Miri reports data races on anything they share. They now run a second time with `-Zmiri-many-seeds=0..8`, so Miri tries 8 thread schedules instead of one.

- Before: the gate with one run of the tests, [`scripts/miri_gate.sh` L1-L10 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/scripts/miri_gate.sh#L1-L10).
- After: the gate with the second run of `observer_handle_tests` over 8 seeds, [`scripts/miri_gate.sh` L1-L12 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/scripts/miri_gate.sh#L1-L12).
- Proof: the `miri` job in the [v0.2.2 run](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38677659061/job/116234827614), and locally on `c520dd9`: seeds 0 through 7, 11 passed each, exit 0.

#### Speed

Measured with [`proofs/bench-0.2.1-vs-0.2.2`](proofs/bench-0.2.1-vs-0.2.2/): the crate's criterion benches on tag `v0.2.1` and tag `v0.2.2`, one after the other, 3 rounds, each case's median of the 3. The hot groups are [`benches/observable_runtime_benches.rs` L197-L295 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/benches/observable_runtime_benches.rs#L197-L295). On an Apple M4 Pro, 40 of the 51 cases came out within 5% either way. The rest:

| case | 0.2.1 ns | 0.2.2 ns | change |
| --- | ---: | ---: | ---: |
| `hot_bitpacked_publish_atomic/512` | 1766.3 | 1106.6 | -37.3% |
| `hot_bitpacked_publish_scan_sparse/2048` | 5147.6 | 3591.2 | -30.2% |
| `tagged_payload_publish/512` | 2320.5 | 1852.6 | -20.2% |
| `hot_bitpacked_publish_atomic/2048` | 7565.0 | 6320.0 | -16.5% |
| `hot_bitpacked_publish_scan_dense/512` | 2572.6 | 2316.4 | -10.0% |
| `hot_bitpacked_publish_auto_dense/512` | 2536.5 | 2333.2 | -8.0% |
| `hot_bitpacked_publish_scan_sparse/512` | 733.7 | 686.9 | -6.4% |
| `cold_state_predicates_auto_dense/64` | 173.1 | 163.8 | -5.4% |
| `hot_bitpacked_publish_scan_sparse/64` | 97.0 | 91.9 | -5.3% |
| `hot_bitpacked_publish_auto_dense/2048` | 11420.4 | 11997.5 | +5.1% |
| `baseline_arc_mutex_vec_scan/2048` | 723.0 | 834.4 | +15.4% |

`baseline_arc_mutex_vec_scan` runs no code from this crate, so its 15% is not the crate. A single case can move that much on this machine, and the changes near 5% are inside that. The large drops are steady across the rounds: `hot_bitpacked_publish_atomic/512` ran 1743, 1766 and 1783 ns on 0.2.1 and 1107, 1125 and 1079 ns on 0.2.2.

The scan loops walk the observer table. Each entry in it used to hold four 64-byte-aligned atomics inline ([the `CacheLineAtomicU64` fields of `ObserverEntry` @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L538-L560)); now it holds one pointer ([`ObserverEntry` with `cells: Arc<ObserverCells>` @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L540-L544)).

The speed line in `README.md` at 0.2.2 ([`README.md` L82-L85 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/README.md#L82-L85)) came from an earlier run and doesn't match this one. It says the dense and sparse hot scans of 2048 observers got 4% to 6% slower. Here `hot_bitpacked_publish_auto_dense/2048` is +5.1%, `hot_bitpacked_publish_scan_dense/2048` is +3.5%, `hot_bitpacked_publish_auto_sparse/2048` is +1.7%, and `hot_bitpacked_publish_scan_sparse/2048` is 30% faster.

### Fixed

#### Crossbeam: a failed send no longer counts as delivered

`deliver_crossbeam` bumped `delivery_epoch` first, then looked for the sender, then sent. With no sender (`WakeError::CrossbeamSenderMissing`) or a dropped receiver (`WakeError::CrossbeamSendFailed`), `publish` reported the error and the delivery count went up anyway. Now the bump comes after `send` returns `Ok`. With handles on other threads the order matters a second way: a handle that sees delivery `n` can already receive event `n`.

- Before: `deliver_crossbeam` bumping the count before it looks for the sender, [`src/lib.rs` L1118-L1128 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1118-L1128).
- After: the bump after `send` returns `Ok`, [`src/lib.rs` L1093-L1106 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1093-L1106).
- Proof: [`a_failed_send_is_not_counted_as_delivered`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L214-L229) asserts `(wake_epoch, delivery_epoch) == (1, 0)` for both cases. [`a_counted_delivery_is_already_in_the_channel`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L231-L258) has a reader spin on `delivery_epoch` across 50,000 publishes and `try_recv` every counted delivery. Loom can't check this one because the channel's atomics aren't loom's.
- In 0.1.0: `cargo run` in [`proofs/crossbeam-failed-send`](proofs/crossbeam-failed-send/), which runs the same two cases on 0.1.0 and then 0.2.2 and reads the counts through `observer_epoch` and `observer_delivery_epoch`, gave:

  ```
  0.1.0 no sender:   errors [CrossbeamSenderMissing] wake 1 delivery 1
  0.1.0 no receiver: errors [CrossbeamSendFailed] wake 1 delivery 1
  0.2.2 no sender:   errors [CrossbeamSenderMissing] wake 1 delivery 0
  0.2.2 no receiver: errors [CrossbeamSendFailed] wake 1 delivery 0
  ```
- Introduced: 0.1.0, `deliver_crossbeam` as first written, [`src/lib.rs` L1119-L1129 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L1119-L1129).
## 0.2.1 (2026-10-15 01:48 UTC)

From tag `v0.2.1`, commit [`f77ef14`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb). The first version published by the CI `publish` job ([run 38667418389](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389)). Two commits since 0.2.0: [`0df7915`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/0df7915291b2f632ea69dcdb9aee403b31f8dec6) added CI and fixed what it turned up, and [`f77ef14`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb) tightened the CI. `0df7915` was never published on its own. The runtime behaves the same as 0.2.0.

### Added

#### CI, and publishing only from a `v*` tag

`.github/workflows/ci.yml` runs on every push to `main`, every `v*` tag and every pull request:

* `test`: all features, doc tests, clippy `-D warnings`, no features and each feature alone (clippy and tests), rustdoc `-D warnings`, every example, the bench build, and `cargo package` the way publish builds it.
* `msrv`: Rust 1.88, with all features and with none.
* `miri`: `scripts/miri_gate.sh`.
* `loom`: `scripts/loom_gate.sh`, and clippy under `--cfg loom`.
* `latest-deps`: below.
* `publish`: only on a `v*` tag, only after `test`, `msrv`, `miri` and `loom` pass. It checks the tag matches the version in `Cargo.toml`, gets a crates.io token from `rust-lang/crates-io-auth-action`, and runs `cargo publish --locked`.

0.1.0 had no CI: commit `1a472db` has no `.github/` folder.

- Where: the workflow at 0.2.1, [`.github/workflows/ci.yml` L1-L110 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L1-L110); the first version, [`.github/workflows/ci.yml` L1-L86 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/.github/workflows/ci.yml#L1-L86).
- Proof: every run is green: [`0df7915`](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38665968778), [`f77ef14` on main](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667250912), [tag `v0.2.1`](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389). In the tag run the [`publish` job](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389/job/116202821258) ran from 01:47:31 to 01:48:08 UTC, and crates.io has 0.2.1 at 01:48:04.

#### `scripts/miri_gate.sh`

Runs the whole test suite and every example under Miri with `-Zmiri-strict-provenance -Zmiri-disable-isolation` (some tests read the clock and write temp files). CI and the README both call the script, so there's one command.

- Where: the script, [`scripts/miri_gate.sh` L1-L10 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/scripts/miri_gate.sh#L1-L10).
- Proof: the `miri` job in the [tag `v0.2.1` run](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389).

#### Weekly run against the newest dependencies

`Cargo.lock` is committed and every other job runs `--locked`, so CI only tested the pinned versions. Someone adding the crate gets the newest compatible versions instead. `latest-deps` runs `cargo update` and then the tests on stable and on 1.88, weekly and on every push. It's not a publish gate, so a third-party release can't block a tag; it shows up here instead.

- Where: the weekly schedule, [`.github/workflows/ci.yml` L8-L10 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L8-L10); the `latest-deps` job, [`.github/workflows/ci.yml` L76-L90 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L76-L90).

### Changed

#### The publish job's actions are pinned to commit SHAs

The `publish` job has `id-token: write`, so it can get a crates.io token. `actions/checkout@v4` and `rust-lang/crates-io-auth-action@v1` are tags, and tags can be moved. A moved tag would run different code in a job that can publish. Both are pinned to commit SHAs now, and the `dtolnay/rust-toolchain` step is gone from that job; it uses the runner's stable cargo.

- Before: the `publish` job on `@v4` and `@v1` tags, with `dtolnay/rust-toolchain`, [`.github/workflows/ci.yml` L68-L86 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/.github/workflows/ci.yml#L68-L86).
- After: the same job on commit SHAs, without the toolchain step, [`.github/workflows/ci.yml` L92-L110 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L92-L110).
- Introduced: [`0df7915`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/0df7915291b2f632ea69dcdb9aee403b31f8dec6). 0.1.0 had no CI. Its CI run [skipped `publish`](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38665968778/job/116198468891) (no tag), so the unpinned refs never ran with a token.

#### More of the build is linted

In `0df7915`, clippy only ran with `--all-features`, the per-feature loop only ran `cargo test` without `--all-targets`, MSRV only built with all features, and the loom build wasn't linted. Now clippy and the tests run with no features and with each feature alone over all targets, MSRV also runs with no features, `scripts/loom_gate.sh` runs with `-D warnings --locked`, and the `loom` job runs clippy under `--cfg loom`. Run against `0df7915`, clippy `-D warnings` with no features and with each feature alone is clean, so that part closed a gap and didn't hide a failure. The loom part did find something (Fixed, below). The per-feature loop also dropped `loom-tests`: without `--cfg loom` that feature builds nothing, since `tests/loom_atomic_wake_tests.rs` is `#![cfg(loom)]`.

- Before: the `test`, `msrv` and `loom` jobs as `0df7915` wrote them, [`.github/workflows/ci.yml` L16-L66 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/.github/workflows/ci.yml#L16-L66).
- After: the same jobs at 0.2.1, [`.github/workflows/ci.yml` L19-L74 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L19-L74); the loom gate with `-D warnings --locked`, [`scripts/loom_gate.sh` L8-L9 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/scripts/loom_gate.sh#L8-L9).
- Introduced: [`0df7915`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/0df7915291b2f632ea69dcdb9aee403b31f8dec6), the first CI.

#### Clippy cleanups

`for &id in &candidates` instead of `candidates[index]` in `ObservableWakeRuntime::publish_indexed` and `ColdObservableRuntime::evaluate_cold_indexed`, and `#[derive(Default)]` on `ProtocolBridgeReport` instead of a hand-written impl. Same behavior. In the tests, `ServiceEvent` in `tests/cold_state_tests.rs` gets `#[allow(clippy::enum_variant_names)]`, since every variant starts with `Set`. `cargo clippy --all-features --all-targets -- -D warnings` on `4a951d6` stops in the library with 15 errors: the 12 `unexpected_cfgs` ones (Fixed, below), `needless_range_loop` at `src/lib.rs:931` and `src/cold.rs:615`, and `derivable_impls` at `src/protocol_bridge.rs:131`. With those three lints allowed, it stops at `tests/cold_state_tests.rs:16` with ``all variants have the same prefix: `Set` ``.

- Before: the indexed loop in `publish_indexed`, [`src/lib.rs` L930-L933 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/lib.rs#L930-L933); the same loop in `evaluate_cold_indexed`, [`src/cold.rs` L613-L617 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/cold.rs#L613-L617); the hand-written `Default` for `ProtocolBridgeReport`, [`src/protocol_bridge.rs` L121-L142 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/protocol_bridge.rs#L121-L142); `ServiceEvent` without the allow, [`tests/cold_state_tests.rs` L15-L20 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/tests/cold_state_tests.rs#L15-L20).
- After: `for &id in &candidates` in `publish_indexed`, [`src/lib.rs` L930-L932 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/lib.rs#L930-L932); the same in `evaluate_cold_indexed`, [`src/cold.rs` L613-L616 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/cold.rs#L613-L616); `#[derive(Default)]`, [`src/protocol_bridge.rs` L121-L130 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/protocol_bridge.rs#L121-L130); the allow on `ServiceEvent`, [`tests/cold_state_tests.rs` L15-L21 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/tests/cold_state_tests.rs#L15-L21).

### Fixed

#### Broken and redundant doc links

The docs on `ColdObservableRuntime::publish_stateful_auto` linked `` [`DispatchPolicy`] ``, which isn't in scope in `cold.rs`, so the link didn't resolve. The module docs in `cold.rs` and `tagged_payload.rs` also spelled out targets rustdoc already resolves. `RUSTDOCFLAGS="-D warnings -A unexpected_cfgs" cargo doc --no-deps --all-features` on `4a951d6` fails with ``unresolved link to `DispatchPolicy` `` at `src/cold.rs:478` and 5 `redundant explicit link target` errors, at `src/cold.rs` lines 3, 4 and 16 and `src/tagged_payload.rs` lines 5 and 6. Without `-A unexpected_cfgs`, rustdoc stops first on the 12 ``unexpected `cfg` condition name: `loom` `` errors from the next entry.

- Before: the `DispatchPolicy` link that doesn't resolve, [`src/cold.rs` L474-L479 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/cold.rs#L474-L479); the `cold.rs` module docs with explicit targets, [`src/cold.rs` L1-L17 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/cold.rs#L1-L17); the `tagged_payload.rs` module docs with explicit targets, [`src/tagged_payload.rs` L1-L7 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/tagged_payload.rs#L1-L7).
- After: the link as `crate::DispatchPolicy`, [`src/cold.rs` L474-L479 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/cold.rs#L474-L479); the `cold.rs` module docs without the targets, [`src/cold.rs` L1-L17 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/cold.rs#L1-L17); the `tagged_payload.rs` module docs without them, [`src/tagged_payload.rs` L1-L7 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/tagged_payload.rs#L1-L7).
- Proof: the rustdoc step with `RUSTDOCFLAGS: -D warnings`, [`.github/workflows/ci.yml` L37-L39 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L37-L39), passes in the [tag `v0.2.1` run](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389).
- In 0.1.0: the same command gave the same 6 errors at the same lines.
- Introduced: 0.1.0, [`1a472db`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/1a472db9907b09585bd98d61262bc8bf08291713).

#### `cfg(loom)` is declared

Every `#[cfg(loom)]` raised Rust's `unexpected_cfgs` warning, because nothing declared `loom` as a cfg name. Clippy `-D warnings` on `4a951d6` fails with 12 of them across `src/lib.rs`, `src/sync.rs` and `src/cold.rs`. `Cargo.toml` declares it under `[lints.rust]` now.

- Before: the `loom` dependency under `cfg(loom)` and no `[lints]` table, [`Cargo.toml` L33-L39 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/Cargo.toml#L33-L39).
- After: `check-cfg = ['cfg(loom)']` under `[lints.rust]`, [`Cargo.toml` L38-L40 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/Cargo.toml#L38-L40).
- In 0.1.0: `cargo clippy --all-features -- -D warnings` gave the same 12 ``unexpected `cfg` condition name: `loom` `` errors: 4 in `src/lib.rs`, 6 in `src/sync.rs` and 2 in `src/cold.rs`.
- Introduced: 0.1.0, [`1a472db`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/1a472db9907b09585bd98d61262bc8bf08291713).

#### The loom module in `src/lib.rs` only builds in test builds

`mod loom_wake_tests` was `#[cfg(loom)]`. When the library builds under `--cfg loom` without `test`, which is how it builds for an integration test, the `#[test]` functions drop out and leave three unused `use` lines and an unused `WAKE_KEY`. Nothing denied warnings in the loom build yet, so it went by. It's `#[cfg(all(test, loom))]` now. With the module gone from that build, the loom-side `Arc` and `thread` re-exports in `src/sync.rs` are unused too, so they get `#[allow(unused_imports)]` like the std side already had.

Both halves are checked with [`proofs/patched-loom-gate`](proofs/patched-loom-gate/). `sh proofs/patched-loom-gate/run.sh 0df7915 0df7915-with-the-0.2.1-loom-gate.patch` runs `0df7915` under the loom gate the way `f77ef14` writes it (`-D warnings --locked`), and the build stops with 4 errors in `src/lib.rs`: unused imports `super::CacheLineAtomicU64`, `crate::sync::atomic::Ordering` and `Arc` and `thread`, and ``constant `WAKE_KEY` is never used``. `sh proofs/patched-loom-gate/run.sh v0.2.1 0.2.1-without-the-loom-allows.patch` takes the two new allows back out, and the build stops with ``unused import: `loom::sync::Arc` `` and ``unused import: `loom::thread` ``.

- Before: the module under `#[cfg(loom)]` with its `use` lines and `WAKE_KEY`, [`src/lib.rs` L1589-L1595 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/lib.rs#L1589-L1595); the loom-side re-exports in `src/sync.rs` without allows, [`src/sync.rs` L17-L27 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/sync.rs#L17-L27).
- After: `#[cfg(all(test, loom))]`, [`src/lib.rs` L1589-L1590 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1589-L1590); the re-exports with `#[allow(unused_imports)]`, [`src/sync.rs` L17-L29 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/sync.rs#L17-L29).
- Proof: the [`loom` job](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389/job/116202421517) in the tag run, which runs the gate with `-D warnings` and clippy under `--cfg loom`.
- In 0.1.0: `RUSTFLAGS="--cfg loom" cargo build --all-features --tests` gave 16 warnings for the library: the 12 ``unexpected `cfg` `` ones, ``unused import: `super::CacheLineAtomicU64` `` at `src/lib.rs:1592`, ``unused import: `crate::sync::atomic::Ordering` `` at `src/lib.rs:1593`, ``unused imports: `Arc` and `thread` `` at `src/lib.rs:1594`, and ``constant `WAKE_KEY` is never used`` at `src/lib.rs:1596`. It exits 0, and 0.1.0 had no CI to deny them.
- Introduced: 0.1.0, the module as first written, [`src/lib.rs` L1590-L1591 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L1590-L1591).

#### README install line

The 0.2.0 commit bumped the version but left the README at `ob-serve-a-bowl = "0.1"`, so the 0.2.0 page on crates.io told people to add 0.1. It says `"0.2"` now.

- Before: `ob-serve-a-bowl = "0.1"` at 0.2.0, [`README.md` L19-L22 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/README.md#L19-L22).
- After: `ob-serve-a-bowl = "0.2"`, [`README.md` L19-L22 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/README.md#L19-L22).
- In 0.1.0: not there. `grep -n 'ob-serve-a-bowl =' README.md` gave `21:ob-serve-a-bowl = "0.1"`, and `Cargo.toml` has `version = "0.1.0"`.
- Introduced: [`4a951d6`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/4a951d650d987fc9ffdca2b9cf7ace9401e4714d) (0.2.0), which changed `Cargo.toml` and not this line.

## 0.2.0 (2026-10-15 01:24 UTC)

From commit [`4a951d6`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/4a951d650d987fc9ffdca2b9cf7ace9401e4714d); the `.cargo_vcs_info.json` in the published crate names it. Published by hand (crates.io lists `jameshuntt`). Breaking for anyone on the `protocol` feature.

### Security

#### `protocol` moves to `waker_protocol` 0.3, which checks a message's task type

In `waker_protocol` 0.2 a message written for one task type could run against a task of another type. `typed_envelope::<T, M>` builds a message whose `dispatch` casts the task pointer to `*mut T` and checks nothing. The scheduler puts any `SchedulerMsg::Deliver` into the target task's mailbox, and `poll_one` dispatches every envelope in that mailbox into the task's raw pointer. `WakerProtocol::tasks` and `TaskEntry::mailbox` were public too, so safe code could push an envelope straight into another task's mailbox. The tasks themselves are `unsafe impl ProtocolTask`, as every task has to be, but getting the wrong message to one took no `unsafe`. 0.3 makes `Envelope` an `unsafe trait` with `target_type()`, and the scheduler refuses an envelope whose `target_type()` isn't the live task's type.

This crate's own code never dispatches anything; the hole was in the dependency. But the `protocol` feature brought it in from 0.1.0 on, and `WakeEvent::to_protocol_deliver_msg` builds exactly that `SchedulerMsg::Deliver`, from an observer's target and a caller's envelope.

- Before: `waker_protocol = "0.2"`, [`Cargo.toml` L27 @ b96530f](https://github.com/jameshuntt/ob-serve-a-bowl/blob/b96530fd1b86ce39956efef0b387856edd9399f1/Cargo.toml#L27-L27); the bridge's deliver message, [`src/protocol_bridge.rs` L93-L108 @ b96530f](https://github.com/jameshuntt/ob-serve-a-bowl/blob/b96530fd1b86ce39956efef0b387856edd9399f1/src/protocol_bridge.rs#L93-L108). In `waker_protocol` 0.2.0: the unchecked cast, [waker_protocol `src/envelope.rs` L29-L33 @ 23872ee](https://github.com/jameshuntt/waker_protocol/blob/23872ee6f934867c5bc0f79357128dad77531d94/src/envelope.rs#L29-L33); any `Deliver` into the mailbox, [waker_protocol `src/scheduler.rs` L345-L353 @ 23872ee](https://github.com/jameshuntt/waker_protocol/blob/23872ee6f934867c5bc0f79357128dad77531d94/src/scheduler.rs#L345-L353); the dispatch, [waker_protocol `src/scheduler.rs` L604-L606 @ 23872ee](https://github.com/jameshuntt/waker_protocol/blob/23872ee6f934867c5bc0f79357128dad77531d94/src/scheduler.rs#L604-L606).
- After: `waker_protocol = "0.3"`, [`Cargo.toml` L28 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/Cargo.toml#L28-L28). In `waker_protocol` 0.3.0: the `unsafe trait Envelope` with `target_type()`, [waker_protocol `src/envelope.rs` L1-L20 @ 1ca72a5](https://github.com/jameshuntt/waker_protocol/blob/1ca72a5803549914aa822c42cbb91705e267875f/src/envelope.rs#L1-L20); the scheduler refusing a wrong type, [waker_protocol `src/scheduler.rs` L669-L681 @ 1ca72a5](https://github.com/jameshuntt/waker_protocol/blob/1ca72a5803549914aa822c42cbb91705e267875f/src/scheduler.rs#L669-L681).
- Proof: [`proofs/protocol-wrong-type`](proofs/protocol-wrong-type/) spawns a `Small(u8)` task, builds a `typed_envelope::<Big, u64>` (`Big` holds a `Vec<u64>`) through `WakeEvent::to_protocol_deliver_msg` with the `Small` task as target, pushes it into that task's public `mailbox` and calls `run_once`. On the 0.3 side, waker_protocol's [`a_message_for_another_type_is_refused_not_delivered`](https://github.com/jameshuntt/waker_protocol/blob/1ca72a5803549914aa822c42cbb91705e267875f/tests/scheduler.rs#L174-L188) sends a message written for `Other` to a `Counter` and gets the refusal; run locally from the 0.3.0 package, it passes.
- In 0.1.0: `cargo +nightly miri run` in `proofs/protocol-wrong-type`, pinned to 0.1.0 and `waker_protocol` 0.2.0, gave `Undefined Behavior: constructing invalid value of type &mut Big: encountered a dangling reference (going beyond the bounds of its allocation)` at `waker_protocol-0.2.0/src/envelope.rs:31:29`.
- Introduced: 0.1.0, `waker_protocol = "0.2"`, [`Cargo.toml` L27 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/Cargo.toml#L27-L27).

### Changed

#### `protocol` uses `waker_protocol` 0.3's types

`TaskId`, `WakeReason` and `SchedulerMsg` in the bridge are 0.3's types now. An `Envelope` you write yourself has to change in three places, the way the crate's own test envelope did: `unsafe impl Envelope`, a new `fn target_type(&self) -> TypeId`, and `unsafe fn dispatch`. 0.1.0's own test file shows the break. On a checkout of `4a951d6`, `git show 1a472db:tests/protocol_feature_tests.rs > tests/protocol_feature_tests.rs` and then `cargo test --all-features --test protocol_feature_tests` gives ``error[E0200]: the trait `Envelope` requires an `unsafe impl` declaration`` at `tests/protocol_feature_tests.rs:155`.

- Before: the test's `NoopEnvelope` on 0.2's `Envelope`, [`tests/protocol_feature_tests.rs` L153-L163 @ b96530f](https://github.com/jameshuntt/ob-serve-a-bowl/blob/b96530fd1b86ce39956efef0b387856edd9399f1/tests/protocol_feature_tests.rs#L153-L163).
- After: the same envelope with the three changes, [`tests/protocol_feature_tests.rs` L153-L168 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/tests/protocol_feature_tests.rs#L153-L168).
- Proof: [`wake_event_can_build_protocol_deliver_message_with_caller_envelope`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/tests/protocol_feature_tests.rs#L170-L193) builds a deliver message with that envelope. Run locally on `4a951d6`, `cargo test --all-features` passes 86.

#### Minimum Rust 1.88

`Cargo.toml` declares `rust-version = "1.88"`. With `tagged-payloads`, `petr-core` 0.1 uses `let` chains (`if let ... && ...`), which need 1.88. Without the field, an older toolchain fails inside a dependency instead of saying which Rust it needs. With it, `cargo +1.85 check --locked` on `4a951d6` refuses up front: `rustc 1.85.1 is not supported by the following package: ob-serve-a-bowl@0.2.0 requires rustc 1.88`. The refusal also covers builds without `tagged-payloads`, which worked on 1.85 before: `cargo +1.85 check --locked` on `1a472db` finishes.

- Where: `rust-version = "1.88"`, [`Cargo.toml` L5 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/Cargo.toml#L5-L5).
- Proof: the [`msrv` job](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389/job/116202421297) builds and tests on 1.88, from 0.2.1 on.
- In 0.1.0: `cargo +1.85 check --locked --features tagged-payloads` gave ``error[E0658]: `let` expressions in this position are unstable`` at `petr-core-0.1.0/src/keyed.rs:154:12`.

## 0.1.1 (2026-10-15 00:52 UTC)

Not published from this repo. The `.crate` on crates.io has no `.cargo_vcs_info.json`, and its `Cargo.toml.orig` is cargo's normalized manifest (it opens with `# THIS FILE IS AUTOMATICALLY GENERATED BY CARGO`), so it was packaged from an unpacked package directory. Every other file in it (sources, tests, examples, benches, README, both licenses and `Cargo.lock`) matches commit [`b96530f`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/b96530fd1b86ce39956efef0b387856edd9399f1) byte for byte, and that commit is the same change. Published by hand (crates.io lists `jameshuntt`).

### Fixed

#### The Apache license names its copyright holder

`LICENSE-APACHE` still had the template line `Copyright [yyyy] [name of copyright owner]`. It's `Copyright 2026 1o LLC` now. No code changed.

- Before: the template line, [`LICENSE-APACHE` L190 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/LICENSE-APACHE#L190-L190).
- After: `Copyright 2026 1o LLC`, [`LICENSE-APACHE` L190 @ b96530f](https://github.com/jameshuntt/ob-serve-a-bowl/blob/b96530fd1b86ce39956efef0b387856edd9399f1/LICENSE-APACHE#L190-L190).
- In 0.1.0: `sed -n 190p LICENSE-APACHE` gave `Copyright [yyyy] [name of copyright owner]`.
- Introduced: 0.1.0, [`1a472db`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/1a472db9907b09585bd98d61262bc8bf08291713).

## 0.1.0 (2026-10-13 01:17 UTC)

The first release, from commit [`1a472db`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/1a472db9907b09585bd98d61262bc8bf08291713); the `.cargo_vcs_info.json` in the published crate names it. Published by hand (crates.io lists `jameshuntt`).

### Added

- `WakeKey64`: a bit-packed 64-bit wake key (reason, profile, QoS, field mask, phase, flags) that the hot path matches on. [`src/lib.rs` L281-L332 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L281-L332)
- `ObservableWakeRuntime`: hot observers matched by interest mask and `FieldMatcher`s, with `publish` (scan every observer), `publish_indexed` (field-mask index) and `publish_auto` (picks one by `DispatchPolicy`). [`src/lib.rs` L737-L755 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L737-L755), [`src/lib.rs` L876-L992 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L876-L992), [`src/config.rs` L6-L29 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/config.rs#L6-L29)
- Delivery chosen per observer by `WakeProfileCode`: `InlineQueue`, `AtomicEpoch`, `CrossbeamQueue` (feature `crossbeam-wake`), `ConditionCache`, `Manual`. [`src/lib.rs` L108-L129 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L108-L129)
- `ColdObservableRuntime`: materialized state (`ObservableState`) plus cold predicates that wake only when the cached state crosses a condition, re-armed by `ColdRearmPolicy` (`EveryMatch` or `RisingEdge`). [`src/cold.rs` L72-L80 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/cold.rs#L72-L80), [`src/cold.rs` L287-L299 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/cold.rs#L287-L299)
- `ObservableEventLog`: a replayable log of the observable boundary, with JSON and JSONL under `serialization`. [`src/event_log.rs` L207-L212 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/event_log.rs#L207-L212)
- `tagged-payloads`: `TaggedPayloadRef` and `TaggedPayloadArena` on `petr-core` and `petr-collections` keyed arenas, resolved only after an observer has matched. [`src/tagged_payload.rs` L24-L32 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/tagged_payload.rs#L24-L32)
- `protocol`: a bridge from wake events to `waker_protocol` 0.2 scheduler messages. [`src/protocol_bridge.rs` L1-L292 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/protocol_bridge.rs#L1-L292)
- 87 tests (24 unit, 63 integration), 5 loom tests behind `scripts/loom_gate.sh`, 5 examples and a criterion bench. Run locally on `1a472db`, `cargo test --all-features` passes 86: the unit test `crossbeam_profile_without_feature_is_reported` only runs without `crossbeam-wake`.
