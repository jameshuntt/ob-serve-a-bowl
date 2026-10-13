use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ob_serve_a_bowl::{
    ColdObservableRuntime, ColdObserverConfig, ColdRearmPolicy, DispatchPolicy, ObservableEventLog,
    ObservablePacket, ObservableState, ObservableWakeRuntime, ObserverConfig, PayloadRef,
    StateTransition, WakeKey64, WakeProfileCode, WakeReasonCode,
};
use std::sync::{Arc, Mutex};

const FIELD_QUEUE: u16 = 1 << 0;
const FIELD_HEALTH: u16 = 1 << 1;
#[cfg(feature = "tagged-payloads")]
const FIELD_PAYLOAD: u16 = 1 << 2;
const PHASE_RUNTIME: u8 = 7;

fn wake_key(field_mask: u16, profile: WakeProfileCode) -> WakeKey64 {
    WakeKey64::typed(
        WakeReasonCode::Condition,
        profile,
        4,
        field_mask,
        PHASE_RUNTIME,
        (WakeKey64::FLAG_HOT | WakeKey64::FLAG_COLD) as u16,
    )
    .expect("benchmark wake key should fit")
}

fn packet(field_mask: u16, profile: WakeProfileCode) -> ObservablePacket {
    ObservablePacket::new(
        wake_key(field_mask, profile),
        PayloadRef::new(7, 1, 128, 64),
    )
}

fn build_hot_runtime(
    observer_count: usize,
    matching_stride: usize,
    profile: WakeProfileCode,
) -> ObservableWakeRuntime {
    let mut runtime = ObservableWakeRuntime::new();

    for idx in 0..observer_count {
        let interest = if matching_stride == 0 || idx % matching_stride == 0 {
            FIELD_QUEUE
        } else {
            FIELD_HEALTH
        };

        runtime.register(
            ObserverConfig::new("bench-hot", profile).with_interest_mask(interest),
        );
    }

    runtime
}

fn build_hot_runtime_with_policy(
    observer_count: usize,
    matching_stride: usize,
    profile: WakeProfileCode,
    policy: DispatchPolicy,
) -> ObservableWakeRuntime {
    let mut runtime = ObservableWakeRuntime::with_config(ob_serve_a_bowl::ObservableRuntimeConfig {
        dispatch_policy: policy,
        ..ob_serve_a_bowl::ObservableRuntimeConfig::default()
    });

    for idx in 0..observer_count {
        let interest = if matching_stride == 0 || idx % matching_stride == 0 {
            FIELD_QUEUE
        } else {
            FIELD_HEALTH
        };

        runtime.register(
            ObserverConfig::new("bench-hot", profile).with_interest_mask(interest),
        );
    }

    runtime
}

#[derive(Clone, Copy)]
struct BaselineObserver {
    interest_mask: u16,
    active: bool,
}

fn build_baseline_observers(observer_count: usize, matching_stride: usize) -> Arc<Mutex<Vec<BaselineObserver>>> {
    let observers = (0..observer_count)
        .map(|idx| BaselineObserver {
            interest_mask: if matching_stride == 0 || idx % matching_stride == 0 {
                FIELD_QUEUE
            } else {
                FIELD_HEALTH
            },
            active: true,
        })
        .collect();

    Arc::new(Mutex::new(observers))
}

fn baseline_arc_mutex_publish(observers: &Mutex<Vec<BaselineObserver>>, key: WakeKey64) -> usize {
    let field_mask = key.field_mask() as u16;
    let guard = observers.lock().expect("benchmark mutex should not be poisoned");

    guard
        .iter()
        .filter(|observer| {
            observer.active
                && (observer.interest_mask == 0 || (observer.interest_mask & field_mask) != 0)
        })
        .count()
}

#[derive(Debug, Clone, Default)]
struct BenchState {
    queue_depth: u64,
}

#[derive(Debug, Clone, Copy)]
struct BenchEvent {
    queue_depth: u64,
}

impl ObservableState for BenchState {
    type Event = BenchEvent;

    fn apply(&mut self, event: &Self::Event) -> StateTransition {
        self.queue_depth = event.queue_depth;
        StateTransition::changed(FIELD_QUEUE)
    }
}

fn build_cold_runtime(observer_count: usize, profile: WakeProfileCode) -> ColdObservableRuntime<BenchState> {
    let mut runtime = ColdObservableRuntime::new(BenchState::default());

    for idx in 0..observer_count {
        let threshold = 1_000 + idx as u64;
        runtime.register_cold(
            ColdObserverConfig::new("bench-cold", profile, move |state: &BenchState, transition| {
                transition.touches(FIELD_QUEUE) && state.queue_depth >= threshold
            })
            .with_interest_mask(FIELD_QUEUE)
            .with_rearm_policy(ColdRearmPolicy::EveryMatch),
        );
    }

    runtime
}

