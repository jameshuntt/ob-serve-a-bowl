#![cfg(feature = "tagged-payloads")]

use ob_serve_a_bowl::{
    ObservableEnvelope, ObservableWakeRuntime, ObserverConfig, TaggedPayloadArena, TaggedPayloadError,
    TaggedPayloadRef, TaggedWakeError, WakeKey64, WakeProfileCode, WakeReasonCode,
};
use petr_core::{Key, PortHint, ProphecyBits, SelectorWord};

const FIELD_PAYLOAD: u16 = 1 << 0;
const FIELD_HEALTH: u16 = 1 << 1;
const LOW_BITS: usize = 3;

type PayloadArena = TaggedPayloadArena<TestPayload, LOW_BITS>;
type PayloadRef = TaggedPayloadRef<TestPayload, LOW_BITS>;

#[repr(align(8))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct TestPayload {
    id: u64,
    value: u64,
}

fn selector(function: u8) -> SelectorWord {
    SelectorWord::new(1, 2, function)
}

fn prophecy(skip_hint: bool) -> ProphecyBits {
    ProphecyBits {
        maybe_zero: false,
        maybe_empty: false,
        hot: true,
        skip_hint,
        port_hint: PortHint::Dispatch,
    }
}

fn arena(seed: u64) -> PayloadArena {
    TaggedPayloadArena::with_key(Key::from_seed(seed))
}

fn payload(id: u64) -> TestPayload {
    TestPayload { id, value: id * 10 }
}

fn condition_key(mask: u16, profile: WakeProfileCode) -> WakeKey64 {
    WakeKey64::typed(
        WakeReasonCode::Condition,
        profile,
        4,
        mask,
        7,
        WakeKey64::FLAG_COLD as u16,
    )
    .unwrap()
}

fn insert_payload(arena: &mut PayloadArena, id: u64) -> PayloadRef {
    arena.insert(payload(id), 3, selector(9), prophecy(false)).unwrap()
}

#[test]
fn tagged_payload_arena_resolves_encoded_handles() {
    let mut arena = arena(10);
    let handle = insert_payload(&mut arena, 7);

    assert_eq!(handle.index(), 0);
    assert_eq!(handle.tag(), 3);
    assert_eq!(handle.selector_raw(), selector(9).raw());
    assert!(arena.contains(handle));
    assert_eq!(arena.get(handle).unwrap().id, 7);

    let encoded = handle.encoded();
    let rebuilt = TaggedPayloadRef::from_encoded(encoded);
    assert_eq!(arena.get(rebuilt).unwrap().value, 70);
}

#[test]
fn tagged_payload_remove_rejects_stale_handle_after_slot_reuse() {
    let mut arena = arena(11);
    let first = insert_payload(&mut arena, 1);
    assert_eq!(arena.remove(first).unwrap().id, 1);

    let second = insert_payload(&mut arena, 2);
    assert_eq!(first.index(), second.index());
    assert_ne!(first.generation(), second.generation());
    assert!(arena.get(first).is_err());
    assert_eq!(arena.get(second).unwrap().id, 2);
}

#[test]
fn tagged_payload_rejects_forged_generation_check_selector_and_prophecy() {
    let mut arena = arena(12);
    let handle = insert_payload(&mut arena, 5);

    let mut bad_generation = handle.encoded();
    bad_generation.generation = bad_generation.generation.wrapping_add(1);
    assert!(arena.get(TaggedPayloadRef::from_encoded(bad_generation)).is_err());

    let mut bad_check = handle.encoded();
    bad_check.check ^= 0x10;
    if bad_check.check == 0 {
        bad_check.check = 1;
    }
    assert!(arena.get(TaggedPayloadRef::from_encoded(bad_check)).is_err());

    let mut bad_selector = handle.encoded();
    bad_selector.selector = selector(10).raw();
    assert!(arena.get(TaggedPayloadRef::from_encoded(bad_selector)).is_err());

    let mut bad_prophecy = handle.encoded();
    bad_prophecy.prophecy ^= 0x04;
    assert!(arena.get(TaggedPayloadRef::from_encoded(bad_prophecy)).is_err());
}

#[test]
fn tagged_payload_zero_check_word_is_invalid_before_arena_validation() {
    let mut arena = arena(13);
    let handle = insert_payload(&mut arena, 6);
    let mut encoded = handle.encoded();
    encoded.check = 0;

    let err = arena.get(TaggedPayloadRef::from_encoded(encoded)).unwrap_err();
    assert!(matches!(err, TaggedPayloadError::InvalidEncodedHandle));
}

#[test]
fn tagged_payload_owner_mismatch_is_rejected() {
    let mut owner = arena(14);
    let mut other = arena(15);
    let handle = insert_payload(&mut owner, 9);
    let _other_handle = insert_payload(&mut other, 10);

    assert_eq!(owner.get(handle).unwrap().id, 9);
    assert!(other.get(handle).is_err());
}

