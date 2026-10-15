# ob-serve-a-bowl

An observable wake runtime. Say it out loud.

The crate glues three ideas together:

- a bit-packed 64-bit wake key for cheap hot-path routing,
- cold observers declared as field masks and matchers, with re-arm policy,
- profile-selected wake delivery, so the backend is a choice, not the identity of the protocol.

Around that: a replayable event log of the observable boundary, an optional
tagged-payload plane kept separate from the wake plane, and an optional bridge
into the [`waker_protocol`](https://crates.io/crates/waker_protocol) scheduler.

Any thread can follow an observer's wakes through an `ObserverHandle` while the
runtime keeps publishing: `runtime.observer_handle(id)` (or `cold_observer_handle`)
gives a cloneable, `Send + Sync` handle whose `observe()` returns the wake count and
the key of that wake or a later one, never an earlier one.

That publication is checked under [loom](https://crates.io/crates/loom): the real
runtime publishing on one thread against a real handle on another, under every
thread interleaving and weak-memory reordering. See `scripts/loom_gate.sh`.

```toml
[dependencies]
ob-serve-a-bowl = "0.2"
```

## Features

| feature | adds |
|---|---|
| `crossbeam-wake` | a crossbeam channel wake profile |
| `serialization` | serde on the event log, JSONL replay |
| `tagged-payloads` | payload handles backed by `petr-core` and `petr-collections` keyed arenas |
| `protocol` | the bridge from wake events to `waker_protocol` scheduler messages |

## Modules

| module | what it holds |
|---|---|
| `api` | the small happy path: `wake_key`, `hot_observer`, `cold_observer`, `packet`, `payload_ref`, `target_task` |
| root | `ObservableWakeRuntime`, `WakeKey64`, `WakeEvent`, `WakeReport`, `ObserverId`, `TargetTaskId`, reason and profile codes, `ObserverHandle` and `ObservedWake` |
| `config` | `ObservableRuntimeConfig`, `DispatchPolicy`, the builder |
| `cold` | the stateful cold-observable layer: `ColdObserverConfig`, `ColdRearmPolicy`, `ColdPublishReport`, field masks |
| `event_log` | `ObservableEventLog`, records, replay summaries |
| `tagged_payload` | tagged payload integration (feature `tagged-payloads`) |
| `protocol_bridge` | wake events into `waker_protocol` messages (feature `protocol`) |

## Examples

```bash
cargo run --example basic_observable
cargo run --example cold_state_observable
cargo run --example event_log_replay
cargo run --example tagged_payload_observable --features tagged-payloads
cargo run --example server_event_observable --features protocol,serialization,tagged-payloads
```

## Testing

```bash
cargo test --all-features     # 98 tests: 23 unit, 74 integration, 1 doc
scripts/loom_gate.sh          # 5 loom tests over the wake/epoch atomics
scripts/miri_gate.sh          # the 98 tests and every example under Miri
cargo bench                   # criterion benches for the hot path
```

## 0.2.2

- `ObserverHandle`: read an observer's wake count, delivery count, latest key and state from any
  thread while the runtime publishes. `ObservableWakeRuntime::observer_handle` and
  `ColdObservableRuntime::cold_observer_handle` give one. A handle reports `Inactive` for an
  observer registered with `.inactive()`, and `Removed` once the observer is removed or its
  runtime is dropped.
- The hot, cold and tagged-payload paths publish through one shared block per observer instead
  of their own copies of the same atomics.
- The crossbeam profile counts a delivery only after the event is in the channel; a failed send
  (no sender, or no receiver) is no longer counted as delivered.
- The loom tests drive that real code (and the whole runtime against a handle on another thread)
  instead of hand-written copies of it; weakening the epoch or delivery ordering fails them.
- Speed, measured with the crate's benches against 0.2.1 (median of 3 alternating runs): most
  cases within 5% either way; hot publishing to 512 observers 30% faster and sparse hot scans
  of 2048 observers 29% faster (each observer's table entry is smaller); dense and sparse hot
  scans of 2048 observers 4% to 6% slower.

## 0.2.1

GitHub Actions run the tests (all features and each feature alone), clippy, docs, every example,
Rust 1.88, Miri and loom on every push, and publish a `v*` tag only when all of them pass.

## 0.2.0

The `protocol` feature uses `waker_protocol` 0.3, whose messages are checked against the task type
they are for. Its `TaskId`, `WakeReason` and `SchedulerMsg` are 0.3's types now. Minimum Rust 1.88.

## License

MIT or Apache-2.0, at your option.
