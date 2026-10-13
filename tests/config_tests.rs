use ob_serve_a_bowl::{
    DispatchPolicy, ObservableRuntimeConfig, ObservableWakeRuntime, ObservableWakeRuntimeBuilder, WakeProfileCode,
};

#[test]
fn default_runtime_config_uses_atomic_epoch_profile() {
    let cfg = ObservableRuntimeConfig::default();
    assert_eq!(cfg.default_profile, WakeProfileCode::AtomicEpoch);
    assert_eq!(cfg.default_qos, 3);
    assert!(!cfg.strict_packet_profile_hint);
    assert_eq!(cfg.dispatch_policy, DispatchPolicy::default());
}

#[test]
fn runtime_can_be_constructed_with_explicit_config() {
    let cfg = ObservableRuntimeConfig {
        default_profile: WakeProfileCode::InlineQueue,
        default_qos: 5,
        strict_packet_profile_hint: true,
        dispatch_policy: DispatchPolicy::new(16, 50),
    };
    let runtime = ObservableWakeRuntime::with_config(cfg);
    assert_eq!(runtime.config(), cfg);
}

#[test]
fn builder_preserves_config_values() {
    let runtime = ObservableWakeRuntimeBuilder::new()
        .with_default_profile(WakeProfileCode::ConditionCache)
        .with_default_qos(4)
        .strict_packet_profile_hint(true)
        .with_dispatch_policy(DispatchPolicy::prefer_indexed())
        .build();

    assert_eq!(runtime.config().default_profile, WakeProfileCode::ConditionCache);
    assert_eq!(runtime.config().default_qos, 4);
    assert!(runtime.config().strict_packet_profile_hint);
    assert_eq!(runtime.config().dispatch_policy, DispatchPolicy::prefer_indexed());
}

#[test]
fn dispatch_policy_can_tune_hot_and_cold_independently() {
    let policy = DispatchPolicy::hot_cold(64, 25, 32, 60);

    assert!(!policy.can_consider_hot_indexed(64));
    assert!(policy.can_consider_hot_indexed(65));
    assert!(!policy.can_consider_cold_indexed(32));
    assert!(policy.can_consider_cold_indexed(33));

    assert!(policy.should_use_hot_indexed(128, 80));
    assert!(!policy.should_use_cold_indexed(128, 80));
    assert!(policy.should_use_cold_indexed(128, 32));
}
