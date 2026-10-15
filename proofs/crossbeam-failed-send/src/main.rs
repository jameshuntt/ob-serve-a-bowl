// One wake to a `CrossbeamQueue` observer whose send fails, on 0.1.0 and on 0.2.2. Prints the
// errors `publish` reports and the observer's wake and delivery counts after it.

macro_rules! failed_sends {
    ($version:literal, $obs:ident) => {{
        use $obs::*;
        let key = WakeKey64::typed(WakeReasonCode::Manual, WakeProfileCode::CrossbeamQueue, 0, 0b1_u16, 0, 1).unwrap();

        let mut no_sender = ObservableWakeRuntime::new();
        let id = no_sender.register(ObserverConfig::new("channel", WakeProfileCode::CrossbeamQueue));
        let report = no_sender.publish(ObservablePacket::new(key, PayloadRef::default()));
        println!(
            "{} no sender:   errors {:?} wake {} delivery {}",
            $version,
            report.errors,
            no_sender.observer_epoch(id).unwrap(),
            no_sender.observer_delivery_epoch(id).unwrap()
        );

        let (tx, rx) = crossbeam_channel::unbounded();
        drop(rx);
        let mut no_receiver = ObservableWakeRuntime::with_crossbeam_sender(tx);
        let id = no_receiver.register(ObserverConfig::new("channel", WakeProfileCode::CrossbeamQueue));
        let report = no_receiver.publish(ObservablePacket::new(key, PayloadRef::default()));
        println!(
            "{} no receiver: errors {:?} wake {} delivery {}",
            $version,
            report.errors,
            no_receiver.observer_epoch(id).unwrap(),
            no_receiver.observer_delivery_epoch(id).unwrap()
        );
    }};
}

fn main() {
    failed_sends!("0.1.0", ob_serve_a_bowl_0_1_0);
    failed_sends!("0.2.2", ob_serve_a_bowl_0_2_2);
}
