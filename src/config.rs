//! Optional configuration/builder helpers for the observable wake runtime.

use crate::{ObservableWakeRuntime, WakeProfileCode};


/// Policy for choosing between the compatibility scan path and the bitpack-indexed path.
///
/// The scan path is usually best for tiny observer sets or dense matches. The
/// indexed path is usually best once the bitpacked `WakeKey64.field_mask` can
/// eliminate a meaningful percentage of observers before full matcher/predicate
/// evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchPolicy {
    /// Always prefer hot scan while the live hot observer count is at or below this value.
    pub hot_small_observer_threshold: usize,
    /// Always prefer cold scan while the live cold observer count is at or below this value.
    pub cold_small_observer_threshold: usize,
    /// Minimum required hot candidate reduction before the indexed path is selected.
    ///
    /// Example: `25` means hot indexed dispatch is selected only when the
    /// field-mask index removes at least 25% of live hot observers.
    pub hot_indexed_min_reduction_percent: u8,
    /// Minimum required cold candidate reduction before the indexed path is selected.
    ///
    /// Cold predicates are generally heavier and branchier than hot wake routing,
    /// but the indexed cold path also has more policy/index overhead. Requiring a
    /// stronger reduction keeps dense cold sets on the cheaper scan path.
    pub cold_indexed_min_reduction_percent: u8,
}

impl DispatchPolicy {
    pub const DEFAULT_HOT_SMALL_OBSERVER_THRESHOLD: usize = 64;
    pub const DEFAULT_COLD_SMALL_OBSERVER_THRESHOLD: usize = 32;
    pub const DEFAULT_HOT_INDEXED_MIN_REDUCTION_PERCENT: u8 = 25;
    pub const DEFAULT_COLD_INDEXED_MIN_REDUCTION_PERCENT: u8 = 60;

    /// Compatibility constructor: applies the same threshold/reduction to both
    /// hot and cold paths.
    #[must_use]
    pub const fn new(
        small_observer_threshold: usize,
        indexed_min_reduction_percent: u8,
    ) -> Self {
        Self {
            hot_small_observer_threshold: small_observer_threshold,
            cold_small_observer_threshold: small_observer_threshold,
            hot_indexed_min_reduction_percent: indexed_min_reduction_percent,
            cold_indexed_min_reduction_percent: indexed_min_reduction_percent,
        }
    }

    /// Explicit constructor for separately tuning hot routing and cold predicates.
    #[must_use]
    pub const fn hot_cold(
        hot_small_observer_threshold: usize,
        hot_indexed_min_reduction_percent: u8,
        cold_small_observer_threshold: usize,
        cold_indexed_min_reduction_percent: u8,
    ) -> Self {
        Self {
            hot_small_observer_threshold,
            cold_small_observer_threshold,
            hot_indexed_min_reduction_percent,
            cold_indexed_min_reduction_percent,
        }
    }

    #[must_use]
    pub const fn always_scan() -> Self {
        Self {
            hot_small_observer_threshold: usize::MAX,
            cold_small_observer_threshold: usize::MAX,
            hot_indexed_min_reduction_percent: 100,
            cold_indexed_min_reduction_percent: 100,
        }
    }

    #[must_use]
    pub const fn prefer_indexed() -> Self {
        Self {
            hot_small_observer_threshold: 0,
            cold_small_observer_threshold: 0,
            hot_indexed_min_reduction_percent: 0,
            cold_indexed_min_reduction_percent: 0,
        }
    }

    #[must_use]
    pub fn can_consider_hot_indexed(self, live_observers: usize) -> bool {
        live_observers > self.hot_small_observer_threshold
    }

    #[must_use]
    pub fn can_consider_cold_indexed(self, live_observers: usize) -> bool {
        live_observers > self.cold_small_observer_threshold
    }

    /// Compatibility helper using the hot policy. Prefer
    /// [`DispatchPolicy::should_use_hot_indexed`] or
    /// [`DispatchPolicy::should_use_cold_indexed`] for new code.
    #[must_use]
    pub fn should_use_indexed(self, live_observers: usize, candidates: usize) -> bool {
        self.should_use_hot_indexed(live_observers, candidates)
    }

    #[must_use]
    pub fn should_use_hot_indexed(self, live_observers: usize, candidates: usize) -> bool {
        Self::should_use_indexed_with(
            live_observers,
            candidates,
            self.hot_small_observer_threshold,
            self.hot_indexed_min_reduction_percent,
        )
    }

    #[must_use]
    pub fn should_use_cold_indexed(self, live_observers: usize, candidates: usize) -> bool {
        Self::should_use_indexed_with(
            live_observers,
            candidates,
            self.cold_small_observer_threshold,
            self.cold_indexed_min_reduction_percent,
        )
    }

