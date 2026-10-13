//! Realistic server-event integration example.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p ob_serve_a_bowl --example server_event_observable \
//!   --features protocol,serialization,tagged-payloads
//! ```
//!
//! This example models the shape you would use for a UDS/server packet:
//!
//! ```text
//! server packet
//! → WakeKey64
//! → optional TaggedPayloadRef
//! → hot observer wake
//! → cold state predicate wake
//! → waker_protocol SchedulerMsg::Wake
//! → JSONL event-log export
//! ```

use ob_serve_a_bowl::{
    ColdObservableRuntime, ColdObserverConfig, ObservableEnvelope, ObservableEventLog,
    ObservableFieldMask, ObservableState, ObservableWakeRuntime, ObserverConfig, PayloadRef, StateTransition,
    TaggedPayloadArena, TaggedPayloadRef, TargetTaskId, WakeKey64, WakeProfileCode,
    WakeReasonCode,
};
use petr_core::{Key, PortHint, ProphecyBits, SelectorWord};
use waker_protocol::SchedulerMsg;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ServerFields(u16);

impl ServerFields {
    const EMPTY: Self = Self(0);
    const QUEUE_DEPTH: Self = Self(1 << 0);
    const HEALTH: Self = Self(1 << 1);
    const PAYLOAD: Self = Self(1 << 2);

    const fn bits(self) -> u16 {
        self.0
    }
}

impl core::ops::BitOr for ServerFields {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for ServerFields {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl ObservableFieldMask for ServerFields {
    fn field_mask_bits(self) -> u16 {
        self.bits()
    }
}

const LOW_BITS: usize = 3;

#[repr(align(8))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerPacket {
    service: &'static str,
    queue_depth: u32,
    degraded: bool,
    payload_bytes: u32,
}

#[derive(Debug, Default)]
struct ServerState {
    queue_depth: u32,
    degraded: bool,
}

#[derive(Debug, Clone, Copy)]
struct ServerEvent {
    queue_depth: u32,
    degraded: bool,
}

impl ObservableState for ServerState {
    type Event = ServerEvent;

