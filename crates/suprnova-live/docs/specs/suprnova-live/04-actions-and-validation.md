# Suprnova Live -- 04 Actions and Validation

Status: Normative design specification
Last revised: 2026-10-18

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
  localization, and carry the text the server resolved for the application
  user beside each identifier when it has one.
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
- An action body must be safe to invoke more than once before commit. A provider
  that rolls back its revision claim with the host transaction may invoke the
  Rust method again on an idempotent retry, even though only one committed Live
  outcome can be accepted.
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

### Actions as agent tools

A registered action shall be able to be an agent tool (AGENT-012, AGENT-013).
`#[tool(...)]` sits beside `#[action]` inside a `#[live]` impl and declares
the tool's name, description and optional title, the callers it accepts
(`public`, `user`, `agent`), its scopes, its hints and its guard, and with
`headless` makes the action callable over the MCP endpoint with no browser.
The tool runs its action, never a route, so it takes no `input`, `output` or
`route`: its input schema comes from the action's argument codecs. An agent
reads a tool's name, description and schema as instructions, so all of them
are fixed when the component is built.

The engine records the declaration as `ActionToolMetadata` on the action:
the base name, the description, the title, the hints `read_only`,
`untrusted_content` and `consequential` (the tool's `destructive` hint),
whether the browser registers it (it accepts a `public` or a `user`
caller, the callers a page has) and whether it is `headless`.

The input schema is draft 2020-12 JSON Schema derived from the codecs, each
describing exactly what the codec accepts: a string, a boolean, a 64-bit
integer as a JSON number inside the interoperable safe range or else its
canonical decimal string, an unsigned 64-bit integer likewise with no sign,
a finite number, a `date` or `date-time` string, a hyphenated UUID in either
case, an enumeration of its variants, a list of its entry and a map of its
value. The argument object lists every argument in declared order, marks
the required ones required, lets an optional one also be `null`, and allows
no other property, since preparation refuses an unknown name. The Json codec,
which accepts any canonical JSON its Serde type reads, has no schema.

`#[resolve(path)]` on a `#[mount]` parameter names an
`async fn(&ToolCaller) -> Result<T, FrameworkError>` that fills that mount
property for a headless call from the caller. The generated adapter encodes
the value with the parameter's codec, and an `Option<T>` resolver that
returns `None` mounts the component with `None` (spec 02). The mount
properties an agent sends follow the same schema rules as arguments: the
required ones listed, and an optional `Option<T>` one also accepting
`null`.

Acceptance criteria:
- `#[tool]` on a method with no `#[action]` fails to compile, and so does a
  second `#[tool]` on one action, an `input`, `output` or `route` argument,
  a name outside 1 to 128 ASCII letters, digits, `_`, `-` and `.`, a base
  name longer than 64 characters, and a missing, empty or non-literal
  description.
- A tool action whose argument type resolves to the Json codec, directly or
  as the entry of a `Vec` or the value of a `BTreeMap`, fails to compile
  naming the argument. The macro falls back to Json for every type it does
  not know, so an agent is never told less than the action enforces.
- A Live tool emits the same build-time name marker a handler tool emits, so
  a Live tool and a handler tool of one name in one crate fail the build.
- `ActionToolMetadata::new` refuses a base name outside `[A-Za-z0-9_.-]` or
  longer than 64 bytes and a blank description or one over 4 KiB;
  `with_title` refuses a blank title or one over 256 bytes.
- `ActionMetadata::with_tool` refuses a second declaration and an action
  with any argument whose codec has no schema, with
  `InvalidActionMetadata`. Two actions of one component cannot share a tool
  name.
- The action's contract-digest entry gains a `tool` key, holding the base
  name, description, title, hints, browser and headless flags, only when the
  action declares a tool. A component without a tool keeps the digest it
  had before agent tools existed.
- `ActionArgumentSchema::field_issues` names every argument preparation would
  refuse, with its closed reason: an unknown name, a missing or `null`
  required argument, or a value its codec refuses. Preparation still runs
  and decides.
- A headless tool is refused when the application boots, naming the tool,
  when its component has `#[session]` fields (a call with no browser has no
  session), when its action is an upload finalize action (the browser that
  sent the file finishes it), when a mount property the agent sends has the
  Json codec, or when a `#[resolve]` names a parameter the mount does not
  take.
- A resolved mount property is left out of the headless input schema, and an
  agent that sends it gets an unknown-property refusal. Resolvers run only
  after the mount's Gate check allows the call (spec 07).
- `#[resolve]` on a parameter of anything but `#[mount]`, or on a component
  with no headless tool, fails to compile.

UX flow:
1. Application developer marks an action `#[tool(...)]` -> the browser and,
   for a headless tool, the MCP endpoint offer it with a schema derived from
   the action's own argument codecs.
2. Application developer marks an action whose argument has no schema, or a
   headless action that needs a session -> the build or the boot fails naming
   the tool and what to change.

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
denial, idempotency, redaction, and response construction.

Iteration 005 Task 4 now provides the immutable Suprnova host graph for current
authorization, database transactions, component-keyed typed validation,
accepted-outcome event reporting, bounded execution telemetry, request-lifetime
cancellation, and HTTP response-intent projection. Component descriptors and
their generated `#[validate]` callbacks are sealed in one registry graph. The
engine's validation request carries both the exact component identity and the
request-owned hydrated, model-bound target, so rules run against current typed
Rust state rather than a second deserialized shadow model. Ordinary Suprnova
`validator::ValidationErrors` become stable Live field paths and message
identifiers; positional list indices collapse to the stable collection path
rather than entering signed/model identity. Bare `#[validate]` hooks own whole
component and selected-path validation, with selected results restricted to the
declared paths. `#[validate(action = "name")]` hooks own argument validation for
that exact registered action and receive its schema-decoded Rust arguments.
`all` runs both in that order. Macro expansion rejects missing hooks, unknown
action hooks, and hook signatures that do not exactly match their action;
startup registration also fails closed for a non-generated descriptor that
selects validation without a callback.
The event reporter uses Suprnova's existing event dispatcher; durable external
work still belongs to the action's transaction-owned outbox or equivalent
delivery contract. Real endpoint construction and invocation consume these
adapters in the following route integration task rather than introducing a
second execution pipeline.

The Tier 0 accepted-action order is fixed: claim the base revision; hydrate and
apply permitted models; authorize and validate; begin an explicitly requested
host transaction; run before/action/after phases; construct and successfully
render, dehydrate, sign, and validate the complete outcome; commit the host
transaction; commit the accepted ledger outcome; then run non-outcome-changing
post-commit reporting. Failure before the host commit rolls that transaction
back and consumes Tier 0 revision authority. Failure after the host commit but
before ledger acceptance requires fresh rendering and no automatic action
replay. Durable after-commit work uses an outbox or equivalent transaction-owned
facility rather than relying on a fallible reporting hook.

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
- An action becomes an agent tool only through `#[tool]`, with a name,
  description and schema fixed at build, and a headless tool only where a
  call with no browser can run it.

## Decisions and revisions

- 2026-10-18 -- An optional mount property resolved to `None`, or left
  `null` by the agent, mounts with `None` instead of failing the call.
- 2026-10-18 -- Added actions as agent tools (AGENT-012, AGENT-013).
  `#[tool]` beside `#[action]` records `ActionToolMetadata`; the input schema
  comes from the argument codecs, and an argument with the Json codec is
  refused at build. The digest gains the `tool` key only for a declared tool.
  A headless tool is refused at boot on a component with session fields or
  on an upload finalize action, and `#[resolve]` fills a mount property from
  the caller.
- 2026-10-04 -- An action's argument schema keeps the order the action
  declares its parameters in, beside the name order it decodes by. A
  directive's positional literal arguments bind in that order (spec 09), and
  the island root names each action's parameters in it, so a template can
  invoke an action with parameters. Decoding is unchanged: the server still
  receives a named argument object and validates it against the schema.
- 2026-09-13 -- Refused the Required transaction policy at registration
  until the host honors it. The framework's transaction port opens a
  transaction only to roll it back and answers commit and rollback with
  success, so an action declared `transaction = "required"` committed each
  write on its own and rolled nothing back when a later stage failed
  (audit finding ASTRA-05). The framework's `LiveRegistry` now fails with
  `RegistryErrorKind::RequiredTransactionUnsupported` for any component
  whose action declares the policy; actions without it register as before.
  A refused contract is honest; a successful no-op is not. The refusal
  lifts when the port installs a real ambient transaction the action's
  writes join, which is the work the owner declined to carry inside the
  hardening commitment. Recorded under Action hooks and transactional
  ordering; the framework's requirement is LIVE-017 in `docs/spec/live.md`,
  decided by the owner on 2026-09-13.

- 2026-08-31 -- Integrated the Task 4 Suprnova action host graph. Validation
  requests now bind the exact component identity and request-owned typed target,
  then dispatch only through the generated selection-aware `#[validate]`
  callbacks sealed with that component's descriptor. Bare hooks own
  component/selected paths; action-named hooks receive exact schema-decoded
  arguments; `all` runs both. Missing or mismatched generated hooks fail during
  macro expansion and non-generated omissions fail at startup. Accepted
  outcomes enter Suprnova's existing event system only after acceptance; this
  reporting does not create exactly-once external side effects or replace a
  durable transaction-owned outbox.
- 2026-08-21 -- Locked Tier 0 transaction/render/ledger ordering so rendered
  output is complete before the host commits, the host commits before the
  ledger accepts, and failures after durable commit recover by fresh rendering
  without replay. Post-commit reporting cannot retroactively reject an accepted
  outcome.
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
