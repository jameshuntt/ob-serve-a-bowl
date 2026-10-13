use ob_serve_a_bowl::{
    ObservableEnvelope, ObservableWakeRuntime, ObserverConfig, TaggedPayloadArena, WakeKey64,
    WakeProfileCode, WakeReasonCode,
};
use petr_core::{Key, PortHint, ProphecyBits, SelectorWord};

const FIELD_PAYLOAD: u16 = 1 << 0;
const LOW_BITS: usize = 3;

#[repr(align(8))]
#[derive(Debug, PartialEq, Eq)]
struct Payload {
    id: u64,
    message: &'static str,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = ObservableWakeRuntime::new();
    let mut arena = TaggedPayloadArena::<Payload, LOW_BITS>::with_key(Key::from_seed(42));

    runtime.register(
        ObserverConfig::new("tagged-payload-observer", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_PAYLOAD),
    );

    let handle = arena.insert(
        Payload {
            id: 1,
            message: "payload is resolved only after the wake key matches",
        },
        3,
        SelectorWord::new(1, 1, 7),
        ProphecyBits {
            maybe_zero: false,
            maybe_empty: false,
            hot: true,
            skip_hint: false,
            port_hint: PortHint::Dispatch,
        },
    )?;

    let key = WakeKey64::typed(
        WakeReasonCode::Condition,
        WakeProfileCode::InlineQueue,
        4,
        FIELD_PAYLOAD,
        1,
        WakeKey64::FLAG_COLD as u16,
    )?;

    let report = runtime.publish_tagged(ObservableEnvelope::new(key, handle));
    for resolved in report.resolve_events(&arena)? {
        println!(
            "observer={:?} payload={} message={}",
            resolved.event.observer_id, resolved.payload.id, resolved.payload.message
        );
    }

    Ok(())
}
