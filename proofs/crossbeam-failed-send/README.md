# crossbeam-failed-send

One wake to a `CrossbeamQueue` observer whose send fails (no sender, then a dropped receiver), on ob-serve-a-bowl 0.1.0 and 0.2.2. Prints the errors `publish` reports and the observer's wake and delivery counts.

Run `cargo run` here. Expected output:

```
0.1.0 no sender:   errors [CrossbeamSenderMissing] wake 1 delivery 1
0.1.0 no receiver: errors [CrossbeamSendFailed] wake 1 delivery 1
0.2.2 no sender:   errors [CrossbeamSenderMissing] wake 1 delivery 0
0.2.2 no receiver: errors [CrossbeamSendFailed] wake 1 delivery 0
```
