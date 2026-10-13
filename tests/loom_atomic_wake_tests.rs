#![cfg(loom)]
#![cfg(feature = "loom-tests")]

use loom::sync::atomic::{AtomicU64, Ordering};
use loom::sync::Arc;
use loom::thread;

const KEY: u64 = 0xfeed_beef_cafe_babe;

#[test]
fn wake_epoch_publish_order_makes_key_visible_after_epoch() {
    loom::model(|| {
        let last_key = Arc::new(AtomicU64::new(0));
        let wake_epoch = Arc::new(AtomicU64::new(0));

        let writer_key = Arc::clone(&last_key);
        let writer_epoch = Arc::clone(&wake_epoch);
        let writer = thread::spawn(move || {
            // This mirrors the runtime's publication order:
            // key first, then epoch. Readers treat the epoch as the signal.
            writer_key.store(KEY, Ordering::Release);
            writer_epoch.fetch_add(1, Ordering::AcqRel);
        });

        let reader = thread::spawn(move || {
            let observed_epoch = wake_epoch.load(Ordering::Acquire);
            if observed_epoch > 0 {
                assert_eq!(last_key.load(Ordering::Acquire), KEY);
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
    });
}

#[test]
fn delivery_epoch_can_lag_wake_epoch_without_losing_wake_cause() {
    loom::model(|| {
        let last_key = Arc::new(AtomicU64::new(0));
        let wake_epoch = Arc::new(AtomicU64::new(0));
        let delivery_epoch = Arc::new(AtomicU64::new(0));

        let writer_key = Arc::clone(&last_key);
        let writer_wake = Arc::clone(&wake_epoch);
        let writer_delivery = Arc::clone(&delivery_epoch);
        let writer = thread::spawn(move || {
            writer_key.store(KEY, Ordering::Release);
            writer_wake.fetch_add(1, Ordering::AcqRel);
            // Delivery may happen later than the semantic wake. That is okay;
            // the key remains the authority for the observed wake epoch.
            writer_delivery.fetch_add(1, Ordering::AcqRel);
        });

        let reader = thread::spawn(move || {
            let observed_wake = wake_epoch.load(Ordering::Acquire);
            let _observed_delivery = delivery_epoch.load(Ordering::Acquire);
            if observed_wake > 0 {
                assert_eq!(last_key.load(Ordering::Acquire), KEY);
            }
        });

        writer.join().unwrap();
        reader.join().unwrap();
    });
}