    fn apply(&mut self, event: &Self::Event) -> StateTransition {
        let mut changed = ServerFields::EMPTY;

        if self.queue_depth != event.queue_depth {
            self.queue_depth = event.queue_depth;
            changed |= ServerFields::QUEUE_DEPTH;
        }

        if self.degraded != event.degraded {
            self.degraded = event.degraded;
            changed |= ServerFields::HEALTH;
        }

        StateTransition::changed(changed)
    }
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

fn make_key(mask: ServerFields) -> WakeKey64 {
    WakeKey64::typed(
        WakeReasonCode::Condition,
        WakeProfileCode::InlineQueue,
        7,
        mask,
        2,
        (WakeKey64::FLAG_HOT | WakeKey64::FLAG_COLD | WakeKey64::FLAG_EXTERNAL) as u16,
    )
    .expect("example wake key should fit WakeKey64")
}

fn payload_ref_from_tagged(
    handle: TaggedPayloadRef<ServerPacket, LOW_BITS>,
    len: u32,
) -> PayloadRef {
    PayloadRef::new(handle.index(), handle.generation(), 0, len)
}

fn print_protocol_messages(label: &str, messages: &[SchedulerMsg]) {
    for message in messages {
        match message {
            SchedulerMsg::Wake { id, reason } => {
                println!(
                    "{label}: SchedulerMsg::Wake slot={} gen={} reason={reason:?}",
                    id.slot, id.generation
                );
            }
            SchedulerMsg::Deliver { id, envelope } => {
                println!(
                    "{label}: SchedulerMsg::Deliver slot={} gen={} envelope_kind={}",
                    id.slot,
                    id.generation,
                    envelope.kind()
                );
            }
            SchedulerMsg::Checkpoint { id, reason } => {
                println!(
                    "{label}: SchedulerMsg::Checkpoint slot={} gen={} reason={reason:?}",
                    id.slot, id.generation
                );
            }
            SchedulerMsg::Capitulate { id, .. } => {
                println!(
                    "{label}: SchedulerMsg::Capitulate slot={} gen={}",
                    id.slot, id.generation
                );
            }
            SchedulerMsg::Cancel { id } => {
                println!("{label}: SchedulerMsg::Cancel slot={} gen={}", id.slot, id.generation);
            }
            SchedulerMsg::Shutdown => println!("{label}: SchedulerMsg::Shutdown"),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_task = TargetTaskId::new(42, 7);
    let packet = ServerPacket {
        service: "archive-ingest",
        queue_depth: 12_500,
        degraded: true,
        payload_bytes: 4096,
    };

    // Optional payload plane: store the incoming packet in an arena and pass an
    // opaque validated handle through the wake system. The handle is not a raw
    // pointer and is only resolved after a wake has already matched.
    let mut arena = TaggedPayloadArena::<ServerPacket, LOW_BITS>::with_key(Key::from_seed(99));
    let tagged_payload = arena.insert(
        packet.clone(),
        3,
        SelectorWord::new(1, 1, 9),
        prophecy(),
    )?;

    let mut log = ObservableEventLog::new();
    let key = make_key(ServerFields::QUEUE_DEPTH | ServerFields::HEALTH | ServerFields::PAYLOAD);

    // Tagged payload observer: proves the payload handle can be logged and later
    // resolved without making payload resolution part of the routing decision.
    let mut payload_runtime = ObservableWakeRuntime::new();
    payload_runtime.register(
        ObserverConfig::new("payload-audit", WakeProfileCode::InlineQueue)
            .with_interest_mask(ServerFields::PAYLOAD)
            .with_target(target_task),
    );
    let tagged_report = payload_runtime.publish_tagged_logged(
        &mut log,
        ObservableEnvelope::new(key, tagged_payload),
    );
    for resolved in tagged_report.resolve_events(&arena)? {
        println!(
            "resolved-after-match: observer={:?} service={} bytes={}",
            resolved.event.observer_id, resolved.payload.service, resolved.payload.payload_bytes
        );
    }

    let payload_bridge = payload_runtime.drain_inline_protocol_wake_messages();
    print_protocol_messages("payload bridge", &payload_bridge.messages);
    log.append_protocol_bridge(&payload_bridge);

    // Hot/cold state plane: the hot observer wakes immediately on queue/health
    // changes. The cold observer wakes only when the materialized state crosses
    // the backpressure condition.
    let mut hot_runtime = ObservableWakeRuntime::new();
    hot_runtime.register(
        ObserverConfig::new("hot-ingest-router", WakeProfileCode::InlineQueue)
            .with_interest_mask(ServerFields::QUEUE_DEPTH | ServerFields::HEALTH)
            .with_target(target_task),
    );

    let mut runtime = ColdObservableRuntime::with_hot_runtime(ServerState::default(), hot_runtime);
    runtime.register_cold(
        ColdObserverConfig::new(
            "cold-backpressure-alert",
            WakeProfileCode::InlineQueue,
            |state: &ServerState, transition| {
                transition.touches(ServerFields::QUEUE_DEPTH | ServerFields::HEALTH)
                    && state.queue_depth >= 10_000
                    && state.degraded
            },
        )
        .with_interest_mask(ServerFields::QUEUE_DEPTH | ServerFields::HEALTH)
        .with_target(target_task),
    );

    let state_event = ServerEvent {
        queue_depth: packet.queue_depth,
        degraded: packet.degraded,
    };
    let plain_payload = payload_ref_from_tagged(tagged_payload, packet.payload_bytes);
    let report = runtime.publish_stateful_auto_logged(&mut log, key, plain_payload, state_event);

    println!(
        "publish: hot_delivered={} cold_delivered={} state_changed={} dirty_mask=0x{:04x}",
        report.hot.delivered(),
        report.cold_delivered,
        report.transition.changed,
        report.transition.field_mask
    );

    let hot_bridge = runtime.hot_runtime_mut().drain_inline_protocol_wake_messages();
    print_protocol_messages("hot bridge", &hot_bridge.messages);
    log.append_protocol_bridge(&hot_bridge);

    let cold_bridge = runtime.drain_cold_inline_protocol_wake_messages();
    print_protocol_messages("cold bridge", &cold_bridge.messages);
    log.append_protocol_bridge(&cold_bridge);

    let summary = log.replay_summary()?;
    println!(
        "replay: records={} delivered={} tagged={} cold={} protocol={}",
        summary.records,
        summary.delivered,
        summary.tagged_publishes,
        summary.cold_publishes,
        summary.protocol_bridges
    );

    let jsonl = log.to_jsonl_string()?;
    println!("jsonl first record: {}", jsonl.lines().next().unwrap_or(""));

    Ok(())
}
