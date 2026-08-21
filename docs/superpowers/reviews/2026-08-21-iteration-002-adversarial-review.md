# Iteration 002 Adversarial Architecture Review

**Reviewed:** 2026-08-21
**Scope:** [`iterations/002.md`](../../specs/suprnova-live/iterations/002.md)
and every normative specification it activates
**Verdict:** Ready for implementation planning after the remediations recorded
below. No unresolved architecture blocker remains inside the standalone kernel
boundary.

## Review question

Could an implementation satisfy the written Iteration 002 definition of done
while still violating Suprnova Live's authority, replay, integration, rendering,
or final-facade contracts?

The first pass found several ways it could. They were contract defects rather
than new product scope, so the normative specifications were repaired before
implementation planning.

## Findings and locked remediations

| ID | Adversarial finding | Locked remediation |
| --- | --- | --- |
| A1 | Protocol v1 can express only model sync and actions, not child updates, lazy completion, or fresh-render recovery. | Keep v1 stable and introduce protocol v2 for `params_changed`, `lazy_complete`, `fresh_render`, signed child delivery, and typed URL intent. Snapshot schema v1 remains independent. |
| A2 | The ledger can promote a public seed but cannot establish authority for an identity-bound initial render. | Add atomic `mount_instance`, distinct from promotion. Complete mount/render/dehydrate/sign first, create ledger authority immediately before publishing, and bound identity-collision retries. |
| A3 | The old field profile made ordinary state indistinguishable from public reusable seed state, omitted session-backed state, and did not explain how a partial public seed reconstructs omitted fields. | Make `State` the instance-only default, require explicit `Public`, add nondehydrated `Session`, and freshly run the repeatable/effect-free mount initializer before overlaying verified public seed values. |
| A4 | A public zero-input `PromotionAttestations::verified()` turns host authority into a boolean assertion. | Production context uses typed per-check dispositions, a scope fingerprint, and a verified mount-catalog match. Synthetic construction exists only in a dev-only package. |
| A5 | A home-grown or flat-sample template checker could disagree with Askama control flow or miss malformed branch structure. | Parse Askama with exact `askama_parser` 0.16 AST/spans, tokenize HTML with exact `html5ever` 0.39, join branch stack states, and label dynamic structure unproved. |
| A6 | Standalone macro tests could pass while expansions name development-crate paths that fail after integration. | Expansions name only final `::suprnova::live` and `::suprnova::live::__private` paths and compile through a dev-only facade fixture. |
| A7 | Raw Askama `safe` permits unaudited unescaped markup. | Checked Live templates reject untyped raw `safe`; unescaped output requires Suprnova-owned `TrustedHtml` and a checked filter with a visible reason. |
| A8 | Hashing the whole transport request makes correlation metadata part of idempotency, while hashing too little permits changed work under one retry identity. | Define a versioned semantic digest over scope, instance, base revision, component contract, idempotency identity, authority digest, ordered operations, proposals, and semantic extensions; exclude transport-only metadata. |
| A9 | A metadata-only accepted-outcome ledger cannot replay a response body lost after acceptance. | Do not add a hidden component/response store. A duplicate with no retained body returns refresh-required without action replay. |
| A10 | Allowing island renders to emit general response metadata lets a child inject headers, cache policy, cookies, or status. | Separate document response intent from island render metadata. The endpoint exclusively owns Live media type, no-store/security headers, cookies, and status. |
| A11 | An underspecified transaction order can publish state before rendering succeeds or misreport a durable domain commit as replayable. | Lock the Tier 0 order from ledger claim through complete render/sign, host commit, ledger acceptance, and non-authoritative reporting. Failure after durable host commit requires fresh render and never action replay. Durable after-commit work uses an outbox or equivalent. |
| A12 | Browser-selected type names or linker inventory could bypass deliberate registration and destabilize contract identity. | Use an explicitly built immutable `ComponentRegistry`, canonical contract digests, mount-catalog resolution, and no global/linker inventory. |
| A13 | Signed component state alone does not authorize browser-supplied child parameters. | Use a purpose-separated signed child-parameter envelope bound to parent scope/instance/revision, child key/contract, parameter schema/value, and bounded validity. Verification yields a distinct capability. |

## Cross-contract consistency decisions

- The locked authoring surface is `#[derive(LiveComponent)]`, one struct-level
  `#[live(...)]`, field helpers, and one impl-level `#[live]` that consumes
  mount/action/computed/validation/lifecycle helpers.
- Protocol version, snapshot schema version, component contract version, state
  schema version, action identity version, and view-checker version remain
  independently explicit.
- Accepted-outcome metadata is concurrency authority, not server-resident
  component state and not a response cache.
- Host-neutral conformance proves the kernel and adapter contract. It must not be
  described as Suprnova router, middleware, session, CSRF, auth, tenant, or HTTP
  integration.
- The standalone engine and macro development packages may be implemented here,
  but production facade/re-export placement waits for the atomic move into
  Suprnova. Generated paths already target that final facade.

## Residual risks to verify during implementation

These are verification obligations, not unresolved design choices:

1. Branch-aware template checking must remain allocation-, depth-, and
   token-bounded on hostile templates.
2. The private mount service must prove that no HTML or snapshot escapes before
   ledger authority exists.
3. The host-transaction/ledger split must have deterministic fault-injection at
   every boundary, especially after durable host commit.
4. Macro expansion and checker cost must be measured at 1, 10, and 100
   components so generated metadata does not hide superlinear work.
5. Actual Suprnova middleware ordering and request-truth construction remain
   integration responsibilities for the later atomic move; standalone fixtures
   must never be relabelled as that proof.

## Conclusion

The hardened contract now closes the known authority, replay, rendering, macro,
and versioning gaps without importing browser-runtime work or pretending the
standalone project is framework-integrated. Implementation can be split into
ordered, independently verified subsystems without weakening the 25 completion
conditions.
