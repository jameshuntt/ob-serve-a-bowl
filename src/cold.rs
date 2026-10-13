//! Stateful cold-observable layer.
//!
//! The base [`ObservableWakeRuntime`](crate::ObservableWakeRuntime) is the hot
//! binary wake router: it matches a [`WakeKey64`](crate::WakeKey64) and delivers
//! a wake immediately. This module adds the missing cold-observable piece:
//! materialized state plus predicates that wake only when cached state crosses a
//! condition.
//!
//! The important ordering is:
//!
//! ```text
//! WakeKey64 match → apply event to state cache → evaluate cold predicates → wake
//! ```
//!
//! Payload resolution stays outside this layer. Callers can use plain events,
//! [`PayloadRef`](crate::PayloadRef), or the optional tagged-payload feature to
//! build the state event before publishing.

use big_bad_bool::PackedWord;
use big_bad_bool_interact::FieldMatcher;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use crate::sync::atomic::{AtomicU64, Ordering};

use crate::{
    ObservableFieldMask, ObservablePacket, ObservableWakeRuntime, ObserverId, ObserverState, PayloadRef, TargetTaskId,
    WakeError, WakeEvent, WakeKey64, WakeProfileCode,
};

/// State owned by the cold-observable layer.
///
/// Implementors apply an event into a materialized state cache and return which
/// observable fields changed. Cold observers then evaluate predicates against
/// the updated state.
pub trait ObservableState {
    type Event;

    fn apply(&mut self, event: &Self::Event) -> StateTransition;
}

/// Result of applying one event into a state cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateTransition {
    pub changed: bool,
    pub field_mask: u16,
}

impl StateTransition {
    #[must_use]
    pub const fn unchanged() -> Self {
        Self {
            changed: false,
            field_mask: 0,
        }
    }

    #[must_use]
    pub fn changed<M: ObservableFieldMask>(field_mask: M) -> Self {
        let field_mask = field_mask.field_mask_bits();
        Self {
            changed: field_mask != 0,
            field_mask,
        }
    }

    #[must_use]
    pub fn touches<M: ObservableFieldMask>(self, mask: M) -> bool {
        (self.field_mask & mask.field_mask_bits()) != 0
    }
}

/// Cold observer re-arm behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColdRearmPolicy {
    /// Wake every time the key matches and the predicate is true.
    EveryMatch,
    /// Wake only on a false → true predicate transition.
    #[default]
    RisingEdge,
}

type ColdPredicate<S> = Box<dyn Fn(&S, &StateTransition) -> bool + Send + Sync + 'static>;

/// Configuration for one stateful cold observer.
pub struct ColdObserverConfig<S> {
    pub name: &'static str,
    pub profile: WakeProfileCode,
    pub interest_mask: u16,
    pub matchers: Vec<FieldMatcher>,
    pub target: Option<TargetTaskId>,
    pub active: bool,
    pub rearm: ColdRearmPolicy,
    predicate: ColdPredicate<S>,
}

impl<S> fmt::Debug for ColdObserverConfig<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColdObserverConfig")
            .field("name", &self.name)
            .field("profile", &self.profile)
            .field("interest_mask", &self.interest_mask)
            .field("matchers", &self.matchers)
            .field("target", &self.target)
            .field("active", &self.active)
            .field("rearm", &self.rearm)
            .finish_non_exhaustive()
    }
}

