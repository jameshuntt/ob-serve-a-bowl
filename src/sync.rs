//! `cfg(loom)`-swappable concurrency primitives.
//!
//! Under `--cfg loom` the wake runtime's atomics (and the test's Arc/threads)
//! become loom's model-checked versions, so `scripts/loom_gate.sh` can permute
//! every thread interleaving AND weak-memory reordering of the wake/epoch
//! protocol. Under a normal build these are `std`, byte-for-byte the same as
//! before — loom adds a testing lens, never a runtime cost.
//!
//! Only the wake/epoch atomics route through here; `Ordering` is identical in
//! both worlds (loom re-exports std's), so callers keep using it directly.

#[cfg(loom)]
pub use loom::sync::atomic;
#[cfg(not(loom))]
pub use std::sync::atomic;

#[cfg(loom)]
pub use loom::sync::Arc;
#[cfg(not(loom))]
#[allow(unused_imports)]
pub use std::sync::Arc;

#[cfg(loom)]
pub use loom::thread;
#[cfg(not(loom))]
#[allow(unused_imports)]
pub use std::thread;
