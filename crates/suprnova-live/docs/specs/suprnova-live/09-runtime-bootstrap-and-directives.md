# Suprnova Live -- 09 Runtime Bootstrap and Directives

Status: Normative design specification
Last revised: 2026-10-18

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
  request, an action or a model synchronization, requests atomic promotion; the
  nonce grants no authority.
- Local interactions that send nothing may occur before promotion, and the
  first request, an action or an immediate model synchronization, promotes and
  enters the ordinary island scheduler without an eager connect round trip.
- A server-declared lazy-completion trigger queues one protocol-v2
  `lazy_complete` operation through the owning island scheduler when its checked
  activation policy is satisfied. Discovery, reinsertion, or a surrounding
  morph cannot enqueue duplicate completion for the same surviving identity.
- Invalid metadata cannot cause arbitrary endpoint calls or controller lookup.
- Multiple islands connect independently in deterministic document order.
- The optional lifecycle driver retains one port for every connected island
  the server rendered, with no count cap; retiring an island releases its port.
- The upload slot receives one typed `proposeUploadHandle` capability. Core
  validates current island ownership, the declared upload field, canonical
  handle grammar, cross-field/cross-island claims, and retirement; no generic
  model write or authority mutation port is exposed.
- The ESM upload artifact registers one default feature immediately. Applications
  configure that same singleton before document connection with bounded typed
  ports; they do not register a competing feature. Explicit post-reload
  reacquisition resolves the already-connected island element through the
  artifact's scoped owner and fails closed for an absent or retired owner.
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
- An action directive (`live:click`, `live:submit`, `live:change`,
  `live:input`, `live:keydown`, `live:init`) names a registered action and may
  pass positional literal arguments, as in `remove(42)` or
  `rename('draft', true)`: JSON numbers, strings in single or double quotes
  with JSON escapes plus `\'`, `true`, `false`, and `null`, comma-separated,
  with JSON whitespace, at most 128. The action name follows the directive
  token grammar (a lowercase letter, then lowercase letters, digits, `_`,
  `.`, `:`, or `-`, at most 64 bytes), and the whole value is at most 2,048
  UTF-16 units, on both sides. Each literal binds the action's
  declared parameter at its position, and a trailing optional parameter may
  be left out. The checker refuses too many literals, a missing required
  parameter, and a literal the parameter's codec refuses. The island root
  lists each action's parameter names in declared order in
  `data-suprnova-live-actions` (`remove(id) rename(title,publish)`), and the
  runtime sends the literals as the named arguments the server decodes; a
  literal with no parameter to bind sends nothing and records a
  `directive_invalid` diagnostic. The checker and the runtime parse the
  shared vectors in `tests/fixtures/checker/action-call-grammar.json`.
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
- Optional island controllers expose stable `beforeMorph`, `afterMorph`, and
  `abortMorph` callback names. Production minification preserves these public
  lifecycle properties, and built-artifact browser tests exercise the exact
  callbacks rather than source-only substitutes.
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

### WebMCP tool registration

The runtime shall register a connected island's tool actions with the page's
WebMCP model context and remove them when the island leaves (AGENT-012). The
server describes the tools on the island root; the runtime registers exactly
what the root lists, each tool under its own abort signal, and runs a call
as a click of the tool's action. A browser without WebMCP registers nothing,
and nothing else in the runtime depends on it.

Every island root a mount, a public seed or a successor render assembles
carries `data-suprnova-live-tools` when the component offers the browser a
tool and the application serves agents, and no such attribute otherwise. Its value is canonical JSON:

```json
{"fields":["count"],
 "integers":["count"],
 "tools":[{"action":"increment","annotations":{"consequentialHint":false,
   "readOnlyHint":false,"untrustedContentHint":false},
   "description":"Add one to the counter.","inputSchema":{},
   "name":"counter.increment.main","title":"Increment"}],
 "v":1}
```

`fields` names the agent-visible fields (spec 03), and `integers` those of
them with a 64-bit integer codec, the only fields whose lossless tags a tool
result unwraps (spec 05); each tool names its
action, its full name `{base}.{documentKey}`, its optional title, its
description, its input schema (spec 04) and the three WebMCP hints. The
attribute is built from the generated metadata alone, so it is the same for
every viewer: a public seed's bytes stay shareable and do not change when the
seed is promoted, and who may call a tool is decided when it is called.

Acceptance criteria:

- The island key of a component with a browser tool fits the tool instance
  grammar `[A-Za-z0-9_-]{1,63}`, because it becomes the last segment of each
  tool name, which allows neither `:` nor a second `.`. `LiveMount` refuses
  such a key when it is declared, root assembly refuses it at render, and
  the runtime refuses it in the attribute.