impl<S> ColdObserverConfig<S> {
    pub fn new(
        name: &'static str,
        profile: WakeProfileCode,
        predicate: impl Fn(&S, &StateTransition) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            profile,
            interest_mask: 0,
            matchers: Vec::new(),
            target: None,
            active: true,
            rearm: ColdRearmPolicy::RisingEdge,
            predicate: Box::new(predicate),
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
    pub fn with_rearm_policy(mut self, rearm: ColdRearmPolicy) -> Self {
        self.rearm = rearm;
        self
    }

    #[must_use]
    pub fn wake_every_match(self) -> Self {
        self.with_rearm_policy(ColdRearmPolicy::EveryMatch)
    }

    #[must_use]
    pub fn wake_on_rising_edge(self) -> Self {
        self.with_rearm_policy(ColdRearmPolicy::RisingEdge)
    }

    #[must_use]
    pub fn inactive(mut self) -> Self {
        self.active = false;
        self
    }

    #[must_use]
    pub fn key_matches(&self, key: WakeKey64) -> bool {
        if !self.active {
            return false;
        }

        if self.interest_mask != 0 && !key.has_any_field(self.interest_mask) {
            return false;
        }

        self.matchers.iter().copied().all(|matcher| matcher.matches(key))
    }

    #[must_use]
    pub fn predicate_matches(&self, state: &S, transition: &StateTransition) -> bool {
        (self.predicate)(state, transition)
    }
}

#[repr(align(64))]
struct ColdAtomicU64(AtomicU64);

impl ColdAtomicU64 {
    #[cfg(not(loom))]
    const fn new(value: u64) -> Self {
        Self(AtomicU64::new(value))
    }
    #[cfg(loom)]
    fn new(value: u64) -> Self {
        Self(AtomicU64::new(value))
    }
}

struct ColdObserverEntry<S> {
    generation: u32,
    config: ColdObserverConfig<S>,
    state: ColdAtomicU64,
    wake_epoch: ColdAtomicU64,
    delivery_epoch: ColdAtomicU64,
    last_key: ColdAtomicU64,
    last_predicate_true: bool,
}

impl<S> ColdObserverEntry<S> {
    fn new(generation: u32, config: ColdObserverConfig<S>) -> Self {
        Self {
            generation,
            config,
            state: ColdAtomicU64::new(ObserverState::Active as u64),
            wake_epoch: ColdAtomicU64::new(0),
            delivery_epoch: ColdAtomicU64::new(0),
            last_key: ColdAtomicU64::new(0),
            last_predicate_true: false,
        }
    }

    fn id(&self, slot: u32) -> ObserverId {
        ObserverId::new(slot, self.generation)
    }
}

#[derive(Default)]
struct ColdObserverIndex {
    wildcard: Vec<ObserverId>,
    by_interest_mask: BTreeMap<u16, Vec<ObserverId>>,
    has_nonzero_interest: bool,
    common_interest_bits: u16,
}

impl ColdObserverIndex {
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

/// Runtime that combines hot binary routing with stateful cold predicates.
pub struct ColdObservableRuntime<S: ObservableState> {
    hot: ObservableWakeRuntime,
    state: S,
    cold_observers: Vec<Option<ColdObserverEntry<S>>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    index: ColdObserverIndex,
    index_scratch: Vec<ObserverId>,
    live_cold_observers: usize,
    inline_ready: VecDeque<WakeEvent>,
    condition_ready: VecDeque<WakeEvent>,
}

impl<S: ObservableState> ColdObservableRuntime<S> {
    #[must_use]
    pub fn new(state: S) -> Self {
        Self::with_hot_runtime(state, ObservableWakeRuntime::new())
    }

    #[must_use]
    pub fn with_hot_runtime(state: S, hot: ObservableWakeRuntime) -> Self {
        Self {
            hot,
            state,
            cold_observers: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            index: ColdObserverIndex::default(),
            index_scratch: Vec::new(),
            live_cold_observers: 0,
            inline_ready: VecDeque::new(),
            condition_ready: VecDeque::new(),
        }
    }

