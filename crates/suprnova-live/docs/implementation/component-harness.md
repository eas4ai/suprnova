# Browserless Live component harness

`suprnova-live-test-support` supplies a dev-only harness over the real
host-neutral mount, snapshot, ledger, validation, action, transaction, and
execution services. It is not linked through a production feature and is not a
framework adapter.

## Browserless component harness

`ComponentHarness` starts from an explicit component descriptor, trusted
request context, expected snapshot binding, key ring, limits, deterministic
host services, and the actual `PrivateMountService` and `ExecutionService`. It
can mount typed parameters, retain the current verified/encoded snapshot, apply
prepared model proposals, invoke registered actions, and advance accepted
revisions without HTTP or a browser.

The representative acceptance test exercises session read/write intent,
initial private mount authority, nested parameter input, valid and invalid
model proposals, validation suppression, current authorization, a later action,
rendered HTML, signed successor state, and ledger acceptance through one
component instance flow.

The harness proves the server kernel contract only. Synthetic contexts are
created by the complete production validator, not by bypassing it. Harness
success does not mean a Suprnova route, middleware stack, session store, policy,
database transaction, or browser runtime is registered.

## Host controls and fault injection

`HarnessServices` provides controlled clock, instance identity generation,
Tier 0 ledger, typed session storage, current authorization, validation,
transaction behavior, and a shared semantic trace. Tests can advance time,
change authorization/validation decisions, inject transaction failures, and
observe exact execution phases.

Lifecycle and execution fixtures add deterministic barriers and one named
failure point for claim, hydrate, bind, authorize, validate, transaction begin,
before/action/after hooks, render, dehydrate, sign, outcome validation, host
commit, ledger acceptance, and reporting. Concurrency tests coordinate with
barriers rather than sleeps, so stale revisions, duplicates, publication
ordering, and failure recovery are repeatable.

## Assertions and redaction

`HarnessAssertions` checks accepted/rejected class, revision, HTML fragments,
validation issue IDs, events, effects, redirects, URL intent, and recovery
without formatting signed snapshots or arbitrary state. `HarnessTrace` records
closed phase enums rather than application payloads.

Harness and fixture errors expose stable kinds and registered safe identities.
Normal formatting never includes signing roots, signed envelopes, component
state, session values, proposals, action arguments, trusted context facts, or
host credentials. Fault injection can prove that failures suppress downstream
work and partial publication without turning secrets into test diagnostics.
