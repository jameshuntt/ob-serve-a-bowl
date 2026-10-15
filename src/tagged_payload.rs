//! Optional tagged-payload integration for the observable wake runtime.
//!
//! This module keeps the observable wake plane separate from the payload plane:
//!
//! - [`WakeKey64`] still decides which observers match,
//! - [`WakeProfileCode`] still decides the delivery mode,
//! - [`TaggedPayloadRef`] is only resolved after an observer has already matched.
//!
//! The handle is intentionally encoded/opaque. It is not a raw pointer and it
//! has no authority without the owning [`TaggedPayloadArena`].

use core::marker::PhantomData;

use big_bad_bool::PackedWord;
use petr_collections::{EncodedArenaHandle, KeyedArena, KeyedArenaError, KeyedArenaHandle};
use petr_core::{Key, ProphecyBits, SelectorWord, TagError};

use crate::{
    ObservableWakeRuntime, ObserverId, ObserverState, TargetTaskId, WakeDecodeError, WakeKey64,
    WakeProfileCode, WakeReasonCode,
};
use std::sync::atomic::Ordering;

/// Opaque, encoded handle to a payload stored in a [`TaggedPayloadArena`].
///
/// The handle is copyable and safe to log/pass around, but it must be validated
/// by the owning arena before it can produce a reference.
#[derive(Debug, PartialEq, Eq)]
pub struct TaggedPayloadRef<T, const LOW_BITS: usize> {
    encoded: EncodedArenaHandle,
    _marker: PhantomData<fn() -> T>,
}

impl<T, const LOW_BITS: usize> Copy for TaggedPayloadRef<T, LOW_BITS> {}

impl<T, const LOW_BITS: usize> Clone for TaggedPayloadRef<T, LOW_BITS> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, const LOW_BITS: usize> TaggedPayloadRef<T, LOW_BITS> {
    /// Rebuild a tagged payload reference from encoded metadata.
    #[must_use]
    pub const fn from_encoded(encoded: EncodedArenaHandle) -> Self {
        Self {
            encoded,
            _marker: PhantomData,
        }
    }

    /// Return the encoded/session-local representation.
    #[must_use]
    pub const fn encoded(self) -> EncodedArenaHandle {
        self.encoded
    }

    /// Slot index in the issuing arena.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.encoded.index
    }

    /// Slot generation in the issuing arena.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.encoded.generation
    }

    /// Raw keyed check word. This is not a pointer authority.
    #[must_use]
    pub const fn check_word(self) -> usize {
        self.encoded.check
    }

    /// Raw 4/4/8 selector word carried by the payload handle.
    #[must_use]
    pub const fn selector_raw(self) -> u16 {
        self.encoded.selector
    }

    /// Selector metadata carried by the payload handle.
    #[must_use]
    pub const fn selector(self) -> SelectorWord {
        SelectorWord::new(
            ((self.encoded.selector >> 12) & 0x0f) as u8,
            ((self.encoded.selector >> 8) & 0x0f) as u8,
            (self.encoded.selector & 0xff) as u8,
        )
    }

    /// Packed prophecy/hint byte carried by the payload handle.
    #[must_use]
    pub const fn prophecy_raw(self) -> u8 {
        self.encoded.prophecy
    }

    /// Prophecy metadata carried by the payload handle.
    #[must_use]
    pub const fn prophecy(self) -> ProphecyBits {
        ProphecyBits::unpack(self.encoded.prophecy)
    }

    /// Low tag copied from the keyed check word for logging/debugging.
    #[must_use]
    pub const fn tag(self) -> u16 {
        self.encoded.tag
    }

    /// Snapshot the non-authoritative handle metadata.
    #[must_use]
    pub const fn meta(self) -> TaggedPayloadMeta {
        TaggedPayloadMeta {
            index: self.index(),
            generation: self.generation(),
            selector: self.selector(),
            prophecy: self.prophecy(),
            tag: self.tag(),
        }
    }

    fn to_live_handle(self) -> Result<KeyedArenaHandle<T, LOW_BITS>, TaggedPayloadError> {
        self.encoded
            .to_handle::<T, LOW_BITS>()
            .ok_or(TaggedPayloadError::InvalidEncodedHandle)
    }
}

impl<T, const LOW_BITS: usize> From<KeyedArenaHandle<T, LOW_BITS>>
    for TaggedPayloadRef<T, LOW_BITS>
{
    fn from(handle: KeyedArenaHandle<T, LOW_BITS>) -> Self {
        Self::from_encoded(EncodedArenaHandle::from_handle(handle))
    }
}

