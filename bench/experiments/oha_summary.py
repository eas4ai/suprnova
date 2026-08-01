#!/usr/bin/env python3
"""Reduce one `oha --output-format json` result to a single summary line.

Prints: `<rps> <p50_ms> <p99_ms> <errors>`

# Why there is no try/except here

An unreadable result means the run did not happen. The first version of
this returned zeros on any parse failure, and the sweep dutifully printed
ten rows of `0 rps` for a server that was serving 346,000 — because `oha`
1.15 spells JSON output `--output-format json` and rejects the `-j` it was
being given. Ten lines of fabricated data, no error, and the only tell was
that thirty-second steps were finishing in two.

That is the same silent-failure shape this whole benchmark exists to find,
so it is worth being explicit: a benchmark that cannot read its own result
must stop, not substitute a number. Any exception here propagates, the
caller sees a non-zero exit, and the run aborts with `oha`'s stderr
attached.
"""

import json
import sys


def main() -> int:
    with open(sys.argv[1], encoding="utf-8") as fh:
        d = json.load(fh)

    rps = d["summary"]["requestsPerSec"]
    # `metrics.latency_ms` is already in milliseconds; the sibling
    # `latencyPercentiles` block is in seconds. Mixing them up silently
    # scales every latency in the report by 1000.
    lat = d["metrics"]["latency_ms"]

    codes = d.get("statusCodeDistribution", {})
    ok = sum(v for k, v in codes.items() if k.startswith("2"))
    errors = sum(codes.values()) - ok

    # "aborted due to deadline" is the generator cutting off requests that
    # were still in flight when the measurement window closed. That is the
    # harness stopping, not the server refusing, and counting it as a
    # server error would make every clean run look slightly broken.
    errors += sum(
        v for k, v in d.get("errorDistribution", {}).items() if "deadline" not in k
    )

    print(f"{rps:.0f} {lat['p50']:.3f} {lat['p99']:.3f} {errors}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