#[test]
fn tagged_payload_metadata_updates_invalidate_old_handles() {
    let mut arena = arena(16);
    let handle = insert_payload(&mut arena, 11);

    let changed_selector = arena.set_selector(handle, selector(11)).unwrap();
    assert!(arena.get(handle).is_err());
    assert_eq!(arena.get(changed_selector).unwrap().id, 11);

    let changed_prophecy = arena.set_prophecy(changed_selector, prophecy(true)).unwrap();
    assert!(arena.get(changed_selector).is_err());
    assert_eq!(arena.get(changed_prophecy).unwrap().id, 11);

    let changed_tag = arena.set_tag(changed_prophecy, 4).unwrap();
    assert!(arena.get(changed_prophecy).is_err());
    assert_eq!(arena.get(changed_tag).unwrap().id, 11);
}

#[test]
fn publish_tagged_matches_first_and_resolves_payload_second() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut arena = arena(17);
    let handle = insert_payload(&mut arena, 22);

    let mut forged = handle.encoded();
    forged.generation = forged.generation.wrapping_add(99);
    let forged_handle = TaggedPayloadRef::from_encoded(forged);

    runtime.register(
        ObserverConfig::new("health-only", WakeProfileCode::InlineQueue).with_interest_mask(FIELD_HEALTH),
    );

    let report = runtime.publish_tagged(ObservableEnvelope::new(
        condition_key(FIELD_PAYLOAD, WakeProfileCode::InlineQueue),
        forged_handle,
    ));

    assert_eq!(report.matched, 0);
    assert_eq!(report.delivered(), 0);
    assert!(report.errors.is_empty());
    assert!(arena.get(forged_handle).is_err());
}

#[test]
fn publish_tagged_inline_delivers_typed_event_and_payload_can_resolve() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut arena = arena(18);
    let handle = insert_payload(&mut arena, 30);

    let observer = runtime.register(
        ObserverConfig::new("payload-inline", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_PAYLOAD),
    );

    let report = runtime.publish_tagged(ObservableEnvelope::new(
        condition_key(FIELD_PAYLOAD, WakeProfileCode::InlineQueue),
        handle,
    ));

    assert_eq!(report.matched, 1);
    assert_eq!(report.inline_delivered, 1);
    assert_eq!(report.delivered(), 1);
    assert_eq!(runtime.observer_epoch(observer).unwrap(), 1);
    assert_eq!(runtime.observer_delivery_epoch(observer).unwrap(), 1);
    assert_eq!(runtime.inline_len(), 0, "tagged publish returns typed events instead of pushing PayloadRef queues");

    let event = report.events[0];
    assert_eq!(event.observer_id, observer);
    assert_eq!(event.payload.index(), handle.index());
    assert_eq!(arena.get(event.payload).unwrap().id, 30);
}

#[test]
fn publish_tagged_atomic_condition_and_manual_profiles_are_counted() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut arena = arena(19);
    let handle = insert_payload(&mut arena, 31);

    runtime.register(
        ObserverConfig::new("payload-atomic", WakeProfileCode::AtomicEpoch)
            .with_interest_mask(FIELD_PAYLOAD),
    );
    runtime.register(
        ObserverConfig::new("payload-condition", WakeProfileCode::ConditionCache)
            .with_interest_mask(FIELD_PAYLOAD),
    );
    runtime.register(
        ObserverConfig::new("payload-manual", WakeProfileCode::Manual)
            .with_interest_mask(FIELD_PAYLOAD),
    );

    let report = runtime.publish_tagged(ObservableEnvelope::new(
        condition_key(FIELD_PAYLOAD, WakeProfileCode::Manual),
        handle,
    ));

    assert_eq!(report.matched, 3);
    assert_eq!(report.atomic_delivered, 1);
    assert_eq!(report.condition_delivered, 1);
    assert_eq!(report.manual_delivered, 1);
    assert_eq!(report.delivered(), 3);
    assert_eq!(report.events.len(), 3);
    assert!(report.events.iter().all(|event| arena.get(event.payload).unwrap().id == 31));
}

#[test]
fn publish_tagged_records_last_key_for_matched_observer() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut arena = arena(20);
    let handle = insert_payload(&mut arena, 32);
    let observer = runtime.register(
        ObserverConfig::new("payload-last-key", WakeProfileCode::AtomicEpoch)
            .with_interest_mask(FIELD_PAYLOAD),
    );
    let key = condition_key(FIELD_PAYLOAD, WakeProfileCode::AtomicEpoch);

    let report = runtime.publish_tagged(ObservableEnvelope::new(key, handle));

    assert_eq!(report.atomic_delivered, 1);
    assert_eq!(runtime.observer_last_key(observer).unwrap(), Some(key));
}

#[test]
fn publish_tagged_reports_unknown_reason_without_payload_resolution() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut arena = arena(21);
    let handle = insert_payload(&mut arena, 33);
    runtime.register(
        ObserverConfig::new("payload-bad-reason", WakeProfileCode::Manual)
            .with_interest_mask(FIELD_PAYLOAD),
    );
    let bad_key = WakeKey64::new_checked(200, WakeProfileCode::Manual.as_u64(), 1, u64::from(FIELD_PAYLOAD), 0, 0).unwrap();

    let report = runtime.publish_tagged(ObservableEnvelope::new(bad_key, handle));

    assert_eq!(report.matched, 1);
    assert_eq!(report.delivered(), 0);
    assert!(matches!(report.errors.as_slice(), [TaggedWakeError::Decode(_)]));
    assert_eq!(arena.get(handle).unwrap().id, 33);
}