/// Non-authoritative metadata copied from a tagged payload handle.
///
/// This is useful for logging, routing, and late dispatch. It must never be
/// treated as payload access authority; the owning arena still validates the
/// encoded handle before returning a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaggedPayloadMeta {
    pub index: u32,
    pub generation: u32,
    pub selector: SelectorWord,
    pub prophecy: ProphecyBits,
    pub tag: u16,
}

/// Payload arena that owns values and issues generation/check/selector/prophecy
/// validated handles.
#[derive(Debug)]
pub struct TaggedPayloadArena<T, const LOW_BITS: usize> {
    inner: KeyedArena<T, LOW_BITS>,
}

impl<T, const LOW_BITS: usize> Default for TaggedPayloadArena<T, LOW_BITS> {
    fn default() -> Self {
        Self::new_randomized()
    }
}

impl<T, const LOW_BITS: usize> TaggedPayloadArena<T, LOW_BITS> {
    /// Create an arena with randomized key material.
    #[must_use]
    pub fn new_randomized() -> Self {
        Self {
            inner: KeyedArena::new_randomized(),
        }
    }

    /// Create an arena with deterministic key material, useful for tests.
    #[must_use]
    pub fn with_key(key: Key<LOW_BITS>) -> Self {
        Self {
            inner: KeyedArena::with_key(key),
        }
    }

    /// Return the arena key value.
    #[must_use]
    pub const fn key(&self) -> Key<LOW_BITS> {
        self.inner.key()
    }

    /// Return the number of live payloads.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.inner.len()
    }

    /// Return true when no payloads are live.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Number of allocated slots, including vacant reusable slots.
    #[must_use]
    pub fn slot_capacity(&self) -> usize {
        self.inner.slot_capacity()
    }

    /// Number of vacant reusable slots.
    #[must_use]
    pub fn free_len(&self) -> usize {
        self.inner.free_len()
    }

    /// Reserve backing slot capacity.
    pub fn reserve_slots(&mut self, additional: usize) {
        self.inner.reserve_slots(additional);
    }

    /// Insert a payload and return an opaque tagged payload reference.
    pub fn insert(
        &mut self,
        value: T,
        tag: usize,
        selector: SelectorWord,
        prophecy: ProphecyBits,
    ) -> Result<TaggedPayloadRef<T, LOW_BITS>, TaggedPayloadError> {
        let handle = self.inner.insert(value, tag, selector, prophecy)?;
        Ok(TaggedPayloadRef::from(handle))
    }

    /// Validate a tagged payload reference against this owning arena.
    pub fn validate(
        &self,
        handle: TaggedPayloadRef<T, LOW_BITS>,
    ) -> Result<(), TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        self.inner.validate_handle(live)?;
        Ok(())
    }

    /// Return true when this arena currently owns a live payload for `handle`.
    #[must_use]
    pub fn contains(&self, handle: TaggedPayloadRef<T, LOW_BITS>) -> bool {
        self.validate(handle).is_ok()
    }

    /// Resolve a tagged payload reference into a shared reference.
    pub fn get(&self, handle: TaggedPayloadRef<T, LOW_BITS>) -> Result<&T, TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        Ok(self.inner.get(live)?)
    }

    /// Resolve a tagged payload reference into a mutable reference.
    pub fn get_mut(
        &mut self,
        handle: TaggedPayloadRef<T, LOW_BITS>,
    ) -> Result<&mut T, TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        Ok(self.inner.get_mut(live)?)
    }

    /// Return selector metadata after arena validation.
    pub fn selector(
        &self,
        handle: TaggedPayloadRef<T, LOW_BITS>,
    ) -> Result<SelectorWord, TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        Ok(self.inner.selector(live)?)
    }

    /// Return prophecy metadata after arena validation.
    pub fn prophecy(
        &self,
        handle: TaggedPayloadRef<T, LOW_BITS>,
    ) -> Result<ProphecyBits, TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        Ok(self.inner.prophecy(live)?)
    }

    /// Return all currently live handles in slot order.
    #[must_use]
    pub fn live_handles(&self) -> Vec<TaggedPayloadRef<T, LOW_BITS>> {
        self.inner
            .live_handles()
            .into_iter()
            .map(TaggedPayloadRef::from)
            .collect()
    }

    /// Remove a payload. The old handle becomes invalid immediately.
    pub fn remove(
        &mut self,
        handle: TaggedPayloadRef<T, LOW_BITS>,
    ) -> Result<T, TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        Ok(self.inner.remove(live)?)
    }

    /// Refresh only the generation, invalidating older cached handles without
    /// moving the payload.
    pub fn refresh_generation(
        &mut self,
        handle: TaggedPayloadRef<T, LOW_BITS>,
    ) -> Result<TaggedPayloadRef<T, LOW_BITS>, TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        Ok(TaggedPayloadRef::from(self.inner.refresh_generation(live)?))
    }

    /// Update selector metadata and return the refreshed handle.
    pub fn set_selector(
        &mut self,
        handle: TaggedPayloadRef<T, LOW_BITS>,
        selector: SelectorWord,
    ) -> Result<TaggedPayloadRef<T, LOW_BITS>, TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        Ok(TaggedPayloadRef::from(self.inner.set_selector(live, selector)?))
    }

    /// Update prophecy metadata and return the refreshed handle.
    pub fn set_prophecy(
        &mut self,
        handle: TaggedPayloadRef<T, LOW_BITS>,
        prophecy: ProphecyBits,
    ) -> Result<TaggedPayloadRef<T, LOW_BITS>, TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        Ok(TaggedPayloadRef::from(self.inner.set_prophecy(live, prophecy)?))
    }

    /// Update the low tag and return the refreshed handle.
    pub fn set_tag(
        &mut self,
        handle: TaggedPayloadRef<T, LOW_BITS>,
        tag: usize,
    ) -> Result<TaggedPayloadRef<T, LOW_BITS>, TaggedPayloadError> {
        let live = handle.to_live_handle()?;
        Ok(TaggedPayloadRef::from(self.inner.set_tag(live, tag)?))
    }

    /// Remove all live payloads and invalidate previous handles.
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

