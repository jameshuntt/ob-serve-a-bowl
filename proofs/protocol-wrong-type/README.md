# protocol-wrong-type

ob-serve-a-bowl 0.1.0 with the `protocol` feature and waker_protocol 0.2.0. Spawns a `Small(u8)` task, builds a `typed_envelope::<Big, u64>` through `WakeEvent::to_protocol_deliver_msg` with the `Small` task as target, pushes it into that task's public `mailbox` and calls `run_once`. The only `unsafe` is the `unsafe impl ProtocolTask` every task needs.

Run `cargo +nightly miri run` here. Expected: exit 1 with `Undefined Behavior: constructing invalid value of type &mut Big: encountered a dangling reference (going beyond the bounds of its allocation)` at `waker_protocol-0.2.0/src/envelope.rs:31:29`, reached from `WakerProtocol::run_once` through `poll_one`.
