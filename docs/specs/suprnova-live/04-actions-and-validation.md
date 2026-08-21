# Suprnova Live -- 04 Actions and Validation

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns registered server actions, typed arguments, validation,
action-level lifecycle, application-service invocation, and semantic outcomes.
It depends on component lifecycle and state binding and feeds snapshots, the
wire protocol, events, navigation, and testing. Transport security and browser
scheduling are enforced by their neighboring domains.

## Capabilities

### Registered typed actions

Only methods explicitly registered as Live actions shall be invocable through
the Live protocol. Actions shall receive typed arguments and authorized context
instead of dynamically dispatching arbitrary browser-supplied method names.

Acceptance criteria:
- Generated metadata maps each public action name to exactly one Rust method.
- Unknown, private, duplicate, or malformed action names are rejected before
  execution.
- Arguments use explicit schemas and safe conversion with bounded sizes.
- Action visibility can be limited by component type and application policy.
- Sync and async actions share the same observable outcome contract.
- Panics or internal failures never become successful action responses.

UX flow:
1. Application user invokes a declared control -> the owning registered action
   receives allowed typed input.
2. The action or arguments are not registered -> execution is rejected and the
   runtime follows the protocol-error recovery path.

### Validation and binding errors

Actions shall integrate with Suprnova validation for model fields, action
arguments, and form-wide invariants. Binding failures and validation failures
shall remain distinct enough for useful diagnostics while sharing predictable
application-user presentation.

Acceptance criteria:
- Rules can be declared on fields, action arguments, and cross-field validators.
- Applications can validate selected fields, the whole component, or an
  action-specific input type.
- Validation errors use stable field paths and message identifiers suitable for
  localization.
- Invalid input prevents protected domain effects unless an action explicitly
  implements a safe partial workflow.
- Error bags clear, retain, or replace messages according to documented action
  boundaries.
- Errors render through semantic associations and accessible status behavior.

UX flow:
1. Application user submits invalid state -> no prohibited domain effect occurs
   and the returned island associates errors with affected controls.
2. Application user corrects input and retries -> stale errors clear according to the
   validation policy and the valid action continues.

### Authorization and application-service boundary

An action shall recheck authorization for the current request and delegate
durable business operations to ordinary Suprnova application or domain
services. Sharing services with an HTTP handler is supported; sharing the
transport-level handler is not required.

Acceptance criteria:
- Authorization occurs after trusted identity resolution and before protected
  reads or writes.
- Locked identifiers and prior authorization results are not treated as proof
  of current permission.
- Transactions, ORM operations, queues, events, storage, and other application
  facilities remain usable through their normal service contracts.
- The same domain service can be called from a Live action and a normal HTTP
  handler without either transport wrapping the other.
- Authorization denial produces no protected side effect and no sensitive
  diagnostic disclosure.

UX flow:
1. Action reaches a protected operation -> the current principal and resource
   are authorized through normal Suprnova policy.
2. Permission changed since render -> the action is denied and the application
   user receives the declared sign-in, forbidden, refresh, or navigation path.

### Action hooks and transactional ordering

Actions shall have explicit before, after, success, validation-failure, and
error behavior with defined transaction and event ordering. Hooks may compose
cross-cutting application behavior but must not obscure whether the core action
ran or committed. An action body shall be safe to invoke again after an
uncommitted attempt, because a transaction rollback may also roll back its
revision claim and permit an idempotent retry.

Acceptance criteria:
- Hook ordering is deterministic and included in generated/test metadata.
- A before hook can deny or short-circuit without partially executing the
  action.
- Domain events intended to describe committed work are not published before
  the owning transaction commits.
- After-commit work uses Suprnova's established queue/event facilities where
  durability is required.
- Nontransactional external effects are not performed as though a database
  rollback could retract them; they require domain idempotency, compensation,
  or an established outbox/delivery contract.
- Live guarantees at most one committed outcome for an island base revision,
  not at-most-once Rust method invocation or exactly-once external effects.
- Hook failure has an explicit rollback, error, or post-commit reporting rule.

UX flow:
1. Valid action begins -> hooks, transaction, domain service, and outcome run in
   documented order.
2. A phase fails -> later incompatible phases do not run and the recovery state
   reflects whether durable work committed.

### Semantic action outcomes

An action shall return a typed outcome that can request rerendering, leave HTML
unchanged, redirect to a real route, update flash/session state, dispatch
declared events, or emit permitted browser effects. The wire protocol owns
encoding and application order.

Acceptance criteria:
- The default successful action produces a fresh render and snapshot.
- A no-render outcome is explicit and still advances necessary state/revision
  metadata.
- Redirect targets use normal Suprnova route/URL safety rules.
- Flash state follows ordinary session semantics and is consumed predictably.
- Events and browser effects are typed or registered, bounded, and testable.
- Conflicting outcomes fail during construction or follow one documented
  precedence rule.

UX flow:
1. Action succeeds -> the browser applies the declared outcome in protocol
  order and exposes proportionate success feedback.
2. Outcome redirects -> normal document navigation replaces the document; no
  client-route emulation is introduced.

### Explicit JavaScript dependency

Live actions shall be invoked only through the Live runtime. The framework
shall not synthesize ordinary handlers or hidden fallback forms for them.

Acceptance criteria:
- Action metadata does not generate a second transport entry point.
- Initial rendered content remains visible without JavaScript.
- Applications can explicitly build a normal form, link, or handler that calls
  the same domain service when equivalent no-JavaScript interaction is needed.
- The checker does not report the absence of such an application-authored path
  as a Live contract failure.

UX flow:
1. Live runtime is available -> declared Live controls invoke their actions.
2. Runtime is unavailable -> no Live action is attempted or falsely reported as
   successful; any ordinary alternative is application-authored.

## Iteration 002 implementation profile

Iteration 002 implements registered typed action dispatch, bounded argument
conversion, binding and validation error bags, lifecycle hooks, current
authorization calls, transaction/after-commit ports, and typed semantic
outcomes. It integrates those phases with the iteration-001 ledger so a claim is
committed only with an accepted outcome and a rolled-back host transaction may
permit safe method reinvocation without promising exactly-once external
effects.

The standalone kernel receives a trusted Live request context and host service
capabilities; it cannot manufacture authentication, tenant, transaction, queue,
event, or session authority. Conformance adapters prove call order, rollback,
denial, idempotency, redaction, and response construction. Actual Suprnova
validation, authorization, database transaction, session/flash, queue, and
event adapters are reserved for the atomic integration move.

## Acceptance criteria

- Only registered typed actions execute.
- Validation, authorization, transaction, hook, and outcome ordering are
  explicit and testable.
- Action bodies tolerate reinvocation before commit, while at most one outcome
  can commit for the same island base revision.
- Actions compose with existing Suprnova domain services without duplicating
  transport handlers.
- Redirects, events, effects, and no-render outcomes remain typed and bounded.
- Live does not generate no-JavaScript action parity.

## Decisions and revisions

- 2026-08-21 -- Assigned the complete host-neutral action/validation pipeline
  and its ledger/transaction ordering to iteration 002. Conformance host ports
  prove the contract without being labelled actual Suprnova service adapters.
- 2026-08-21 -- Actions are explicit generated registry entries; rejected
  arbitrary dynamic method invocation.
- 2026-08-21 -- Live and ordinary HTTP transports may share domain services but
  do not require mirrored action handlers.
- 2026-08-21 -- The concurrency guarantee is one committed Live outcome per
  island base revision. A rolled-back method may run again; external effects
  require their own idempotency or delivery contract.