/// Generic observable envelope: same wake key, caller-selected payload handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservableEnvelope<H> {
    pub key: WakeKey64,
    pub payload: H,
}

impl<H> ObservableEnvelope<H> {
    #[must_use]
    pub const fn new(key: WakeKey64, payload: H) -> Self {
        Self { key, payload }
    }
}

/// Wake event carrying a tagged payload reference.
#[derive(Debug, PartialEq, Eq)]
pub struct TaggedWakeEvent<T, const LOW_BITS: usize> {
    pub observer_id: ObserverId,
    pub target: Option<TargetTaskId>,
    pub key: WakeKey64,
    pub payload: TaggedPayloadRef<T, LOW_BITS>,
    pub reason: WakeReasonCode,
    pub profile: WakeProfileCode,
    pub wake_epoch: u64,
}

impl<T, const LOW_BITS: usize> Copy for TaggedWakeEvent<T, LOW_BITS> {}

impl<T, const LOW_BITS: usize> Clone for TaggedWakeEvent<T, LOW_BITS> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, const LOW_BITS: usize> TaggedWakeEvent<T, LOW_BITS> {
    /// Resolve the event payload through the owning arena.
    pub fn resolve<'a>(
        &self,
        arena: &'a TaggedPayloadArena<T, LOW_BITS>,
    ) -> Result<&'a T, TaggedPayloadError> {
        arena.get(self.payload)
    }

    /// Resolve the event payload and keep the event metadata beside it.
    pub fn resolve_with_event<'a>(
        &self,
        arena: &'a TaggedPayloadArena<T, LOW_BITS>,
    ) -> Result<TaggedResolvedWakeEvent<'a, T, LOW_BITS>, TaggedPayloadError> {
        Ok(TaggedResolvedWakeEvent {
            event: *self,
            payload: self.resolve(arena)?,
        })
    }
}

/// Resolved tagged wake event.
#[derive(Debug)]
pub struct TaggedResolvedWakeEvent<'a, T, const LOW_BITS: usize> {
    pub event: TaggedWakeEvent<T, LOW_BITS>,
    pub payload: &'a T,
}

/// Report returned by [`ObservableWakeRuntime::publish_tagged`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedWakeReport<T, const LOW_BITS: usize> {
    pub scanned: usize,
    pub matched: usize,
    pub inline_delivered: usize,
    pub atomic_delivered: usize,
    pub condition_delivered: usize,
    pub manual_delivered: usize,
    pub events: Vec<TaggedWakeEvent<T, LOW_BITS>>,
    pub errors: Vec<TaggedWakeError>,
}

impl<T, const LOW_BITS: usize> Default for TaggedWakeReport<T, LOW_BITS> {
    fn default() -> Self {
        Self {
            scanned: 0,
            matched: 0,
            inline_delivered: 0,
            atomic_delivered: 0,
            condition_delivered: 0,
            manual_delivered: 0,
            events: Vec::new(),
            errors: Vec::new(),
        }
    }
}

impl<T, const LOW_BITS: usize> TaggedWakeReport<T, LOW_BITS> {
    /// Total successful tagged deliveries.
    #[must_use]
    pub const fn delivered(&self) -> usize {
        self.inline_delivered
            + self.atomic_delivered
            + self.condition_delivered
            + self.manual_delivered
    }

