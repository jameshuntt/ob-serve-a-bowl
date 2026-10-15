# cold-crossbeam-sender

A hot and a cold `CrossbeamQueue` observer on one `ColdObservableRuntime` whose hot runtime has a working sender, on ob-serve-a-bowl 0.1.0 and 0.2.2. One `publish_stateful` wakes both. The hot one sends; the cold one reports `CrossbeamSenderMissing` on both versions.

Run `cargo run` here. Expected output:

```
0.1.0 hot:  crossbeam_delivered 1 delivery 1
0.1.0 cold: cold_delivered 0 errors [CrossbeamSenderMissing] delivery 0
0.1.0 events in the channel: 1
0.2.2 hot:  crossbeam_delivered 1 delivery 1
0.2.2 cold: cold_delivered 0 errors [CrossbeamSenderMissing] delivery 0
0.2.2 events in the channel: 1
```
