#!/usr/bin/env python3
"""Reduce one `vegeta report -type=json` result to a single summary line.

Reads the report on stdin. Prints:
`<achieved_rps> <p50_ms> <p95_ms> <p99_ms> <p999_ms> <success_pct>`

Vegeta reports latencies in nanoseconds; everything here is converted to
milliseconds once, at the boundary, so no caller has to remember which
unit it is holding.

Like `oha_summary.py`, this deliberately has no exception handling — see
that file for why. A benchmark that cannot read its own result has to stop
rather than substitute a number.

`achieved_rps` matters as much as the percentiles. Vegeta drives an open
loop: it sends at the requested rate whether or not the server keeps up.
If achieved falls short of target, the generator could not keep up and the
percentiles describe a run that never reached the intended load.
"""

import json
import sys

NS_PER_MS = 1e6


def main() -> int:
    d = json.load(sys.stdin)
    lat = d["latencies"]

    def ms(key: str, fallback: str = "max") -> float:
        return lat.get(key, lat[fallback]) / NS_PER_MS

    print(
        f"{d['throughput']:.0f} "
        f"{ms('50th'):.3f} "
        f"{ms('95th'):.3f} "
        f"{ms('99th'):.3f} "
        f"{ms('999th'):.3f} "
        f"{d['success'] * 100:.2f}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
