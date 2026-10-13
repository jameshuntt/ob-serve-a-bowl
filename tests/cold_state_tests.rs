use ob_serve_a_bowl::*;
use big_bad_bool_interact::FieldMatcher;

const FIELD_HEALTH: u16 = 1 << 0;
const FIELD_QUEUE: u16 = 1 << 1;
const FIELD_LAG: u16 = 1 << 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ServiceState {
    queue_depth: u32,
    healthy: bool,
    lagging: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceEvent {
    SetQueue(u32),
    SetHealth(bool),
    SetLagging(bool),
}

impl ObservableState for ServiceState {
    type Event = ServiceEvent;

    fn apply(&mut self, event: &Self::Event) -> StateTransition {
        match *event {
            ServiceEvent::SetQueue(depth) => {
                if self.queue_depth == depth {
                    StateTransition::unchanged()
                } else {
                    self.queue_depth = depth;
                    StateTransition::changed(FIELD_QUEUE)
                }
            }
            ServiceEvent::SetHealth(healthy) => {
                if self.healthy == healthy {
                    StateTransition::unchanged()
                } else {
                    self.healthy = healthy;
                    StateTransition::changed(FIELD_HEALTH)
                }
            }
            ServiceEvent::SetLagging(lagging) => {
                if self.lagging == lagging {
                    StateTransition::unchanged()
                } else {
                    self.lagging = lagging;
                    StateTransition::changed(FIELD_LAG)
                }
            }
        }
    }
}

fn condition_key(mask: u16, profile: WakeProfileCode) -> WakeKey64 {
    WakeKey64::typed(
        WakeReasonCode::Condition,
        profile,
        4,
        mask,
        9,
        WakeKey64::FLAG_COLD as u16,
    )
    .unwrap()
}

fn queue_threshold_observer(profile: WakeProfileCode, threshold: u32) -> ColdObserverConfig<ServiceState> {
    ColdObserverConfig::new("queue-threshold", profile, move |state: &ServiceState, transition| {
        transition.touches(FIELD_QUEUE) && state.queue_depth >= threshold
    })
    .with_interest_mask(FIELD_QUEUE)
    .with_matchers(cold_condition_matchers(1, u128::from(FIELD_QUEUE)))
}

#[test]
fn hot_observer_wakes_without_state_predicate() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    runtime.hot_runtime_mut().register(
        ObserverConfig::new("hot-queue", WakeProfileCode::InlineQueue).with_interest_mask(FIELD_QUEUE),
    );

    let report = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::new(1, 0, 0, 8),
        ServiceEvent::SetQueue(1),
    );

    assert_eq!(report.hot.inline_delivered, 1);
    assert_eq!(runtime.hot_runtime_mut().drain_inline().len(), 1);
    assert_eq!(report.cold_delivered(), 0);
}

#[test]
fn cold_observer_does_not_wake_before_threshold() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    let id = runtime.register_cold(queue_threshold_observer(WakeProfileCode::InlineQueue, 10_000));

    let report = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        ServiceEvent::SetQueue(9_999),
    );

    assert_eq!(report.cold_key_matched, 1);
    assert_eq!(report.cold_predicate_true, 0);
    assert_eq!(report.cold_delivered(), 0);
    assert_eq!(runtime.cold_observer_epoch(id).unwrap(), 0);
}

#[test]
fn cold_observer_wakes_after_cache_crosses_threshold() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    let id = runtime.register_cold(queue_threshold_observer(WakeProfileCode::InlineQueue, 10_000));

    let report = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::new(2, 0, 4, 12),
        ServiceEvent::SetQueue(10_000),
    );

    assert_eq!(runtime.state().queue_depth, 10_000);
    assert_eq!(report.cold_predicate_true, 1);
    assert_eq!(report.cold_inline_delivered, 1);
    assert_eq!(runtime.cold_observer_epoch(id).unwrap(), 1);
    let events = runtime.drain_cold_inline();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].observer_id, id);
    assert_eq!(events[0].payload, PayloadRef::new(2, 0, 4, 12));
}

#[test]
fn cold_observer_does_not_double_wake_until_condition_resets() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    runtime.register_cold(queue_threshold_observer(WakeProfileCode::InlineQueue, 10_000));

    let first = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_000),
    );
    let second = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_001),
    );

    assert_eq!(first.cold_inline_delivered, 1);
    assert_eq!(second.cold_predicate_true, 1);
    assert_eq!(second.cold_inline_delivered, 0);
    assert_eq!(runtime.drain_cold_inline().len(), 1);
}

