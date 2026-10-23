# Live protocol v1 implementation contract

Iteration 001 implements strict parsers and a pure response-application model.
It does not implement the Live HTTP route, middleware integration, component
dispatch, network scheduler, or DOM runtime.

## Request envelope

A v1 request has exactly these top-level fields:

`protocol_version`, `runtime_contract_version`, `snapshot_schema_version`,
`correlation_id`, `idempotency_key`, `component`, `base_revision`, `snapshot`,
`model_proposals`, `operations`, and `extensions`.

All three versions are exactly 1. Correlation and idempotency identities are
separate 128-bit-or-stronger base64url values; revisions are decimal strings.
The component is a validated registry-shaped name, not a Rust path.

`snapshot.kind` is either `instance` with a canonical signed envelope or
`seed_promotion` with an envelope and at-least-128-bit browser nonce. Seed
promotion requires base revision zero. The parser does not verify the embedded
snapshot or resolve the component; those are distinct later trusted stages.

Operations are an ordered non-empty list. Every `sync_model` references a
separately supplied bounded proposal, occurs at most once per field, and
precedes the optional first `invoke_action`. An action contains a validated name
and bounded canonical argument map. Unknown, duplicate, ambiguous, reordered,
or over-limit input fails before dispatch. Extension keys are bounded and must
use the `x_` namespace.

## Response envelope

A response has `protocol_version`, `correlation_id`, `outcome`, validation,
events, effects, and extensions, plus only the fields permitted by its outcome.
The closed outcomes are `accepted`, `duplicate`, `rejected`,
`refresh_required`, and `fatal`.

Accepted and duplicate results are exactly one of:

- a committed successor revision, canonical signed snapshot, explicit `html`
  or `no_render` payload, validation, events, and registered effects; or
- a terminal same-origin route redirect with no snapshot, revision, render,
  validation, event, or effect state.

Rejected results carry a safe error and may retain validation, but cannot
smuggle committed state, redirects, events, or effects. Refresh-required and
fatal results carry no validation or executable output and must pair with a
compatible recovery instruction. Redirect strings reject schemes, protocol
relative targets, control bytes, backslashes, oversized targets, and non-route
forms.

The stable error envelope contains only closed `category`, `recovery`, and
`detail` values. Normal formatting contains no hostile input or state. The
response parser validates the complete bounded envelope before any field is
eligible for application.

## Application state machine

The implemented planner returns semantic steps and has no DOM access.

1. A valid redirect is terminal and performs real document navigation only.
2. An HTML result preflights and morphs before installing snapshot/revision.
3. A no-render result validates that disposition before installation.
4. After successful morph/no-render validation, the runtime commits snapshot
   and revision, reconciles model/validation data, restores focus, dispatches
   events, runs registered effects, and settles feedback.
5. Morph failure after server acceptance requests a fresh render without
   committing browser state and without replaying the original action.
6. Rejected output retains DOM; refresh-required requests fresh authorized
   island state; fatal output stops Live unless its safe recovery is navigation.

Iteration 003 must implement this exact model around the real morph adapter. It
must not turn the model into SPA navigation or executable server-returned code.

## Compatibility and limits

Iteration 001 supports the exact v1 protocol/runtime/snapshot triplet. A
breaking mismatch allows one refresh decision and then stops rather than
looping. Every whole envelope and nested class has independent byte/count/depth
limits. The named A8/16 assertions cap fixed control overhead at 1 KiB and
snapshot framework overhead at 768 bytes.

The future HTTP adapter in iteration 002 owns methods, media types, CSRF,
origin, session, principal, tenant, authorization, and status mapping. A parsed
request is not authenticated, authorized, snapshot-verified, or dispatched.