fn build_cold_runtime_with_policy(
    observer_count: usize,
    matching_stride: usize,
    profile: WakeProfileCode,
    policy: DispatchPolicy,
) -> ColdObservableRuntime<BenchState> {
    let hot = ObservableWakeRuntime::with_config(ob_serve_a_bowl::ObservableRuntimeConfig {
        dispatch_policy: policy,
        ..ob_serve_a_bowl::ObservableRuntimeConfig::default()
    });
    let mut runtime = ColdObservableRuntime::with_hot_runtime(BenchState::default(), hot);

    for idx in 0..observer_count {
        let threshold = 1_000 + idx as u64;
        let interest = if matching_stride == 0 || idx % matching_stride == 0 {
            FIELD_QUEUE
        } else {
            FIELD_HEALTH
        };

        runtime.register_cold(
            ColdObserverConfig::new("bench-cold-auto", profile, move |state: &BenchState, transition| {
                transition.touches(FIELD_QUEUE) && state.queue_depth >= threshold
            })
            .with_interest_mask(interest)
            .with_rearm_policy(ColdRearmPolicy::EveryMatch),
        );
    }

    runtime
}

fn seed_log(record_count: usize) -> ObservableEventLog {
    let mut runtime = build_hot_runtime(32, 2, WakeProfileCode::AtomicEpoch);
    let mut log = ObservableEventLog::new();
    let packet = packet(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);

    for _ in 0..record_count {
        let report = runtime.publish_logged(&mut log, packet);
        black_box(report.delivered());
    }

    log
}

