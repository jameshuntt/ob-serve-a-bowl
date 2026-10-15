//! `ObserverHandle`: reading an observer's wakes from another thread while the runtime publishes.
//! Miri runs these too, so a data race on the shared counters would be reported.

use ob_serve_a_bowl::*;

fn key(n: u16) -> WakeKey64 {
    WakeKey64::typed(WakeReasonCode::Manual, WakeProfileCode::AtomicEpoch, 0, 0b1_u16, 0, n).unwrap()
}

fn atomic_observer(runtime: &mut ObservableWakeRuntime, name: &'static str) -> ObserverId {
    runtime.register(ObserverConfig::new(name, WakeProfileCode::AtomicEpoch))
}

#[test]
fn a_reader_thread_follows_every_wake_in_order() {
    const WAKES: u16 = if cfg!(miri) { 20 } else { 500 };
    let mut runtime = ObservableWakeRuntime::new();
    let id = atomic_observer(&mut runtime, "followed");
    let handle = runtime.observer_handle(id).unwrap();
    let keys: Vec<WakeKey64> = (1..=WAKES).map(key).collect();

    let reader = {
        let keys = keys.clone();
        std::thread::spawn(move || {
            let mut last_epoch = 0;
            loop {
                let seen = handle.observe();
                assert!(seen.epoch >= last_epoch, "epoch went backwards");
                if seen.epoch > 0 {
                    // The key is wake number `epoch`'s or a later one's, never an earlier one's.
                    let position = keys.iter().position(|k| Some(*k) == seen.key).expect("an unknown key");
                    assert!(position + 1 >= seen.epoch as usize, "epoch {} with the key of wake {}", seen.epoch, position + 1);
                }
                assert!(handle.delivery_epoch() <= handle.wake_epoch());
                last_epoch = seen.epoch;
                if seen.epoch == u64::from(WAKES) {
                    return seen;
                }
                std::thread::yield_now();
            }
        })
    };

    for k in &keys {
        runtime.publish(ObservablePacket::new(*k, PayloadRef::default()));
    }
    let last = reader.join().unwrap();
    assert_eq!(last.key, keys.last().copied());
    assert_eq!(runtime.observer_delivery_epoch(id).unwrap(), u64::from(WAKES));
}

#[test]
fn handles_are_cloneable_send_and_sync() {
    fn send_sync<T: Send + Sync + Clone>() {}
    send_sync::<ObserverHandle>();
}

#[test]
fn a_handle_outlives_removal_and_stops_moving() {
    let mut runtime = ObservableWakeRuntime::new();
    let id = atomic_observer(&mut runtime, "removed");
    let handle = runtime.observer_handle(id).unwrap();
    runtime.publish(ObservablePacket::new(key(1), PayloadRef::default()));
    assert_eq!(handle.state(), ObserverState::Active);

    runtime.remove(id).unwrap();
    assert_eq!(handle.state(), ObserverState::Removed);
    assert_eq!(handle.observe(), ObservedWake { epoch: 1, key: Some(key(1)) });

    let reused = atomic_observer(&mut runtime, "reused");
    assert_eq!(reused.slot, id.slot);
    runtime.publish(ObservablePacket::new(key(2), PayloadRef::default()));
    assert_eq!(handle.observe(), ObservedWake { epoch: 1, key: Some(key(1)) }, "the old handle saw the new observer");
    assert_eq!(handle.id(), id);
    assert!(matches!(runtime.observer_handle(id), Err(WakeError::StaleObserver(_))));
    assert_eq!(runtime.observer_handle(reused).unwrap().observe().key, Some(key(2)));
}

#[test]
fn a_new_handle_reports_no_wake_yet() {
    let mut runtime = ObservableWakeRuntime::new();
    let id = atomic_observer(&mut runtime, "fresh");
    let handle = runtime.observer_handle(id).unwrap();
    assert_eq!(handle.observe(), ObservedWake { epoch: 0, key: None });
    assert_eq!(handle.delivery_epoch(), 0);
    assert_eq!(handle.last_key(), None);
    let clone = handle.clone();
    runtime.publish(ObservablePacket::new(key(3), PayloadRef::default()));
    assert_eq!(clone.wake_epoch(), 1);
    assert_eq!(handle.wake_epoch(), 1);
}

#[test]
fn a_missing_observer_has_no_handle() {
    let runtime = ObservableWakeRuntime::new();
    assert!(matches!(
        runtime.observer_handle(ObserverId::new(3, 0)),
        Err(WakeError::MissingObserver(_))
    ));
}

const FIELD_QUEUE: u16 = 1 << 1;

#[derive(Default)]
struct Queue {
    depth: u32,
}

struct SetDepth(u32);

impl ObservableState for Queue {
    type Event = SetDepth;

    fn apply(&mut self, event: &SetDepth) -> StateTransition {
        if self.depth == event.0 {
            return StateTransition::unchanged();
        }
        self.depth = event.0;
        StateTransition::changed(FIELD_QUEUE)
    }
}

#[test]
fn a_cold_observer_has_a_handle_too() {
    let mut runtime = ColdObservableRuntime::new(Queue::default());
    let id = runtime.register_cold(
        ColdObserverConfig::new("deep", WakeProfileCode::InlineQueue, |state: &Queue, transition| {
            transition.touches(FIELD_QUEUE) && state.depth >= 10
        })
        .with_interest_mask(FIELD_QUEUE)
        .with_matchers(cold_condition_matchers(1, u128::from(FIELD_QUEUE))),
    );
    let handle = runtime.cold_observer_handle(id).unwrap();
    let cold_key = WakeKey64::typed(
        WakeReasonCode::Condition,
        WakeProfileCode::InlineQueue,
        4,
        FIELD_QUEUE,
        1,
        WakeKey64::FLAG_COLD as u16,
    )
    .unwrap();

    let reader = std::thread::spawn(move || {
        while handle.wake_epoch() == 0 {
            std::thread::yield_now();
        }
        handle.observe()
    });
    runtime.publish_stateful(cold_key, PayloadRef::default(), SetDepth(10));
    let seen = reader.join().unwrap();
    assert_eq!(seen, ObservedWake { epoch: 1, key: Some(cold_key) });
    assert_eq!(runtime.cold_observer_epoch(id).unwrap(), 1);
}

