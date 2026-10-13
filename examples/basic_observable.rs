use ob_serve_a_bowl::{
    cold_condition_matchers, ObservablePacket, ObservableWakeRuntime, ObserverConfig, PayloadRef,
    WakeKey64, WakeProfileCode, WakeReasonCode,
};

const FIELD_HEALTH: u16 = 1 << 0;
const FIELD_QUEUE_DEPTH: u16 = 1 << 1;

fn main() {
    let mut runtime = ObservableWakeRuntime::new();

    runtime.register(
        ObserverConfig::new("queue-depth-cold-observer", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_QUEUE_DEPTH)
            .with_matchers(cold_condition_matchers(3, u128::from(FIELD_QUEUE_DEPTH))),
    );

    let key = WakeKey64::typed(
        WakeReasonCode::Condition,
        WakeProfileCode::InlineQueue,
        4,
        FIELD_HEALTH | FIELD_QUEUE_DEPTH,
        1,
        WakeKey64::FLAG_COLD as u16,
    )
    .expect("valid wake key");

    let report = runtime.publish(ObservablePacket::new(key, PayloadRef::new(10, 0, 0, 128)));
    println!("matched={} delivered={}", report.matched, report.delivered());

    for event in runtime.drain_inline() {
        println!("wake {:?} payload={:?}", event.reason, event.payload);
    }
}
