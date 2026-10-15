//! Observable wake runtime.
//!
//! This crate glues three ideas together:
//!
//! - a bitpacked wake key for cheap hot-path routing,
//! - cold observers declared as masks + field matchers,
//! - profile-selected wake delivery backends.
//!
//! Crossbeam is intentionally a feature-gated wake profile (`crossbeam-wake`),
//! not the identity of the protocol. The protocol stays backend-agnostic.

use big_bad_bool::{bitpack, BitpackError, PackedWord};
use big_bad_bool_interact::FieldMatcher;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
mod sync;
use crate::sync::atomic::{AtomicU64, Ordering};

pub mod config;
pub use config::{DispatchPolicy, ObservableRuntimeConfig, ObservableWakeRuntimeBuilder};

pub mod api;
pub use api::{
    cold_observer, hot_observer, packet, payload_ref, target_task, wake_key, HotObserverBuilder,
    LoggedColdRuntime, WakeKeyBuilder,
};

pub mod cold;
pub use cold::*;

pub mod event_log;
pub use event_log::*;

#[cfg(feature = "tagged-payloads")]
pub mod tagged_payload;
#[cfg(feature = "tagged-payloads")]
pub use tagged_payload::*;

#[cfg(feature = "protocol")]
pub mod protocol_bridge;
#[cfg(feature = "protocol")]
pub use protocol_bridge::*;

/// Recommended imports for application/runtime integrations.
pub mod prelude {
    pub use crate::api::{
        cold_observer, hot_observer, packet, payload_ref, target_task, wake_key,
        HotObserverBuilder, LoggedColdRuntime, WakeKeyBuilder,
    };
    pub use crate::{
        ColdObserverConfig, ColdPublishReport, ColdRearmPolicy, DispatchPolicy, ObservableEventLog,
        ObservablePacket, ObservableRuntimeConfig, ObservableState, ObservableWakeRuntimeBuilder,
        FieldMask, ObservableFieldMask, ObserverId, PayloadRef, StateTransition, TargetTaskId, WakeEvent, WakeKey64,
        WakeProfileCode, WakeReasonCode, WakeReport,
    };
}

#[cfg(feature = "crossbeam-wake")]
use crossbeam_channel::Sender;

/// Semantic wake reason encoded in [`WakeKey64`].
///
/// Keep these as stable protocol values. They deliberately mirror the shape of
/// `waker_protocol::WakeReason`, but this crate does not require the protocol
/// crate unless the `protocol` feature is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum WakeReasonCode {
    Spawn = 1,
    Manual = 2,
    Inbox = 3,
    Timer = 4,
    Condition = 5,
    Resume = 6,
    Qos = 7,
    MemoryPressure = 8,
    Cancel = 9,
    Shutdown = 10,
}

impl WakeReasonCode {
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self as u64
    }
}

impl TryFrom<u64> for WakeReasonCode {
    type Error = WakeDecodeError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Spawn),
            2 => Ok(Self::Manual),
            3 => Ok(Self::Inbox),
            4 => Ok(Self::Timer),
            5 => Ok(Self::Condition),
            6 => Ok(Self::Resume),
            7 => Ok(Self::Qos),
            8 => Ok(Self::MemoryPressure),
            9 => Ok(Self::Cancel),
            10 => Ok(Self::Shutdown),
            other => Err(WakeDecodeError::UnknownReason(other)),
        }
    }
}

/// Delivery profile encoded in [`WakeKey64`].
///
/// This is separate from [`WakeReasonCode`]. Reason answers "why did this wake?";
/// profile answers "which delivery mechanism should carry the wake?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum WakeProfileCode {
    /// Store the wake event in the runtime's local ready queue.
    InlineQueue = 1,
    /// Increment observer-local atomic epochs only.
    AtomicEpoch = 2,
    /// Send through a `crossbeam_channel::Sender` when the `crossbeam-wake`
    /// feature is enabled.
    CrossbeamQueue = 3,
    /// Condition-cache profile: useful for derived/cache predicates. This crate
    /// records it as a local condition hit and leaves cache inspection to the
    /// caller's state layer.
    ConditionCache = 4,
    /// Manual/no-op delivery profile for tests or external integrations that only
    /// need match reports.
    Manual = 5,
}

impl WakeProfileCode {
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self as u64
    }
}

impl TryFrom<u64> for WakeProfileCode {
    type Error = WakeDecodeError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::InlineQueue),
            2 => Ok(Self::AtomicEpoch),
            3 => Ok(Self::CrossbeamQueue),
            4 => Ok(Self::ConditionCache),
            5 => Ok(Self::Manual),
            other => Err(WakeDecodeError::UnknownProfile(other)),
        }
    }
}

/// Semantic changed-field mask used by [`WakeKey64`], observer interest masks,
/// and cold-state transitions.
///
/// This newtype is intentionally tiny: the runtime stores field masks in the
/// 16-bit `WakeKey64.field_mask` lane. Users may pass raw `u16` values, this
/// wrapper, or their own generated bitmask/bitflag type by implementing
/// [`ObservableFieldMask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FieldMask(u16);

impl FieldMask {
    pub const EMPTY: Self = Self(0);

    #[must_use]
    pub const fn new(bits: u16) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub fn contains_any<M: ObservableFieldMask>(self, other: M) -> bool {
        (self.0 & other.field_mask_bits()) != 0
    }

    #[must_use]
    pub fn contains_all<M: ObservableFieldMask>(self, other: M) -> bool {
        let bits = other.field_mask_bits();
        (self.0 & bits) == bits
    }
}

