# Review - live-issuance-credentials

## Specification review, 2026-09-14 (before agreement)

Reviewed LIVE-022 in `docs/spec/live.md` against the issuance path. The
engine's `issue` builds deterministic claims from the request context and
the current millisecond (`crates/suprnova-live/src/async_updates/authorization.rs`,
`issue`), and `codec.sign` adds no nonce, so two issuances of one scope in
one millisecond mint one descriptor and one binding. The host's credential
store (`framework/src/live/ports/subscription.rs`) keyed one secret per
binding, so the second issuance replaced the first's secret; the connect
that issuance performs (`framework/src/live/async_updates.rs`, `issue`,
which calls `service.connect` before answering) then rejected the first
request with `InvalidCredential`, answered as 403 `async_authority_invalid`.

### Attacked: contradictions

- LIVE-022 against LIVE-018's cap: the cap reserves slots before
  authorization and refuses the overflow with 409; LIVE-022 speaks only
  of the requests the cap admits. No contradiction.
- The decision (secrets kept per binding) against the credential entry
  cap: the cap now bounds secrets rather than bindings, so a binding
  holding several secrets cannot exceed what one-per-binding allowed.
- Rotation on connect: the successor is issued for the same binding, so
  renewal secrets accumulate under the same key by design; each
  subscriber consumes its own.

### Attacked: falsifiers that would not catch a violation

- A burst too small to collide in one millisecond would pass on the old
  tree. The probe holds 512 requests at a delayed authorizer and releases
  them together, the shape that produced 39 of 512 on 2026-09-13 and 61
  of 512 in the baseline here.
- Asserting "no 403" alone would pass while a request failed another way.
  The probe asserts every admitted request answers 201, which is how
  LIVE-023 was found.

### Attacked: requirements no mechanism can check

- None: both requirements are checked by one probe whose failure names
  every non-201 status and body.

## Agreement record

LIVE-022 and its decision were presented at 09:05 on 2026-09-14 with the
recommended option; the developer at 09:21: "confirmed". Recorded as
`Status: Agreed 2026-09-14` on the block, the decision under
`docs/decisions/`, and the commitment's decisions section.

## Agreement record, LIVE-023

Raised as escalation `live-022` at 09:33 after the baseline receipt showed
one admitted request in 512 answering 503 `async_unavailable`. The
developer at 10:06 restated the recommendation as the answer ("fold it
into this commitment as LIVE-023, checked by the same probe, fixed by
keying the in-construction claims by subscription id instead of one
slot"), recorded as `ok`. Recorded as `Status: Agreed 2026-09-14` on the
block.

## Mechanism demonstrations

### LIVE-022, `live-issuance-credentials`

Safe violating example: the probe as written, on the tree before the fix.
Baseline receipt `.cairn/evidence/LIVE-022/20260914T132842262Z` (fail):

    62 of 512 admitted issuances did not answer 201; statuses seen:
    {"201": 450, "403 {\"error\":\"async_authority_invalid\"}": 61,
     "503 {\"error\":\"async_unavailable\"}": 1}

After the fix the same probe passes: a binding holds every secret issued
for it until each is consumed or expires, and consuming removes the one
that matches. Receipt `.cairn/evidence/LIVE-022/20260914T133245362Z`
(pass) on `6546bb31`, and again `20260914T140939381Z` on `a62136ab`.

### LIVE-023, `live-issuance-credentials`

Safe violating example: the same probe on `6546bb31`, where the
credential fix is in and the context race is not. The race is
probabilistic (one in 512 in the baseline; the pass receipt on `6546bb31`
did not hit it), so the baseline receipt above is the recorded failure.
After the fix the claims under construction are keyed by subscription id
(`framework/src/live/async_updates.rs`, `construct_context` and
`MembershipRegistryPort::validate_current`). Receipt
`.cairn/evidence/LIVE-023/20260914T140939381Z` (pass) on `a62136ab`; the
probe passed five further runs on the same tree, and the whole live test
binary (112 tests) passes.

## Limits recorded

- **Probabilistic falsifier.** The context race fires in about one
  admitted request in five hundred, so a single passing run on a tree
  without the fix is possible; the receipt trail records the failure on
  the baseline tree and the fix by code reading.
- **Rotation timing.** `consume` compares every secret under a binding
  through digests so the time taken does not depend on which matched;
  the number of secrets under one binding is still observable, and is
  bounded by the entry cap.
- **Shared gate key in the probes.** Both burst probes define the same
  Gate ability for the process; under nextest each test is its own
  process, which is how the mechanism and the gate run them. Under a
  threaded `cargo test` the two definitions would race.