    fn should_use_indexed_with(
        live_observers: usize,
        candidates: usize,
        small_observer_threshold: usize,
        indexed_min_reduction_percent: u8,
    ) -> bool {
        if live_observers == 0 {
            return false;
        }

        if candidates == 0 {
            return true;
        }

        if live_observers <= small_observer_threshold {
            return false;
        }

        if candidates >= live_observers {
            return false;
        }

        let removed = live_observers - candidates;
        let required = usize::from(indexed_min_reduction_percent.min(100));
        removed.saturating_mul(100) >= live_observers.saturating_mul(required)
    }
}

impl Default for DispatchPolicy {
    fn default() -> Self {
        Self {
            hot_small_observer_threshold: Self::DEFAULT_HOT_SMALL_OBSERVER_THRESHOLD,
            cold_small_observer_threshold: Self::DEFAULT_COLD_SMALL_OBSERVER_THRESHOLD,
            hot_indexed_min_reduction_percent: Self::DEFAULT_HOT_INDEXED_MIN_REDUCTION_PERCENT,
            cold_indexed_min_reduction_percent: Self::DEFAULT_COLD_INDEXED_MIN_REDUCTION_PERCENT,
        }
    }
}

/// Runtime configuration for profile selection defaults.
///
/// Per-observer configuration still wins. This exists so higher-level builders
/// can carry policy without hard-coding a backend into the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservableRuntimeConfig {
    pub default_profile: WakeProfileCode,
    pub default_qos: u8,
    pub strict_packet_profile_hint: bool,
    pub dispatch_policy: DispatchPolicy,
}

impl Default for ObservableRuntimeConfig {
    fn default() -> Self {
        Self {
            default_profile: WakeProfileCode::AtomicEpoch,
            default_qos: 3,
            strict_packet_profile_hint: false,
            dispatch_policy: DispatchPolicy::default(),
        }
    }
}

/// Builder for [`ObservableWakeRuntime`].
pub struct ObservableWakeRuntimeBuilder {
    config: ObservableRuntimeConfig,
    #[cfg(feature = "crossbeam-wake")]
    crossbeam_sender: Option<crossbeam_channel::Sender<crate::WakeEvent>>,
}

impl Default for ObservableWakeRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ObservableWakeRuntimeBuilder {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            config: ObservableRuntimeConfig {
                default_profile: WakeProfileCode::AtomicEpoch,
                default_qos: 3,
                strict_packet_profile_hint: false,
                dispatch_policy: DispatchPolicy {
                    hot_small_observer_threshold: DispatchPolicy::DEFAULT_HOT_SMALL_OBSERVER_THRESHOLD,
                    cold_small_observer_threshold: DispatchPolicy::DEFAULT_COLD_SMALL_OBSERVER_THRESHOLD,
                    hot_indexed_min_reduction_percent: DispatchPolicy::DEFAULT_HOT_INDEXED_MIN_REDUCTION_PERCENT,
                    cold_indexed_min_reduction_percent: DispatchPolicy::DEFAULT_COLD_INDEXED_MIN_REDUCTION_PERCENT,
                },
            },
            #[cfg(feature = "crossbeam-wake")]
            crossbeam_sender: None,
        }
    }

    #[must_use]
    pub const fn with_default_profile(mut self, profile: WakeProfileCode) -> Self {
        self.config.default_profile = profile;
        self
    }

    #[must_use]
    pub const fn with_default_qos(mut self, qos: u8) -> Self {
        self.config.default_qos = qos;
        self
    }

    #[must_use]
    pub const fn strict_packet_profile_hint(mut self, enabled: bool) -> Self {
        self.config.strict_packet_profile_hint = enabled;
        self
    }

    #[must_use]
    pub const fn with_dispatch_policy(mut self, policy: DispatchPolicy) -> Self {
        self.config.dispatch_policy = policy;
        self
    }

    #[cfg(feature = "crossbeam-wake")]
    #[must_use]
    pub fn with_crossbeam_sender(mut self, sender: crossbeam_channel::Sender<crate::WakeEvent>) -> Self {
        self.crossbeam_sender = Some(sender);
        self
    }

    #[must_use]
    pub fn build(self) -> ObservableWakeRuntime {
        #[cfg(feature = "crossbeam-wake")]
        {
            let mut runtime = ObservableWakeRuntime::with_config(self.config);
            if let Some(sender) = self.crossbeam_sender {
                runtime.set_crossbeam_sender(sender);
            }
            runtime
        }

        #[cfg(not(feature = "crossbeam-wake"))]
        {
            ObservableWakeRuntime::with_config(self.config)
        }
    }
}
