# Suprnova Live -- 09 Runtime Bootstrap and Directives

Status: Normative design specification
Last revised: 2026-08-24

## Scope

This domain owns delivery and startup of the Suprnova Live browser runtime,
discovery and connection of server-rendered islands, the `live:` directive
grammar, delegated browser event capture, runtime lifecycle, and teardown. It
depends on canonical documents and the wire protocol and feeds local reactivity,
scheduling, morphing, navigation, and asynchronous updates. Directive-specific
state semantics belong to those downstream domains.

## Capabilities

### Runtime asset delivery and startup

Suprnova shall deliver a versioned, cacheable browser runtime that starts from
server-emitted configuration without requiring an application bundler or client
component framework. Applications may bundle the same supported runtime through
their asset pipeline without changing its protocol contract.

Acceptance criteria:
- Runtime assets have deterministic versioned identities and production cache
  headers.
- The universal core remains independently usable; trusted checked document
  metadata may require manifest roles for optional Stimulus, upload, or
  asynchronous ESM/classic artifacts without exposing element-selected
  URLs/modules. The Stimulus adapter may instead be imported through the
  equivalent package export when the application owns a bundler.
- The asset manifest binds role, format, hash, integrity, size,
  protocol/capability versions, and compatible core range. Loading is
  deduplicated, CSP-safe, and isolates a missing/incompatible optional feature
  to dependent directives without starting a second runtime.
- Startup configuration supplies only validated endpoints, protocol versions,
  asset metadata, and non-secret feature flags.
- External script, module, nonce, and hash-based CSP deployment are supported.
- Loading the runtime twice does not connect islands twice or duplicate
  delegated listeners.
- An optional adapter registers through the one exact versioned lifecycle
  driver before boot. Repeating the same adapter is idempotent; a conflicting or
  incompatible adapter fails only its role and cannot start another runtime.
- A missing or failed runtime leaves initial HTML intact and reports a
  developer-visible diagnostic.
- Startup does not require Inertia, Turbo, React, Vue, Svelte, or a global
  application store.

UX flow:
1. Canonical document loads -> the browser displays its SSR content immediately.
2. Compatible runtime loads -> it initializes once and begins island discovery.

### Island discovery and connection

The runtime shall discover only valid server-rendered island roots, associate
each with its instanced snapshot or public seed and metadata, and establish one
document-local runtime record and scheduler per island. Discovering a public
seed shall not create server ledger state merely because the page loaded.

Acceptance criteria:
- Discovery validates root shape, unique document-local slot identity,
  component identity, protocol version, and required instanced-or-seed metadata
  before connection.
- For a seed-backed island, the runtime creates a cryptographically random
  at-least-128-bit proposed nonce locally and includes it only when the first
  action requests atomic promotion; the nonce grants no authority.
- Multiple local interactions may occur before promotion, and one first action
  promotes and enters the ordinary island scheduler without an eager connect
  round trip.
- A server-declared lazy-completion trigger queues one protocol-v2
  `lazy_complete` operation through the owning island scheduler when its checked
  activation policy is satisfied. Discovery, reinsertion, or a surrounding
  morph cannot enqueue duplicate completion for the same surviving identity.
- Invalid metadata cannot cause arbitrary endpoint calls or controller lookup.
- Multiple islands connect independently in deterministic document order.
- Dynamically inserted islands connect through the same validated path.
- Removing an island cancels or retires its pending browser resources according
  to scheduling and upload policy.
- A nested island is not also parsed as ordinary directives owned by its parent.

UX flow:
1. Runtime discovers a valid island -> it marks the island ready and enables its
   Live-dependent controls.
2. Island metadata is invalid -> the region remains rendered but disconnected
   and exposes a bounded compatibility/error state.

### Directive grammar and parsing

Live shall define a namespaced, declarative `live:` attribute grammar with
explicit directive names, values, targets, arguments, and modifiers. Parsing
shall be deterministic and closed to arbitrary expression or JavaScript
evaluation.

Acceptance criteria:
- Grammar covers action, model, submission, keyboard/input events, feedback,
  identity, preservation, local behavior, initialization, navigation, and
  asynchronous-update directives owned by their specs.
- Grammar version 4 promotes `live:upload`, `live:progress`, `live:poll`, and
  `live:stream` with registered values, typed literal configuration, stable
  ownership, explicit conflicts, and no browser-selected endpoint or secret.
- Directive and modifier names are case and normalization stable across HTML
  parsing.
- Unknown directives and modifiers produce development diagnostics and follow a
  documented production behavior.
- Values identify registered actions, fields, events, or literal configuration;
  they are not executable JavaScript strings.
- Conflicting directives on the same element are rejected or have one explicit
  precedence rule.
- The view checker and browser parser share generated grammar metadata or
  conformance fixtures.

