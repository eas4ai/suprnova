# Local reactivity

Local reactivity handles UI behavior that does not need database, session,
authorization, or server-rendered truth. It avoids round trips for disclosure,
selection, classes, attributes, focus, and other small interactions while
keeping application state and HTML rendering on the server.

## Local signals

`live:signal` declares a bounded typed mapping on a keyed scope. Values are
null, boolean, finite number, or bounded string; expressions are identifiers
and literal mappings, not JavaScript. Descendants may read the nearest owning
scope to a bounded depth. Duplicate declarations, type changes, missing names,
cycles, and operations after disposal are rejected.

```html
<section live:key="filters" live:signal="open:false">
  <button type="button" live:toggle="open" live:expanded="open">
    Filters
  </button>
  <div live:show="open" live:class="open:ring-2">...</div>
</section>
```

`live:toggle` mutates a boolean. `live:show`, `live:class`, `live:attr`,
`live:selected`, `live:expanded`, `live:inert`, and `live:focus` project state
through browser semantics and accessible ARIA/native attributes. Baseline DOM
state is restored when a binding retires. Signal updates batch notifications,
survive an identity-preserving morph, and dispose with their keyed scope.

## Optional Stimulus

Applications with richer local controllers may pass an existing Stimulus
application and a bounded list of definitions at boot. Load
`suprnova-live.stimulus.esm.js` or `suprnova-live.stimulus.classic.js` before the
unchanged `boot({ stimulus: { application, definitions } })` call; bundler users
may import the equivalent `@suprnova/live/stimulus` export. Duplicate adapter
loading is idempotent. If options are supplied without a compatible adapter,
Live reports one bounded Stimulus-unavailable diagnostic and continues ordinary
Live behavior.

Neither Stimulus nor Suprnova's bridge/continuity implementation is bundled
into the core runtime. `@hotwired/stimulus` remains a test-only compatibility
dependency here, and the application owns its chosen version and controllers;
the adapter imports no Stimulus package.

The bridge starts and stops that application, loads/unloads only supplied
definitions, captures controller-root continuity before a morph, validates the
same scope identity afterward, and releases pending continuity records when a
scope or runtime is disposed. Controller exceptions become redacted lifecycle
diagnostics and cannot replace the morph or scheduler protocol.

## Local and server boundaries

Use local signals when the answer is already in the document and has no
authoritative side effect. Use a Live model/action when Rust must validate,
authorize, load domain data, commit a transaction, render new HTML, or issue a
redirect. A dropdown can open locally while its Apply button schedules a server
action; these are complementary layers, not duplicate state stores.

Local values are never dehydrated into a signed component snapshot, cannot
unlock server fields, and cannot select server actions. A morph may preserve a
compatible keyed signal scope for continuity, but accepted server HTML remains
the structural source of truth. Normal Suprnova routes and forms remain the
explicit non-JavaScript alternative when an application requires one.