#[test]
fn publish_tagged_crossbeam_profile_requires_explicit_bridge() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut arena = arena(22);
    let handle = insert_payload(&mut arena, 34);
    runtime.register(
        ObserverConfig::new("payload-crossbeam", WakeProfileCode::CrossbeamQueue)
            .with_interest_mask(FIELD_PAYLOAD),
    );

    let report = runtime.publish_tagged(ObservableEnvelope::new(
        condition_key(FIELD_PAYLOAD, WakeProfileCode::CrossbeamQueue),
        handle,
    ));

    assert_eq!(report.matched, 1);
    assert_eq!(report.delivered(), 0);
    assert_eq!(report.errors, vec![TaggedWakeError::CrossbeamTaggedPayloadUnsupported]);
}

#[test]
fn tagged_payload_ref_can_survive_envelope_copy_without_raw_pointer_exposure() {
    let mut arena = arena(23);
    let handle = insert_payload(&mut arena, 35);
    let envelope = ObservableEnvelope::new(condition_key(FIELD_PAYLOAD, WakeProfileCode::Manual), handle);
    let copied = envelope;

    assert_eq!(copied.payload.index(), handle.index());
    assert_eq!(copied.payload.generation(), handle.generation());
    assert_eq!(copied.payload.selector_raw(), selector(9).raw());
    assert_eq!(arena.get(copied.payload).unwrap().id, 35);
}


#[test]
fn tagged_payload_meta_and_validated_metadata_are_available() {
    let mut arena = arena(24);
    let handle = insert_payload(&mut arena, 36);
    let meta = handle.meta();

    assert_eq!(meta.index, handle.index());
    assert_eq!(meta.generation, handle.generation());
    assert_eq!(meta.selector.raw(), selector(9).raw());
    assert_eq!(meta.prophecy.pack(), prophecy(false).pack());
    assert_eq!(meta.tag, 3);
    assert_eq!(handle.selector().raw(), selector(9).raw());
    assert_eq!(handle.prophecy().pack(), prophecy(false).pack());
    assert_eq!(arena.selector(handle).unwrap().raw(), selector(9).raw());
    assert_eq!(arena.prophecy(handle).unwrap().pack(), prophecy(false).pack());
    assert!(arena.validate(handle).is_ok());
}

#[test]
fn tagged_wake_event_and_report_resolve_helpers_work() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut arena = arena(25);
    let handle = insert_payload(&mut arena, 37);

    runtime.register(
        ObserverConfig::new("payload-inline", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_PAYLOAD),
    );

    let report = runtime.publish_tagged(ObservableEnvelope::new(
        condition_key(FIELD_PAYLOAD, WakeProfileCode::InlineQueue),
        handle,
    ));

    assert!(!report.is_empty());
    assert!(!report.has_errors());
    let first = report.first_event().unwrap();
    assert_eq!(first.resolve(&arena).unwrap().id, 37);

    let resolved = report.resolve_events(&arena).unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].event.observer_id, first.observer_id);
    assert_eq!(resolved[0].payload.value, 370);
}

#[test]
fn tagged_payload_live_handles_and_clear_invalidate_previous_refs() {
    let mut arena = arena(26);
    arena.reserve_slots(4);
    let first = insert_payload(&mut arena, 38);
    let second = insert_payload(&mut arena, 39);

    let handles = arena.live_handles();
    assert_eq!(handles.len(), 2);
    assert!(handles.iter().any(|handle| handle.index() == first.index()));
    assert!(handles.iter().any(|handle| handle.index() == second.index()));
    assert_eq!(arena.slot_capacity(), 2);
    assert_eq!(arena.free_len(), 0);

    arena.clear();
    assert!(arena.is_empty());
    assert!(arena.get(first).is_err());
    assert!(arena.get(second).is_err());
}

#[test]
fn tagged_report_resolve_events_rejects_stale_payload_after_publish() {
    let mut runtime = ObservableWakeRuntime::new();
    let mut arena = arena(27);
    let handle = insert_payload(&mut arena, 40);

    runtime.register(
        ObserverConfig::new("payload-inline", WakeProfileCode::InlineQueue)
            .with_interest_mask(FIELD_PAYLOAD),
    );

    let report = runtime.publish_tagged(ObservableEnvelope::new(
        condition_key(FIELD_PAYLOAD, WakeProfileCode::InlineQueue),
        handle,
    ));
    assert_eq!(report.delivered(), 1);

    assert_eq!(arena.remove(handle).unwrap().id, 40);
    let replacement = insert_payload(&mut arena, 41);
    assert_eq!(replacement.index(), handle.index());
    assert!(report.resolve_events(&arena).is_err());
}