impl core::ops::BitOr for FieldMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for FieldMask {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl core::ops::BitAnd for FieldMask {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl From<u8> for FieldMask {
    fn from(value: u8) -> Self {
        Self(u16::from(value))
    }
}

impl From<u16> for FieldMask {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

/// Converts application-specific bitmask/bitflag types into the runtime's
/// 16-bit observable field mask.
///
/// Implement this for generated bitmask types so public APIs can use semantic
/// masks directly:
///
/// ```rust,ignore
/// impl ObservableFieldMask for ServerFields {
///     fn field_mask_bits(self) -> u16 {
///         self.bits()
///     }
/// }
/// ```
pub trait ObservableFieldMask: Copy {
    fn field_mask_bits(self) -> u16;
}

impl ObservableFieldMask for FieldMask {
    fn field_mask_bits(self) -> u16 {
        self.bits()
    }
}

impl ObservableFieldMask for u16 {
    fn field_mask_bits(self) -> u16 {
        self
    }
}

impl ObservableFieldMask for u8 {
    fn field_mask_bits(self) -> u16 {
        u16::from(self)
    }
}

impl ObservableFieldMask for u32 {
    fn field_mask_bits(self) -> u16 {
        u16::try_from(self).expect("observable field mask must fit in u16")
    }
}

impl ObservableFieldMask for usize {
    fn field_mask_bits(self) -> u16 {
        u16::try_from(self).expect("observable field mask must fit in u16")
    }
}

impl ObservableFieldMask for i32 {
    fn field_mask_bits(self) -> u16 {
        u16::try_from(self).expect("observable field mask must be non-negative and fit in u16")
    }
}


/// A compact wake/control word.
///
/// Layout:
///
/// ```text
/// bits 0..8    reason
/// bits 8..16   profile
/// bits 16..24  qos
/// bits 24..40  field_mask
/// bits 40..48  phase
/// bits 48..64  flags
/// ```
#[bitpack(storage = u64, version = 1)]
pub struct WakeKey64 {
    #[bits(8)]
    #[field_id(1)]
    #[describe("Semantic reason this task/observer is being woken.")]
    #[labels(spawn = 1, manual = 2, inbox = 3, timer = 4, condition = 5, resume = 6, qos = 7, memory_pressure = 8, cancel = 9, shutdown = 10)]
    #[reserved(0, 11..=254)]
    #[invalid(255)]
    reason: u64,

    #[bits(8)]
    #[field_id(2)]
    #[describe("Backend/delivery profile used to carry this wake.")]
    #[labels(inline_queue = 1, atomic_epoch = 2, crossbeam_queue = 3, condition_cache = 4, manual = 5)]
    #[reserved(0, 6..=254)]
    #[invalid(255)]
    profile: u64,

    #[bits(8)]
    #[field_id(3)]
    #[describe("Scheduler quality-of-service hint. Higher means more urgent.")]
    #[default(3)]
    qos: u64,

    #[bits(16)]
    #[field_id(4)]
    #[describe("Changed-field mask used to select cold observers cheaply.")]
    field_mask: u64,

    #[bits(8)]
    #[field_id(5)]
    #[describe("Runtime phase or stage id.")]
    phase: u64,

    #[bits(16)]
    #[field_id(6)]
    #[describe("Wake flags. Bit 0 means hot observer eligible; bit 1 means cold observer eligible.")]
    #[flags(hot = 0, cold = 1, checkpoint = 2, restore = 3, external = 4)]
    flags: u64,
}

impl WakeKey64 {
    pub const FLAG_HOT: u64 = 1 << 0;
    pub const FLAG_COLD: u64 = 1 << 1;
    pub const FLAG_CHECKPOINT: u64 = 1 << 2;
    pub const FLAG_RESTORE: u64 = 1 << 3;
    pub const FLAG_EXTERNAL: u64 = 1 << 4;

    /// Build a key from typed reason/profile values.
    pub fn typed<M>(
        reason: WakeReasonCode,
        profile: WakeProfileCode,
        qos: u8,
        field_mask: M,
        phase: u8,
        flags: u16,
    ) -> Result<Self, BitpackError>
    where
        M: ObservableFieldMask,
    {
        Self::new_checked(
            reason.as_u64(),
            profile.as_u64(),
            u64::from(qos),
            u64::from(field_mask.field_mask_bits()),
            u64::from(phase),
            u64::from(flags),
        )
    }

    /// Return the semantic reason as a typed value.
    pub fn reason_code(self) -> Result<WakeReasonCode, WakeDecodeError> {
        WakeReasonCode::try_from(self.reason())
    }

    /// Return the delivery profile as a typed value.
    pub fn profile_code(self) -> Result<WakeProfileCode, WakeDecodeError> {
        WakeProfileCode::try_from(self.profile())
    }

    #[must_use]
    pub fn has_any_field<M: ObservableFieldMask>(self, mask: M) -> bool {
        (self.field_mask() & u64::from(mask.field_mask_bits())) != 0
    }

    #[must_use]
    pub fn has_all_fields<M: ObservableFieldMask>(self, mask: M) -> bool {
        let bits = u64::from(mask.field_mask_bits());
        (self.field_mask() & bits) == bits
    }

    #[must_use]
    pub fn has_flags(self, flags: u16) -> bool {
        (self.flags() & u64::from(flags)) == u64::from(flags)
    }
}

/// Payload reference. Prefer slot/generation/offset over raw pointers for stale
/// handle protection and IPC compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PayloadRef {
    pub slot: u32,
    pub generation: u32,
    pub offset: u32,
    pub len: u32,
}

impl PayloadRef {
    #[must_use]
    pub const fn new(slot: u32, generation: u32, offset: u32, len: u32) -> Self {
        Self {
            slot,
            generation,
            offset,
            len,
        }
    }
}

/// An incoming observable packet: compact control key + stable payload handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservablePacket {
    pub key: WakeKey64,
    pub payload: PayloadRef,
}

impl ObservablePacket {
    #[must_use]
    pub const fn new(key: WakeKey64, payload: PayloadRef) -> Self {
        Self { key, payload }
    }
}

/// Stable observer id using slot/generation to avoid ABA-style stale handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObserverId {
    pub slot: u32,
    pub generation: u32,
}

impl ObserverId {
    #[must_use]
    pub const fn new(slot: u32, generation: u32) -> Self {
        Self { slot, generation }
    }
}

/// Optional protocol task id stored without depending on `waker_protocol`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetTaskId {
    pub slot: u32,
    pub generation: u32,
}

impl TargetTaskId {
    #[must_use]
    pub const fn new(slot: u32, generation: u32) -> Self {
        Self { slot, generation }
    }
}

#[cfg(feature = "protocol")]
impl From<waker_protocol::TaskId> for TargetTaskId {
    fn from(value: waker_protocol::TaskId) -> Self {
        Self {
            slot: value.slot,
            generation: value.generation,
        }
    }
}

#[cfg(feature = "protocol")]
impl From<TargetTaskId> for waker_protocol::TaskId {
    fn from(value: TargetTaskId) -> Self {
        Self {
            slot: value.slot,
            generation: value.generation,
        }
    }
}

/// Observer configuration. Registration is the slow/configuration path, so it
/// may use normal vectors and symbolic matchers. Dispatch remains mask/atomic
/// oriented.
#[derive(Debug, Clone)]
pub struct ObserverConfig {
    pub name: &'static str,
    pub profile: WakeProfileCode,
    pub interest_mask: u16,
    pub matchers: Vec<FieldMatcher>,
    pub target: Option<TargetTaskId>,
    pub active: bool,
}

impl ObserverConfig {
    #[must_use]
    pub fn new(name: &'static str, profile: WakeProfileCode) -> Self {
        Self {
            name,
            profile,
            interest_mask: 0,
            matchers: Vec::new(),
            target: None,
            active: true,
        }
    }