- `data-suprnova-live-tools` is a known root attribute. The runtime accepts
  it only with exactly the keys `v`, `fields`, `integers` and `tools`; `v` is
  1; `fields` holds at most 512 unique field names; `integers` holds unique
  names each also in `fields`; `tools` holds 1 to 64 tools of
  exactly the keys above, each with a unique name of 1 to 128 ASCII letters,
  digits, `_`, `-` and `.` that ends in a `.` and the island's document key
  and is longer than it, an action identity, a non-blank description, an
  optional non-blank title, an object input schema, and annotations of
  exactly the three boolean hints. Anything else marks the island invalid.
- A page offers a tool only where the tool's checks run. A router serves
  agents when `Router::agents` installed the MCP endpoint and with it the
  access check and budget of every browser tool call (spec 07); it marks
  each of its mounts (`MountCatalogEntry::with_agent_tools`), and the engine
  lists tools only in a root of a marked mount. On a router without
  `Router::agents` no root carries the attribute, so WebMCP registers
  nothing.
- The attribute counts toward the root's configured size bound.
- On connect, when `document.modelContext` offers `registerTool`, or else
  `navigator.modelContext` does, the runtime registers each tool with its
  name, title, description, input schema and hints and its own
  `AbortController`. Retiring the island aborts its registrations, a page
  suspended into the back/forward cache aborts every registration and
  registers them again on resume, disposing the runtime aborts them all, and
  a navigation unloads them with the document.
- A morph cannot change an island's tools: the successor and same-metadata
  checks compare the tool contract, as they compare the snapshot.
- A registration the browser refuses, a duplicate or invalid name above all,
  records the `agent_tool_rejected` diagnostic at warning severity in the
  lifecycle phase with the `operation_rejected` detail, and the island keeps
  working.
- A call is refused unless the island is still the connected island the tool
  was registered for and the input is a plain object. Otherwise it is
  scheduled as the action directive's click is: through the island's own
  scheduler and transport, with its pending deferred model edits, promoting
  a public seed, and with the `Suprnova-Tool-Call` header (spec 06). An
  abort signal WebMCP passes cancels the call and rejects it with
  `AbortError`.
- A call resolves to `{content, structuredContent, isError}`. A committed
  response gives the outcome, `accepted`, `no_render` or `invalid` with the
  validation issues the agent may read (spec 05), the tool's arguments being
  the properties its input schema lists less any a field of the committed
  snapshot's state not listed in `fields` shares a name with, and a path
  matched by its field root, everything before its first `.` or `[`, and
  the agent-visible fields of the
  committed snapshot, with a field `integers` lists as its decimal string and
  every other field as the snapshot carries it; a navigation gives
  `redirect`. A refused response gives an error. The result carries nothing
  else of the island's state, HTML, events or effects.
- A refused call delivers its `isError` result to the agent before the island
  applies any recovery from the refusal, a page reload included, so the
  agent hears of every refusal. A response that would reload or navigate
  the page, a navigation or a refusal recovered by reloading, remounting or
  navigating, ends every island's calls, whichever island it answers: before
  applying it the runtime unregisters every island's tools at once, answers
  every island's pending calls, the response's own call with its result and
  any other with an error saying whether it ran, and holds the response
  250 ms when any island had a call pending: the browser passes a result to
  the agent asynchronously and the page cannot observe when it has. A
  refusal that only stops its island does the same for that island alone.
  A call that arrives meanwhile, through a registration that has left, is
  answered with an error and never queued. With no call pending anywhere
  the recovery runs at once, as a refused click's does, and islands the
  response leaves in place register their tools again.

UX flow:

1. A page with a tool island loads in a browser with WebMCP -> each tool the
   root lists is registered under its full name and the agent can call it.
2. The island leaves the page, or the page is cached or navigated away ->
   its tools leave the agent's view; a call the island refuses answers the
   agent with an error before the page recovers.

## Acceptance criteria

- The runtime loads independently of client frameworks and never owns initial
  document rendering.
- Islands are discovered, validated, connected, and retired exactly once.
- Directive grammar is deterministic, declarative, and shared with tooling.
- Delegated events preserve native and nested ownership semantics.
- Extensions and diagnostics use bounded public hooks rather than private
  monkey-patching.
- Tool islands register exactly the tools their roots list with WebMCP and
  remove them when they leave, and a tool call runs as a click.

## Decisions and revisions

- 2026-10-18 -- A page-level reload answers every island's WebMCP calls
  first. Recovery reloads the whole page, but only the refused island's
  calls were answered and held for, so a refused click on one island ended
  another island's call in flight with no result, and its tools stayed
  registered meanwhile. An argument named like a hidden field of the
  snapshot no longer shows that field's issue, and a keyed path such as
  `items[k].qty` is matched by `items`.
