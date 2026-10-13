#![cfg(feature = "crossbeam-wake")]

use ob_serve_a_bowl::{
    ObservablePacket, ObservableWakeRuntime, ObserverConfig, PayloadRef, WakeKey64,
    WakeProfileCode, WakeReasonCode,
};
use std::time::Duration;

const FIELD_QUEUE: u16 = 1 << 1;

fn key() -> WakeKey64 {
    WakeKey64::typed(
        WakeReasonCode::Condition,
        WakeProfileCode::CrossbeamQueue,
        4,
        FIELD_QUEUE,
        2,
        WakeKey64::FLAG_COLD as u16 | WakeKey64::FLAG_EXTERNAL as u16,
    )
    .unwrap()
}

#[test]
fn crossbeam_profile_sends_wake_event_when_feature_enabled() {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut runtime = ObservableWakeRuntime::with_crossbeam_sender(tx);
    let id = runtime.register(
        ObserverConfig::new("crossbeam", WakeProfileCode::CrossbeamQueue)
            .with_interest_mask(FIELD_QUEUE),
    );

    let report = runtime.publish(ObservablePacket::new(
        key(),
        PayloadRef::new(3, 0, 64, 16),
    ));

    assert_eq!(report.matched, 1);
    assert_eq!(report.crossbeam_delivered, 1);
    assert!(report.errors.is_empty());

    let event = rx.recv_timeout(Duration::from_millis(250)).unwrap();
    assert_eq!(event.observer_id, id);
    assert_eq!(event.payload.offset, 64);
    assert_eq!(event.profile, WakeProfileCode::CrossbeamQueue);
}

#[test]
fn crossbeam_profile_requires_a_sender() {
    let mut runtime = ObservableWakeRuntime::new();
    runtime.register(
        ObserverConfig::new("crossbeam", WakeProfileCode::CrossbeamQueue)
            .with_interest_mask(FIELD_QUEUE),
    );

    let report = runtime.publish(ObservablePacket::new(key(), PayloadRef::default()));
    assert_eq!(report.matched, 1);
    assert_eq!(report.crossbeam_delivered, 0);
    assert_eq!(report.errors.len(), 1);
}
