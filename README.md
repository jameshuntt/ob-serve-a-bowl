# ob-serve-a-bowl

An observable wake runtime. Say it out loud.

The crate glues three ideas together:

- a bit-packed 64-bit wake key for cheap hot-path routing,
- cold observers declared as field masks and matchers, with re-arm policy,
- profile-selected wake delivery, so the backend is a choice, not the identity of the protocol.

Around that: a replayable event log of the observable boundary, an optional
tagged-payload plane kept separate from the wake plane, and an optional bridge
into the [`waker_protocol`](https://crates.io/crates/waker_protocol) scheduler.

The wake and epoch protocol is checked under [loom](https://crates.io/crates/loom):
every thread interleaving and weak-memory reordering of the real atomics the
runtime uses. See `scripts/loom_gate.sh`.

```toml
[dependencies]
ob-serve-a-bowl = "0.1"
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
| root | `ObservableWakeRuntime`, `WakeKey64`, `WakeEvent`, `WakeReport`, `ObserverId`, `TargetTaskId`, reason and profile codes |
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
cargo test --all-features     # 86 tests: 23 unit, 63 integration
scripts/loom_gate.sh          # 5 loom tests over the wake/epoch atomics
cargo bench                   # criterion benches for the hot path
```

## License

MIT or Apache-2.0, at your option.
