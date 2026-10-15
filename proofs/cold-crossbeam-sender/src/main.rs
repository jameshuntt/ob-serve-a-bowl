// A hot and a cold `CrossbeamQueue` observer on one `ColdObservableRuntime` whose hot runtime has a
// working sender, on 0.1.0 and on 0.2.2. One stateful publish wakes both. Prints what each one did
// and how many events reached the channel.

const FIELD: u16 = 1 << 1;

macro_rules! hot_and_cold {
    ($version:literal, $obs:ident) => {{
        use $obs::*;
        struct Changed;
        impl ObservableState for Changed {
            type Event = ();
            fn apply(&mut self, _: &()) -> StateTransition {
                StateTransition::changed(FIELD)
            }
        }

        let (tx, rx) = crossbeam_channel::unbounded();
        let mut runtime = ColdObservableRuntime::with_hot_runtime(Changed, ObservableWakeRuntime::with_crossbeam_sender(tx));
        let hot = runtime
            .hot_runtime_mut()
            .register(ObserverConfig::new("hot", WakeProfileCode::CrossbeamQueue).with_interest_mask(FIELD));
        let cold = runtime.register_cold(
            ColdObserverConfig::new("cold", WakeProfileCode::CrossbeamQueue, |_: &Changed, _: &StateTransition| true)
                .with_interest_mask(FIELD),
        );
        let key = WakeKey64::typed(WakeReasonCode::Condition, WakeProfileCode::CrossbeamQueue, 4, FIELD, 2, WakeKey64::FLAG_COLD as u16).unwrap();
        let report = runtime.publish_stateful(key, PayloadRef::default(), ());
        println!(
            "{} hot:  crossbeam_delivered {} delivery {}",
            $version,
            report.hot.crossbeam_delivered,
            runtime.hot_runtime().observer_delivery_epoch(hot).unwrap()
        );
        println!(
            "{} cold: cold_delivered {} errors {:?} delivery {}",
            $version,
            report.cold_delivered,
            report.errors,
            runtime.cold_observer_delivery_epoch(cold).unwrap()
        );
        println!("{} events in the channel: {}", $version, rx.try_iter().count());
    }};
}

fn main() {
    hot_and_cold!("0.1.0", ob_serve_a_bowl_0_1_0);
    hot_and_cold!("0.2.2", ob_serve_a_bowl_0_2_2);
}
