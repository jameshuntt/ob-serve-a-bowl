use ob_serve_a_bowl::prelude::*;

const FIELD_QUEUE: u16 = 1 << 0;
const FIELD_HEALTH: u16 = 1 << 1;

#[derive(Debug, Clone, Copy, Default)]
struct ServiceState {
    queue_depth: u32,
    unhealthy: bool,
}

#[derive(Debug, Clone, Copy)]
enum ServiceEvent {
    QueueDepth(u32),
    Health(bool),
}

impl ObservableState for ServiceState {
    type Event = ServiceEvent;

    fn apply(&mut self, event: &Self::Event) -> StateTransition {
        match *event {
            ServiceEvent::QueueDepth(depth) => {
                self.queue_depth = depth;
                StateTransition::changed(FIELD_QUEUE)
            }
            ServiceEvent::Health(unhealthy) => {
                self.unhealthy = unhealthy;
                StateTransition::changed(FIELD_HEALTH)
            }
        }
    }
}


#[test]
fn service_health_event_marks_health_field() {
    let mut state = ServiceState::default();

    let transition = state.apply(&ServiceEvent::Health(true));

    assert!(state.unhealthy);
    assert!(transition.changed);
    assert!(transition.touches(FIELD_HEALTH));
    assert!(!transition.touches(FIELD_QUEUE));
}

#[test]
fn wake_key_builder_builds_typed_compact_key() {
    let key = wake_key()
        .reason(WakeReasonCode::Condition)
        .profile(WakeProfileCode::InlineQueue)
        .qos(9)
        .field_mask(FIELD_QUEUE)
        .phase(3)
        .hot()
        .cold()
        .external()
        .build()
        .unwrap();

    assert_eq!(key.reason_code().unwrap(), WakeReasonCode::Condition);
    assert_eq!(key.profile_code().unwrap(), WakeProfileCode::InlineQueue);
    assert_eq!(key.qos(), 9);
    assert!(key.has_any_field(FIELD_QUEUE));
    assert!(key.has_flags(WakeKey64::FLAG_HOT as u16));
    assert!(key.has_flags(WakeKey64::FLAG_COLD as u16));
    assert!(key.has_flags(WakeKey64::FLAG_EXTERNAL as u16));
}

#[test]
fn hot_observer_builder_registers_recommended_config() {
    let mut runtime = ob_serve_a_bowl::ObservableWakeRuntime::new();
    let target = target_task(7, 2);

    let id = runtime.register(
        hot_observer("queue-hot")
            .inline_queue()
            .interested_in(FIELD_QUEUE)
            .target(target)
            .build(),
    );

    let key = wake_key()
        .reason(WakeReasonCode::Condition)
        .profile(WakeProfileCode::InlineQueue)
        .field_mask(FIELD_QUEUE)
        .hot()
        .build()
        .unwrap();

    let report = runtime.publish_auto(packet(key, payload_ref(1, 0, 0, 64)));
    assert_eq!(report.delivered(), 1);
    assert_eq!(runtime.observer_epoch(id).unwrap(), 1);

    let events = runtime.drain_inline();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].target, Some(target));
}

#[test]
fn logged_cold_runtime_exposes_recommended_happy_path() {
    let mut runtime = LoggedColdRuntime::new(ServiceState::default());
    let target = target_task(42, 7);

    runtime.register_hot(
        hot_observer("queue-hot")
            .inline_queue()
            .interested_in(FIELD_QUEUE)
            .target(target)
            .build(),
    );

    runtime.register_cold(
        cold_observer("queue-backpressure", WakeProfileCode::InlineQueue, |state: &ServiceState, transition: &StateTransition| {
            transition.touches(FIELD_QUEUE) && state.queue_depth >= 10_000
        })
        .with_interest_mask(FIELD_QUEUE)
        .with_target(target),
    );

    let key = wake_key()
        .reason(WakeReasonCode::Condition)
        .profile(WakeProfileCode::InlineQueue)
        .field_mask(FIELD_QUEUE)
        .hot()
        .cold()
        .build()
        .unwrap();

    let report = runtime.publish(key, payload_ref(2, 0, 0, 128), ServiceEvent::QueueDepth(12_000));
    assert_eq!(report.hot.delivered(), 1);
    assert_eq!(report.cold_delivered(), 1);
    assert!(report.transition.changed);

    let hot = runtime.drain_hot_inline_logged();
    let cold = runtime.drain_cold_inline_logged();
    assert_eq!(hot.len(), 1);
    assert_eq!(cold.len(), 1);

    let summary = runtime.log().replay().unwrap();
    assert_eq!(summary.cold_publishes, 1);
    assert_eq!(summary.wake_events, 2);
    assert_eq!(summary.delivered, 4);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ApiFieldMask(u16);

impl ApiFieldMask {
    const QUEUE: Self = Self(FIELD_QUEUE);
    const HEALTH: Self = Self(FIELD_HEALTH);

    const fn bits(self) -> u16 {
        self.0
    }
}

impl core::ops::BitOr for ApiFieldMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl ObservableFieldMask for ApiFieldMask {
    fn field_mask_bits(self) -> u16 {
        self.bits()
    }
}

#[test]
fn api_accepts_custom_bitmask_outputs_for_field_masks() {
    let combined = ApiFieldMask::QUEUE | ApiFieldMask::HEALTH;

    let key = wake_key()
        .reason(WakeReasonCode::Condition)
        .profile(WakeProfileCode::InlineQueue)
        .field_mask(combined)
        .hot()
        .cold()
        .build()
        .unwrap();

    assert!(key.has_any_field(ApiFieldMask::QUEUE));
    assert!(key.has_all_fields(combined));

    let hot = hot_observer("typed-mask-hot")
        .inline_queue()
        .interested_in(ApiFieldMask::QUEUE)
        .build();
    assert_eq!(hot.interest_mask, FIELD_QUEUE);

    let transition = StateTransition::changed(ApiFieldMask::HEALTH);
    assert!(transition.touches(ApiFieldMask::HEALTH));
    assert!(!transition.touches(ApiFieldMask::QUEUE));

    let cold = cold_observer(
        "typed-mask-cold",
        WakeProfileCode::InlineQueue,
        |_state: &ServiceState, transition: &StateTransition| transition.touches(ApiFieldMask::HEALTH),
    )
    .with_interest_mask(ApiFieldMask::HEALTH);
    assert_eq!(cold.interest_mask, FIELD_HEALTH);
}
