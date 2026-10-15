# Reads the criterion results run.sh saved (r1-0.2.1 ... r3-0.2.2) and prints, per case,
# the median of the three runs' median times for each version and the change from 0.2.1 to 0.2.2.
import glob, json, os, statistics, sys

work = sys.argv[1]
times = {}  # (version, case) -> [ns per round]
for path in glob.glob(os.path.join(work, "r[123]-*", "**", "new", "benchmark.json"), recursive=True):
    version = os.path.relpath(path, work).split(os.sep)[0].split("-", 1)[1]
    case = json.load(open(path))["full_id"]
    est = json.load(open(os.path.join(os.path.dirname(path), "estimates.json")))
    times.setdefault((version, case), []).append(est["median"]["point_estimate"])

cases = sorted({case for (_, case) in times})
within = 0
print(f"{'case':58} {'0.2.1 ns':>12} {'0.2.2 ns':>12} {'change':>8}")
for case in cases:
    old = statistics.median(times[("0.2.1", case)])
    new = statistics.median(times[("0.2.2", case)])
    change = (new / old - 1) * 100
    within += abs(change) <= 5
    print(f"{case:58} {old:12.1f} {new:12.1f} {change:+7.1f}%")
print(f"{within} of {len(cases)} cases within 5% either way")