fn bench_hot_routing_scan_with_stride(c: &mut Criterion, group_name: &str, matching_stride: usize) {
    let mut group = c.benchmark_group(group_name);

    for observer_count in [8_usize, 64, 512, 2_048] {
        group.throughput(Throughput::Elements(observer_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(observer_count),
            &observer_count,
            |bench, &count| {
                let mut runtime = build_hot_runtime(count, matching_stride, WakeProfileCode::AtomicEpoch);
                let packet = packet(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);

                bench.iter(|| {
                    let report = runtime.publish(black_box(packet));
                    black_box(report.delivered());
                    black_box(report.scanned);
                    black_box(report.matched);
                });
            },
        );
    }

    group.finish();
}

fn bench_hot_routing(c: &mut Criterion) {
    bench_hot_routing_scan_with_stride(c, "hot_bitpacked_publish_atomic", 2);
}

fn bench_hot_routing_scan_dense(c: &mut Criterion) {
    bench_hot_routing_scan_with_stride(c, "hot_bitpacked_publish_scan_dense", 0);
}

fn bench_hot_routing_scan_sparse(c: &mut Criterion) {
    bench_hot_routing_scan_with_stride(c, "hot_bitpacked_publish_scan_sparse", 4);
}

fn bench_hot_routing_indexed(c: &mut Criterion) {
    let mut group = c.benchmark_group("hot_bitpacked_publish_indexed_atomic");

    for observer_count in [8_usize, 64, 512, 2_048] {
        group.throughput(Throughput::Elements(observer_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(observer_count),
            &observer_count,
            |bench, &count| {
                let mut runtime = build_hot_runtime(count, 2, WakeProfileCode::AtomicEpoch);
                let packet = packet(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);

                bench.iter(|| {
                    let report = runtime.publish_indexed(black_box(packet));
                    black_box(report.delivered());
                    black_box(report.scanned);
                    black_box(report.matched);
                });
            },
        );
    }

    group.finish();
}

fn bench_hot_routing_auto_with_stride(c: &mut Criterion, group_name: &str, matching_stride: usize) {
    let mut group = c.benchmark_group(group_name);

    for observer_count in [8_usize, 64, 512, 2_048] {
        group.throughput(Throughput::Elements(observer_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(observer_count),
            &observer_count,
            |bench, &count| {
                let mut runtime = build_hot_runtime_with_policy(
                    count,
                    matching_stride,
                    WakeProfileCode::AtomicEpoch,
                    DispatchPolicy::default(),
                );
                let packet = packet(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);

                bench.iter(|| {
                    let report = runtime.publish_auto(black_box(packet));
                    black_box(report.delivered());
                    black_box(report.scanned);
                    black_box(report.matched);
                });
            },
        );
    }

    group.finish();
}

fn bench_hot_routing_auto_dense(c: &mut Criterion) {
    bench_hot_routing_auto_with_stride(c, "hot_bitpacked_publish_auto_dense", 0);
}

fn bench_hot_routing_auto_sparse(c: &mut Criterion) {
    bench_hot_routing_auto_with_stride(c, "hot_bitpacked_publish_auto_sparse", 4);
}

fn bench_baseline_arc_mutex(c: &mut Criterion) {
    let mut group = c.benchmark_group("baseline_arc_mutex_vec_scan");

    for observer_count in [8_usize, 64, 512, 2_048] {
        group.throughput(Throughput::Elements(observer_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(observer_count),
            &observer_count,
            |bench, &count| {
                let observers = build_baseline_observers(count, 2);
                let key = wake_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);

                bench.iter(|| {
                    let matched = baseline_arc_mutex_publish(black_box(&observers), black_box(key));
                    black_box(matched);
                });
            },
        );
    }

    group.finish();
}

fn bench_cold_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("cold_state_predicates");

    for observer_count in [8_usize, 64, 512] {
        group.throughput(Throughput::Elements(observer_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(observer_count),
            &observer_count,
            |bench, &count| {
                let mut runtime = build_cold_runtime(count, WakeProfileCode::AtomicEpoch);
                let key = wake_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);
                let payload = PayloadRef::new(3, 0, 0, 8);
                let event = BenchEvent { queue_depth: 10_000 };

                bench.iter(|| {
                    let report = runtime.publish_stateful(black_box(key), black_box(payload), black_box(event));
                    black_box(report.total_delivered());
                    black_box(report.cold_predicate_true);
                });
            },
        );
    }

    group.finish();
}

fn bench_cold_state_indexed(c: &mut Criterion) {
    let mut group = c.benchmark_group("cold_state_predicates_indexed");

    for observer_count in [8_usize, 64, 512] {
        group.throughput(Throughput::Elements(observer_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(observer_count),
            &observer_count,
            |bench, &count| {
                let mut runtime = build_cold_runtime(count, WakeProfileCode::AtomicEpoch);
                let key = wake_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);
                let payload = PayloadRef::new(3, 0, 0, 8);
                let event = BenchEvent { queue_depth: 10_000 };

                bench.iter(|| {
                    let report = runtime.publish_stateful_indexed(
                        black_box(key),
                        black_box(payload),
                        black_box(event),
                    );
                    black_box(report.total_delivered());
                    black_box(report.cold_scanned);
                    black_box(report.cold_predicate_true);
                });
            },
        );
    }

    group.finish();
}

fn bench_cold_state_auto_with_stride(c: &mut Criterion, group_name: &str, matching_stride: usize) {
    let mut group = c.benchmark_group(group_name);

    for observer_count in [8_usize, 64, 512] {
        group.throughput(Throughput::Elements(observer_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(observer_count),
            &observer_count,
            |bench, &count| {
                let mut runtime = build_cold_runtime_with_policy(
                    count,
                    matching_stride,
                    WakeProfileCode::AtomicEpoch,
                    DispatchPolicy::default(),
                );
                let key = wake_key(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);
                let payload = PayloadRef::new(3, 0, 0, 8);
                let event = BenchEvent { queue_depth: 10_000 };

                bench.iter(|| {
                    let report = runtime.publish_stateful_auto(
                        black_box(key),
                        black_box(payload),
                        black_box(event),
                    );
                    black_box(report.total_delivered());
                    black_box(report.cold_scanned);
                    black_box(report.cold_predicate_true);
                });
            },
        );
    }

    group.finish();
}

fn bench_cold_state_auto_dense(c: &mut Criterion) {
    bench_cold_state_auto_with_stride(c, "cold_state_predicates_auto_dense", 0);
}

fn bench_cold_state_auto_sparse(c: &mut Criterion) {
    bench_cold_state_auto_with_stride(c, "cold_state_predicates_auto_sparse", 4);
}

fn bench_event_log(c: &mut Criterion) {
    let mut append_group = c.benchmark_group("event_log_append");

    append_group.bench_function("publish_logged_hot_32_observers", |bench| {
        let mut runtime = build_hot_runtime(32, 2, WakeProfileCode::AtomicEpoch);
        let mut log = ObservableEventLog::new();
        let packet = packet(FIELD_QUEUE, WakeProfileCode::AtomicEpoch);

        bench.iter(|| {
            let report = runtime.publish_logged(&mut log, black_box(packet));
            if log.len() > 4_096 {
                log.clear();
            }
            black_box(report.delivered());
            black_box(log.len());
        });
    });

    append_group.finish();

    let mut replay_group = c.benchmark_group("event_log_replay_summary");

    for record_count in [64_usize, 1_024, 16_384] {
        replay_group.throughput(Throughput::Elements(record_count as u64));
        replay_group.bench_with_input(
            BenchmarkId::from_parameter(record_count),
            &record_count,
            |bench, &count| {
                let log = seed_log(count);

                bench.iter(|| {
                    let summary = log.replay_summary().expect("benchmark replay should be monotonic");
                    black_box(summary.delivered);
                    black_box(summary.hot_publishes);
                });
            },
        );
    }

    replay_group.finish();
}

fn bench_serialization(c: &mut Criterion) {
    #[cfg(feature = "serialization")]
    {
        let log = seed_log(1_024);
        let json = log.to_json_string().expect("benchmark JSON export should work");
        let jsonl = log.to_jsonl_string().expect("benchmark JSONL export should work");

        let mut group = c.benchmark_group("event_log_serialization");

        group.bench_function("to_json_snapshot_1024", |bench| {
            bench.iter(|| {
                let output = log.to_json_string().expect("benchmark JSON export should work");
                black_box(output.len());
            });
        });

        group.bench_function("from_json_snapshot_1024", |bench| {
            bench.iter(|| {
                let imported = ObservableEventLog::from_json_str(black_box(&json))
                    .expect("benchmark JSON import should work");
                black_box(imported.len());
            });
        });

        group.bench_function("to_jsonl_1024", |bench| {
            bench.iter(|| {
                let output = log.to_jsonl_string().expect("benchmark JSONL export should work");
                black_box(output.len());
            });
        });

        group.bench_function("from_jsonl_1024", |bench| {
            bench.iter(|| {
                let imported = ObservableEventLog::from_jsonl_str(black_box(&jsonl))
                    .expect("benchmark JSONL import should work");
                black_box(imported.len());
            });
        });

        group.finish();
    }

    #[cfg(not(feature = "serialization"))]
    {
        let _ = c;
    }
}

fn bench_tagged_payloads(c: &mut Criterion) {
    #[cfg(feature = "tagged-payloads")]
    {
        use ob_serve_a_bowl::{ObservableEnvelope, TaggedPayloadArena};
        use petr_core::{Key, PortHint, ProphecyBits, SelectorWord};

        #[repr(align(8))]
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct BenchPayload {
            id: u64,
            value: u64,
        }

        const LOW_BITS: usize = 3;

        fn selector(function: u8) -> SelectorWord {
            SelectorWord::new(1, 2, function)
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

        let mut group = c.benchmark_group("tagged_payload_publish");

        for observer_count in [8_usize, 64, 512] {
            group.throughput(Throughput::Elements(observer_count as u64));
            group.bench_with_input(
                BenchmarkId::from_parameter(observer_count),
                &observer_count,
                |bench, &count| {
                    let mut runtime = build_hot_runtime(count, 2, WakeProfileCode::AtomicEpoch);
                    let mut arena = TaggedPayloadArena::<BenchPayload, LOW_BITS>::with_key(Key::from_seed(77));
                    arena.reserve_slots(4);
                    let payload = arena
                        .insert(
                            BenchPayload { id: 1, value: 10 },
                            3,
                            selector(9),
                            prophecy(),
                        )
                        .expect("benchmark insert should work");
                    let key = wake_key(FIELD_QUEUE | FIELD_PAYLOAD, WakeProfileCode::AtomicEpoch);

                    bench.iter(|| {
                        let envelope = ObservableEnvelope { key, payload };
                        let report = runtime.publish_tagged(black_box(envelope));
                        black_box(report.delivered());
                        black_box(arena.get(payload).expect("benchmark payload should resolve").value);
                    });
                },
            );
        }

        group.finish();
    }

    #[cfg(not(feature = "tagged-payloads"))]
    {
        let _ = c;
    }
}

criterion_group!(
    benches,
    bench_hot_routing,
    bench_hot_routing_scan_dense,
    bench_hot_routing_scan_sparse,
    bench_hot_routing_indexed,
    bench_hot_routing_auto_dense,
    bench_hot_routing_auto_sparse,
    bench_baseline_arc_mutex,
    bench_cold_state,
    bench_cold_state_indexed,
    bench_cold_state_auto_dense,
    bench_cold_state_auto_sparse,
    bench_event_log,
    bench_serialization,
    bench_tagged_payloads
);
criterion_main!(benches);
