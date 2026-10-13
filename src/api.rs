//! Recommended public API façade.
//!
//! The crate intentionally exposes lower-level runtime pieces for systems code,
//! benchmarking, and experimentation. This module provides the smaller happy
//! path that most integrations should start with:
//!
//! - build a [`WakeKey64`] with [`WakeKeyBuilder`],
//! - register hot observers with [`HotObserverBuilder`],
//! - register cold observers with [`cold_observer`],
//! - publish through [`LoggedColdRuntime`] when state transitions and replay logs
//!   are desired.

use big_bad_bool::BitpackError;
use big_bad_bool_interact::FieldMatcher;

use crate::{
    ColdObservableRuntime, ColdObserverConfig, ColdPublishReport, ObservableEventLog,
    ObservableFieldMask, ObservablePacket, ObservableState, ObservableWakeRuntime, ObserverConfig, ObserverId,
    PayloadRef, StateTransition, TargetTaskId, WakeEvent, WakeKey64, WakeProfileCode,
    WakeReasonCode,
};

/// Build a typed [`WakeKey64`] without calling the generated bitpack constructor
/// directly.
///
/// This is the recommended entry point for application/runtime code. It keeps
/// the bit-level layout in one place while still producing the same compact
/// binary key used by the hot path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeKeyBuilder {
    reason: WakeReasonCode,
    profile: WakeProfileCode,
    qos: u8,
    field_mask: u16,
    phase: u8,
    flags: u16,
}

impl Default for WakeKeyBuilder {
    fn default() -> Self {
        Self {
            reason: WakeReasonCode::Condition,
            profile: WakeProfileCode::AtomicEpoch,
            qos: 3,
            field_mask: 0,
            phase: 0,
            flags: WakeKey64::FLAG_HOT as u16,
        }
    }
}

impl WakeKeyBuilder {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            reason: WakeReasonCode::Condition,
            profile: WakeProfileCode::AtomicEpoch,
            qos: 3,
            field_mask: 0,
            phase: 0,
            flags: WakeKey64::FLAG_HOT as u16,
        }
    }

    #[must_use]
    pub const fn reason(mut self, reason: WakeReasonCode) -> Self {
        self.reason = reason;
        self
    }

    #[must_use]
    pub const fn profile(mut self, profile: WakeProfileCode) -> Self {
        self.profile = profile;
        self
    }

    #[must_use]
    pub const fn qos(mut self, qos: u8) -> Self {
        self.qos = qos;
        self
    }

    #[must_use]
    pub fn field_mask<M: ObservableFieldMask>(mut self, field_mask: M) -> Self {
        self.field_mask = field_mask.field_mask_bits();
        self
    }

    #[must_use]
    pub const fn phase(mut self, phase: u8) -> Self {
        self.phase = phase;
        self
    }

    #[must_use]
    pub const fn flags(mut self, flags: u16) -> Self {
        self.flags = flags;
        self
    }

    #[must_use]
    pub const fn hot(mut self) -> Self {
        self.flags |= WakeKey64::FLAG_HOT as u16;
        self
    }

    #[must_use]
    pub const fn cold(mut self) -> Self {
        self.flags |= WakeKey64::FLAG_COLD as u16;
        self
    }

    #[must_use]
    pub const fn external(mut self) -> Self {
        self.flags |= WakeKey64::FLAG_EXTERNAL as u16;
        self
    }

    pub fn build(self) -> Result<WakeKey64, BitpackError> {
        WakeKey64::typed(
            self.reason,
            self.profile,
            self.qos,
            self.field_mask,
            self.phase,
            self.flags,
        )
    }
}

/// Convenience constructor for [`WakeKeyBuilder`].
#[must_use]
pub const fn wake_key() -> WakeKeyBuilder {
    WakeKeyBuilder::new()
}

/// Build hot observer configurations without exposing every internal field.
#[derive(Debug, Clone)]
pub struct HotObserverBuilder {
    name: &'static str,
    profile: WakeProfileCode,
    interest_mask: u16,
    matchers: Vec<FieldMatcher>,
    target: Option<TargetTaskId>,
    active: bool,
}

