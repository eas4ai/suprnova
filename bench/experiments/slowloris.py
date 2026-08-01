#!/usr/bin/env python3
"""Phase 1.1 - is hyper's header-read timeout actually armed?

Claim under test: `server.rs` builds its connection with
`http1::Builder::new()` and never calls `.timer(..)`. hyper documents that
`header_read_timeout` needs a timer installed to have any effect, so the
configured timeout may be inert and a client could hold a connection open
indefinitely by never finishing its request head.

Method: open N connections, send an incomplete head (no terminating blank
line), trickle one byte every 5s, and record when the server hangs up.

  PASS (claim false) - every connection closed by the server well before
                       the deadline.
  FAIL (claim true)  - connections still open at the deadline.

Exit status: 0 PASS, 1 FAIL, 2 setup problem (nothing accepted a
connection, so there is no result either way).

Usage:
    slowloris.py --host 127.0.0.1 --port 8080 [--sockets 8] [--deadline 75]
    slowloris.py ... --json results/run/slowloris.json
"""

from __future__ import annotations

import argparse
import json
import socket
import sys
import time

# Deliberately incomplete: one header and no terminating blank line, so
# the server is still waiting on the request head for as long as we care
# to make it wait.
PARTIAL_HEAD = "GET / HTTP/1.1\r\nHost: {host}\r\n"


def open_stalled(host: str, port: int, count: int, connect_timeout: float):
    """Open `count` connections and send a partial request head on each."""
    socks = []
    for _ in range(count):
        try:
            s = socket.create_connection((host, port), timeout=connect_timeout)
        except OSError as e:
            print(f"connect failed: {e}", file=sys.stderr)
            break
        s.settimeout(1.0)
        try:
            s.sendall(PARTIAL_HEAD.format(host=host).encode())
        except OSError as e:
            print(f"initial send failed: {e}", file=sys.stderr)
            s.close()
            continue
        socks.append(s)
    return socks


def still_open(sock: socket.socket) -> bool:
    """Has the server closed this connection?

    A trickled byte keeps the head incomplete while giving the peer a
    chance to reject it. An empty read means an orderly close; ECONNRESET
    and EPIPE mean the same thing less politely.
    """
    try:
        sock.sendall(b"X")
    except OSError:
        return False
    try:
        # The server has no reason to send us anything while the head is
        # unfinished, so a readable socket means EOF.
        sock.settimeout(0.2)
        data = sock.recv(1)
        if data == b"":
            return False
    except socket.timeout:
        return True
    except OSError:
        return False
    return True


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8080)
    ap.add_argument("--sockets", type=int, default=8)
    ap.add_argument("--deadline", type=float, default=75.0)
    ap.add_argument("--interval", type=float, default=5.0)
    ap.add_argument("--connect-timeout", type=float, default=5.0)
    ap.add_argument("--json", help="write the result record here")
    args = ap.parse_args()

    socks = open_stalled(args.host, args.port, args.sockets, args.connect_timeout)
    if not socks:
        print(f"SETUP FAILURE: nothing accepted a connection on {args.host}:{args.port}")
        return 2

    print(f"opened {len(socks)} stalled connections to {args.host}:{args.port}")
    started = time.monotonic()
    closures: list[float] = []
    live = list(socks)

    while live:
        elapsed = time.monotonic() - started
        if elapsed >= args.deadline:
            break
        time.sleep(min(args.interval, args.deadline - elapsed))
        elapsed = time.monotonic() - started
        surviving = []
        for s in live:
            if still_open(s):
                surviving.append(s)
            else:
                closures.append(elapsed)
                s.close()
        if len(surviving) != len(live):
            print(f"  t={elapsed:6.1f}s  {len(surviving)}/{len(socks)} still open")
        live = surviving

    held = time.monotonic() - started
    for s in live:
        s.close()

    passed = not live
    record = {
        "experiment": "1.1-slowloris",
        "host": args.host,
        "port": args.port,
        "sockets_opened": len(socks),
        "deadline_s": args.deadline,
        "held_s": round(held, 1),
        "still_open_at_deadline": len(live),
        "closure_times_s": [round(t, 1) for t in closures],
        "first_closure_s": round(min(closures), 1) if closures else None,
        "result": "PASS" if passed else "FAIL",
        "claim": "header_read_timeout is inert because no timer is installed",
        "claim_supported": not passed,
    }

    if args.json:
        with open(args.json, "w", encoding="utf-8") as fh:
            json.dump(record, fh, indent=2)
            fh.write("\n")

    print()
    if passed:
        first = record["first_closure_s"]
        print(f"PASS - server closed all {len(socks)} stalled connections "
              f"(first at {first}s). The header-read timeout is armed.")
        return 0

    print(f"FAIL - {len(live)}/{len(socks)} connections still open after "
          f"{held:.0f}s. The header-read timeout is inert: a client can hold "
          f"a connection open indefinitely without completing its request head.")
    print()
    print("Follow-up (spec 1.1): set SERVER_MAX_CONNECTIONS to a small number, "
          "hold that many stalled connections, then try a normal request. If it "
          "blocks, the permit pool is exhaustible and this is a DoS rather than "
          "a curiosity.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
