use ob_serve_a_bowl::*;

const FIELD_QUEUE: u16 = 1 << 1;

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
            QueueEvent::SetDepth(depth) if self.depth != depth => {
                self.depth = depth;
                StateTransition::changed(FIELD_QUEUE)
            }
            QueueEvent::SetDepth(_) => StateTransition::unchanged(),
        }
    }
}

fn main() {
    let mut runtime = ColdObservableRuntime::new(QueueState::default());

    runtime.register_cold(
        ColdObserverConfig::new("queue-threshold", WakeProfileCode::InlineQueue, |state: &QueueState, transition| {
            transition.touches(FIELD_QUEUE) && state.depth >= 10_000
        })
        .with_interest_mask(FIELD_QUEUE)
        .with_matchers(cold_condition_matchers(1, u128::from(FIELD_QUEUE))),
    );

    let key = WakeKey64::typed(
        WakeReasonCode::Condition,
        WakeProfileCode::InlineQueue,
        4,
        FIELD_QUEUE,
        1,
        WakeKey64::FLAG_COLD as u16,
    )
    .unwrap();

    let report = runtime.publish_stateful(key, PayloadRef::default(), QueueEvent::SetDepth(10_000));
    assert_eq!(report.cold_inline_delivered, 1);
    assert_eq!(runtime.drain_cold_inline().len(), 1);

}
