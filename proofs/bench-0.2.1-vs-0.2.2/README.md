# bench-0.2.1-vs-0.2.2

Runs the crate's criterion benches on tag `v0.2.1` and tag `v0.2.2`, one after the other, for 3 rounds, then prints each case's median of the 3 runs for both versions and the change from 0.2.1 to 0.2.2. `compare.py` reads criterion's `estimates.json` (the median) from each saved round.

Run `sh proofs/bench-0.2.1-vs-0.2.2/run.sh` from anywhere in the repo. Set `WORK=<dir>` to choose where the two trees, their builds and the saved rounds go (default: a new temp dir). The script leaves `WORK` in place, about 400M. It takes about an hour.

Run on 2026-10-15 on an Apple M4 Pro, with other jobs on the machine, it printed:

```
case                                                           0.2.1 ns     0.2.2 ns   change
baseline_arc_mutex_vec_scan/2048                                  723.0        834.4   +15.4%
baseline_arc_mutex_vec_scan/512                                   200.0        200.6    +0.3%
baseline_arc_mutex_vec_scan/64                                     25.8         26.3    +1.8%
baseline_arc_mutex_vec_scan/8                                       7.1          7.1    -0.4%
cold_state_predicates/512                                        2464.2       2405.4    -2.4%
cold_state_predicates/64                                          165.1        157.7    -4.5%
cold_state_predicates/8                                            26.1         25.8    -1.0%
cold_state_predicates_auto_dense/512                             2474.9       2402.7    -2.9%
cold_state_predicates_auto_dense/64                               173.1        163.8    -5.4%
cold_state_predicates_auto_dense/8                                 33.5         33.5    -0.0%
cold_state_predicates_auto_sparse/512                             410.8        397.0    -3.4%
cold_state_predicates_auto_sparse/64                               68.4         65.8    -3.8%
cold_state_predicates_auto_sparse/8                                22.6         21.6    -4.2%
cold_state_predicates_indexed/512                                2625.3       2690.4    +2.5%
cold_state_predicates_indexed/64                                  202.8        198.1    -2.3%
cold_state_predicates_indexed/8                                    37.3         37.1    -0.5%
event_log_append/publish_logged_hot_32_observers                   92.4         92.2    -0.2%
event_log_replay_summary/1024                                    1748.4       1734.7    -0.8%
event_log_replay_summary/16384                                  27391.6      27194.6    -0.7%
event_log_replay_summary/64                                        98.0        100.5    +2.5%
event_log_serialization/from_json_snapshot_1024                395001.5     393803.8    -0.3%
event_log_serialization/from_jsonl_1024                        399231.6     398208.1    -0.3%
event_log_serialization/to_json_snapshot_1024                  249087.8     250471.7    +0.6%
event_log_serialization/to_jsonl_1024                          356082.1     357124.5    +0.3%
hot_bitpacked_publish_atomic/2048                                7565.0       6320.0   -16.5%
hot_bitpacked_publish_atomic/512                                 1766.3       1106.6   -37.3%
hot_bitpacked_publish_atomic/64                                   139.8        144.7    +3.5%
hot_bitpacked_publish_atomic/8                                     22.5         22.4    -0.6%
hot_bitpacked_publish_auto_dense/2048                           11420.4      11997.5    +5.1%
hot_bitpacked_publish_auto_dense/512                             2536.5       2333.2    -8.0%
hot_bitpacked_publish_auto_dense/64                               247.3        249.9    +1.0%
hot_bitpacked_publish_auto_dense/8                                 36.5         36.5    +0.1%
hot_bitpacked_publish_auto_sparse/2048                           2833.7       2881.5    +1.7%
hot_bitpacked_publish_auto_sparse/512                             553.9        540.0    -2.5%
hot_bitpacked_publish_auto_sparse/64                               97.1         93.4    -3.8%
hot_bitpacked_publish_auto_sparse/8                                16.2         16.2    -0.0%
hot_bitpacked_publish_indexed_atomic/2048                        6589.3       6481.7    -1.6%
hot_bitpacked_publish_indexed_atomic/512                         1114.3       1067.0    -4.2%
hot_bitpacked_publish_indexed_atomic/64                           146.2        140.1    -4.2%
hot_bitpacked_publish_indexed_atomic/8                             25.3         24.7    -2.2%
hot_bitpacked_publish_scan_dense/2048                           11578.9      11979.8    +3.5%
hot_bitpacked_publish_scan_dense/512                             2572.6       2316.4   -10.0%
hot_bitpacked_publish_scan_dense/64                               250.0        249.2    -0.3%
hot_bitpacked_publish_scan_dense/8                                 36.1         36.1    +0.1%
hot_bitpacked_publish_scan_sparse/2048                           5147.6       3591.2   -30.2%
hot_bitpacked_publish_scan_sparse/512                             733.7        686.9    -6.4%
hot_bitpacked_publish_scan_sparse/64                               97.0         91.9    -5.3%
hot_bitpacked_publish_scan_sparse/8                                15.9         15.7    -1.2%
tagged_payload_publish/512                                       2320.5       1852.6   -20.2%
tagged_payload_publish/64                                         317.0        331.4    +4.5%
tagged_payload_publish/8                                           57.6         60.2    +4.6%
40 of 51 cases within 5% either way
```

`baseline_arc_mutex_vec_scan` runs no code from this crate, and its 2048 case still came out 15% apart (per round: 734, 723, 720 ns on 0.2.1 against 861, 834, 710 ns on 0.2.2). A single case can move that much here for reasons outside the crate, so read the small changes as noise.