- 2026-10-18 -- A page offers no tool on a router that does not serve
  agents. The attribute came from the component's metadata alone, so an
  application without `Router::agents` registered WebMCP tools whose calls
  nothing checked or counted. The router now marks its mounts when it
  serves agents, and only a marked mount's roots list tools.
- 2026-10-18 -- A response that ends an island waits for the island's
  pending WebMCP results. Answering the call and yielding one task was not
  enough: Chromium dropped a result when the page reloaded within about
  10 ms of it, so the agent never heard of the refusal, and a second call
  could queue behind the refused one and meet the reload. The island's
  tools now leave at once, every pending call is answered, and the response
  is held 250 ms.
- 2026-10-18 -- The tools attribute gained `integers`. The runtime unwrapped
  any value shaped like a 64-bit integer tag, in any field, while the
  server unwraps only fields with a 64-bit integer codec, so a Json field
  holding that shape read differently on the two paths. The browser now
  unwraps only the fields the server lists.
- 2026-10-18 -- Added WebMCP tool registration (AGENT-012). Island roots of
  components with browser tools carry `data-suprnova-live-tools`, the
  island key of such a component fits `[A-Za-z0-9_-]{1,63}`, and the runtime
  registers each listed tool with `document.modelContext` under its own abort
  signal, aborting it on retire, suspend and dispose. A refused registration
  records `agent_tool_rejected`. A refused call must deliver its `isError`
  result before any recovery reload; the runtime at this revision can lose it
  to a reload, a defect ruled for the fix round.
- 2026-10-04 -- An action directive passes positional literal arguments,
  `live:click="remove(42)"`; it had always sent an empty argument object, so
  an action with parameters could not be invoked from a template. The
  literals bind the declared parameters in order, the checker validates them
  against the action's signature, and the island root's
  `data-suprnova-live-actions` attribute gives the runtime the parameter
  names to send them under. The grammar stays closed: literals only, no
  expressions.
- 2026-10-04 -- The configuration element also carries
  `async_max_queued_events`, `async_max_replay_events`, `max_redirect_bytes`
  and the five `upload_*` limits, and the runtime hands them to every optional
  feature through its document context, with a callback that prints a tripped
  limit's key. A feature holds no limit of its own.
- 2026-10-04 -- The configuration element carries every page limit the
  browser applies, as the server configured it: `max_request_bytes`,
  `max_response_bytes`, `max_html_bytes`, `max_json_depth`,
  `max_json_entries`, `max_request_items`, `max_response_items` and the
  `morph_*` limits, alongside `request_timeout_ms` (60 seconds by default,
  since a 16 MiB response takes tens of seconds on a slow link). The runtime
  requires every key and keeps no constant of its own beyond fallbacks equal to
  the server's defaults. Directive, feature, signal, binding, transition,
  bfcache, metadata and island-count scans no longer stop at 256 to 10,000
  items: those caps silently dropped behavior past them on large pages. A
  tripped limit prints its name, both values and the key to the console
  unless diagnostics are off.
- 2026-09-30 -- An immediate `live:model` edit on a seed-backed island is its
  first request and promotes the seed; the server now accepts that model
  synchronization as it accepts a first action.
- 2026-09-15 -- `live:key` is part of the directive set the runtime reads,
  not only the checker's: it names an element's stable morph identity (spec
  12, LIVE-024). `data-suprnova-live-key` remains the engine's spelling on
  engine-rendered roots.

- 2026-09-14 -- Manifest schema 3 adds a stylesheet artifact: the suprnova-ui
  token stylesheet and base layer ships as role `ui-styles` (`suprnova-ui.css`,
  `text/css`, script kind `stylesheet`, preload relation `preload`) under the
  same identity, integrity, and cache contract as the scripts, and the
  bootstrap emits it as a `<link rel="stylesheet">` only when a document opts
  in through `LiveBootstrapOptions::with_suprnova_ui()` (Cairn UI-008,
  UI-019).
- 2026-08-25 -- Made optional island morph callbacks an explicit stable
  production-artifact boundary. Their public names are excluded from property
  mangling and the generated package declarations expose the same lifecycle.
- 2026-08-25 -- Made optional upload configuration operate on the one
  automatically registered feature before boot and exposed a scoped resume
  entry for explicit application-owned reacquisition; the public artifact no
  longer advertises a factory whose result cannot replace its default slot.
- 2026-08-25 -- Added the sealed upload-handle proposal capability to the
  optional island port. It is a core-validated narrow write into the existing
  typed model batch, not a generic action, snapshot, or model mutation API.
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