#[test]
fn an_inactive_observer_reads_as_inactive() {
    let mut runtime = ObservableWakeRuntime::new();
    let id = runtime.register(ObserverConfig::new("off", WakeProfileCode::AtomicEpoch).inactive());
    let handle = runtime.observer_handle(id).unwrap();
    assert_eq!(handle.state(), ObserverState::Inactive);
    runtime.publish(ObservablePacket::new(key(1), PayloadRef::default()));
    assert_eq!(handle.observe(), ObservedWake { epoch: 0, key: None }, "an inactive observer is never woken");
    runtime.remove(id).unwrap();
    assert_eq!(handle.state(), ObserverState::Removed);
}

#[test]
fn dropping_the_runtime_marks_its_observers_removed() {
    let mut runtime = ObservableWakeRuntime::new();
    let id = atomic_observer(&mut runtime, "dropped");
    let handle = runtime.observer_handle(id).unwrap();
    runtime.publish(ObservablePacket::new(key(1), PayloadRef::default()));
    drop(runtime);
    assert_eq!(handle.state(), ObserverState::Removed);
    assert_eq!(handle.observe(), ObservedWake { epoch: 1, key: Some(key(1)) });
}

fn deep_queue_observer(active: bool) -> ColdObserverConfig<Queue> {
    let config = ColdObserverConfig::new("deep", WakeProfileCode::InlineQueue, |state: &Queue, transition: &StateTransition| {
        transition.touches(FIELD_QUEUE) && state.depth >= 10
    })
    .with_interest_mask(FIELD_QUEUE)
    .with_matchers(cold_condition_matchers(1, u128::from(FIELD_QUEUE)));
    if active { config } else { config.inactive() }
}

#[test]
fn cold_handles_report_inactive_and_removed_on_drop() {
    let mut runtime = ColdObservableRuntime::new(Queue::default());
    let on = runtime.register_cold(deep_queue_observer(true));
    let off = runtime.register_cold(deep_queue_observer(false));
    let (on, off) = (runtime.cold_observer_handle(on).unwrap(), runtime.cold_observer_handle(off).unwrap());
    assert_eq!(on.state(), ObserverState::Active);
    assert_eq!(off.state(), ObserverState::Inactive);
    drop(runtime);
    assert_eq!(on.state(), ObserverState::Removed);
    assert_eq!(off.state(), ObserverState::Removed);
}

#[cfg(feature = "crossbeam-wake")]
mod crossbeam {
    use super::*;

    fn crossbeam_key(n: u16) -> WakeKey64 {
        WakeKey64::typed(WakeReasonCode::Manual, WakeProfileCode::CrossbeamQueue, 0, 0b1_u16, 0, n).unwrap()
    }

    fn crossbeam_observer(runtime: &mut ObservableWakeRuntime) -> ObserverHandle {
        let id = runtime.register(ObserverConfig::new("channel", WakeProfileCode::CrossbeamQueue));
        runtime.observer_handle(id).unwrap()
    }

    #[test]
    fn a_failed_send_is_not_counted_as_delivered() {
        let mut no_sender = ObservableWakeRuntime::new();
        let handle = crossbeam_observer(&mut no_sender);
        let report = no_sender.publish(ObservablePacket::new(crossbeam_key(1), PayloadRef::default()));
        assert_eq!(report.errors, vec![WakeError::CrossbeamSenderMissing]);
        assert_eq!((handle.wake_epoch(), handle.delivery_epoch()), (1, 0));

        let (tx, rx) = crossbeam_channel::unbounded();
        drop(rx);
        let mut no_receiver = ObservableWakeRuntime::with_crossbeam_sender(tx);
        let handle = crossbeam_observer(&mut no_receiver);
        let report = no_receiver.publish(ObservablePacket::new(crossbeam_key(1), PayloadRef::default()));
        assert_eq!(report.errors, vec![WakeError::CrossbeamSendFailed]);
        assert_eq!((handle.wake_epoch(), handle.delivery_epoch()), (1, 0));
    }

    /// A handle that sees delivery n on another thread can already receive event n. The window this
    /// guards is a few instructions wide, so the reader spins without yielding and the runtime
    /// publishes many wakes (loom cannot check it: the channel's atomics are not loom's).
    #[test]
    fn a_counted_delivery_is_already_in_the_channel() {
        const WAKES: u16 = if cfg!(miri) { 10 } else { 50_000 };
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut runtime = ObservableWakeRuntime::with_crossbeam_sender(tx);
        let handle = crossbeam_observer(&mut runtime);

        let reader = std::thread::spawn(move || {
            let mut received = 0u64;
            while received < u64::from(WAKES) {
                let delivered = handle.delivery_epoch();
                while received < delivered {
                    rx.try_recv().expect("delivery counted before the event was in the channel");
                    received += 1;
                }
                std::hint::spin_loop();
            }
            received
        });

        for n in 1..=WAKES {
            runtime.publish(ObservablePacket::new(crossbeam_key(n), PayloadRef::default()));
        }
        assert_eq!(reader.join().unwrap(), u64::from(WAKES));
    }
}
