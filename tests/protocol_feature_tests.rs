#![cfg(feature = "protocol")]

use ob_serve_a_bowl::*;
use waker_protocol::{Envelope, SchedulerMsg, TaskId, WakeReason};

#[test]
fn protocol_task_id_converts_into_target_task_id() {
    let protocol = TaskId::new(42, 7);
    let target = TargetTaskId::from(protocol);
    assert_eq!(target.slot, 42);
    assert_eq!(target.generation, 7);
}

#[test]
fn target_task_id_converts_back_into_protocol_task_id() {
    let target = TargetTaskId::new(5, 9);
    let protocol: TaskId = target.into();
    assert_eq!(protocol, TaskId::new(5, 9));
}

#[test]
fn wake_reason_codes_map_to_protocol_reasons() {
    assert_eq!(WakeReasonCode::Spawn.to_protocol_reason(), WakeReason::Spawn);
    assert_eq!(WakeReasonCode::Manual.to_protocol_reason(), WakeReason::Manual);
    assert_eq!(WakeReasonCode::Inbox.to_protocol_reason(), WakeReason::Inbox);
    assert_eq!(WakeReasonCode::Timer.to_protocol_reason(), WakeReason::Timer);
    assert_eq!(WakeReasonCode::Condition.to_protocol_reason(), WakeReason::Condition);
    assert_eq!(WakeReasonCode::Resume.to_protocol_reason(), WakeReason::Resume);
    assert_eq!(WakeReasonCode::Qos.to_protocol_reason(), WakeReason::QoS);
    assert_eq!(WakeReasonCode::MemoryPressure.to_protocol_reason(), WakeReason::MemoryPressure);
    assert_eq!(WakeReasonCode::Cancel.to_protocol_reason(), WakeReason::Cancel);
    assert_eq!(WakeReasonCode::Shutdown.to_protocol_reason(), WakeReason::Shutdown);
}

fn protocol_key(mask: u16, profile: WakeProfileCode) -> WakeKey64 {
    WakeKey64::typed(
        WakeReasonCode::Condition,
        profile,
        5,
        mask,
        2,
        WakeKey64::FLAG_HOT as u16 | WakeKey64::FLAG_COLD as u16,
    )
    .unwrap()
}

#[test]
fn hot_inline_events_convert_to_protocol_wake_messages() {
    let mut runtime = ObservableWakeRuntime::new();
    let target = TargetTaskId::new(77, 3);
    runtime.register(
        ObserverConfig::new("protocol-hot", WakeProfileCode::InlineQueue)
            .with_interest_mask(0b10)
            .with_target(target),
    );

    let report = runtime.publish(ObservablePacket::new(
        protocol_key(0b10, WakeProfileCode::InlineQueue),
        PayloadRef::new(1, 0, 0, 16),
    ));
    assert_eq!(report.inline_delivered, 1);

    let bridge = runtime.drain_inline_protocol_wake_messages();
    assert_eq!(bridge.scanned, 1);
    assert_eq!(bridge.converted, 1);
    assert_eq!(bridge.message_count(), 1);
    assert!(!bridge.has_errors());

    let mut messages = bridge.take_messages();
    match messages.pop().unwrap() {
        SchedulerMsg::Wake { id, reason } => {
            assert_eq!(id, TaskId::new(77, 3));
            assert_eq!(reason, WakeReason::Condition);
        }
        _ => panic!("expected protocol wake message"),
    }
}

#[test]
fn bridge_reports_missing_targets_without_panicking() {
    let mut runtime = ObservableWakeRuntime::new();
    let observer = runtime.register(
        ObserverConfig::new("untargeted", WakeProfileCode::InlineQueue).with_interest_mask(0b1),
    );

    runtime.publish(ObservablePacket::new(
        protocol_key(0b1, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
    ));

    let bridge = runtime.drain_inline_protocol_wake_messages();
    assert_eq!(bridge.scanned, 1);
    assert_eq!(bridge.converted, 0);
    assert_eq!(bridge.skipped, 1);
    assert_eq!(bridge.errors, vec![ProtocolBridgeError::MissingTarget { observer_id: observer }]);
}

#[derive(Debug, Default)]
struct BridgeState {
    queue_depth: u32,
}

#[derive(Debug, Clone, Copy)]
enum BridgeEvent {
    SetQueue(u32),
}

impl ObservableState for BridgeState {
    type Event = BridgeEvent;

    fn apply(&mut self, event: &Self::Event) -> StateTransition {
        match *event {
            BridgeEvent::SetQueue(depth) => {
                self.queue_depth = depth;
                StateTransition::changed(0b100)
            }
        }
    }
}

#[test]
fn cold_inline_events_convert_to_protocol_wake_messages() {
    let mut runtime = ColdObservableRuntime::new(BridgeState::default());
    runtime.register_cold(
        ColdObserverConfig::new("cold-protocol", WakeProfileCode::InlineQueue, |state: &BridgeState, transition| {
            transition.touches(0b100) && state.queue_depth >= 10
        })
        .with_interest_mask(0b100)
        .with_target(TargetTaskId::new(9, 1)),
    );

    let report = runtime.publish_stateful(
        protocol_key(0b100, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
        BridgeEvent::SetQueue(10),
    );
    assert_eq!(report.cold_inline_delivered, 1);

    let bridge = runtime.drain_cold_inline_protocol_wake_messages();
    assert_eq!(bridge.scanned, 1);
    assert_eq!(bridge.converted, 1);

    let mut messages = bridge.take_messages();
    match messages.pop().unwrap() {
        SchedulerMsg::Wake { id, reason } => {
            assert_eq!(id, TaskId::new(9, 1));
            assert_eq!(reason, WakeReason::Condition);
        }
        _ => panic!("expected cold protocol wake message"),
    }
}

struct NoopEnvelope;

// SAFETY: dispatch never reads `target`.
unsafe impl Envelope for NoopEnvelope {
    fn kind(&self) -> &'static str {
        "noop"
    }

    fn target_type(&self) -> std::any::TypeId {
        std::any::TypeId::of::<()>()
    }

    unsafe fn dispatch(self: Box<Self>, _target: *mut ()) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn wake_event_can_build_protocol_deliver_message_with_caller_envelope() {
    let event = WakeEvent {
        observer_id: ObserverId::new(1, 0),
        target: Some(TargetTaskId::new(44, 2)),
        key: protocol_key(0b1, WakeProfileCode::InlineQueue),
        payload: PayloadRef::new(3, 0, 0, 4),
        reason: WakeReasonCode::Inbox,
        profile: WakeProfileCode::InlineQueue,
        wake_epoch: 1,
    };

    let msg = event
        .to_protocol_deliver_msg(Box::new(NoopEnvelope))
        .unwrap();

    match msg {
        SchedulerMsg::Deliver { id, envelope } => {
            assert_eq!(id, TaskId::new(44, 2));
            assert_eq!(envelope.kind(), "noop");
        }
        _ => panic!("expected protocol deliver message"),
    }
}

#[test]
fn send_protocol_wake_reports_success_with_live_scheduler_handle() {
    let protocol = waker_protocol::WakerProtocol::new(waker_protocol::MemoryCheckpointStore::default());
    let handle = protocol.handle();
    let event = WakeEvent {
        observer_id: ObserverId::new(2, 0),
        target: Some(TargetTaskId::new(123, 0)),
        key: protocol_key(0b1, WakeProfileCode::InlineQueue),
        payload: PayloadRef::default(),
        reason: WakeReasonCode::Manual,
        profile: WakeProfileCode::InlineQueue,
        wake_epoch: 1,
    };

    event.send_protocol_wake(&handle).unwrap();
}
