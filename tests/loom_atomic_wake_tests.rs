#![cfg(loom)]
#![cfg(feature = "loom-tests")]

//! The whole runtime on one loom thread, an `ObserverHandle` on another: every interleaving and
//! weak-memory reordering of a real publish against a real handle read.

use loom::thread;
use ob_serve_a_bowl::{
    ObservablePacket, ObservableWakeRuntime, ObserverConfig, ObserverState, PayloadRef, WakeKey64, WakeProfileCode,
    WakeReasonCode,
};

fn key(n: u16) -> WakeKey64 {
    WakeKey64::typed(WakeReasonCode::Manual, WakeProfileCode::AtomicEpoch, 0, 0b1_u16, 0, n).unwrap()
}

#[test]
fn a_handle_reads_a_publish_on_another_thread() {
    loom::model(|| {
        let mut runtime = ObservableWakeRuntime::new();
        let id = runtime.register(ObserverConfig::new("reader", WakeProfileCode::AtomicEpoch));
        let handle = runtime.observer_handle(id).unwrap();

        let reader = thread::spawn(move || {
            let seen = handle.observe();
            let delivered = handle.delivery_epoch();
            (seen, delivered)
        });

        let report = runtime.publish(ObservablePacket::new(key(7), PayloadRef::default()));
        assert_eq!(report.atomic_delivered, 1);

        let (seen, delivered) = reader.join().unwrap();
        match seen.epoch {
            0 => assert_eq!(seen.key, None),
            1 => assert_eq!(seen.key, Some(key(7))),
            n => panic!("epoch {n} after one publish"),
        }
        assert!(delivered <= 1);
        assert_eq!(runtime.observer_epoch(id).unwrap(), 1);
    });
}

#[test]
fn an_old_handle_never_sees_the_observer_that_reuses_its_slot() {
    loom::model(|| {
        let mut runtime = ObservableWakeRuntime::new();
        let old = runtime.register(ObserverConfig::new("old", WakeProfileCode::AtomicEpoch));
        let handle = runtime.observer_handle(old).unwrap();

        let reader = thread::spawn(move || (handle.state(), handle.observe()));

        runtime.remove(old).unwrap();
        let new = runtime.register(ObserverConfig::new("new", WakeProfileCode::AtomicEpoch));
        assert_eq!(new.slot, old.slot);
        runtime.publish(ObservablePacket::new(key(1), PayloadRef::default()));

        let (state, seen) = reader.join().unwrap();
        assert!(matches!(state, ObserverState::Active | ObserverState::Removed));
        assert_eq!(seen.epoch, 0, "the old handle saw the new observer's wake");
        assert_eq!(runtime.observer_epoch(new).unwrap(), 1);
    });
}

