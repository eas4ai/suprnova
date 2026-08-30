# Live directive authoring

Live templates render normal HTML plus declarative `live:*` attributes. The
browser never evaluates directive text as JavaScript and never converts a
directive value into a module, endpoint, Rust path, or arbitrary property walk.

## Closed directive grammar

The accepted names, value shapes, owners, phases, fallbacks, conflicts, and
modifiers are generated from the Rust directive catalog through the reviewed
v3 fixture. The browser table is generated from the Rust directive catalog and
must match its manifest. Unknown names, dynamic suffixes, duplicate/conflicting
directives, invalid modifiers, oversized values, and wrong owners fail closed.

Server events include `live:click`, `live:submit`, `live:change`, `live:input`,
`live:keydown`, and `live:init`. Event modifiers are closed (`prevent`, `stop`,
`once`, `self`, `trusted`, `capture`, plus named keys where allowed). Native
behavior remains the fallback where the catalog says `native`; an inert
fallback never manufactures a request.

```html
<form live:submit.prevent="save">
  <input name="email" live:model.blur="email">
  <button type="submit" live:loading.disabled="save">Save</button>
  <p live:error.live.polite="email"></p>
</form>
```

## Models and server actions

`live:model` binds a registered model field. Timing is explicit: immediate,
change, blur, action, submit, fixed debounce/throttle windows, or a scheduler
policy such as latest, serial, or bounded parallel. The browser tracks edit
sequence and form semantics so a late response cannot overwrite a newer local
edit. Checkboxes, radios, selects, multi-selects, and successful-form controls
use native value rules; IME composition does not schedule partial text.

An action intent enters the island scheduler, builds the bounded v1/v2 request,
uses the configured endpoint and credentials, and applies a response only when
its island, request, revision, and scheduler disposition remain eligible.
Validation state is separate from transport failure. Terminal navigation wins;
nonterminal responses follow the shared response-ordering fixture and commit
successor metadata only after a successful morph.

The browser cannot authorize a field or action. The server re-verifies the
snapshot, trusted request context, component/action registry, model category,
validation, policy, and revision authority described in
[actions and validation](actions-and-validation.md).

## Effects and public calls

`live:effect` and `live:on` select registered client effects or typed event
behavior. Server responses carry bounded effect/event data, never executable
source. Effects run only in their declared phase after accepted application;
missing, invalid, timed-out, failed, canceled, or disposed execution has a
closed result.

Boot-time public calls similarly register an input schema, output schema, and
implementation. `runtime.call(owner, name, input)` resolves ownership from a
current island and validates both sides of the call. A call may explicitly
delegate to a server call through the provided context, but markup cannot name
an arbitrary HTTP endpoint. Optimistic projection is bounded presentation
state, reconciled by the accepted server outcome rather than treated as server
authority.