#[test]
fn cold_observer_rearms_after_predicate_becomes_false() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    runtime.register_cold(queue_threshold_observer(WakeProfileCode::InlineQueue, 10_000));

    runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_000),
    );
    runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        ServiceEvent::SetQueue(1),
    );
    let third = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_000),
    );

    assert_eq!(third.cold_inline_delivered, 1);
    assert_eq!(runtime.drain_cold_inline().len(), 2);
}

#[test]
fn cold_observer_every_match_policy_can_duplicate_wakes() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    runtime.register_cold(queue_threshold_observer(WakeProfileCode::InlineQueue, 10_000).wake_every_match());

    let first = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_000),
    );
    let second = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_001),
    );

    assert_eq!(first.cold_inline_delivered, 1);
    assert_eq!(second.cold_inline_delivered, 1);
    assert_eq!(runtime.drain_cold_inline().len(), 2);
}

#[test]
fn multiple_cold_observers_share_same_state_cache() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    runtime.register_cold(queue_threshold_observer(WakeProfileCode::InlineQueue, 10_000));
    runtime.register_cold(
        ColdObserverConfig::new("lagging", WakeProfileCode::Manual, |state: &ServiceState, transition| {
            transition.touches(FIELD_LAG) && state.lagging
        })
        .with_interest_mask(FIELD_LAG)
        .with_matchers(cold_condition_matchers(1, u128::from(FIELD_LAG))),
    );

    let queue = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_000),
    );
    let lag = runtime.publish_stateful(
        condition_key(FIELD_LAG, WakeProfileCode::Manual),
        PayloadRef::default(),
        ServiceEvent::SetLagging(true),
    );

    assert_eq!(queue.cold_inline_delivered, 1);
    assert_eq!(lag.cold_manual_delivered, 1);
    assert!(runtime.state().queue_depth >= 10_000);
    assert!(runtime.state().lagging);
}

#[test]
fn condition_cache_profile_receives_only_true_conditions() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    runtime.register_cold(queue_threshold_observer(WakeProfileCode::ConditionCache, 10_000));

    runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::ConditionCache),
        PayloadRef::default(),
        ServiceEvent::SetQueue(9_000),
    );
    let hit = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::ConditionCache),
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_000),
    );

    assert_eq!(hit.cold_condition_delivered, 1);
    assert_eq!(runtime.cold_condition_len(), 1);
    assert_eq!(runtime.drain_cold_condition().len(), 1);
}

#[test]
fn cold_key_match_prevents_predicate_evaluation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_predicate = Arc::clone(&calls);
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    runtime.register_cold(
        ColdObserverConfig::new("queue", WakeProfileCode::Manual, move |_state: &ServiceState, _transition| {
            calls_for_predicate.fetch_add(1, Ordering::SeqCst);
            true
        })
        .with_interest_mask(FIELD_QUEUE)
        .with_matchers(vec![FieldMatcher::mask_any("field_mask", u128::from(FIELD_QUEUE))]),
    );

    let report = runtime.publish_stateful(
        condition_key(FIELD_HEALTH, WakeProfileCode::Manual),
        PayloadRef::default(),
        ServiceEvent::SetHealth(true),
    );

    assert_eq!(report.cold_key_matched, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cold_observer_slot_reuse_increments_generation() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    let first = runtime.register_cold(queue_threshold_observer(WakeProfileCode::Manual, 1));
    runtime.remove_cold(first).unwrap();
    let second = runtime.register_cold(queue_threshold_observer(WakeProfileCode::Manual, 1));

    assert_eq!(first.slot, second.slot);
    assert_ne!(first.generation, second.generation);
    assert!(matches!(runtime.cold_observer_epoch(first), Err(WakeError::StaleObserver(_)) | Err(WakeError::MissingObserver(_))));
}

