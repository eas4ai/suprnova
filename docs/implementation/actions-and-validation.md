# Live actions, validation, transactions, and outcomes

Iteration 002 turns verified component authority into closed registered work.
It does not expose reflective method dispatch, arbitrary JavaScript, raw
redirect URLs, or framework service implementations.

## Actions and validation

Each generated action entry owns a stable name/version, typed bounded argument
schema, current-authorization requirement, validation selection, transaction
policy, and erased typed dispatcher. The component registry resolves the
browser-visible action identity against that closed table before any method can
run. Unknown/private actions, malformed arguments, incompatible batches, and
panics produce bounded redacted outcomes.

Current authorization is a request-scoped host capability. It runs after
verified hydration and binding and before protected reads or effects. Snapshot
contents, locked fields, mount metadata, and prior authorization decisions do
not substitute for the current principal, tenant, session, route, resource, or
permission decision.

Binding issues remain separate from validation issues. `ValidationSelection`
supports none, selected paths, whole-component/cross-field rules, typed action
arguments, or component plus arguments. `BagPolicy::{Clear, Retain, Replace}`
defines how a successful validation run updates the bounded localizable error
bag. Validation failure may accept a successor revision and updated validation
state while suppressing the action and protected effects.

Events and browser effects are registered typed payloads with stable names,
schema versions, bounded canonical encoding, and safe diagnostics. The server
can emit only payload types declared by the component metadata. Browser effect
implementations remain outside this iteration.

## Transactions and idempotency

The Tier 0 action path is ordered as follows:

1. claim the expected base revision;
2. verify/hydrate state and apply the prepared model batch;
3. perform current authorization and validation;
4. begin the required host transaction, if any;
5. run before-action, the registered action, and after-action phases;
6. completely render, dehydrate, sign, and validate the successor/outcome;
7. commit the host transaction;
8. accept the committed outcome in the instance ledger; and
9. perform non-authoritative reporting.

Any failure after a successful claim and before host commit rolls back an open
transaction and consumes Tier 0 claim authority. Host-commit failure and a
durable host commit followed by ledger-acceptance failure both require fresh
render; neither may replay the original action. Reporting failure cannot
rewrite an already accepted result.
Durable after-commit work requires a host outbox, transactional queue, or
equivalent application guarantee.

The versioned semantic idempotency digest excludes correlation and
transport-only facts. It binds scope, instance, base revision, component
contract, idempotency identity, snapshot or child authority, operations,
proposals, and semantic extensions. The ledger retains bounded accepted
metadata, not response bodies. An exact duplicate is recognized without
re-execution; when its response body is unavailable, the endpoint returns
refresh-required and never executes the action again.

Live guarantees at most one accepted committed outcome per base revision. This
is not exactly-once method invocation or external effects. An action body must
tolerate reinvocation before commit under a transactional provider retry, and
external services require their own idempotency, compensation, or outbox
contract.

## Outcomes and recovery

`ActionOutcome` is exactly `Render`, `NoRender`, or a safe real-route
`Redirect`. Outcome metadata may add bounded flash/session intent, registered
events/effects, validation, and typed URL intent only in compatible
combinations. Redirect wins and suppresses render, successor DOM state, events,
and effects. It is ordinary document navigation, not SPA routing.

Render produces complete island HTML plus a signed successor. No-render keeps
the current DOM while still completing successor revision semantics. Unsafe or
unregistered output fails before ledger acceptance. Binding/validation
rejection retains the current DOM as directed; conflict, lost response,
consumed authority, render/sign failure, host/ledger split failure, or
unreplayable duplicate requests a fresh authorized render without operation
replay. The browser commit-after-morph state machine is specified in
[protocol v2](protocol-v2.md).
