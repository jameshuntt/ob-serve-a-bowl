use ob_serve_a_bowl::*;

const FIELD_QUEUE: u16 = 1 << 0;

#[derive(Default)]
struct QueueState {
    depth: u32,
}

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

fn main() {
    let key = WakeKey64::typed(
        WakeReasonCode::Condition,
        WakeProfileCode::InlineQueue,
        4,
        FIELD_QUEUE,
        1,
        WakeKey64::FLAG_COLD as u16,
    )
    .unwrap();

    let mut runtime = ColdObservableRuntime::new(QueueState::default());
    let mut log = ObservableEventLog::new();

    runtime.register_cold(
        ColdObserverConfig::new("queue>=10", WakeProfileCode::InlineQueue, |state: &QueueState, transition| {
            transition.touches(FIELD_QUEUE) && state.depth >= 10
        })
        .with_interest_mask(FIELD_QUEUE),
    );

    let report = runtime.publish_stateful_logged(
        &mut log,
        key,
        PayloadRef::default(),
        QueueEvent::SetDepth(10),
    );
    let events = runtime.drain_cold_inline_logged(&mut log);
    let replay = log.replay_summary().unwrap();

    println!("cold delivered: {}", report.cold_delivered());
    println!("drained events: {}", events.len());
    println!("log records: {}", replay.records);
    println!("state changes: {}", replay.state_changes);
    println!("delivered total: {}", replay.delivered);

    #[cfg(feature = "serialization")]
    {
        let jsonl = log.to_jsonl_string().unwrap();
        let restored = ObservableEventLog::from_jsonl_str(&jsonl).unwrap();
        println!("jsonl bytes: {}", jsonl.len());
        println!("restored records: {}", restored.len());
    }
}
