# Registered-event descriptor field casing -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-09-03
Target domain: `14-events-and-asynchronous-updates.md`

## What it is

The issued asynchronous subscription descriptor carries its registered-event
fields as `maximumHops`, `maximumFanout`, and `payloadContract`, the names the
iteration 004 browser runtime contract fixed, while every other public JSON
field of the protocol is `snake_case`. The framework emits the names the
runtime reads and the browser host fails closed when any is absent, so the
inconsistency is contained but real. A future contract revision may rename
the three fields to `maximum_hops`, `maximum_fanout`, and `payload_contract`
with a versioned descriptor schema, the runtime, the reference host, the
fixtures, and the framework moving together.

## Acceptance criteria

- The rename ships as one versioned descriptor contract change: runtime
  decoder, reference host, conformance fixtures, framework issuance, and the
  browser host change in the same iteration, with the previous names refused
  rather than tolerated.
- Every public JSON field of the descriptor is `snake_case` afterwards and a
  generated contract, not a second handwritten schema, mirrors the names into
  TypeScript.
- Existing iteration 004 browser evidence is regenerated against the new
  descriptor, not relabelled.
