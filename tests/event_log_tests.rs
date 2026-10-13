use ob_serve_a_bowl::*;

const FIELD_QUEUE: u16 = 1 << 0;
const FIELD_HEALTH: u16 = 1 << 1;

fn condition_key(mask: u16, profile: WakeProfileCode) -> WakeKey64 {
    WakeKey64::typed(
        WakeReasonCode::Condition,
        profile,
        4,
        mask,
        3,
        WakeKey64::FLAG_HOT as u16 | WakeKey64::FLAG_COLD as u16,
    )
    .unwrap()
}

#[test]
fn hot_publish_logged_records_and_replays_summary() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut log = ObservableEventLog::new();
    runtime.register(
        ObserverConfig::new("hot-queue", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_QUEUE),
    );

    let packet = ObservablePacket::new(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::new(7, 1, 12, 32),
    );
    let report = runtime.publish_logged(&mut log, packet);

    assert_eq!(report.inline_delivered, 1);
    assert_eq!(log.len(), 1);
    assert_eq!(log.records()[0].kind, EventLogKind::HotPublish);
    assert_eq!(log.records()[0].payload.unwrap().slot, 7);

    let summary = log.replay_summary().unwrap();
    assert_eq!(summary.records, 1);
    assert_eq!(summary.hot_publishes, 1);
    assert_eq!(summary.delivered, 1);
    assert_eq!(summary.errors, 0);
}

#[test]
fn hot_publish_auto_logged_records_and_replays_summary() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut log = ObservableEventLog::new();
    runtime.register(
        ObserverConfig::new("hot-auto-queue", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_QUEUE),
    );

    let packet = ObservablePacket::new(
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::new(8, 1, 0, 16),
    );
    let report = runtime.publish_auto_logged(&mut log, packet);

    assert_eq!(report.inline_delivered, 1);
    assert_eq!(log.len(), 1);
    assert_eq!(log.records()[0].kind, EventLogKind::HotPublish);
    assert_eq!(log.replay_summary().unwrap().hot_publishes, 1);
}

#[derive(Debug, Clone, Copy, Default)]
struct QueueState {
    depth: u32,
}

#[derive(Debug, Clone, Copy)]
enum QueueEvent {
    SetDepth(u32),
}

impl ObservableState for QueueState {
    type Event = QueueEvent;

    fn apply(&mut self, event: &Self::Event) -> StateTransition {
        match *event {
            QueueEvent::SetDepth(depth) => {
                if self.depth == depth {
                    StateTransition::unchanged()
                } else {
                    self.depth = depth;
                    StateTransition::changed(FIELD_QUEUE)
                }
            }
        }
    }
}

#[test]
fn cold_publish_logged_records_state_transition() {
    let mut runtime = ColdObservableRuntime::new(QueueState::default());
    let mut log = ObservableEventLog::new();
    runtime.register_cold(
        ColdObserverConfig::new("cold-queue", WakeProfileCode::InlineQueue, |state: &QueueState, transition| {
            transition.touches(FIELD_QUEUE) && state.depth >= 10
        })
        .with_interest_mask(FIELD_QUEUE),
    );

    let report = runtime.publish_stateful_logged(
        &mut log,
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::new(2, 0, 0, 8),
        QueueEvent::SetDepth(10),
    );

    assert_eq!(report.cold_inline_delivered, 1);
    let record = &log.records()[0];
    assert_eq!(record.kind, EventLogKind::ColdPublish);
    assert_eq!(record.transition.unwrap(), StateTransitionTrace { changed: true, field_mask: FIELD_QUEUE });

    let summary = log.replay_summary().unwrap();
    assert_eq!(summary.cold_publishes, 1);
    assert_eq!(summary.state_changes, 1);
    assert_eq!(summary.delivered, 1);
}