    #[must_use]
    pub fn state(&self) -> &S {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut S {
        &mut self.state
    }

    #[must_use]
    pub fn hot_runtime(&self) -> &ObservableWakeRuntime {
        &self.hot
    }

    pub fn hot_runtime_mut(&mut self) -> &mut ObservableWakeRuntime {
        &mut self.hot
    }

    pub fn register_cold(&mut self, config: ColdObserverConfig<S>) -> ObserverId {
        let interest_mask = config.interest_mask;
        let id = if let Some(slot) = self.free.pop() {
            let generation = self.generations[slot as usize];
            self.cold_observers[slot as usize] = Some(ColdObserverEntry::new(generation, config));
            ObserverId::new(slot, generation)
        } else {
            let slot = self.cold_observers.len() as u32;
            self.cold_observers.push(Some(ColdObserverEntry::new(0, config)));
            self.generations.push(0);
            ObserverId::new(slot, 0)
        };

        self.index.insert(id, interest_mask);
        self.live_cold_observers += 1;
        id
    }

    pub fn remove_cold(&mut self, id: ObserverId) -> Result<(), WakeError> {
        let slot = self
            .cold_observers
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
        self.live_cold_observers = self.live_cold_observers.saturating_sub(1);
        Ok(())
    }

    pub fn cold_observer_epoch(&self, id: ObserverId) -> Result<u64, WakeError> {
        let entry = self.get_cold_entry(id)?;
        Ok(entry.wake_epoch.0.load(Ordering::Acquire))
    }

    pub fn cold_observer_delivery_epoch(&self, id: ObserverId) -> Result<u64, WakeError> {
        let entry = self.get_cold_entry(id)?;
        Ok(entry.delivery_epoch.0.load(Ordering::Acquire))
    }

    pub fn cold_observer_last_key(&self, id: ObserverId) -> Result<Option<WakeKey64>, WakeError> {
        let entry = self.get_cold_entry(id)?;
        let raw = entry.last_key.0.load(Ordering::Acquire);
        if raw == 0 {
            return Ok(None);
        }
        Ok(WakeKey64::try_from_raw_u128(u128::from(raw)))
    }

    #[must_use]
    pub fn live_cold_observer_count(&self) -> usize {
        self.live_cold_observers
    }

    #[must_use]
    pub fn indexed_cold_candidate_count(&self, key: WakeKey64) -> usize {
        self.index.candidate_count(key)
    }

    #[must_use]
    pub fn cold_inline_len(&self) -> usize {
        self.inline_ready.len()
    }

    #[must_use]
    pub fn cold_condition_len(&self) -> usize {
        self.condition_ready.len()
    }

    pub fn drain_cold_inline(&mut self) -> Vec<WakeEvent> {
        self.inline_ready.drain(..).collect()
    }

    pub fn drain_cold_condition(&mut self) -> Vec<WakeEvent> {
        self.condition_ready.drain(..).collect()
    }

    /// Publish an event through the hot router, apply it to state, then evaluate
    /// cold observers against the updated state.
    pub fn publish_stateful(
        &mut self,
        key: WakeKey64,
        payload: PayloadRef,
        event: S::Event,
    ) -> ColdPublishReport {
        let packet = ObservablePacket::new(key, payload);
        let hot = self.hot.publish(packet);
        let transition = self.state.apply(&event);
        let mut report = ColdPublishReport {
            hot,
            transition,
            ..ColdPublishReport::default()
        };

        self.evaluate_cold(packet, &transition, &mut report);
        report
    }

    /// Publish using the hot runtime's indexed path and evaluate cold observers
    /// through the cold field-mask index.
    ///
    /// The state transition's dirty field mask is used as an extra predicate
    /// gate: if a cold observer declared an interest mask and the state change
    /// did not touch any of those fields, its predicate is not evaluated.
    pub fn publish_stateful_indexed(
        &mut self,
        key: WakeKey64,
        payload: PayloadRef,
        event: S::Event,
    ) -> ColdPublishReport {
        let packet = ObservablePacket::new(key, payload);
        let hot = self.hot.publish_indexed(packet);
        let transition = self.state.apply(&event);
        let mut report = ColdPublishReport {
            hot,
            transition,
            ..ColdPublishReport::default()
        };

        self.evaluate_cold_indexed(packet, &transition, &mut report);
        report
    }

    /// Publish using adaptive hot/cold dispatch policy.
    ///
    /// The hot half delegates to [`ObservableWakeRuntime::publish_auto`]. The
    /// cold half chooses scan versus indexed predicate evaluation using the hot
    /// runtime's [`DispatchPolicy`], so callers can tune one policy for the whole
    /// observable runtime.
    pub fn publish_stateful_auto(
        &mut self,
        key: WakeKey64,
        payload: PayloadRef,
        event: S::Event,
    ) -> ColdPublishReport {
        let packet = ObservablePacket::new(key, payload);
        let hot = self.hot.publish_auto(packet);
        let transition = self.state.apply(&event);
        let mut report = ColdPublishReport {
            hot,
            transition,
            ..ColdPublishReport::default()
        };

        if self.should_publish_cold_indexed(key) {
            self.evaluate_cold_indexed(packet, &transition, &mut report);
        } else {
            self.evaluate_cold(packet, &transition, &mut report);
        }

        report
    }

    #[must_use]
    pub fn should_publish_cold_indexed(&self, key: WakeKey64) -> bool {
        let total = self.live_cold_observers;
        let policy = self.hot.config().dispatch_policy;

        // Cold predicate evaluation has its own threshold because indexed cold
        // dispatch only wins when dirty-field gating removes enough predicates.
        // Avoid the index/candidate-count overhead for tiny cold sets.
        if !policy.can_consider_cold_indexed(total) {
            return false;
        }

        // Dense cold layouts should stay on the scan path. If every indexed
        // interest-mask bucket is definitely touched by this key, indexed cold
        // dispatch cannot skip predicate evaluation, so avoid the candidate-count
        // pass and scan directly.
        if self.index.definitely_dense_for_key(key) {
            return false;
        }

        let candidates = self.indexed_cold_candidate_count(key);
        policy.should_use_cold_indexed(total, candidates)
    }

    fn evaluate_cold(
        &mut self,
        packet: ObservablePacket,
        transition: &StateTransition,
        report: &mut ColdPublishReport,
    ) {
        for (slot, maybe_entry) in self.cold_observers.iter_mut().enumerate() {
            report.cold_scanned += 1;

            let delivery = {
                let Some(entry) = maybe_entry.as_mut() else {
                    continue;
                };

                // `publish_stateful` has `&mut self`, so this load is only a cheap
                // state gate for slots that may have been removed/reused. Acquire is
                // unnecessary in this single-runtime mutation path; release/acquire
                // accessors remain available for external epoch readers.
                if entry.state.0.load(Ordering::Relaxed) != ObserverState::Active as u64 {
                    continue;
                }

                if !entry.config.key_matches(packet.key) {
                    continue;
                }

                report.cold_key_matched += 1;
                let predicate_true = entry.config.predicate_matches(&self.state, transition);

                if !predicate_true {
                    entry.last_predicate_true = false;
                    continue;
                }

                report.cold_predicate_true += 1;
                let should_wake = match entry.config.rearm {
                    ColdRearmPolicy::EveryMatch => true,
                    ColdRearmPolicy::RisingEdge => !entry.last_predicate_true,
                };
                entry.last_predicate_true = true;

                if !should_wake {
                    continue;
                }

                match make_cold_delivery(slot as u32, entry, packet) {
                    Ok(delivery) => delivery,
                    Err(error) => {
                        report.errors.push(error);
                        continue;
                    }
                }
            };

            match delivery {
                ColdDelivery::Inline(event) => {
                    report.cold_delivered += 1;
                    report.cold_inline_delivered += 1;
                    self.inline_ready.push_back(event);
                }
                ColdDelivery::Atomic => {
                    report.cold_delivered += 1;
                    report.cold_atomic_delivered += 1;
                }
                ColdDelivery::Condition(event) => {
                    report.cold_delivered += 1;
                    report.cold_condition_delivered += 1;
                    self.condition_ready.push_back(event);
                }
                ColdDelivery::Manual => {
                    report.cold_delivered += 1;
                    report.cold_manual_delivered += 1;
                }
                ColdDelivery::Error(error) => report.errors.push(error),
            }
        }
    }

    fn evaluate_cold_indexed(
        &mut self,
        packet: ObservablePacket,
        transition: &StateTransition,
        report: &mut ColdPublishReport,
    ) {
        self.index.collect_candidates(packet.key, &mut self.index_scratch);
        let candidates = std::mem::take(&mut self.index_scratch);

        for index in 0..candidates.len() {
            let id = candidates[index];
            report.cold_scanned += 1;

            let delivery = {
                let Some(Some(entry)) = self.cold_observers.get_mut(id.slot as usize) else {
                    continue;
                };

                if entry.generation != id.generation {
                    continue;
                }

                if entry.state.0.load(Ordering::Relaxed) != ObserverState::Active as u64 {
                    continue;
                }

                if !entry.config.key_matches(packet.key) {
                    continue;
                }

                if transition.changed
                    && entry.config.interest_mask != 0
                    && !transition.touches(entry.config.interest_mask)
                {
                    continue;
                }

                report.cold_key_matched += 1;
                let predicate_true = entry.config.predicate_matches(&self.state, transition);

                if !predicate_true {
                    entry.last_predicate_true = false;
                    continue;
                }

                report.cold_predicate_true += 1;
                let should_wake = match entry.config.rearm {
                    ColdRearmPolicy::EveryMatch => true,
                    ColdRearmPolicy::RisingEdge => !entry.last_predicate_true,
                };
                entry.last_predicate_true = true;

                if !should_wake {
                    continue;
                }

                match make_cold_delivery(id.slot, entry, packet) {
                    Ok(delivery) => delivery,
                    Err(error) => {
                        report.errors.push(error);
                        continue;
                    }
                }
            };

            match delivery {
                ColdDelivery::Inline(event) => {
                    report.cold_delivered += 1;
                    report.cold_inline_delivered += 1;
                    self.inline_ready.push_back(event);
                }
                ColdDelivery::Atomic => {
                    report.cold_delivered += 1;
                    report.cold_atomic_delivered += 1;
                }
                ColdDelivery::Condition(event) => {
                    report.cold_delivered += 1;
                    report.cold_condition_delivered += 1;
                    self.condition_ready.push_back(event);
                }
                ColdDelivery::Manual => {
                    report.cold_delivered += 1;
                    report.cold_manual_delivered += 1;
                }
                ColdDelivery::Error(error) => report.errors.push(error),
            }
        }

        self.index_scratch = candidates;
        self.index_scratch.clear();
    }

    fn get_cold_entry(&self, id: ObserverId) -> Result<&ColdObserverEntry<S>, WakeError> {
        let Some(Some(entry)) = self.cold_observers.get(id.slot as usize) else {
            return Err(WakeError::MissingObserver(id));
        };

        if entry.generation != id.generation {
            return Err(WakeError::StaleObserver(id));
        }

        Ok(entry)
    }
}

fn make_cold_delivery<S>(
    slot: u32,
    entry: &ColdObserverEntry<S>,
    packet: ObservablePacket,
) -> Result<ColdDelivery, WakeError> {
    let reason = packet.key.reason_code()?;
    let profile = entry.config.profile;

    entry.last_key.0.store(packet.key.raw_storage(), Ordering::Release);
    let epoch = entry.wake_epoch.0.fetch_add(1, Ordering::AcqRel) + 1;

    match profile {
        WakeProfileCode::InlineQueue => {
            entry.delivery_epoch.0.fetch_add(1, Ordering::AcqRel);
            Ok(ColdDelivery::Inline(WakeEvent {
                observer_id: entry.id(slot),
                target: entry.config.target,
                key: packet.key,
                payload: packet.payload,
                reason,
                profile,
                wake_epoch: epoch,
            }))
        }
        WakeProfileCode::AtomicEpoch => {
            entry.delivery_epoch.0.fetch_add(1, Ordering::AcqRel);
            Ok(ColdDelivery::Atomic)
        }
        WakeProfileCode::ConditionCache => {
            entry.delivery_epoch.0.fetch_add(1, Ordering::AcqRel);
            Ok(ColdDelivery::Condition(WakeEvent {
                observer_id: entry.id(slot),
                target: entry.config.target,
                key: packet.key,
                payload: packet.payload,
                reason,
                profile,
                wake_epoch: epoch,
            }))
        }
        WakeProfileCode::Manual => {
            entry.delivery_epoch.0.fetch_add(1, Ordering::AcqRel);
            Ok(ColdDelivery::Manual)
        }
        WakeProfileCode::CrossbeamQueue => Ok(ColdDelivery::Error(
            WakeError::CrossbeamSenderMissing,
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ColdDelivery {
    Inline(WakeEvent),
    Atomic,
    Condition(WakeEvent),
    Manual,
    Error(WakeError),
}

/// Per-publish report for stateful hot+cold publishing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColdPublishReport {
    pub hot: crate::WakeReport,
    pub transition: StateTransition,
    pub cold_scanned: usize,
    pub cold_key_matched: usize,
    pub cold_predicate_true: usize,
    pub cold_delivered: usize,
    pub cold_inline_delivered: usize,
    pub cold_atomic_delivered: usize,
    pub cold_condition_delivered: usize,
    pub cold_manual_delivered: usize,
    pub errors: Vec<WakeError>,
}

impl ColdPublishReport {
    #[must_use]
    pub fn total_delivered(&self) -> usize {
        self.hot.delivered() + self.cold_delivered
    }

    #[must_use]
    pub fn cold_delivered(&self) -> usize {
        self.cold_delivered
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.hot.errors.is_empty() || !self.errors.is_empty()
    }
}
