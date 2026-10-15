# Changelog

Every link here points at exact lines at an exact commit, so each entry shows the code it's about. "Before" is the code as it was, "After" is the change, "Proof" is the test or the run that shows it, and "Introduced" is the commit the problem came in with. Dates are when crates.io got the version, in UTC.

## 0.2.2 (2026-10-15 03:44 UTC)

From tag `v0.2.2`, commit [`c520dd9`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/c520dd95be69a55593519a2226e9968b0725b62c). Published by the CI `publish` job after `test`, `msrv`, `miri` and `loom` passed ([run 38677659061](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38677659061)).

### Added

#### `ObserverHandle`: read an observer's wakes from any thread

Up to 0.2.1 an observer's four counters (`state`, `wake_epoch`, `delivery_epoch`, `last_key`) sat inline in its `ObserverEntry`, inside the runtime's observer table. The only way to read them was `observer_epoch`, `observer_delivery_epoch` and `observer_last_key`, which borrow the runtime, and `publish` takes `&mut self`. So no other thread could read an observer while the runtime was publishing, even though the counters were atomics and the key was already stored with `Release` for "external readers".

Now each observer's counters live in one shared `Arc<ObserverCells>`. `ObservableWakeRuntime::observer_handle(id)` and `ColdObservableRuntime::cold_observer_handle(id)` hand out an `ObserverHandle` holding a clone of that `Arc`. It's `Clone + Send + Sync` ([`handles_are_cloneable_send_and_sync`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L52-L56)). `observe()` returns `ObservedWake { epoch, key }`, reading the epoch first and then the key, so `key` is the key of wake `epoch` or of a later one, never an earlier one. The handle holds its own block, so it never reads a different observer that later reuses the slot.