    /// Return true if no observers matched and no events were delivered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matched == 0 && self.events.is_empty() && self.errors.is_empty()
    }

    /// Return true when publishing produced at least one error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Borrow the first typed event, if any.
    #[must_use]
    pub fn first_event(&self) -> Option<&TaggedWakeEvent<T, LOW_BITS>> {
        self.events.first()
    }

    /// Resolve all event payloads through the owning arena.
    pub fn resolve_events<'a>(
        &self,
        arena: &'a TaggedPayloadArena<T, LOW_BITS>,
    ) -> Result<Vec<TaggedResolvedWakeEvent<'a, T, LOW_BITS>>, TaggedPayloadError> {
        self.events
            .iter()
            .map(|event| event.resolve_with_event(arena))
            .collect()
    }
}

/// Errors from tagged-payload arena and tagged-publish behavior.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaggedPayloadError {
    /// The encoded handle could not be converted back into typed validation parts.
    #[error("invalid encoded tagged payload handle")]
    InvalidEncodedHandle,
    /// The owning arena rejected the handle.
    #[error(transparent)]
    Arena(#[from] KeyedArenaError),
    /// Low-tag/key validation failed while inserting or mutating payload metadata.
    #[error(transparent)]
    Tag(#[from] TagError),
}

/// Errors reported by tagged wake publishing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaggedWakeError {
    /// Wake key reason/profile decoding failed.
    #[error("wake decode failed: {0:?}")]
    Decode(WakeDecodeError),
    /// Crossbeam is a concrete `WakeEvent<PayloadRef>` backend in this crate;
    /// tagged-payload publishing returns typed events instead.
    #[error("crossbeam tagged payload delivery requires an explicit bridge")]
    CrossbeamTaggedPayloadUnsupported,
}

impl From<WakeDecodeError> for TaggedWakeError {
    fn from(value: WakeDecodeError) -> Self {
        Self::Decode(value)
    }
}

impl ObservableWakeRuntime {
    /// Publish an envelope that carries a tagged payload reference.
    ///
    /// This intentionally mirrors the normal wake path, but returns typed
    /// [`TaggedWakeEvent`] values instead of pushing into the runtime's
    /// `PayloadRef` queues. Matching still happens before payload resolution.
    pub fn publish_tagged<T, const LOW_BITS: usize>(
        &mut self,
        envelope: ObservableEnvelope<TaggedPayloadRef<T, LOW_BITS>>,
    ) -> TaggedWakeReport<T, LOW_BITS> {
        let mut report = TaggedWakeReport::default();

        for slot in 0..self.observers.len() {
            report.scanned += 1;

            let Some(entry) = self.observers[slot].as_ref() else {
                continue;
            };

            if entry.state.0.load(Ordering::Acquire) != ObserverState::Active as u64 {
                continue;
            }

            if !entry.config.matches(envelope.key) {
                continue;
            }

            report.matched += 1;

            let reason = match envelope.key.reason_code() {
                Ok(reason) => reason,
                Err(error) => {
                    report.errors.push(TaggedWakeError::from(error));
                    continue;
                }
            };

            let profile = entry.config.profile;
            // Publish the key before the epoch bump so external Acquire readers
            // that notice the new epoch can then read the key that caused it.
            entry
                .last_key
                .0
                .store(envelope.key.raw_storage(), Ordering::Release);
            let epoch = entry.wake_epoch.0.fetch_add(1, Ordering::AcqRel) + 1;

            let event = TaggedWakeEvent {
                observer_id: entry.id(slot as u32),
                target: entry.config.target,
                key: envelope.key,
                payload: envelope.payload,
                reason,
                profile,
                wake_epoch: epoch,
            };

            match profile {
                WakeProfileCode::InlineQueue => {
                    let _ = self.bump_delivery_epoch(event.observer_id);
                    report.inline_delivered += 1;
                    report.events.push(event);
                }
                WakeProfileCode::AtomicEpoch => {
                    let _ = self.bump_delivery_epoch(event.observer_id);
                    report.atomic_delivered += 1;
                    report.events.push(event);
                }
                WakeProfileCode::ConditionCache => {
                    let _ = self.bump_delivery_epoch(event.observer_id);
                    report.condition_delivered += 1;
                    report.events.push(event);
                }
                WakeProfileCode::Manual => {
                    let _ = self.bump_delivery_epoch(event.observer_id);
                    report.manual_delivered += 1;
                    report.events.push(event);
                }
                WakeProfileCode::CrossbeamQueue => {
                    report
                        .errors
                        .push(TaggedWakeError::CrossbeamTaggedPayloadUnsupported);
                }
            }
        }

        report
    }
}
