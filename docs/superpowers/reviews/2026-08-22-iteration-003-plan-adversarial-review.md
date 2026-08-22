# Iteration 003 Implementation Plan Adversarial Review

**Reviewed:** 2026-08-22
**Scope:** [`iterations/003.md`](../../specs/suprnova-live/iterations/003.md), the approved browser-runtime design, and [`2026-08-22-iteration-003-browser-runtime.md`](../plans/2026-08-22-iteration-003-browser-runtime.md)
**Verdict:** Ready for implementation after the plan remediations recorded below. All 31 completion conditions have an implementation owner and executable evidence path; no unresolved architecture blocker remains in the standalone boundary.

## Review question

Could an implementation follow the plan, pass its named checks, and still fail the approved browser-runtime contract through a server/browser seam, authority ambiguity, lifecycle leak, false compatibility claim, or incomplete release gate?

The first pass found several such paths. They were planning defects rather than new product scope, so the implementation plan was repaired before approval was recorded.

## Findings and locked remediations

| ID | Adversarial finding | Locked remediation |
| --- | --- | --- |
| P1 | The existing private-mount wrapper is the only engine-owned Live root emitter. A test host could handcraft a seed root and make the browser pass while production still lacked a no-ledger public seed mount. | Task 6 now creates one typed root assembler shared by private mounts, public seed mounts, and accepted successor renders. The public seed path signs only public state, emits revision zero with no instance identity, and proves that it creates neither a promotion nonce nor ledger authority. |
| P2 | Existing accepted action HTML is component render output, not necessarily a complete matching Live root. Browser preflight could therefore demand a contract the server never emits. | Task 15 now assembles accepted HTML through the shared root assembler after successor signing but before host commit and ledger acceptance. Invalid or oversized successor roots prevent a successful outcome; public-seed promotion returns an instanced root. |
| P3 | Seed promotion changes browser identity, but the v2 response has no separate instance field. Committing only revision/snapshot would leave the island unable to receive child delivery or build the next instanced request coherently. | Tasks 6 and 15 add a bounded, explicitly non-authoritative snapshot view. Root/snapshot/response component, slot, instance, and revision must agree; the island adopts the promoted instance only at commit. The server still verifies signatures and authority on every request. |
| P4 | Treating only runtime-originated DOM insertion as connectable would block legitimate dynamically inserted signed islands and contradict the single validation-path contract. Treating all inserted attributes as authority would be worse. | Discovery now validates every candidate through one bounded path. Connection is bookkeeping, not authority; copied or hostile markup cannot bypass signed snapshot verification, registered actions, current authorization, or server limits. Attribute mutations require deliberate subtree revalidation before directives become schedulable. |
| P5 | The directive task named categories but left the actual public names open, allowing checker/runtime agreement around an incomplete or drifted vocabulary. | Task 3 now lists the exact Iteration 003 directive names, modifier families, and reserved out-of-scope forms. One v3 fixture generates Rust and TypeScript descriptors and the gate checks generated-byte drift. |
| P6 | The seed nonce used Web Crypto, but correlation and idempotency identities could still have been implemented with counters, time, or `Math.random`. That would weaken retry identity and make collisions avoidable rather than negligible. | Task 13 now requires at least 128 Web-Crypto bits for correlation and idempotency identities through the injected randomness port, with fail-closed behavior and no predictable fallback. |
| P7 | Async registered effects could hang forever after authority commit, preventing feedback settlement or cleanup while the response was already durable. | Task 9 now gives every async effect a bounded injected deadline and lifecycle epoch. Timeout/cancel/late settlement becomes a scoped failure and cannot hold feedback, navigation, cleanup, or later application indefinitely. |
| P8 | A permanent `beforeunload`/`unload` strategy for dirty guards could make the bfcache tests self-defeating and provide unreliable cleanup semantics. | Task 21 forbids `unload`, attaches `beforeunload` only while an explicit dirty guard is active, and makes `pagehide`/`pageshow` the reliable lifecycle boundary with feature-detected freeze/resume. |
| P9 | Playwright WebKit could accidentally become the only evidence behind a Safari floor claim, or missing legacy-provider evidence could make ordinary local development impossible. | Task 23 separates the normal pinned Chromium/Firefox/WebKit gate from provider-neutral actual Chrome/Edge/Firefox/Safari evidence. Missing actual-floor evidence is explicitly `unqualified`: local work continues, release qualification fails closed, and WebKit is never relabeled Safari. |
| P10 | A bundle-only budget could pass while runtime observers, retained island state, or full Live preflight/lifecycle/commit work regressed. | Task 24 implements the exact D100/M1K/M5K workloads, all architecture-v1 hard caps, environment/sample attestations, honest exploratory labeling, and the 15-percent checked-baseline regression rule without disabling correctness or accessibility work. |
| P11 | The signed snapshot identifies component, slot, instance, and revision but does not retain the document-local mount key. The server therefore could not produce a matching successor root without either changing snapshot schema or trusting an undocumented browser field. | Task 13 reserves one bounded semantic v1/v2 extension, `x_suprnova_live_document_key_v1`, covered by the idempotency digest and parsed into a non-authoritative render context. Task 15 may echo it only into the root; it cannot select component, instance, scope, route, authorization, or ledger authority. |

## Coverage challenges performed

- Traced the current Rust and TypeScript response planners and verified that the plan extends v2 without mutating v1 fixtures.
- Traced the private mount wrapper and accepted execution path, exposing the missing public-seed and successor-root seams captured in P1 and P2.
- Checked every Iteration 003 completion condition against the 27-task matrix; every row has a production owner and a fresh verification artifact.
- Checked that each production parser/state machine has bounded positive, negative, hostile/property, lifecycle, and real-browser coverage where DOM behavior matters.
- Checked dependency and tool boundaries: Idiomorph is pinned and private, Stimulus remains application-supplied, Playwright/axe/fast-check/esbuild remain development tools, and agent-browser/DevTools MCP remain exploratory diagnostics.
- Checked scope and release claims: no Suprnova/Magnetar write, upload, stream, RenderCache, component-library, SPA, no-JavaScript action synthesis, or push is authorized.

## Residual implementation risks

These are verification obligations already assigned by the plan, not unresolved choices:

1. The shared root assembler must be inserted before server outcome publication without perturbing Iteration 002 transaction/ledger ordering.
2. The browser's snapshot view must remain correlation-only; no later convenience API may expose decoded state as trusted authorization or accepted model truth.
3. Form, focus, selection, composition, file-node, signal, and controller continuity must be proven on three engines because DOM behavior differs materially.
4. Cross-document transition and bfcache behavior must always preserve ordinary navigation fallback and must not be inferred from one engine's implementation.
5. Retained-memory evidence needs a documented subtraction method for excluded DOM/raw-byte ownership and an honest B1 versus exploratory classification.
6. Actual minimum Safari evidence requires an actual Safari/macOS provider; Playwright WebKit can never discharge that obligation.

## Conclusion

The plan now closes the current server/browser seams and makes the difficult claims mechanical: one generated grammar, one complete root shape, one scheduler per island, one response application order, one lifecycle ledger, explicit actual-browser qualification, and exact performance workloads. Implementation can proceed task by task without weakening the complete Iteration 003 boundary or pretending the standalone workspace is already integrated into Suprnova.
