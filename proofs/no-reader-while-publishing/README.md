# no-reader-while-publishing

ob-serve-a-bowl 0.1.0: one thread calls `publish` while another calls `observer_epoch` on the same runtime, with no lock. It doesn't compile, so in 0.1.0 no safe code could read an observer from another thread while the runtime published. 0.2.2's `ObserverHandle` is what makes that read possible.

Run `cargo build` here. Expected: exit 101 with `error[E0596]: cannot borrow *shared as mutable, as it is behind a & reference` at `src/main.rs:11` and `error[E0501]: cannot borrow *shared as immutable because previous closure requires unique access` at `src/main.rs:13`.