#[cfg(feature = "tagged-payloads")]
#[test]
fn tagged_payload_can_feed_stateful_cold_observer_without_eager_resolution() {
    use petr_core::{Key, PortHint, ProphecyBits, SelectorWord};

    #[repr(align(8))]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct PayloadDepth(u32);

    let mut arena = TaggedPayloadArena::<PayloadDepth, 3>::with_key(Key::from_seed(0x4440));
    let handle = arena
        .insert(
            PayloadDepth(10_000),
            2,
            SelectorWord::new(1, 1, 7),
            ProphecyBits { maybe_zero: false, maybe_empty: false, hot: true, skip_hint: false, port_hint: PortHint::Dispatch },
        )
        .unwrap();

    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    runtime.register_cold(queue_threshold_observer(WakeProfileCode::InlineQueue, 10_000));

    // The caller resolves only after deciding this key/payload should feed the
    // state cache. The cold layer still receives a normal state event.
    let depth = arena.get(handle).unwrap().0;
    let report = runtime.publish_stateful(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::new(handle.index(), handle.generation(), 0, 4),
        ServiceEvent::SetQueue(depth),
    );

    assert_eq!(report.cold_inline_delivered, 1);
    assert_eq!(runtime.state().queue_depth, 10_000);
}

#[test]
fn indexed_cold_publish_matches_scan_and_skips_non_candidate_predicates() {
    let mut runtime = ColdObservableRuntime::new(ServiceState::default());
    for _ in 0..10 {
        runtime.register_cold(
            ColdObserverConfig::new("health", WakeProfileCode::Manual, |_state: &ServiceState, transition| {
                transition.touches(FIELD_HEALTH)
            })
            .with_interest_mask(FIELD_HEALTH)
            .with_matchers(cold_condition_matchers(1, u128::from(FIELD_HEALTH))),
        );
    }
    for _ in 0..2 {
        runtime.register_cold(
            queue_threshold_observer(WakeProfileCode::Manual, 10_000)
                .wake_every_match(),
        );
    }

    let key = condition_key(FIELD_QUEUE, WakeProfileCode::Manual);
    assert_eq!(runtime.indexed_cold_candidate_count(key), 2);

    let report = runtime.publish_stateful_indexed(
        key,
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_000),
    );

    assert_eq!(report.cold_scanned, 2);
    assert_eq!(report.cold_key_matched, 2);
    assert_eq!(report.cold_predicate_true, 2);
    assert_eq!(report.cold_manual_delivered, 2);
}

#[test]
fn cold_auto_uses_scan_for_dense_observers() {
    let hot = ObservableWakeRuntime::with_config(ObservableRuntimeConfig {
        dispatch_policy: DispatchPolicy::new(16, 25),
        ..ObservableRuntimeConfig::default()
    });
    let mut runtime = ColdObservableRuntime::with_hot_runtime(ServiceState::default(), hot);

    for _ in 0..64 {
        runtime.register_cold(queue_threshold_observer(WakeProfileCode::Manual, 10_000).wake_every_match());
    }

    let key = condition_key(FIELD_QUEUE, WakeProfileCode::Manual);
    assert!(!runtime.should_publish_cold_indexed(key));

    let report = runtime.publish_stateful_auto(
        key,
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_000),
    );

    assert_eq!(report.cold_scanned, 64);
    assert_eq!(report.cold_manual_delivered, 64);
}

#[test]
fn cold_auto_uses_index_for_sparse_observers() {
    let hot = ObservableWakeRuntime::with_config(ObservableRuntimeConfig {
        dispatch_policy: DispatchPolicy::new(16, 25),
        ..ObservableRuntimeConfig::default()
    });
    let mut runtime = ColdObservableRuntime::with_hot_runtime(ServiceState::default(), hot);

    for _ in 0..96 {
        runtime.register_cold(
            ColdObserverConfig::new("health", WakeProfileCode::Manual, |state: &ServiceState, transition| {
                transition.touches(FIELD_HEALTH) && state.healthy
            })
            .with_interest_mask(FIELD_HEALTH),
        );
    }
    for _ in 0..32 {
        runtime.register_cold(queue_threshold_observer(WakeProfileCode::Manual, 10_000).wake_every_match());
    }

    let key = condition_key(FIELD_QUEUE, WakeProfileCode::Manual);
    assert!(runtime.should_publish_cold_indexed(key));
    assert_eq!(runtime.indexed_cold_candidate_count(key), 32);

    let report = runtime.publish_stateful_auto(
        key,
        PayloadRef::default(),
        ServiceEvent::SetQueue(10_000),
    );

    assert_eq!(report.cold_scanned, 32);
    assert_eq!(report.cold_manual_delivered, 32);
}
