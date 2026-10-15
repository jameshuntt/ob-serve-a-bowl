// ob-serve-a-bowl 0.1.0 with waker_protocol 0.2.0: a message written for `Big` is delivered into a
// `Small` task. No `unsafe` from here to the dispatch except the two task impls the scheduler
// requires of every task. Run it under Miri: `cargo +nightly miri run`.
use ob_serve_a_bowl::*;
use waker_protocol::{
    derive_protocol_task_vtable, typed_envelope, MemoryCheckpointStore, ProtocolTask, SchedulerMsg, TaskPolicy,
    TaskPoll, WakerProtocol,
};
use waker_tables::VTableKey;

struct Small(u8);
struct Big {
    items: Vec<u64>,
}

unsafe impl ProtocolTask for Small {
    unsafe fn poll(&mut self) -> TaskPoll { TaskPoll::Yield }
    unsafe fn checkpoint(&mut self) -> Vec<u8> { vec![self.0] }
    unsafe fn restore(bytes: &[u8]) -> Self { Small(bytes[0]) }
}
unsafe impl ProtocolTask for Big {
    unsafe fn poll(&mut self) -> TaskPoll { TaskPoll::Yield }
    unsafe fn checkpoint(&mut self) -> Vec<u8> { Vec::new() }
    unsafe fn restore(_: &[u8]) -> Self { Big { items: Vec::new() } }
}
derive_protocol_task_vtable!(Small, SMALL_VTABLE);

fn main() {
    let key = VTableKey { segment: "repro", kind: "small", version: 1 };
    let mut sched = WakerProtocol::new(MemoryCheckpointStore::default());
    sched.register_vtable(key, &SMALL_VTABLE);
    let small = sched.spawn_box(key, Box::new(Small(1)), TaskPolicy::default()).unwrap();

    // This crate's bridge builds the Deliver message for the observer's target task.
    let event = WakeEvent {
        observer_id: ObserverId::new(0, 0),
        target: Some(TargetTaskId::from(small.id())),
        key: WakeKey64::typed(WakeReasonCode::Inbox, WakeProfileCode::InlineQueue, 0, 0b1_u16, 0, 0).unwrap(),
        payload: PayloadRef::default(),
        reason: WakeReasonCode::Inbox,
        profile: WakeProfileCode::InlineQueue,
        wake_epoch: 1,
    };
    let wrong = typed_envelope::<Big, u64>("push", 7, |task, n| {
        task.items.push(n);
        Ok(())
    });
    let SchedulerMsg::Deliver { id, envelope } = event.to_protocol_deliver_msg(wrong).unwrap() else { unreachable!() };

    // waker_protocol 0.2: `tasks` and `mailbox` are public, and dispatch does not check the type.
    sched.tasks.get_mut(id).unwrap().mailbox.push_back(envelope);
    println!("run_once: {:?}", sched.run_once());
}
