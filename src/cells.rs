//! The atomics an observer's wakes are published through, and the [`ObserverHandle`] that reads
//! them from any thread.
//!
//! The runtime (hot or cold) is the only writer and needs `&mut self` to publish. Each observer's
//! four counters live in one shared, reference-counted block, so a handle on another thread can
//! read them while the runtime keeps publishing. Each counter has its own cache line, so bumping
//! one counter does not disturb a handle reading another. Under `--cfg loom` the
//! atomics and the `Arc` are loom's, and `scripts/loom_gate.sh` checks these exact functions under
//! every interleaving.

use crate::sync::atomic::{AtomicU64, Ordering};
use crate::sync::Arc;
use crate::{ObserverId, ObserverState, WakeKey64};
use big_bad_bool::PackedWord;

#[repr(align(64))]
pub(crate) struct CacheLineAtomicU64(pub(crate) AtomicU64);

impl CacheLineAtomicU64 {
    // loom's atomics are not const-constructible; the std path stays const.
    #[cfg(not(loom))]
    const fn new(value: u64) -> Self {
        Self(AtomicU64::new(value))
    }
    #[cfg(loom)]
    fn new(value: u64) -> Self {
        Self(AtomicU64::new(value))
    }
}

/// One observer's published state. Written only by the runtime that owns the observer.
///
/// The runtime takes an observer out of its table in the same call that marks it removed, so no
/// observer the runtime still holds is removed, and `state` never changes while it is held (an
/// inactive observer is turned away by its config). The publish loops therefore do not read
/// `state`; only handles do.
pub(crate) struct ObserverCells {
    state: CacheLineAtomicU64,
    wake_epoch: CacheLineAtomicU64,
    delivery_epoch: CacheLineAtomicU64,
    last_key: CacheLineAtomicU64,
}

impl ObserverCells {
    pub(crate) fn new(active: bool) -> Arc<Self> {
        let state = if active { ObserverState::Active } else { ObserverState::Inactive };
        Arc::new(Self {
            state: CacheLineAtomicU64::new(state as u64),
            wake_epoch: CacheLineAtomicU64::new(0),
            delivery_epoch: CacheLineAtomicU64::new(0),
            last_key: CacheLineAtomicU64::new(0),
        })
    }

    /// Record a wake caused by `key`; the new wake epoch.
    ///
    /// The key is stored before the epoch is bumped. A reader that sees epoch `n` with an Acquire
    /// load then sees, with an Acquire load, the key of wake `n` or of a later wake, never an
    /// earlier one ([`ObserverCells::observe`]).
    ///
    /// The bumps are Release, not AcqRel: the runtime is the only writer, so it has nothing to
    /// acquire.
    #[inline]
    pub(crate) fn publish_wake(&self, key: WakeKey64) -> u64 {
        self.last_key.0.store(key.raw_storage(), Ordering::Release);
        self.wake_epoch.0.fetch_add(1, Ordering::Release) + 1
    }

    /// Record a delivery of the latest wake; the new delivery epoch. Always after `publish_wake`
    /// for the same wake, so a reader never sees more deliveries than wakes.
    #[inline]
    pub(crate) fn bump_delivery(&self) -> u64 {
        self.delivery_epoch.0.fetch_add(1, Ordering::Release) + 1
    }

    /// Mark the observer removed; handles see it as removed from then on.
    #[inline]
    pub(crate) fn mark_removed(&self) {
        self.state.0.store(ObserverState::Removed as u64, Ordering::Release);
    }

    #[inline]
    pub(crate) fn wake_epoch(&self) -> u64 {
        self.wake_epoch.0.load(Ordering::Acquire)
    }

    #[inline]
    pub(crate) fn delivery_epoch(&self) -> u64 {
        self.delivery_epoch.0.load(Ordering::Acquire)
    }

    #[inline]
    pub(crate) fn last_key(&self) -> Option<WakeKey64> {
        decode_key(self.last_key.0.load(Ordering::Acquire))
    }

    #[inline]
    pub(crate) fn state(&self) -> ObserverState {
        match self.state.0.load(Ordering::Acquire) {
            0 => ObserverState::Inactive,
            1 => ObserverState::Active,
            _ => ObserverState::Removed,
        }
    }