impl HotObserverBuilder {
    #[must_use]
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            profile: WakeProfileCode::AtomicEpoch,
            interest_mask: 0,
            matchers: Vec::new(),
            target: None,
            active: true,
        }
    }

    #[must_use]
    pub fn profile(mut self, profile: WakeProfileCode) -> Self {
        self.profile = profile;
        self
    }

    #[must_use]
    pub fn inline_queue(self) -> Self {
        self.profile(WakeProfileCode::InlineQueue)
    }

    #[must_use]
    pub fn atomic_epoch(self) -> Self {
        self.profile(WakeProfileCode::AtomicEpoch)
    }

    #[must_use]
    pub fn condition_cache(self) -> Self {
        self.profile(WakeProfileCode::ConditionCache)
    }

    #[must_use]
    pub fn manual(self) -> Self {
        self.profile(WakeProfileCode::Manual)
    }

    #[must_use]
    pub fn crossbeam_queue(self) -> Self {
        self.profile(WakeProfileCode::CrossbeamQueue)
    }

    #[must_use]
    pub fn interested_in<M: ObservableFieldMask>(mut self, interest_mask: M) -> Self {
        self.interest_mask = interest_mask.field_mask_bits();
        self
    }

    #[must_use]
    pub fn matching(mut self, matcher: FieldMatcher) -> Self {
        self.matchers.push(matcher);
        self
    }

    #[must_use]
    pub fn with_matchers(mut self, matchers: impl Into<Vec<FieldMatcher>>) -> Self {
        self.matchers = matchers.into();
        self
    }

    #[must_use]
    pub fn target(mut self, target: TargetTaskId) -> Self {
        self.target = Some(target);
        self
    }

    #[must_use]
    pub fn inactive(mut self) -> Self {
        self.active = false;
        self
    }

    #[must_use]
    pub fn build(self) -> ObserverConfig {
        let mut config = ObserverConfig::new(self.name, self.profile)
            .with_interest_mask(self.interest_mask)
            .with_matchers(self.matchers);

        if let Some(target) = self.target {
            config = config.with_target(target);
        }

        if !self.active {
            config = config.inactive();
        }

        config
    }
}

/// Convenience constructor for [`HotObserverBuilder`].
#[must_use]
pub fn hot_observer(name: &'static str) -> HotObserverBuilder {
    HotObserverBuilder::new(name)
}

/// Convenience constructor for stateful cold observers.
pub fn cold_observer<S>(
    name: &'static str,
    profile: WakeProfileCode,
    predicate: impl Fn(&S, &StateTransition) -> bool + Send + Sync + 'static,
) -> ColdObserverConfig<S> {
    ColdObserverConfig::new(name, profile, predicate)
}

/// Convenience constructor for [`PayloadRef`].
#[must_use]
pub const fn payload_ref(slot: u32, generation: u32, offset: u32, len: u32) -> PayloadRef {
    PayloadRef::new(slot, generation, offset, len)
}

/// Convenience constructor for [`TargetTaskId`].
#[must_use]
pub const fn target_task(slot: u32, generation: u32) -> TargetTaskId {
    TargetTaskId::new(slot, generation)
}

/// Convenience constructor for [`ObservablePacket`].
#[must_use]
pub const fn packet(key: WakeKey64, payload: PayloadRef) -> ObservablePacket {
    ObservablePacket::new(key, payload)
}

/// A small release-oriented façade for the common server/runtime use case.
///
/// It owns a stateful cold runtime and an event log, and routes publishes through
/// the adaptive logged path. Advanced callers can still access the inner runtime
/// and log when they need lower-level control.
pub struct LoggedColdRuntime<S: ObservableState> {
    runtime: ColdObservableRuntime<S>,
    log: ObservableEventLog,
}

impl<S: ObservableState> LoggedColdRuntime<S> {
    #[must_use]
    pub fn new(state: S) -> Self {
        Self {
            runtime: ColdObservableRuntime::new(state),
            log: ObservableEventLog::default(),
        }
    }

    #[must_use]
    pub fn with_hot_runtime(state: S, hot: ObservableWakeRuntime) -> Self {
        Self {
            runtime: ColdObservableRuntime::with_hot_runtime(state, hot),
            log: ObservableEventLog::default(),
        }
    }

    #[must_use]
    pub fn from_parts(runtime: ColdObservableRuntime<S>, log: ObservableEventLog) -> Self {
        Self { runtime, log }
    }

    pub fn register_hot(&mut self, config: ObserverConfig) -> ObserverId {
        self.runtime.hot_runtime_mut().register(config)
    }

    pub fn register_cold(&mut self, config: ColdObserverConfig<S>) -> ObserverId {
        self.runtime.register_cold(config)
    }

    pub fn publish(
        &mut self,
        key: WakeKey64,
        payload: PayloadRef,
        event: S::Event,
    ) -> ColdPublishReport {
        self.runtime
            .publish_stateful_auto_logged(&mut self.log, key, payload, event)
    }

    pub fn drain_hot_inline_logged(&mut self) -> Vec<WakeEvent> {
        self.runtime
            .hot_runtime_mut()
            .drain_inline_logged(&mut self.log)
    }

    pub fn drain_hot_condition_logged(&mut self) -> Vec<WakeEvent> {
        self.runtime
            .hot_runtime_mut()
            .drain_condition_logged(&mut self.log)
    }

    pub fn drain_cold_inline_logged(&mut self) -> Vec<WakeEvent> {
        self.runtime.drain_cold_inline_logged(&mut self.log)
    }

    pub fn drain_cold_condition_logged(&mut self) -> Vec<WakeEvent> {
        self.runtime.drain_cold_condition_logged(&mut self.log)
    }

    #[must_use]
    pub fn runtime(&self) -> &ColdObservableRuntime<S> {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut ColdObservableRuntime<S> {
        &mut self.runtime
    }

    #[must_use]
    pub fn log(&self) -> &ObservableEventLog {
        &self.log
    }

    pub fn log_mut(&mut self) -> &mut ObservableEventLog {
        &mut self.log
    }

    #[must_use]
    pub fn into_parts(self) -> (ColdObservableRuntime<S>, ObservableEventLog) {
        (self.runtime, self.log)
    }
}