    #[must_use]
    pub fn with_interest_mask<M: ObservableFieldMask>(mut self, interest_mask: M) -> Self {
        self.interest_mask = interest_mask.field_mask_bits();
        self
    }

    #[must_use]
    pub fn with_matchers(mut self, matchers: impl Into<Vec<FieldMatcher>>) -> Self {
        self.matchers = matchers.into();
        self
    }

    #[must_use]
    pub fn with_target(mut self, target: TargetTaskId) -> Self {
        self.target = Some(target);
        self
    }

    #[must_use]
    pub fn inactive(mut self) -> Self {
        self.active = false;
        self
    }

    #[must_use]
    pub fn matches(&self, key: WakeKey64) -> bool {
        if !self.active {
            return false;
        }

        if self.interest_mask != 0 && !key.has_any_field(self.interest_mask) {
            return false;
        }

        self.matchers.iter().copied().all(|matcher| matcher.matches(key))
    }
}

#[repr(align(64))]
struct CacheLineAtomicU64(AtomicU64);

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

struct ObserverEntry {
    generation: u32,
    config: ObserverConfig,
    state: CacheLineAtomicU64,
    wake_epoch: CacheLineAtomicU64,
    delivery_epoch: CacheLineAtomicU64,
    last_key: CacheLineAtomicU64,
}

impl ObserverEntry {
    fn new(generation: u32, config: ObserverConfig) -> Self {
        Self {
            generation,
            config,
            state: CacheLineAtomicU64::new(ObserverState::Active as u64),
            wake_epoch: CacheLineAtomicU64::new(0),
            delivery_epoch: CacheLineAtomicU64::new(0),
            last_key: CacheLineAtomicU64::new(0),
        }
    }

    fn id(&self, slot: u32) -> ObserverId {
        ObserverId::new(slot, self.generation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ObserverState {
    Inactive = 0,
    Active = 1,
    Removed = 2,
}

/// A delivered wake event. This is what inline/crossbeam backends carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeEvent {
    pub observer_id: ObserverId,
    pub target: Option<TargetTaskId>,
    pub key: WakeKey64,
    pub payload: PayloadRef,
    pub reason: WakeReasonCode,
    pub profile: WakeProfileCode,
    pub wake_epoch: u64,
}

/// Per-publish report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WakeReport {
    pub scanned: usize,
    pub matched: usize,
    pub inline_delivered: usize,
    pub atomic_delivered: usize,
    pub condition_delivered: usize,
    pub crossbeam_delivered: usize,
    pub manual_delivered: usize,
    pub errors: Vec<WakeError>,
}

impl WakeReport {
    #[must_use]
    pub fn delivered(&self) -> usize {
        self.inline_delivered
            + self.atomic_delivered
            + self.condition_delivered
            + self.crossbeam_delivered
            + self.manual_delivered
    }
}

/// Runtime-level wake errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeError {
    StaleObserver(ObserverId),
    MissingObserver(ObserverId),
    UnknownReason(u64),
    UnknownProfile(u64),
    CrossbeamFeatureDisabled,
    CrossbeamSenderMissing,
    CrossbeamSendFailed,
}

impl fmt::Display for WakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleObserver(id) => write!(f, "stale observer id {id:?}"),
            Self::MissingObserver(id) => write!(f, "missing observer id {id:?}"),
            Self::UnknownReason(raw) => write!(f, "unknown wake reason {raw}"),
            Self::UnknownProfile(raw) => write!(f, "unknown wake profile {raw}"),
            Self::CrossbeamFeatureDisabled => write!(f, "crossbeam wake profile used without crossbeam-wake feature"),
            Self::CrossbeamSenderMissing => write!(f, "crossbeam wake profile used without configured sender"),
            Self::CrossbeamSendFailed => write!(f, "crossbeam wake send failed"),
        }
    }
}

impl std::error::Error for WakeError {}

/// Decode errors for key fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeDecodeError {
    UnknownReason(u64),
    UnknownProfile(u64),
}

impl From<WakeDecodeError> for WakeError {
    fn from(value: WakeDecodeError) -> Self {
        match value {
            WakeDecodeError::UnknownReason(raw) => Self::UnknownReason(raw),
            WakeDecodeError::UnknownProfile(raw) => Self::UnknownProfile(raw),
        }
    }
}

/// Field-mask index used by the optional indexed publish path.
///
/// The compatibility [`ObservableWakeRuntime::publish`] path still scans all
/// observer slots. [`ObservableWakeRuntime::publish_indexed`] uses this index to
/// first select only observers whose declared interest mask overlaps the
/// bitpacked `WakeKey64.field_mask`. Removed/reused observers are left as stale
/// ids in the index and filtered by generation at dispatch time, which keeps
/// removal cheap and ABA-safe.
#[derive(Default)]
struct ObserverIndex {
    wildcard: Vec<ObserverId>,
    by_interest_mask: BTreeMap<u16, Vec<ObserverId>>,
    has_nonzero_interest: bool,
    common_interest_bits: u16,
}

impl ObserverIndex {
    fn insert(&mut self, id: ObserverId, interest_mask: u16) {
        if interest_mask == 0 {
            self.wildcard.push(id);
        } else {
            if self.has_nonzero_interest {
                self.common_interest_bits &= interest_mask;
            } else {
                self.has_nonzero_interest = true;
                self.common_interest_bits = interest_mask;
            }

            self.by_interest_mask
                .entry(interest_mask)
                .or_default()
                .push(id);
        }
    }

    fn definitely_dense_for_key(&self, key: WakeKey64) -> bool {
        if self.by_interest_mask.is_empty() {
            return !self.wildcard.is_empty();
        }

        let field_mask = key.field_mask() as u16;
        self.has_nonzero_interest && (self.common_interest_bits & field_mask) != 0
    }

    fn collect_candidates(&self, key: WakeKey64, out: &mut Vec<ObserverId>) {
        out.clear();
        out.extend_from_slice(&self.wildcard);

        for (&interest_mask, ids) in &self.by_interest_mask {
            if key.has_any_field(interest_mask) {
                out.extend_from_slice(ids);
            }
        }
    }

    fn bucket_count(&self) -> usize {
        self.by_interest_mask.len() + usize::from(!self.wildcard.is_empty())
    }

    fn candidate_count(&self, key: WakeKey64) -> usize {
        let mut count = self.wildcard.len();
        for (&interest_mask, ids) in &self.by_interest_mask {
            if key.has_any_field(interest_mask) {
                count += ids.len();
            }
        }
        count
    }
}

