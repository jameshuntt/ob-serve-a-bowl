// 0.1.0: one thread publishes while another reads the same observer's wake count, no lock.
// This does not compile. `publish` takes `&mut self` and `observer_epoch` borrows the runtime,
// so the only way to share the runtime between the two threads is behind a lock.
use ob_serve_a_bowl::*;

fn main() {
    let mut runtime = ObservableWakeRuntime::new();
    let id = runtime.register(ObserverConfig::new("o", WakeProfileCode::AtomicEpoch));
    let key = WakeKey64::typed(WakeReasonCode::Condition, WakeProfileCode::AtomicEpoch, 4, 1u16, 1, 0).unwrap();
    let shared = &runtime;
    std::thread::scope(|s| {
        s.spawn(|| shared.publish(ObservablePacket::new(key, PayloadRef::default())));
        s.spawn(|| shared.observer_epoch(id));
    });
}