#[test]
fn cold_publish_auto_logged_records_state_transition() {
    let mut runtime = ColdObservableRuntime::new(QueueState::default());
    let mut log = ObservableEventLog::new();
    runtime.register_cold(
        ColdObserverConfig::new("cold-auto-queue", WakeProfileCode::InlineQueue, |state: &QueueState, transition| {
            transition.touches(FIELD_QUEUE) && state.depth >= 10
        })
        .with_interest_mask(FIELD_QUEUE),
    );

    let report = runtime.publish_stateful_auto_logged(
        &mut log,
        condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
        PayloadRef::new(3, 0, 0, 8),
        QueueEvent::SetDepth(12),
    );

    assert_eq!(report.cold_inline_delivered, 1);
    assert_eq!(log.records()[0].kind, EventLogKind::ColdPublish);
    assert_eq!(log.replay_summary().unwrap().cold_publishes, 1);
}

#[test]
fn drained_wake_events_log_protocol_targets() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut log = ObservableEventLog::new();
    runtime.register(
        ObserverConfig::new("targeted", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_HEALTH)
            .with_target(TargetTaskId::new(99, 4)),
    );

    runtime.publish(ObservablePacket::new(
        condition_key(FIELD_HEALTH, WakeProfileCode::InlineQueue),
        PayloadRef::default(),
    ));
    let events = runtime.drain_inline_logged(&mut log);

    assert_eq!(events.len(), 1);
    assert_eq!(log.records()[0].kind, EventLogKind::WakeEvent);
    assert_eq!(log.records()[0].target, Some(TargetTrace { slot: 99, generation: 4 }));
    assert_eq!(log.replay_summary().unwrap().last_target, Some(TargetTrace { slot: 99, generation: 4 }));
}

#[test]
fn replay_rejects_non_monotonic_sequences() {
    let record = EventLogRecord {
        seq: LogSequence(7),
        kind: EventLogKind::HotPublish,
        key_raw: 0,
        payload: None,
        tagged_payload: None,
        transition: None,
        target: None,
        delivery: DeliveryTrace::default(),
        error_count: 0,
        notes: Vec::new(),
    };
    let records = vec![record.clone(), record];

    let err = replay_records(&records).unwrap_err();
    assert!(matches!(err, EventLogReplayError::NonMonotonicSequence { .. }));
}

#[cfg(feature = "tagged-payloads")]
mod tagged_log_tests {
    use super::*;
    use petr_core::{Key, PortHint, ProphecyBits, SelectorWord};

    const LOW_BITS: usize = 3;

    #[repr(align(8))]
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Payload {
        value: u64,
    }

    fn prophecy() -> ProphecyBits {
        ProphecyBits {
            maybe_zero: false,
            maybe_empty: false,
            hot: true,
            skip_hint: false,
            port_hint: PortHint::Dispatch,
        }
    }

    #[test]
    fn tagged_publish_logged_records_encoded_handle_without_resolving() {
        let mut runtime = ObservableWakeRuntime::new();
        let mut log = ObservableEventLog::new();
        let mut arena = TaggedPayloadArena::<Payload, LOW_BITS>::with_key(Key::from_seed(123));
        let handle = arena
            .insert(Payload { value: 55 }, 3, SelectorWord::new(1, 2, 9), prophecy())
            .unwrap();

        runtime.register(
            ObserverConfig::new("tagged", WakeProfileCode::InlineQueue)
                .with_interest_mask(FIELD_QUEUE),
        );

        let envelope = ObservableEnvelope::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
            handle,
        );
        let report = runtime.publish_tagged_logged(&mut log, envelope);

        assert_eq!(report.inline_delivered, 1);
        assert_eq!(log.records()[0].kind, EventLogKind::TaggedPublish);
        let trace = log.records()[0].tagged_payload.unwrap();
        assert_eq!(trace.index, handle.index());
        assert_eq!(trace.selector, handle.selector_raw());
        assert_eq!(arena.get(handle).unwrap().value, 55);
        assert_eq!(log.replay_summary().unwrap().tagged_publishes, 1);
    }
}

#[cfg(feature = "protocol")]
mod protocol_log_tests {
    use super::*;