- Before: counters inline in the entry, [`src/lib.rs` L553-L560 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L553-L560); the readers that borrow the runtime, [`src/lib.rs` L857-L874 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L857-L874); the key stored for external readers, [`src/lib.rs` L1069-L1076 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1069-L1076).
- After: `ObserverCells`, `ObservedWake` and `ObserverHandle`, [`src/cells.rs` L31-L213 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L31-L213); `observer_handle`, [`src/lib.rs` L859-L863 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L859-L863); `cold_observer_handle`, [`src/cold.rs` L382-L386 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cold.rs#L382-L386).
- Proof: [`a_reader_thread_follows_every_wake_in_order`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L14-L50) publishes 500 wakes while a reader thread checks every `observe()`: the epoch never goes back, the key is never from an earlier wake, and `delivery_epoch` never passes `wake_epoch`. [`a_handle_outlives_removal_and_stops_moving`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L58-L77) removes the observer, registers a new one into the same slot and publishes; the old handle still reads epoch 1 with the old key. [`a_new_handle_reports_no_wake_yet`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L79-L91) reads `epoch: 0, key: None` before any wake. [`a_cold_observer_has_a_handle_too`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L123-L154) does the same through the cold runtime. Under loom, [`a_handle_reads_a_publish_on_another_thread`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/loom_atomic_wake_tests.rs#L17-L42) and [`an_old_handle_never_sees_the_observer_that_reuses_its_slot`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/loom_atomic_wake_tests.rs#L44-L63) run the real `ObservableWakeRuntime` on one thread against a handle on another. Under Miri the handle tests also run across 8 thread schedules, [`scripts/miri_gate.sh` L9 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/scripts/miri_gate.sh#L9-L9). Run locally on `c520dd9`: `cargo test --all-features` passes 98, `scripts/loom_gate.sh` passes 5, `scripts/miri_gate.sh` exits 0 with seeds 0 through 7.

#### Handles see `Inactive` and `Removed`; dropping a runtime marks its observers removed

`ObserverCells::new(config.active)` starts an observer registered with `.inactive()` as `ObserverState::Inactive`. Before, every entry's `state` started as `Active` whatever the config said. Nothing outside the runtime could read `state` then, so it never showed, but a handle would have shown it. A handle can also outlive its runtime, so `ObservableWakeRuntime` and `ColdObservableRuntime` now implement `Drop` and mark every observer they still hold `Removed`. Without that, a handle on a dropped runtime would keep reading `Active`.

- Before: [`src/lib.rs` L562-L572 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L562-L572), [`src/cold.rs` L211-L222 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L211-L222).
- After: [`src/cells.rs` L44-L53 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L44-L53), [`src/lib.rs` L738-L745 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L738-L745), [`src/cold.rs` L280-L288 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cold.rs#L280-L288).
- Proof: [`an_inactive_observer_reads_as_inactive`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L156-L166), [`dropping_the_runtime_marks_its_observers_removed`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L168-L177), [`cold_handles_report_inactive_and_removed_on_drop`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L188-L199).

### Changed

#### Hot, cold and tagged publishing go through one counter block

The same four atomics and the same "store the key, then bump the epoch" steps were written out three times: `CacheLineAtomicU64` and `ObserverEntry` in `lib.rs`, `ColdAtomicU64` and `ColdObserverEntry` in `cold.rs`, and the tagged-payload publish writing the hot entry's atomics by hand. The copies had already drifted apart: the `state` check loaded with `Acquire` in the hot and tagged loops and with `Relaxed` in the cold ones. Now all three call `ObserverCells::publish_wake` and `bump_delivery`, so the order and the orderings are written once.

- Before: [`src/lib.rs` L538-L572 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L538-L572) and [`src/lib.rs` L1069-L1076 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1069-L1076); [`src/cold.rs` L187-L222 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L187-L222) and [`src/cold.rs` L715-L753 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L715-L753); [`src/tagged_payload.rs` L541-L548 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/tagged_payload.rs#L541-L548).
- After: [`src/cells.rs` L55-L74 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L55-L74); the callers, [`src/lib.rs` L1052 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1052-L1052), [`src/cold.rs` L694-L728 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cold.rs#L694-L728), [`src/tagged_payload.rs` L537 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/tagged_payload.rs#L537-L537).
- Proof: the loom tests below run these exact functions, and the full suite (98 tests) passes.

#### Wake and delivery bumps are `Release`, not `AcqRel`

The runtime is the only writer of an observer's counters, so a bump has nothing to acquire. The pairing that matters is the `Release` bump against the handle's `Acquire` load.

- Before: `fetch_add(1, Ordering::AcqRel)` at [`src/lib.rs` L1076 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1076-L1076), [`src/lib.rs` L1115 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1115-L1115) and [`src/cold.rs` L719-L751 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L719-L751).
- After: [`src/cells.rs` L61-L74 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L61-L74).
- Proof: loom passes with `Release`, and fails when either bump goes down to `Relaxed` (next entry).

#### The loom tests run the real code

In 0.2.1 the loom tests never ran the runtime's code. `tests/loom_atomic_wake_tests.rs` made its own `loom::sync::atomic::AtomicU64`s and repeated the runtime's orderings by hand. The module in `src/lib.rs` used the real `CacheLineAtomicU64` type, but the orderings were typed into the test. A wrong ordering in `make_event` couldn't fail either of them. We checked: with the key store, the wake bump and the delivery bump in 0.2.1 all set to `Relaxed` ([`src/lib.rs` L1075-L1076 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1075-L1076), [`src/lib.rs` L1115 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1115-L1115)), `scripts/loom_gate.sh` still passes all 5 tests.

Now the module in `src/lib.rs` calls `ObserverCells::publish_wake`, `bump_delivery`, `mark_removed`, `observe`, `delivery_epoch` and `state`, and `tests/loom_atomic_wake_tests.rs` runs `ObservableWakeRuntime::publish` against an `ObserverHandle` on another loom thread.

- Before: [`tests/loom_atomic_wake_tests.rs` L1-L66 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/tests/loom_atomic_wake_tests.rs#L1-L66), [`src/lib.rs` L1577-L1675 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1577-L1675).
- After: [`src/lib.rs` L1555-L1643 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1555-L1643), [`tests/loom_atomic_wake_tests.rs` L1-L63 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/loom_atomic_wake_tests.rs#L1-L63).
- Proof: with the wake bump in `publish_wake` set to `Relaxed` ([`src/cells.rs` L66 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L66-L66)), [`a_reader_sees_the_key_of_its_epoch_or_a_later_one`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1575-L1599) fails, and [`a_handle_reads_a_publish_on_another_thread`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/loom_atomic_wake_tests.rs#L17-L42) fails with `left: None, right: Some(WakeKey64 { .. })`, a reader that saw epoch 1 and no key. With `bump_delivery` set to `Relaxed` ([`src/cells.rs` L73 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L73-L73)), [`delivery_never_runs_ahead_of_wakes`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1601-L1621) fails with `1 deliveries but 0 wakes`. Unchanged, all 5 pass.
- Introduced: 0.1.0, [`1a472db`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/1a472db9907b09585bd98d61262bc8bf08291713): [`tests/loom_atomic_wake_tests.rs` L1-L66 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/tests/loom_atomic_wake_tests.rs#L1-L66), [`src/lib.rs` L1590-L1676 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L1590-L1676).

#### Publish loops stop loading `state`

Every hot, cold and tagged publish loaded each observer's `state` and skipped it unless it was `Active`. That check could never skip anything. `remove` stores `Removed` and takes the entry out of the table in the same call, and an `.inactive()` observer was already turned away by `ObserverConfig::matches`. So any entry still in the table read `Active`, and the check was one atomic load per observer per publish for nothing. Now only handles read `state`.

- Before: [`src/lib.rs` L886-L888 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L886-L888), [`src/lib.rs` L943-L945 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L943-L945), [`src/cold.rs` L542-L548 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L542-L548), [`src/cold.rs` L627-L629 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/cold.rs#L627-L629), [`src/tagged_payload.rs` L523-L525 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/tagged_payload.rs#L523-L525). Why it never fired: `remove`, [`src/lib.rs` L849-L851 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L849-L851); `matches`, [`src/lib.rs` L525-L528 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L525-L528).
- After: [`src/cells.rs` L31-L36 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/cells.rs#L31-L36) says why the loops don't read it; [`src/lib.rs` L865-L878 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L865-L878).
- Proof: [`inactive_observer_config_does_not_match`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1205-L1211) and [`an_inactive_observer_reads_as_inactive`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L156-L166) (an inactive observer is never woken), [`a_handle_outlives_removal_and_stops_moving`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L58-L77) (a removed observer's counts stop moving).

#### `scripts/miri_gate.sh` runs the handle tests across 8 schedules

The handle tests use real threads, and Miri reports data races on anything they share. They now run a second time with `-Zmiri-many-seeds=0..8`, so Miri tries 8 thread schedules instead of one.

- Before: [`scripts/miri_gate.sh` L1-L10 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/scripts/miri_gate.sh#L1-L10).
- After: [`scripts/miri_gate.sh` L1-L12 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/scripts/miri_gate.sh#L1-L12).
- Proof: the `miri` job in the [v0.2.2 run](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38677659061/job/116234827614), and locally on `c520dd9`: seeds 0 through 7, 11 passed each, exit 0.

#### Speed

Measured with the crate's criterion benches ([`benches/observable_runtime_benches.rs` L197-L295 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/benches/observable_runtime_benches.rs#L197-L295)), 0.2.2 against 0.2.1, median of 3 alternating runs, as written in [`README.md` L82-L85 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/README.md#L82-L85):

- most cases within 5% either way;
- hot publishing to 512 observers 30% faster, and sparse hot scans of 2048 observers 29% faster. Each observer's table entry used to carry four 64-byte-aligned atomics inline ([`src/lib.rs` L538-L560 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L538-L560)); now it carries one pointer ([`src/lib.rs` L540-L544 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L540-L544));
- dense and sparse hot scans of 2048 observers 4% to 6% slower.

Introduced: the slower cases came in with this commit, [`c520dd9`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/c520dd95be69a55593519a2226e9968b0725b62c).

### Fixed

#### Crossbeam: a failed send no longer counts as delivered

`deliver_crossbeam` bumped `delivery_epoch` first, then looked for the sender, then sent. With no sender (`WakeError::CrossbeamSenderMissing`) or a dropped receiver (`WakeError::CrossbeamSendFailed`), `publish` reported the error and the delivery count went up anyway. Now the bump comes after `send` returns `Ok`. With handles on other threads the order matters a second way: a handle that sees delivery `n` can already receive event `n`.

- Before: [`src/lib.rs` L1118-L1128 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1118-L1128).
- After: [`src/lib.rs` L1093-L1106 @ c520dd9](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/src/lib.rs#L1093-L1106).
- Proof: [`a_failed_send_is_not_counted_as_delivered`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L214-L229) asserts `(wake_epoch, delivery_epoch) == (1, 0)` for both cases. The same two cases run against 0.2.1 (read through `observer_epoch` and `observer_delivery_epoch`, since 0.2.1 has no handle) print:

  ```
  no sender:   errors [CrossbeamSenderMissing] wake 1 delivery 1
  no receiver: errors [CrossbeamSendFailed] wake 1 delivery 1
  ```

  [`a_counted_delivery_is_already_in_the_channel`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/c520dd95be69a55593519a2226e9968b0725b62c/tests/observer_handle_tests.rs#L231-L258) has a reader spin on `delivery_epoch` across 50,000 publishes and `try_recv` every counted delivery. Loom can't check this one because the channel's atomics aren't loom's.
- Introduced: 0.1.0, [`src/lib.rs` L1119-L1129 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L1119-L1129).

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

- Where: [`.github/workflows/ci.yml` L1-L110 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L1-L110); the first version, [`.github/workflows/ci.yml` L1-L86 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/.github/workflows/ci.yml#L1-L86).
- Proof: every run is green: [`0df7915`](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38665968778), [`f77ef14` on main](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667250912), [tag `v0.2.1`](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389). In the tag run the [`publish` job](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389/job/116202821258) ran from 01:47:31 to 01:48:08 UTC, and crates.io has 0.2.1 at 01:48:04.

#### `scripts/miri_gate.sh`

Runs the whole test suite and every example under Miri with `-Zmiri-strict-provenance -Zmiri-disable-isolation` (some tests read the clock and write temp files). CI and the README both call the script, so there's one command.

- Where: [`scripts/miri_gate.sh` L1-L10 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/scripts/miri_gate.sh#L1-L10).
- Proof: the `miri` job in the [tag `v0.2.1` run](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389).

#### Weekly run against the newest dependencies

`Cargo.lock` is committed and every other job runs `--locked`, so CI only tested the pinned versions. Someone adding the crate gets the newest compatible versions instead. `latest-deps` runs `cargo update` and then the tests on stable and on 1.88, weekly and on every push. It's not a publish gate, so a third-party release can't block a tag; it shows up here instead.

- Where: [`.github/workflows/ci.yml` L8-L10 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L8-L10), [`.github/workflows/ci.yml` L76-L90 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L76-L90).

### Changed

#### The publish job's actions are pinned to commit SHAs

The `publish` job has `id-token: write`, so it can get a crates.io token. `actions/checkout@v4` and `rust-lang/crates-io-auth-action@v1` are tags, and tags can be moved. A moved tag would run different code in a job that can publish. Both are pinned to commit SHAs now, and the `dtolnay/rust-toolchain` step is gone from that job; it uses the runner's stable cargo.

- Before: [`.github/workflows/ci.yml` L68-L86 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/.github/workflows/ci.yml#L68-L86).
- After: [`.github/workflows/ci.yml` L92-L110 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L92-L110).
- Introduced: [`0df7915`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/0df7915291b2f632ea69dcdb9aee403b31f8dec6). Its CI run [skipped `publish`](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38665968778/job/116198468891) (no tag), so the unpinned refs never ran with a token.

#### More of the build is linted

In `0df7915`, clippy only ran with `--all-features`, the per-feature loop only ran `cargo test` without `--all-targets`, MSRV only built with all features, and the loom build wasn't linted. Now clippy and the tests run with no features and with each feature alone over all targets, MSRV also runs with no features, `scripts/loom_gate.sh` runs with `-D warnings --locked`, and the `loom` job runs clippy under `--cfg loom`. Run against `0df7915`, clippy `-D warnings` with no features and with each feature alone is clean, so that part closed a gap and didn't hide a failure. The loom part did find something (Fixed, below). The per-feature loop also dropped `loom-tests`: without `--cfg loom` that feature builds nothing, since `tests/loom_atomic_wake_tests.rs` is `#![cfg(loom)]`.

- Before: [`.github/workflows/ci.yml` L16-L66 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/.github/workflows/ci.yml#L16-L66).
- After: [`.github/workflows/ci.yml` L19-L74 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L19-L74), [`scripts/loom_gate.sh` L8-L9 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/scripts/loom_gate.sh#L8-L9).

#### Clippy cleanups

`for &id in &candidates` instead of `candidates[index]` in `ObservableWakeRuntime::publish_indexed` and `ColdObservableRuntime::evaluate_cold_indexed`, and `#[derive(Default)]` on `ProtocolBridgeReport` instead of a hand-written impl. Same behavior. Clippy `-D warnings` on `4a951d6` reports `needless_range_loop` at `src/lib.rs:931` and `src/cold.rs:615`, and `derivable_impls` at `src/protocol_bridge.rs:131`.

- Before: [`src/lib.rs` L930-L933 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/lib.rs#L930-L933), [`src/cold.rs` L613-L617 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/cold.rs#L613-L617), [`src/protocol_bridge.rs` L121-L142 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/protocol_bridge.rs#L121-L142).
- After: [`src/lib.rs` L930-L932 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/lib.rs#L930-L932), [`src/cold.rs` L613-L616 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/cold.rs#L613-L616), [`src/protocol_bridge.rs` L121-L130 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/protocol_bridge.rs#L121-L130).

### Fixed

#### Broken and redundant doc links

The docs on `ColdObservableRuntime::publish_stateful_auto` linked `` [`DispatchPolicy`] ``, which isn't in scope in `cold.rs`, so the link didn't resolve. The module docs in `cold.rs` and `tagged_payload.rs` also spelled out targets rustdoc already resolves. `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --all-features` on `4a951d6` fails with `unresolved link to DispatchPolicy` at `src/cold.rs:478` and 5 `redundant explicit link target` errors.

- Before: [`src/cold.rs` L474-L479 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/cold.rs#L474-L479), [`src/cold.rs` L1-L17 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/cold.rs#L1-L17), [`src/tagged_payload.rs` L1-L7 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/src/tagged_payload.rs#L1-L7).
- After: [`src/cold.rs` L474-L479 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/cold.rs#L474-L479), [`src/cold.rs` L1-L17 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/cold.rs#L1-L17), [`src/tagged_payload.rs` L1-L7 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/tagged_payload.rs#L1-L7).
- Proof: the rustdoc step with `RUSTDOCFLAGS: -D warnings`, [`.github/workflows/ci.yml` L37-L39 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/.github/workflows/ci.yml#L37-L39), passes in the [tag `v0.2.1` run](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389).
- Introduced: 0.1.0, [`1a472db`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/1a472db9907b09585bd98d61262bc8bf08291713).

#### `cfg(loom)` is declared

Every `#[cfg(loom)]` raised Rust's `unexpected_cfgs` warning, because nothing declared `loom` as a cfg name. Clippy `-D warnings` on `4a951d6` fails with 12 of them across `src/lib.rs`, `src/sync.rs` and `src/cold.rs`. `Cargo.toml` declares it under `[lints.rust]` now.

- Before: [`Cargo.toml` L33-L39 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/Cargo.toml#L33-L39).
- After: [`Cargo.toml` L38-L40 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/Cargo.toml#L38-L40).
- Introduced: 0.1.0, [`1a472db`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/1a472db9907b09585bd98d61262bc8bf08291713).

#### The loom module in `src/lib.rs` only builds in test builds

`mod loom_wake_tests` was `#[cfg(loom)]`. When the library builds under `--cfg loom` without `test`, which is how it builds for an integration test, the `#[test]` functions drop out and leave three unused `use` lines and an unused `WAKE_KEY`. Nothing denied warnings in the loom build yet, so it went by. It's `#[cfg(all(test, loom))]` now. With the module gone from that build, the loom-side `Arc` and `thread` re-exports in `src/sync.rs` are unused too, so they get `#[allow(unused_imports)]` like the std side already had. Run on `0df7915`, the loom gate the way `f77ef14` writes it (`RUSTFLAGS="--cfg loom -D warnings" cargo test --all-features --locked --test loom_atomic_wake_tests`) stops with those 4 errors in `src/lib.rs`. Taking the two new allows back out of `f77ef14` stops it with `unused import: loom::sync::Arc` and `unused import: loom::thread`.

- Before: [`src/lib.rs` L1589-L1595 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/lib.rs#L1589-L1595), [`src/sync.rs` L17-L27 @ 0df7915](https://github.com/jameshuntt/ob-serve-a-bowl/blob/0df7915291b2f632ea69dcdb9aee403b31f8dec6/src/sync.rs#L17-L27).
- After: [`src/lib.rs` L1589-L1590 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/lib.rs#L1589-L1590), [`src/sync.rs` L17-L29 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/src/sync.rs#L17-L29).
- Proof: the [`loom` job](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389/job/116202421517) in the tag run, which runs the gate with `-D warnings` and clippy under `--cfg loom`.
- Introduced: 0.1.0, [`src/lib.rs` L1590-L1591 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/src/lib.rs#L1590-L1591).

#### README install line

The 0.2.0 commit bumped the version but left the README at `ob-serve-a-bowl = "0.1"`, so the 0.2.0 page on crates.io told people to add 0.1. It says `"0.2"` now.

- Before: [`README.md` L19-L22 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/README.md#L19-L22).
- After: [`README.md` L19-L22 @ f77ef14](https://github.com/jameshuntt/ob-serve-a-bowl/blob/f77ef14c9f22e0581c7f278826dab2b2da6ac2bb/README.md#L19-L22).
- Introduced: [`4a951d6`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/4a951d650d987fc9ffdca2b9cf7ace9401e4714d) (0.2.0), which changed `Cargo.toml` and not this line.

## 0.2.0 (2026-10-15 01:24 UTC)

From commit [`4a951d6`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/4a951d650d987fc9ffdca2b9cf7ace9401e4714d); the `.cargo_vcs_info.json` in the published crate names it. Published by hand (crates.io lists `jameshuntt`). Breaking for anyone on the `protocol` feature.

### Security

#### `protocol` moves to `waker_protocol` 0.3, which checks a message's task type

In `waker_protocol` 0.2 a message written for one task type could run against a task of another type. `typed_envelope::<T, M>` builds a message whose `dispatch` casts the task pointer to `*mut T` and checks nothing. The scheduler puts any `SchedulerMsg::Deliver` into the target task's mailbox, and `poll_one` dispatches every envelope in that mailbox into the task's raw pointer. `WakerProtocol::tasks` and `TaskEntry::mailbox` were public too, so safe code could push an envelope straight into another task's mailbox. The tasks themselves are `unsafe impl ProtocolTask`, as every task has to be, but getting the wrong message to one took no `unsafe`. 0.3 makes `Envelope` an `unsafe trait` with `target_type()`, and the scheduler refuses an envelope whose `target_type()` isn't the live task's type.

This crate's own code never dispatches anything; the hole was in the dependency. But the `protocol` feature brought it in from 0.1.0 on, and `WakeEvent::to_protocol_deliver_msg` builds exactly that `SchedulerMsg::Deliver`, from an observer's target and a caller's envelope.

- Before: `waker_protocol = "0.2"`, [`Cargo.toml` L27 @ b96530f](https://github.com/jameshuntt/ob-serve-a-bowl/blob/b96530fd1b86ce39956efef0b387856edd9399f1/Cargo.toml#L27-L27); the bridge's deliver message, [`src/protocol_bridge.rs` L93-L108 @ b96530f](https://github.com/jameshuntt/ob-serve-a-bowl/blob/b96530fd1b86ce39956efef0b387856edd9399f1/src/protocol_bridge.rs#L93-L108). In `waker_protocol` 0.2.0: the unchecked cast, [waker_protocol `src/envelope.rs` L29-L33 @ 23872ee](https://github.com/jameshuntt/waker_protocol/blob/23872ee6f934867c5bc0f79357128dad77531d94/src/envelope.rs#L29-L33); any `Deliver` into the mailbox, [waker_protocol `src/scheduler.rs` L345-L353 @ 23872ee](https://github.com/jameshuntt/waker_protocol/blob/23872ee6f934867c5bc0f79357128dad77531d94/src/scheduler.rs#L345-L353); the dispatch, [waker_protocol `src/scheduler.rs` L604-L606 @ 23872ee](https://github.com/jameshuntt/waker_protocol/blob/23872ee6f934867c5bc0f79357128dad77531d94/src/scheduler.rs#L604-L606).
- After: [`Cargo.toml` L28 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/Cargo.toml#L28-L28). In `waker_protocol` 0.3.0: the trait, [waker_protocol `src/envelope.rs` L1-L20 @ 1ca72a5](https://github.com/jameshuntt/waker_protocol/blob/1ca72a5803549914aa822c42cbb91705e267875f/src/envelope.rs#L1-L20); the check, [waker_protocol `src/scheduler.rs` L669-L681 @ 1ca72a5](https://github.com/jameshuntt/waker_protocol/blob/1ca72a5803549914aa822c42cbb91705e267875f/src/scheduler.rs#L669-L681).
- Proof: a small program on 0.1.1 and `waker_protocol` 0.2.0 spawns a `Small(u8)` task, builds a `typed_envelope::<Big, u64>` (`Big` holds a `Vec<u64>`) through `WakeEvent::to_protocol_deliver_msg` with the `Small` task as target, pushes it into that task's public `mailbox` and calls `run_once`. Under Miri it stops with `Undefined Behavior: constructing invalid value of type &mut Big: encountered a dangling reference (going beyond the bounds of its allocation)` at `waker_protocol-0.2.0/src/envelope.rs:31`. On the 0.3 side, waker_protocol's [`a_message_for_another_type_is_refused_not_delivered`](https://github.com/jameshuntt/waker_protocol/blob/1ca72a5803549914aa822c42cbb91705e267875f/tests/scheduler.rs#L174-L188) sends a message written for `Other` to a `Counter` and gets the refusal; run locally from the 0.3.0 package, it passes.
- Introduced: 0.1.0, [`Cargo.toml` L27 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/Cargo.toml#L27-L27).

### Changed

#### `protocol` uses `waker_protocol` 0.3's types

`TaskId`, `WakeReason` and `SchedulerMsg` in the bridge are 0.3's types now. An `Envelope` you write yourself is now `unsafe impl`, with `target_type()` and `unsafe fn dispatch`, the way the crate's own test envelope changed.

- Before: [`tests/protocol_feature_tests.rs` L153-L163 @ b96530f](https://github.com/jameshuntt/ob-serve-a-bowl/blob/b96530fd1b86ce39956efef0b387856edd9399f1/tests/protocol_feature_tests.rs#L153-L163).
- After: [`tests/protocol_feature_tests.rs` L153-L168 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/tests/protocol_feature_tests.rs#L153-L168).
- Proof: [`wake_event_can_build_protocol_deliver_message_with_caller_envelope`](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/tests/protocol_feature_tests.rs#L170-L193) builds a deliver message with that envelope. Run locally on `4a951d6`, `cargo test --all-features` passes 86.

#### Minimum Rust 1.88

`Cargo.toml` declares `rust-version = "1.88"`. With `tagged-payloads`, `petr-core` 0.1 uses `let` chains (`if let ... && ...`), which need 1.88. Without the field, an older toolchain fails inside a dependency instead of saying which Rust it needs: on 1.85, `cargo check --features tagged-payloads` on 0.1.0 fails in `petr-core` with `` `let` expressions in this position are unstable `` at `src/keyed.rs:154`. With the field, 1.85 refuses up front with `ob-serve-a-bowl@0.2.0 requires rustc 1.88`.

- Where: [`Cargo.toml` L5 @ 4a951d6](https://github.com/jameshuntt/ob-serve-a-bowl/blob/4a951d650d987fc9ffdca2b9cf7ace9401e4714d/Cargo.toml#L5-L5).
- Proof: the [`msrv` job](https://github.com/jameshuntt/ob-serve-a-bowl/actions/runs/38667418389/job/116202421297) builds and tests on 1.88, from 0.2.1 on.

## 0.1.1 (2026-10-15 00:52 UTC)

Not published from this repo. The `.crate` on crates.io has no `.cargo_vcs_info.json`, and its `Cargo.toml.orig` is cargo's normalized manifest (it opens with `# THIS FILE IS AUTOMATICALLY GENERATED BY CARGO`), so it was packaged from an unpacked package directory. Every other file in it (sources, tests, examples, benches, README, both licenses and `Cargo.lock`) matches commit [`b96530f`](https://github.com/jameshuntt/ob-serve-a-bowl/commit/b96530fd1b86ce39956efef0b387856edd9399f1) byte for byte, and that commit is the same change. Published by hand (crates.io lists `jameshuntt`).

### Fixed

#### The Apache license names its copyright holder

`LICENSE-APACHE` still had the template line `Copyright [yyyy] [name of copyright owner]`. It's `Copyright 2026 1o LLC` now. No code changed.

- Before: [`LICENSE-APACHE` L190 @ 1a472db](https://github.com/jameshuntt/ob-serve-a-bowl/blob/1a472db9907b09585bd98d61262bc8bf08291713/LICENSE-APACHE#L190-L190).
- After: [`LICENSE-APACHE` L190 @ b96530f](https://github.com/jameshuntt/ob-serve-a-bowl/blob/b96530fd1b86ce39956efef0b387856edd9399f1/LICENSE-APACHE#L190-L190).
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
- 86 tests (23 unit, 63 integration), 5 loom tests behind `scripts/loom_gate.sh`, 5 examples and a criterion bench. Run locally on `1a472db`, `cargo test --all-features` passes 86.