/// Main runtime.
///
/// The runtime owns observer slots, records atomic wake epochs, and fans matched
/// packets into delivery profiles. It intentionally does not own the payload
/// arena or the higher-level scheduler; it only carries `PayloadRef` and optional
/// `TargetTaskId` handles.
pub struct ObservableWakeRuntime {
    config: ObservableRuntimeConfig,
    observers: Vec<Option<ObserverEntry>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    index: ObserverIndex,
    index_scratch: Vec<ObserverId>,
    live_observers: usize,
    inline_ready: VecDeque<WakeEvent>,
    condition_ready: VecDeque<WakeEvent>,
    #[cfg(feature = "crossbeam-wake")]
    crossbeam_sender: Option<Sender<WakeEvent>>,
}

impl Default for ObservableWakeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ObservableWakeRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: ObservableRuntimeConfig::default(),
            observers: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            index: ObserverIndex::default(),
            index_scratch: Vec::new(),
            live_observers: 0,
            inline_ready: VecDeque::new(),
            condition_ready: VecDeque::new(),
            #[cfg(feature = "crossbeam-wake")]
            crossbeam_sender: None,
        }
    }


    #[must_use]
    pub fn with_config(config: ObservableRuntimeConfig) -> Self {
        Self {
            config,
            observers: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            index: ObserverIndex::default(),
            index_scratch: Vec::new(),
            live_observers: 0,
            inline_ready: VecDeque::new(),
            condition_ready: VecDeque::new(),
            #[cfg(feature = "crossbeam-wake")]
            crossbeam_sender: None,
        }
    }

    #[must_use]
    pub const fn config(&self) -> ObservableRuntimeConfig {
        self.config
    }

    #[cfg(feature = "crossbeam-wake")]
    #[must_use]
    pub fn with_crossbeam_sender(sender: Sender<WakeEvent>) -> Self {
        let mut runtime = Self::new();
        runtime.crossbeam_sender = Some(sender);
        runtime
    }

    #[cfg(feature = "crossbeam-wake")]
    pub fn set_crossbeam_sender(&mut self, sender: Sender<WakeEvent>) {
        self.crossbeam_sender = Some(sender);
    }

    pub fn register(&mut self, config: ObserverConfig) -> ObserverId {
        let interest_mask = config.interest_mask;
        let id = if let Some(slot) = self.free.pop() {
            let generation = self.generations[slot as usize];
            self.observers[slot as usize] = Some(ObserverEntry::new(generation, config));
            ObserverId::new(slot, generation)
        } else {
            let slot = self.observers.len() as u32;
            self.observers.push(Some(ObserverEntry::new(0, config)));
            self.generations.push(0);
            ObserverId::new(slot, 0)
        };

        self.index.insert(id, interest_mask);
        self.live_observers += 1;
        id
    }

    pub fn remove(&mut self, id: ObserverId) -> Result<(), WakeError> {
        let slot = self
            .observers
            .get_mut(id.slot as usize)
            .ok_or(WakeError::MissingObserver(id))?;

        let Some(entry) = slot else {
            return Err(WakeError::MissingObserver(id));
        };

        if entry.generation != id.generation {
            return Err(WakeError::StaleObserver(id));
        }

        entry.state.0.store(ObserverState::Removed as u64, Ordering::Release);
        self.generations[id.slot as usize] = entry.generation.wrapping_add(1);
        *slot = None;
        self.free.push(id.slot);
        self.live_observers = self.live_observers.saturating_sub(1);
        Ok(())
    }

    pub fn observer_epoch(&self, id: ObserverId) -> Result<u64, WakeError> {
        let entry = self.get_entry(id)?;
        Ok(entry.wake_epoch.0.load(Ordering::Acquire))
    }

    pub fn observer_delivery_epoch(&self, id: ObserverId) -> Result<u64, WakeError> {
        let entry = self.get_entry(id)?;
        Ok(entry.delivery_epoch.0.load(Ordering::Acquire))
    }

    pub fn observer_last_key(&self, id: ObserverId) -> Result<Option<WakeKey64>, WakeError> {
        let entry = self.get_entry(id)?;
        let raw = entry.last_key.0.load(Ordering::Acquire);
        if raw == 0 {
            return Ok(None);
        }
        Ok(WakeKey64::try_from_raw_u128(u128::from(raw)))
    }

    pub fn publish(&mut self, packet: ObservablePacket) -> WakeReport {
        let mut report = WakeReport::default();

        for slot in 0..self.observers.len() {
            report.scanned += 1;

            let Some(entry) = self.observers[slot].as_ref() else {
                continue;
            };

            if entry.state.0.load(Ordering::Acquire) != ObserverState::Active as u64 {
                continue;
            }

            if !entry.config.matches(packet.key) {
                continue;
            }

            report.matched += 1;
            let event = match self.make_event(slot as u32, entry, packet) {
                Ok(event) => event,
                Err(error) => {
                    report.errors.push(error);
                    continue;
                }
            };

            let profile = event.profile;
            match self.deliver(profile, event) {
                Ok(delivery) => match delivery {
                    DeliveryKind::Inline => report.inline_delivered += 1,
                    DeliveryKind::Atomic => report.atomic_delivered += 1,
                    DeliveryKind::Condition => report.condition_delivered += 1,
                    #[cfg(feature = "crossbeam-wake")]
                    DeliveryKind::Crossbeam => report.crossbeam_delivered += 1,
                    DeliveryKind::Manual => report.manual_delivered += 1,
                },
                Err(error) => report.errors.push(error),
            }
        }

        report
    }

    /// Publish through the field-mask index instead of scanning every observer.
    ///
    /// This uses the bitpacked `WakeKey64.field_mask` as the primary routing
    /// surface: observers are bucketed by their declared `interest_mask`, and
    /// only buckets overlapping the event's field mask are examined. The normal
    /// [`publish`](Self::publish) method remains the compatibility scan path.
    pub fn publish_indexed(&mut self, packet: ObservablePacket) -> WakeReport {
        let mut report = WakeReport::default();
        self.index.collect_candidates(packet.key, &mut self.index_scratch);

        let candidates = std::mem::take(&mut self.index_scratch);
        for &id in &candidates {
            report.scanned += 1;

            let event = {
                let Some(Some(entry)) = self.observers.get(id.slot as usize) else {
                    continue;
                };

                if entry.generation != id.generation {
                    continue;
                }

                if entry.state.0.load(Ordering::Acquire) != ObserverState::Active as u64 {
                    continue;
                }

                if !entry.config.matches(packet.key) {
                    continue;
                }

                report.matched += 1;
                match self.make_event(id.slot, entry, packet) {
                    Ok(event) => event,
                    Err(error) => {
                        report.errors.push(error);
                        continue;
                    }
                }
            };

            let profile = event.profile;
            match self.deliver(profile, event) {
                Ok(delivery) => match delivery {
                    DeliveryKind::Inline => report.inline_delivered += 1,
                    DeliveryKind::Atomic => report.atomic_delivered += 1,
                    DeliveryKind::Condition => report.condition_delivered += 1,
                    #[cfg(feature = "crossbeam-wake")]
                    DeliveryKind::Crossbeam => report.crossbeam_delivered += 1,
                    DeliveryKind::Manual => report.manual_delivered += 1,
                },
                Err(error) => report.errors.push(error),
            }
        }

        self.index_scratch = candidates;
        self.index_scratch.clear();
        report
    }

    /// Publish using the runtime's adaptive dispatch policy.
    ///
    /// The compatibility scan path is selected for tiny observer sets or dense
    /// matches. The indexed path is selected only when the bitpacked field-mask
    /// index eliminates enough observers to satisfy [`DispatchPolicy`].
    pub fn publish_auto(&mut self, packet: ObservablePacket) -> WakeReport {
        if self.should_publish_indexed(packet.key) {
            self.publish_indexed(packet)
        } else {
            self.publish(packet)
        }
    }

    #[must_use]
    pub fn should_publish_indexed(&self, key: WakeKey64) -> bool {
        let total = self.live_observers;
        let policy = self.config.dispatch_policy;

        // Keep tiny/dense hot sets on the scan path without paying the indexed
        // candidate-count cost. This is the common auto-dispatch fast reject.
        if !policy.can_consider_hot_indexed(total) {
            return false;
        }

        // If all indexed interest-mask buckets are definitely touched by this
        // key, the indexed route cannot reduce the candidate set. Fall straight
        // back to scan without computing the full candidate count. This keeps
        // hot auto dispatch close to scan cost for dense observer layouts.
        if self.index.definitely_dense_for_key(key) {
            return false;
        }

        let candidates = self.indexed_candidate_count(key);
        policy.should_use_hot_indexed(total, candidates)
    }

    pub fn drain_inline(&mut self) -> Vec<WakeEvent> {
        self.inline_ready.drain(..).collect()
    }

    pub fn drain_condition(&mut self) -> Vec<WakeEvent> {
        self.condition_ready.drain(..).collect()
    }

    #[must_use]
    pub fn inline_len(&self) -> usize {
        self.inline_ready.len()
    }

    #[must_use]
    pub fn condition_len(&self) -> usize {
        self.condition_ready.len()
    }

    #[must_use]
    pub fn indexed_bucket_count(&self) -> usize {
        self.index.bucket_count()
    }

    #[must_use]
    pub fn indexed_candidate_count(&self, key: WakeKey64) -> usize {
        self.index.candidate_count(key)
    }

    #[must_use]
    pub fn live_observer_count(&self) -> usize {
        self.live_observers
    }

    fn get_entry(&self, id: ObserverId) -> Result<&ObserverEntry, WakeError> {
        let Some(Some(entry)) = self.observers.get(id.slot as usize) else {
            return Err(WakeError::MissingObserver(id));
        };

        if entry.generation != id.generation {
            return Err(WakeError::StaleObserver(id));
        }

        Ok(entry)
    }

    fn make_event(
        &self,
        slot: u32,
        entry: &ObserverEntry,
        packet: ObservablePacket,
    ) -> Result<WakeEvent, WakeError> {
        let reason = packet.key.reason_code()?;
        let profile = entry.config.profile;
        // Publish the key before the epoch bump. External readers that observe
        // the new epoch with an Acquire load can then safely load the recorded
        // key as the wake cause for that epoch.
        entry
            .last_key
            .0
            .store(packet.key.raw_storage(), Ordering::Release);
        let epoch = entry.wake_epoch.0.fetch_add(1, Ordering::AcqRel) + 1;

        Ok(WakeEvent {
            observer_id: entry.id(slot),
            target: entry.config.target,
            key: packet.key,
            payload: packet.payload,
            reason,
            profile,
            wake_epoch: epoch,
        })
    }

    fn deliver(&mut self, profile: WakeProfileCode, event: WakeEvent) -> Result<DeliveryKind, WakeError> {
        match profile {
            WakeProfileCode::InlineQueue => {
                self.bump_delivery_epoch(event.observer_id)?;
                self.inline_ready.push_back(event);
                Ok(DeliveryKind::Inline)
            }
            WakeProfileCode::AtomicEpoch => {
                self.bump_delivery_epoch(event.observer_id)?;
                Ok(DeliveryKind::Atomic)
            }
            WakeProfileCode::ConditionCache => {
                self.bump_delivery_epoch(event.observer_id)?;
                self.condition_ready.push_back(event);
                Ok(DeliveryKind::Condition)
            }
            WakeProfileCode::Manual => {
                self.bump_delivery_epoch(event.observer_id)?;
                Ok(DeliveryKind::Manual)
            }
            WakeProfileCode::CrossbeamQueue => self.deliver_crossbeam(event),
        }
    }

    fn bump_delivery_epoch(&self, id: ObserverId) -> Result<u64, WakeError> {
        let entry = self.get_entry(id)?;
        Ok(entry.delivery_epoch.0.fetch_add(1, Ordering::AcqRel) + 1)
    }

    #[cfg(feature = "crossbeam-wake")]
    fn deliver_crossbeam(&mut self, event: WakeEvent) -> Result<DeliveryKind, WakeError> {
        self.bump_delivery_epoch(event.observer_id)?;
        let Some(sender) = &self.crossbeam_sender else {
            return Err(WakeError::CrossbeamSenderMissing);
        };
        sender
            .send(event)
            .map_err(|_| WakeError::CrossbeamSendFailed)?;
        Ok(DeliveryKind::Crossbeam)
    }

    #[cfg(not(feature = "crossbeam-wake"))]
    fn deliver_crossbeam(&mut self, _event: WakeEvent) -> Result<DeliveryKind, WakeError> {
        Err(WakeError::CrossbeamFeatureDisabled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryKind {
    Inline,
    Atomic,
    Condition,
    #[cfg(feature = "crossbeam-wake")]
    Crossbeam,
    Manual,
}

/// Helper for callers that want a matcher set for common cold-observer routing.
#[must_use]
pub fn cold_condition_matchers(min_qos: u128, interest_mask: u128) -> Vec<FieldMatcher> {
    vec![
        FieldMatcher::eq("reason", WakeReasonCode::Condition.as_u64().into()),
        FieldMatcher::range("qos", min_qos, u128::from(u8::MAX)),
        FieldMatcher::mask_any("field_mask", interest_mask),
    ]
}

/// Helper for callers that want a matcher set for timer-driven observers.
#[must_use]
pub fn timer_matchers(phase: u128) -> Vec<FieldMatcher> {
    vec![
        FieldMatcher::eq("reason", WakeReasonCode::Timer.as_u64().into()),
        FieldMatcher::eq("phase", phase),
    ]
}

// The crate's ordinary tests use std atomics directly; under `--cfg loom`
// those would touch loom atomics outside a `loom::model` and panic, so they
// compile only in the normal build. The loom gate has its own module below.
#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    const FIELD_HEALTH: u16 = 1 << 0;
    const FIELD_QUEUE: u16 = 1 << 1;

    fn condition_key(mask: u16, profile: WakeProfileCode) -> WakeKey64 {
        WakeKey64::typed(
            WakeReasonCode::Condition,
            profile,
            4,
            mask,
            7,
            WakeKey64::FLAG_COLD as u16,
        )
        .unwrap()
    }

    #[test]
    fn wake_key_round_trips_typed_fields() {
        let key = condition_key(FIELD_HEALTH | FIELD_QUEUE, WakeProfileCode::InlineQueue);
        assert_eq!(key.reason_code().unwrap(), WakeReasonCode::Condition);
        assert_eq!(key.profile_code().unwrap(), WakeProfileCode::InlineQueue);
        assert_eq!(key.qos(), 4);
        assert_eq!(key.field_mask(), u64::from(FIELD_HEALTH | FIELD_QUEUE));
        assert_eq!(key.phase(), 7);
        assert!(key.has_flags(WakeKey64::FLAG_COLD as u16));
    }

    #[test]
    fn wake_key_checked_constructor_rejects_overflow() {
        let err = WakeKey64::new_checked(256, 1, 1, 1, 1, 1).unwrap_err();
        assert!(matches!(err.kind, big_bad_bool::BitpackErrorKind::FieldOverflow { field: "reason", .. }));
    }

    #[test]
    fn cold_condition_helpers_match_condition_key() {
        let key = condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue);
        let matchers = cold_condition_matchers(3, u128::from(FIELD_QUEUE));
        assert!(matchers.iter().copied().all(|m| m.matches(key)));
    }

    #[test]
    fn cold_condition_helpers_reject_wrong_mask() {
        let key = condition_key(FIELD_HEALTH, WakeProfileCode::InlineQueue);
        let matchers = cold_condition_matchers(3, u128::from(FIELD_QUEUE));
        assert!(!matchers.iter().copied().all(|m| m.matches(key)));
    }

    #[test]
    fn observer_config_checks_interest_mask_before_matchers() {
        let cfg = ObserverConfig::new("queue", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_QUEUE)
            .with_matchers(cold_condition_matchers(1, u128::from(FIELD_QUEUE)));
        assert!(cfg.matches(condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue)));
        assert!(!cfg.matches(condition_key(FIELD_HEALTH, WakeProfileCode::InlineQueue)));
    }

    #[test]
    fn inactive_observer_config_does_not_match() {
        let cfg = ObserverConfig::new("inactive", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_QUEUE)
            .inactive();
        assert!(!cfg.matches(condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue)));
    }

    #[test]
    fn runtime_registers_and_counts_observers() {
        let mut runtime = ObservableWakeRuntime::new();
        let id = runtime.register(ObserverConfig::new("obs", WakeProfileCode::Manual));
        assert_eq!(id, ObserverId::new(0, 0));
        assert_eq!(runtime.live_observer_count(), 1);
    }

    #[test]
    fn runtime_reuses_slots_with_generation_increment() {
        let mut runtime = ObservableWakeRuntime::new();
        let first = runtime.register(ObserverConfig::new("first", WakeProfileCode::Manual));
        runtime.remove(first).unwrap();
        let second = runtime.register(ObserverConfig::new("second", WakeProfileCode::Manual));
        assert_eq!(first.slot, second.slot);
        assert_ne!(first.generation, second.generation);
        assert!(matches!(runtime.observer_epoch(first), Err(WakeError::StaleObserver(_)) | Err(WakeError::MissingObserver(_))));
    }

    #[test]
    fn inline_profile_queues_wake_event() {
        let mut runtime = ObservableWakeRuntime::new();
        let id = runtime.register(
            ObserverConfig::new("inline", WakeProfileCode::InlineQueue)
                .with_interest_mask(FIELD_QUEUE),
        );
        let report = runtime.publish(ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
            PayloadRef::new(9, 1, 20, 4),
        ));
        assert_eq!(report.matched, 1);
        assert_eq!(report.inline_delivered, 1);
        assert_eq!(runtime.inline_len(), 1);
        assert_eq!(runtime.observer_epoch(id).unwrap(), 1);
        assert_eq!(runtime.observer_delivery_epoch(id).unwrap(), 1);
        let events = runtime.drain_inline();
        assert_eq!(events[0].observer_id, id);
        assert_eq!(events[0].payload.slot, 9);
    }

    #[test]
    fn atomic_profile_only_updates_epochs() {
        let mut runtime = ObservableWakeRuntime::new();
        let id = runtime.register(
            ObserverConfig::new("atomic", WakeProfileCode::AtomicEpoch)
                .with_interest_mask(FIELD_QUEUE),
        );
        let report = runtime.publish(ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch),
            PayloadRef::default(),
        ));
        assert_eq!(report.atomic_delivered, 1);
        assert_eq!(runtime.inline_len(), 0);
        assert_eq!(runtime.condition_len(), 0);
        assert_eq!(runtime.observer_epoch(id).unwrap(), 1);
        assert_eq!(runtime.observer_delivery_epoch(id).unwrap(), 1);
    }

    #[test]
    fn condition_cache_profile_records_condition_queue() {
        let mut runtime = ObservableWakeRuntime::new();
        let id = runtime.register(
            ObserverConfig::new("condition-cache", WakeProfileCode::ConditionCache)
                .with_interest_mask(FIELD_HEALTH),
        );
        let report = runtime.publish(ObservablePacket::new(
            condition_key(FIELD_HEALTH, WakeProfileCode::ConditionCache),
            PayloadRef::default(),
        ));
        assert_eq!(report.condition_delivered, 1);
        assert_eq!(runtime.condition_len(), 1);
        assert_eq!(runtime.observer_epoch(id).unwrap(), 1);
        let event = runtime.drain_condition().pop().unwrap();
        assert_eq!(event.reason, WakeReasonCode::Condition);
    }

    #[test]
    fn manual_profile_matches_without_queueing() {
        let mut runtime = ObservableWakeRuntime::new();
        let id = runtime.register(
            ObserverConfig::new("manual", WakeProfileCode::Manual).with_interest_mask(FIELD_HEALTH),
        );
        let report = runtime.publish(ObservablePacket::new(
            condition_key(FIELD_HEALTH, WakeProfileCode::Manual),
            PayloadRef::default(),
        ));
        assert_eq!(report.manual_delivered, 1);
        assert_eq!(runtime.inline_len(), 0);
        assert_eq!(runtime.observer_epoch(id).unwrap(), 1);
    }

    #[test]
    fn non_matching_mask_does_not_wake() {
        let mut runtime = ObservableWakeRuntime::new();
        let id = runtime.register(
            ObserverConfig::new("queue", WakeProfileCode::InlineQueue).with_interest_mask(FIELD_QUEUE),
        );
        let report = runtime.publish(ObservablePacket::new(
            condition_key(FIELD_HEALTH, WakeProfileCode::InlineQueue),
            PayloadRef::default(),
        ));
        assert_eq!(report.matched, 0);
        assert_eq!(runtime.observer_epoch(id).unwrap(), 0);
        assert_eq!(runtime.inline_len(), 0);
    }

    #[test]
    fn multiple_observers_can_match_same_packet_with_different_profiles() {
        let mut runtime = ObservableWakeRuntime::new();
        runtime.register(
            ObserverConfig::new("inline", WakeProfileCode::InlineQueue).with_interest_mask(FIELD_QUEUE),
        );
        runtime.register(
            ObserverConfig::new("atomic", WakeProfileCode::AtomicEpoch).with_interest_mask(FIELD_QUEUE),
        );
        runtime.register(
            ObserverConfig::new("manual", WakeProfileCode::Manual).with_interest_mask(FIELD_QUEUE),
        );
        let report = runtime.publish(ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
            PayloadRef::default(),
        ));
        assert_eq!(report.matched, 3);
        assert_eq!(report.inline_delivered, 1);
        assert_eq!(report.atomic_delivered, 1);
        assert_eq!(report.manual_delivered, 1);
        assert_eq!(report.delivered(), 3);
    }

    #[test]
    fn field_matchers_can_filter_by_phase() {
        let mut runtime = ObservableWakeRuntime::new();
        runtime.register(
            ObserverConfig::new("timer-phase-9", WakeProfileCode::InlineQueue)
                .with_matchers(timer_matchers(9)),
        );
        let timer_phase_7 = WakeKey64::typed(
            WakeReasonCode::Timer,
            WakeProfileCode::InlineQueue,
            3,
            0,
            7,
            0,
        )
        .unwrap();
        let report = runtime.publish(ObservablePacket::new(timer_phase_7, PayloadRef::default()));
        assert_eq!(report.matched, 0);
    }

    #[test]
    fn field_matchers_accept_correct_phase() {
        let mut runtime = ObservableWakeRuntime::new();
        runtime.register(
            ObserverConfig::new("timer-phase-7", WakeProfileCode::InlineQueue)
                .with_matchers(timer_matchers(7)),
        );
        let timer_phase_7 = WakeKey64::typed(
            WakeReasonCode::Timer,
            WakeProfileCode::InlineQueue,
            3,
            0,
            7,
            0,
        )
        .unwrap();
        let report = runtime.publish(ObservablePacket::new(timer_phase_7, PayloadRef::default()));
        assert_eq!(report.matched, 1);
        assert_eq!(report.inline_delivered, 1);
    }

    #[test]
    fn last_key_is_recorded_for_matched_observer() {
        let mut runtime = ObservableWakeRuntime::new();
        let id = runtime.register(
            ObserverConfig::new("last-key", WakeProfileCode::AtomicEpoch)
                .with_interest_mask(FIELD_QUEUE),
        );
        let key = condition_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);
        let _ = runtime.publish(ObservablePacket::new(key, PayloadRef::default()));
        assert_eq!(runtime.observer_last_key(id).unwrap(), Some(key));
    }

    #[test]
    fn unknown_reason_is_reported_not_panicked() {
        let mut runtime = ObservableWakeRuntime::new();
        runtime.register(ObserverConfig::new("bad-reason", WakeProfileCode::Manual));
        let key = WakeKey64::new_checked(200, WakeProfileCode::Manual.as_u64(), 1, 0, 0, 0).unwrap();
        let report = runtime.publish(ObservablePacket::new(key, PayloadRef::default()));
        assert_eq!(report.matched, 1);
        assert_eq!(report.errors, vec![WakeError::UnknownReason(200)]);
    }

    #[cfg(not(feature = "crossbeam-wake"))]
    #[test]
    fn crossbeam_profile_without_feature_is_reported() {
        let mut runtime = ObservableWakeRuntime::new();
        runtime.register(
            ObserverConfig::new("crossbeam", WakeProfileCode::CrossbeamQueue)
                .with_interest_mask(FIELD_QUEUE),
        );
        let report = runtime.publish(ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::CrossbeamQueue),
            PayloadRef::default(),
        ));
        assert_eq!(report.errors, vec![WakeError::CrossbeamFeatureDisabled]);
    }
    #[test]
    fn indexed_publish_matches_scan_but_scans_only_candidate_buckets() {
        fn build_runtime() -> ObservableWakeRuntime {
            let mut runtime = ObservableWakeRuntime::new();
            for _ in 0..10 {
                runtime.register(
                    ObserverConfig::new("health", WakeProfileCode::AtomicEpoch)
                        .with_interest_mask(FIELD_HEALTH),
                );
            }
            for _ in 0..2 {
                runtime.register(
                    ObserverConfig::new("queue", WakeProfileCode::AtomicEpoch)
                        .with_interest_mask(FIELD_QUEUE),
                );
            }
            runtime
        }

        let packet = ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch),
            PayloadRef::default(),
        );

        let mut scan = build_runtime();
        let mut indexed = build_runtime();

        let scan_report = scan.publish(packet);
        let indexed_report = indexed.publish_indexed(packet);

        assert_eq!(scan_report.matched, indexed_report.matched);
        assert_eq!(scan_report.atomic_delivered, indexed_report.atomic_delivered);
        assert_eq!(scan_report.scanned, 12);
        assert_eq!(indexed_report.scanned, 2);
        assert_eq!(indexed.indexed_candidate_count(packet.key), 2);
    }

    #[test]
    fn indexed_publish_handles_multi_field_interest_masks() {
        let mut runtime = ObservableWakeRuntime::new();
        let id = runtime.register(
            ObserverConfig::new("multi", WakeProfileCode::AtomicEpoch)
                .with_interest_mask(FIELD_HEALTH | FIELD_QUEUE),
        );

        let report = runtime.publish_indexed(ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch),
            PayloadRef::default(),
        ));

        assert_eq!(report.scanned, 1);
        assert_eq!(report.matched, 1);
        assert_eq!(runtime.observer_epoch(id).unwrap(), 1);
    }


    #[test]
    fn publish_auto_uses_scan_for_small_dense_sets() {
        let mut runtime = ObservableWakeRuntime::new();
        for _ in 0..8 {
            runtime.register(
                ObserverConfig::new("queue", WakeProfileCode::AtomicEpoch)
                    .with_interest_mask(FIELD_QUEUE),
            );
        }

        let packet = ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch),
            PayloadRef::default(),
        );

        assert!(!runtime.should_publish_indexed(packet.key));
        let report = runtime.publish_auto(packet);
        assert_eq!(report.scanned, 8);
        assert_eq!(report.matched, 8);
    }

    #[test]
    fn publish_auto_uses_index_for_large_sparse_sets() {
        let mut runtime = ObservableWakeRuntime::with_config(ObservableRuntimeConfig {
            dispatch_policy: DispatchPolicy::new(16, 25),
            ..ObservableRuntimeConfig::default()
        });

        for _ in 0..96 {
            runtime.register(
                ObserverConfig::new("health", WakeProfileCode::AtomicEpoch)
                    .with_interest_mask(FIELD_HEALTH),
            );
        }
        for _ in 0..32 {
            runtime.register(
                ObserverConfig::new("queue", WakeProfileCode::AtomicEpoch)
                    .with_interest_mask(FIELD_QUEUE),
            );
        }

        let packet = ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch),
            PayloadRef::default(),
        );

        assert!(runtime.should_publish_indexed(packet.key));
        assert_eq!(runtime.indexed_candidate_count(packet.key), 32);
        let report = runtime.publish_auto(packet);
        assert_eq!(report.scanned, 32);
        assert_eq!(report.matched, 32);
    }

    #[test]
    fn publish_auto_uses_scan_for_large_dense_sets() {
        let mut runtime = ObservableWakeRuntime::with_config(ObservableRuntimeConfig {
            dispatch_policy: DispatchPolicy::new(16, 25),
            ..ObservableRuntimeConfig::default()
        });

        for _ in 0..128 {
            runtime.register(
                ObserverConfig::new("queue", WakeProfileCode::AtomicEpoch)
                    .with_interest_mask(FIELD_QUEUE),
            );
        }

        let packet = ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch),
            PayloadRef::default(),
        );

        assert!(!runtime.should_publish_indexed(packet.key));
        let report = runtime.publish_auto(packet);
        assert_eq!(report.scanned, 128);
        assert_eq!(report.matched, 128);
    }

}