UX flow:
1. Application developer writes a valid directive -> checking and runtime agree
   on its meaning.
2. Directive is invalid -> source-oriented checking catches it where possible;
   runtime fails safely when static checking was unavailable.

### Delegated event handling

The runtime shall use bounded delegated listeners where browser semantics allow
so morphing does not require rebinding every Live control. Event resolution
shall find the nearest owning island and respect disabled, prevented, stopped,
trusted, and keyboard semantics.

Acceptance criteria:
- Supported DOM events and listener phases are enumerated.
- Event modifiers for prevent, stop, once, self, key filters, and timing have
  explicit meanings and accessibility guidance.
- Disabled controls, repeated activation, composed paths, shadow boundaries,
  and nested islands have tested behavior.
- A child island event does not invoke a parent action accidentally.
- Handler data is bounded before it enters scheduling or transport.
- Native browser behavior is preserved unless the directive explicitly and
  validly changes it.

UX flow:
1. Application user activates a Live control -> delegated handling resolves its
   directive and owning island once.
2. Event is outside a valid island or blocked by semantics -> Live performs no
   action and preserves appropriate native behavior.

### Runtime lifecycle and extension registration

The runtime shall expose defined boot, island-connect, before-request,
after-response, before-morph, after-morph, disconnect, and shutdown hooks for
supported framework integrations. Extensions shall register typed capabilities
rather than monkey-patching private runtime state.

Acceptance criteria:
- Hook order and synchronous/asynchronous constraints are documented.
- Extension failure is isolated where possible and cannot silently bypass
  security or scheduling.
- Effect handlers, controller bridges, and diagnostics register through stable
  names with duplicate detection.
- Private runtime internals are not part of the compatibility contract.
- Document replacement and browser lifecycle events trigger bounded cleanup.

UX flow:
1. Application developer installs a supported extension -> it participates at
   declared hooks without owning the Live protocol.
2. Extension throws or violates a hook -> the runtime reports it and retains or
   disconnects only the affected scope as policy permits.

### Runtime configuration and observability

Runtime behavior shall be configurable through typed server and build settings,
with safe production defaults and correlated diagnostics. Configuration shall
not accept arbitrary per-element security policy from the browser.

Acceptance criteria:
- Endpoint, timeout, concurrency, diagnostics, feature, and compatibility
  settings have documented scopes and precedence.
- Production diagnostics omit snapshots, tokens, cookies, and sensitive action
  data.
- Runtime events expose bounded timing and state transitions to observability
  hooks.
- Debug mode clearly identifies itself and cannot be enabled by an untrusted
  query parameter or directive.
- Misconfiguration produces actionable application-developer feedback.

UX flow:
1. Application developer enables supported diagnostics -> correlated island and
   request state becomes inspectable without sensitive payload disclosure.
2. Invalid configuration is served -> runtime rejects the unsafe value and
   reports its source.

## Acceptance criteria

- The runtime loads independently of client frameworks and never owns initial
  document rendering.
- Islands are discovered, validated, connected, and retired exactly once.
- Directive grammar is deterministic, declarative, and shared with tooling.
- Delegated events preserve native and nested ownership semantics.
- Extensions and diagnostics use bounded public hooks rather than private
  monkey-patching.

## Decisions and revisions

- 2026-08-24 -- Corrected optional Stimulus delivery to a manifest-typed
  ESM/classic adapter pair, with an equivalent package export for bundler users.
  The adapter must register before unchanged `boot({ stimulus })` startup;
  missing or incompatible registration emits one bounded unavailable diagnostic
  and leaves ordinary Live operational. Duplicate adapter loading is idempotent.
- 2026-08-23 -- Iteration 004 promotes `live:upload`, `live:progress`,
  `live:poll`, and `live:stream` through shared version-4 checker/runtime
  conformance. Their values remain registered names or bounded typed literals,
  never transfer grants, subscription secrets, arbitrary endpoints, or
  executable expressions. Upload and async behavior ship as manifest-selected
  optional ESM/classic feature artifacts while the universal core retains its
  existing transfer cap and single-runtime ownership.
- 2026-08-22 -- Iteration 003 owns the browser half of lazy server completion:
  checked activation queues one ordinary protocol-v2 operation through the
  island scheduler, with stable identity, duplicate suppression, cancellation,
  and normal response ordering rather than a second fragment protocol.
- 2026-08-21 -- Suprnova owns the Live runtime and `live:` directive contract;
  Stimulus complements it but does not define the protocol.
- 2026-08-21 -- Rejected arbitrary inline JavaScript expressions in Live
  directives.
- 2026-08-21 -- Public seed-backed islands remain connectionless on page load;
  their first action proposes an untrusted random nonce and performs atomic
  promotion through the normal protocol.