    #[test]
    fn protocol_bridge_report_can_be_logged() {
        let mut runtime = ObservableWakeRuntime::new();
        let mut log = ObservableEventLog::new();
        runtime.register(
            ObserverConfig::new("protocol", WakeProfileCode::InlineQueue)
                .with_interest_mask(FIELD_QUEUE)
                .with_target(TargetTaskId::new(5, 6)),
        );
        runtime.publish(ObservablePacket::new(
            condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
            PayloadRef::default(),
        ));

        let bridge = runtime.drain_inline_protocol_wake_messages();
        log.append_protocol_bridge(&bridge);

        let summary = log.replay_summary().unwrap();
        assert_eq!(summary.protocol_bridges, 1);
        assert_eq!(summary.last_target, Some(TargetTrace { slot: 5, generation: 6 }));
        assert_eq!(summary.delivered, 1);
    }
}

#[cfg(feature = "serialization")]
mod serialization_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sample_log() -> ObservableEventLog {
        let mut runtime = ObservableWakeRuntime::new();
        let mut log = ObservableEventLog::new();
        runtime.register(
            ObserverConfig::new("hot-queue", WakeProfileCode::InlineQueue)
                .with_interest_mask(FIELD_QUEUE)
                .with_target(TargetTaskId::new(11, 2)),
        );

        runtime.publish_logged(
            &mut log,
            ObservablePacket::new(
                condition_key(FIELD_QUEUE, WakeProfileCode::InlineQueue),
                PayloadRef::new(3, 4, 16, 32),
            ),
        );
        runtime.drain_inline_logged(&mut log);
        log
    }

    #[test]
    fn json_snapshot_roundtrips_and_replays() {
        let log = sample_log();
        let json = log.to_json_pretty_string().unwrap();
        assert!(json.contains("HotPublish"));
        assert!(json.contains("WakeEvent"));

        let restored = ObservableEventLog::from_json_str(&json).unwrap();
        assert_eq!(restored.records(), log.records());
        assert_eq!(restored.replay_summary().unwrap(), log.replay_summary().unwrap());
    }

    #[test]
    fn jsonl_roundtrips_records_without_snapshot_wrapper() {
        let log = sample_log();
        let jsonl = log.to_jsonl_string().unwrap();
        assert_eq!(jsonl.lines().count(), log.len());

        let restored = ObservableEventLog::from_jsonl_str(&jsonl).unwrap();
        assert_eq!(restored.records(), log.records());
        assert_eq!(restored.replay_summary().unwrap().wake_events, 1);
    }

    #[test]
    fn export_and_import_paths_work_for_json_and_jsonl() {
        let log = sample_log();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let json_path = std::env::temp_dir().join(format!("observable-event-log-{stamp}.json"));
        let jsonl_path = std::env::temp_dir().join(format!("observable-event-log-{stamp}.jsonl"));

        log.export_json_path(&json_path).unwrap();
        log.export_jsonl_path(&jsonl_path).unwrap();

        let json_log = ObservableEventLog::import_json_path(&json_path).unwrap();
        let jsonl_log = ObservableEventLog::import_jsonl_path(&jsonl_path).unwrap();

        assert_eq!(json_log.records(), log.records());
        assert_eq!(jsonl_log.records(), log.records());

        let _ = std::fs::remove_file(json_path);
        let _ = std::fs::remove_file(jsonl_path);
    }

    #[test]
    fn unsupported_snapshot_version_is_rejected() {
        let mut snapshot = sample_log().snapshot();
        snapshot.version = EVENT_LOG_EXPORT_VERSION + 1;

        let err = ObservableEventLog::from_snapshot(snapshot).unwrap_err();
        assert!(matches!(
            err,
            EventLogSerializationError::UnsupportedVersion { .. }
        ));
    }

    #[test]
    fn jsonl_import_rejects_non_monotonic_records() {
        let log = sample_log();
        let first = serde_json::to_string(&log.records()[0]).unwrap();
        let input = format!("{first}\n{first}\n");

        let err = ObservableEventLog::from_jsonl_str(&input).unwrap_err();
        assert!(matches!(err, EventLogSerializationError::Replay(_)));
    }
}