// ===========================================================================
// THE LOOM GATE — concurrent memory-ordering verification of the wake/epoch
// protocol. Miri (scripts/miri_gate.sh) checks single-threaded UB; loom
// checks that the Acquire/Release/AcqRel discipline on the REAL
// `CacheLineAtomicU64` is correct under EVERY thread interleaving and
// weak-memory reordering. Run: scripts/loom_gate.sh (RUSTFLAGS="--cfg loom").
//
// These drive the real `CacheLineAtomicU64` type with the exact operations and
// orderings the runtime uses on the wake path:
//   waker:  last_key.store(K, Release);  wake_epoch.fetch_add(1, AcqRel)   (lib.rs ~1068)
//   reader: e = wake_epoch.load(Acquire); key = last_key.load(Acquire)     (observer_epoch / last_wake_key)
// ===========================================================================
#[cfg(loom)]
mod loom_wake_tests {
    use super::CacheLineAtomicU64;
    use crate::sync::atomic::Ordering;
    use crate::sync::{thread, Arc};

    const WAKE_KEY: u64 = 0xA5A5_1234;

    /// MESSAGE PASSING: a reader that observes the bumped wake epoch MUST also
    /// observe the published key. If the Release/Acquire pairing were wrong
    /// (e.g. a Relaxed slipped in), loom would find an interleaving where the
    /// epoch is seen fresh but the key is still stale — a torn/lost wake.
    #[test]
    fn a_reader_that_sees_the_epoch_sees_the_key() {
        loom::model(|| {
            let wake_epoch = Arc::new(CacheLineAtomicU64::new(0));
            let last_key = Arc::new(CacheLineAtomicU64::new(0));

            let (we, lk) = (wake_epoch.clone(), last_key.clone());
            let waker = thread::spawn(move || {
                lk.0.store(WAKE_KEY, Ordering::Release);
                we.0.fetch_add(1, Ordering::AcqRel);
            });

            // Reader: exactly the runtime's observe order — epoch first, then key.
            if wake_epoch.0.load(Ordering::Acquire) >= 1 {
                assert_eq!(
                    last_key.0.load(Ordering::Acquire),
                    WAKE_KEY,
                    "observed a bumped epoch with a stale key — the wake was torn/lost"
                );
            }
            waker.join().unwrap();
        });
    }