    /// The wake epoch, then the key, in the order that makes the pair meaningful.
    #[inline]
    pub(crate) fn observe(&self) -> ObservedWake {
        let epoch = self.wake_epoch();
        let key = if epoch == 0 { None } else { self.last_key() };
        ObservedWake { epoch, key }
    }
}

#[inline]
fn decode_key(raw: u64) -> Option<WakeKey64> {
    if raw == 0 {
        return None;
    }
    WakeKey64::try_from_raw_u128(u128::from(raw))
}

/// What a handle saw: how many wakes there have been, and the key of the latest one.
///
/// `key` is the key of wake number `epoch` or of a later wake, never of an earlier one. A later
/// wake's key can already be visible while that wake is not yet counted in `epoch`: the runtime
/// stores the key first, then bumps the count. `key` is `None` only when `epoch` is 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedWake {
    /// How many times the observer has been woken.
    pub epoch: u64,
    /// The key of wake `epoch` or of a later one whose count may not show yet.
    pub key: Option<WakeKey64>,
}

/// A read-only view of one observer that can be cloned and sent to other threads.
///
/// It reads the same counters the runtime writes, while the runtime keeps publishing. It stays
/// valid after the observer is removed or the runtime is dropped (it then reports
/// [`ObserverState::Removed`] and the counts stop moving), and it never sees a different observer
/// that later reuses the slot.
///
/// ```
/// use ob_serve_a_bowl::*;
///
/// let mut runtime = ObservableWakeRuntime::new();
/// let id = runtime.register(ObserverConfig::new("reader", WakeProfileCode::AtomicEpoch));
/// let handle = runtime.observer_handle(id).unwrap();
///
/// let reader = std::thread::spawn(move || {
///     while handle.wake_epoch() == 0 {
///         std::thread::yield_now();
///     }
///     handle.observe()
/// });
///
/// let key = WakeKey64::typed(WakeReasonCode::Manual, WakeProfileCode::AtomicEpoch, 0, 0b1_u16, 0, 0).unwrap();
/// runtime.publish(ObservablePacket::new(key, PayloadRef::default()));
///
/// let seen = reader.join().unwrap();
/// assert_eq!(seen.epoch, 1);
/// assert_eq!(seen.key, Some(key));
/// ```
#[derive(Clone)]
pub struct ObserverHandle {
    id: ObserverId,
    cells: Arc<ObserverCells>,
}

impl ObserverHandle {
    pub(crate) fn new(id: ObserverId, cells: &Arc<ObserverCells>) -> Self {
        Self { id, cells: Arc::clone(cells) }
    }

    /// The observer this handle reads.
    #[must_use]
    pub fn id(&self) -> ObserverId {
        self.id
    }

    /// How many times the observer has been woken.
    #[must_use]
    pub fn wake_epoch(&self) -> u64 {
        self.cells.wake_epoch()
    }

    /// How many of those wakes have been delivered. Never more than a `wake_epoch` read after it.
    /// A wake whose delivery failed (for example a crossbeam send with no receiver) is not counted.
    #[must_use]
    pub fn delivery_epoch(&self) -> u64 {
        self.cells.delivery_epoch()
    }

    /// The key of the latest wake, if any. To know which wake it belongs to, use
    /// [`ObserverHandle::observe`].
    #[must_use]
    pub fn last_key(&self) -> Option<WakeKey64> {
        self.cells.last_key()
    }

    /// The wake epoch and the latest key, read in the order that ties them together.
    #[must_use]
    pub fn observe(&self) -> ObservedWake {
        self.cells.observe()
    }

    /// [`ObserverState::Active`], [`ObserverState::Inactive`] for an observer registered with
    /// `.inactive()`, or [`ObserverState::Removed`] once it is removed or its runtime is dropped.
    #[must_use]
    pub fn state(&self) -> ObserverState {
        self.cells.state()
    }
}

impl std::fmt::Debug for ObserverHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObserverHandle")
            .field("id", &self.id)
            .field("state", &self.state())
            .field("wake_epoch", &self.wake_epoch())
            .finish()
    }
}
