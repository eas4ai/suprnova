# Live lifecycle, state, binding, and composition

The server kernel owns one Rust component instance for one request. Signed
browser state and ledger metadata are authority inputs; neither is a sticky
server-resident component object.

## Lifecycle and mount authority

An initial private mount runs the component phases in this order:

1. `mount`
2. `rendering`
3. checked render
4. `rendered`
5. `dehydrating`
6. typed state dehydration and memo production
7. `teardown`

After complete render, dehydration, signing, and island validation, the private
mount service calls the create-only `mount_instance` ledger operation. Output is
publishable only after that operation creates instance authority. A bounded
instance-ID collision retry uses a fresh server identity and repeats only the
effect-free mount/render/sign preparation. Private mounts are never disguised
as public-seed promotion.

An action request verifies signed authority, creates a fresh owned instance,
reconstructs and hydrates state, calls `hydrated`, and then enters binding,
authorization, validation, action, rendering, dehydration, signing, and
acceptance coordination. Rendering phases match the initial sequence, and
`teardown` runs exactly once for every successfully owned instance, including
panic and downstream failure paths. A failed constructor has no instance to
tear down.

Public seeds contain no instance ID or revision. Promotion runs the registered
repeatable effect-free mount initializer under current host context, overlays
only verified `Public` fields, initializes omitted categories from current
defaults/host capabilities, signs complete instanced state, and creates ledger
authority atomically. Advisory dependency generations are memo. A component
with `refresh_on_promote` requests current state instead of applying the
original proposals or action.

## State categories

| Category | Snapshot exposure | Browser proposal | Purpose |
| --- | --- | --- | --- |
| `State` | Instance only | No | Default component state |
| `Public` | Public seed and instance | No | Explicit reusable public state |
| `Model` | Instance only | Yes, through its registered codec | Persistent form/input state |
| `Locked` | Instance only | No | Server-issued signed identity or invariant |
| `ServerOnly` | Never | No | Request-owned server dependency or derived state |
| `Session` | Never | No direct proposal | Typed host session read/write intent |
| `Computed` | Never | No | Recomputed presentation state |
| `Transient` | Never | Yes, for the current request only | Nondehydrated model input |
| `Secret` | Never and never rendered | No | Sensitive server data |

Only `Public` is eligible for reusable public seeds. Instanced snapshots may
contain `State`, `Public`, `Model`, and `Locked` values. `ServerOnly`, `Session`,
`Computed`, `Transient`, and `Secret` never dehydrate. Session access uses a
registered field and typed `SessionPort`; no raw cookie or session secret enters
component state or diagnostics.

## Model binding

Each model field declares a Rust-aware codec, stable path, and timing metadata.
The proposal boundary preserves all four states as
`ProposedValue::{Missing, Null, Invalid, Valid}`. Supported codecs cover
scalars, options, lists, maps, nested values, enums, date/time values, UUIDs,
checkboxes, and multi-select controls without stringly or lossy coercion.

The parser bounds whole bytes, path depth and segments, collection entries,
decoded bytes, issue count, and diagnostic text. Unknown, forbidden,
conflicting, oversized, or unstable paths fail before a generated setter or
action runs. A failed decode leaves the component unchanged; valid changes are
applied as a prepared batch.

Binding timing is closed metadata: immediate, change, blur, submit, or a bounded
debounce declaration. URL bindings produce either same-route reflection intent
or real-route navigation intent. Sensitive, transient, session, or invalid URL
state is rejected. This server iteration emits and checks intent but does not
execute browser events, debounce timers, `history.replaceState`, or navigation.

## Child composition

Nested components have developer-supplied stable keys and independent instance
ownership. Composition bounds depth/count, rejects duplicate or unstable keys
and circular ancestry, and classifies each child as `Unchanged`,
`PendingParams`, `Remount`, or `Removed`. Identity, component-contract, or
parameter-schema drift remounts rather than mutating incompatible authority.

A surviving child's changed typed parameters become a separately signed
parent-issued capability only after the parent successor revision is accepted.
Historical envelope v1 binds the parent scope/instance/revision, child key and
contract, parameter schema/digest, canonical value hash, key, and expiry. The
separate v2 envelope adds the exact child instance and uses its own signing
purpose; v1 is never reinterpreted as v2.

V2 verification returns typed `VerifiedChildParametersV2`, which is still not
delivery eligibility. `authorize_child_parameters_v2` first matches its parent
scope/instance/revision to a verified parent snapshot, then requires the exact
child key/contract/instance tuple in the signed composition extension, and
finally requires the ledger's current accepted revision to equal the issuing
revision. Only `EligibleChildParametersV2` represents that server-side result.
Missing/expired/consumed authority, a later accepted revision, foreign lineage,
or a provider error fails closed before component work. `lazy_complete` and
`params_changed` remain registered lifecycle operations, not arbitrary method
dispatch or streamed HTML. The production framework endpoint admits only the
exact v2 carrier, resolves the signed parent's route/slot/build expectations
from the immutable mount catalog, obtains ledger eligibility, applies typed
mount-backed values, and runs the registered child lifecycle once. Historical
v1 execution remains a non-endpoint compatibility harness.

Parent morph and child parameter application are intentionally non-atomic. The
child enters a bounded pending state; a child failure refreshes or remounts that
child alone without rolling back the accepted parent.

## Failure and recovery

Lifecycle errors have a closed phase and kind. A hook failure suppresses later
phases, render/dehydrate/sign failures publish no partial successor, and panics
or future-drop panics stay inside the lifecycle boundary. Teardown failure is
reported once and never causes a second teardown attempt.

Before host commit, action-path failure consumes Tier 0 claim authority and
requires a fresh render. After a durable host commit, failed ledger acceptance
also requires fresh render and prohibits replay of the original operation.
Child-parameter failure is child-local. Browser-side recovery order and
commit-after-morph are defined in [protocol v2](protocol-v2.md).