    /// NO LOST WAKE: two concurrent wakers each fetch_add the epoch; under
    /// AcqRel neither increment may be lost, whatever the interleaving.
    #[test]
    fn concurrent_wakes_never_lose_an_increment() {
        loom::model(|| {
            let wake_epoch = Arc::new(CacheLineAtomicU64::new(0));
            let a = wake_epoch.clone();
            let b = wake_epoch.clone();
            let ta = thread::spawn(move || {
                a.0.fetch_add(1, Ordering::AcqRel);
            });
            let tb = thread::spawn(move || {
                b.0.fetch_add(1, Ordering::AcqRel);
            });
            ta.join().unwrap();
            tb.join().unwrap();
            assert_eq!(
                wake_epoch.0.load(Ordering::Acquire),
                2,
                "an increment was lost — a wake was dropped"
            );
        });
    }

    /// DELIVERY ORDERING: wake then delivery epochs advance independently; a
    /// reader observing delivery >= wake must never see delivery ahead of a
    /// wake it hasn't observed (the runtime's back-pressure invariant).
    #[test]
    fn delivery_never_races_ahead_of_an_unobserved_wake() {
        loom::model(|| {
            let wake_epoch = Arc::new(CacheLineAtomicU64::new(0));
            let delivery_epoch = Arc::new(CacheLineAtomicU64::new(0));

            let (we, de) = (wake_epoch.clone(), delivery_epoch.clone());
            let worker = thread::spawn(move || {
                we.0.fetch_add(1, Ordering::AcqRel); // wake published
                de.0.fetch_add(1, Ordering::AcqRel); // then delivered
            });

            let delivered = delivery_epoch.0.load(Ordering::Acquire);
            if delivered >= 1 {
                // If we saw a delivery, the wake that caused it is at least as advanced.
                assert!(
                    wake_epoch.0.load(Ordering::Acquire) >= delivered,
                    "delivery epoch ran ahead of the wake epoch — inverted ordering"
                );
            }
            worker.join().unwrap();
        });
    }
}
