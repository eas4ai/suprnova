# Live protocol v2 implementation contract

Protocol v2 extends the byte- and meaning-stable v1 interaction spine. Snapshot
schema v1 remains independently versioned. Rust and strict TypeScript parse the
same reviewed v2 fixture corpus; server-only lifecycle and child-capability
execution remains Rust-owned.

## Protocol v2 request

A v2 request has exactly `protocol_version`, `runtime_contract_version`,
`snapshot_schema_version`, `correlation_id`, `idempotency_key`, `component`,
`base_revision`, `snapshot`, `child_parameters`, `model_proposals`,
`operations`, and `extensions`. All three protocol/runtime/snapshot versions
are validated independently. The snapshot remains either an instanced envelope
or a public seed plus non-authoritative browser nonce.

The closed v2 operations are:

- `sync_model`, followed by at most one registered `invoke_action`;
- `params_changed` with one exact admission carrier;
- `lazy_complete`; and
- `fresh_render`.

Each lifecycle operation must be the only operation, carry no model proposals,
and use exactly its required authority: `params_changed` requires child
parameters; `lazy_complete` and `fresh_render` forbid them. The
`child_parameters` carrier has exactly `envelope` and `parent_snapshot`; the
current child snapshot remains the ordinary top-level request snapshot.
Lifecycle names do
not expose arbitrary Rust hooks. Unknown, duplicated, reordered, oversized, or
incompatible work fails during complete structural parsing.

The semantic idempotency digest is versioned separately from the wire shape. It
omits correlation, media, and transport details while binding current scope,
instance/base revision, component contract, idempotency identity, signed
authority, requested operations, proposals, and semantic extensions. For v2
`params_changed`, it additionally binds a purpose-separated digest of the
bounded canonical admission carrier; changing that carrier under the same base
and retry identity is an idempotency conflict. The historical v1 profile bytes
remain unchanged.

## Child parameter envelopes

The `child-params-v1` envelope uses a purpose-derived HMAC key distinct from
seed and instance snapshots. Its canonical signed body contains:

- form and schema version;
- parent scope, instance ID, and accepted revision;
- child stable key and component contract digest;
- parameter schema version and schema digest;
- bounded canonical parameters and their value digest;
- issue and expiry times; and
- signing key ID.

Verification returns a non-forgeable `VerifiedChildParametersV1` capability.
It checks key purpose/rotation, signature, canonical shape, time bounds, current
parent scope/instance/accepted revision, child key/contract, schema
version/digest, and value digest. Raw browser parameter substitution, a signed
snapshot, or a trusted request context cannot replace that capability.

Parent rendering may prepare an update, but the envelope is publishable only
after the matching parent successor revision is accepted. A v2 response child
delivery binds the target child instance and parameter hash to that signed
envelope. Superseded-parent replay and cross-child delivery fail closed.

The exact-child foundation adds `child-params-v2` as a separate canonical body
schema and HKDF purpose. It preserves every v1 binding and adds
`child_instance`; `PreparedChildParametersV2`, `ExpectedChildParametersV2`, and
`VerifiedChildParametersV2` are distinct types, and neither verifier decodes the
other version as its own contract. Before server delivery, verified v2 data must
match the signed parent snapshot's exact composition child entry and
`LiveInstanceLedger::current_accepted_revision` must still equal the issuing
parent revision. The resulting `EligibleChildParametersV2` is server-only.
Missing or unavailable ledger authority and a valid but superseded browser
snapshot fail closed.

Accepted parent execution builds one `ChildParameterDelivery` for each changed
surviving independently owned child. Each delivery contains only exact child
instance, parameter hash, and signed v2 envelope. The response's single
top-level successor snapshot supplies the parent proof and is not copied into
every delivery. Every envelope and the complete bounded response are signed,
serialized, and sealed before host commit and ledger acceptance; any failure
rolls back/releases the parent attempt without publishing partial bytes.

After successful parent morph and browser snapshot commit, the runtime pairs
that exact parent snapshot with each validated delivery and queues an ordinary
child `params_changed` intent. Its request sends the child's current snapshot
plus the exact carrier and no raw parameters. The endpoint verifies child
snapshot, parent snapshot, and v2 envelope independently, then creates
`EligibleChildParametersV2` from current parent-ledger authority. Only that
eligible capability reaches modern lifecycle execution. Historical
`VerifiedChildParametersV1` executor/harness APIs remain byte compatible but
cannot authorize the production endpoint; even a correctly signed v1 envelope
placed inside the exact two-member carrier is rejected before component or
ledger work.

## Response ordering

A v2 response retains the v1 fields and adds bounded `child_deliveries` and
typed `url_intent`. URL intent is either same-route reflection or safe
real-route navigation; it is not a client router or arbitrary URL. Response
outcomes remain accepted, duplicate, rejected, refresh-required, or fatal.

The browser runtime must apply a completely validated response in this order:

1. A safe redirect is terminal; navigate without morphing or running events or
   effects.
2. For HTML, preflight and morph the island while the old snapshot/revision
   remains authoritative. For no-render, validate that disposition.
3. Only after morph/no-render success, install the successor snapshot and
   revision.
4. Reconcile model and validation state and restore focus/form continuity.
5. Pair/queue eligible child deliveries and apply same-route URL reflection.
6. Dispatch typed events, run registered effects, and settle feedback.

Signed child deliveries and reflected URL intent are eligible only with that
same committed successor. Redirect/navigation, malformed response, failed
morph, stale or removed child identity, and unchanged parameter hash suppress
child scheduling. Child scheduling begins only after the parent rollback
boundary: one construction/enqueue failure is child-local and later deliveries
continue. Navigated URL intent is terminal and mutually exclusive with committed
state or child delivery.

The server may advance to revision N+1 before the browser morphs revision N's
DOM. Committing browser authority before morph success would attach new state
to old DOM and is forbidden.

## Failure and recovery

Morph failure after server acceptance leaves browser authority at revision N
while the server holds N+1. Recovery is `fresh_render`; the runtime must never
retry the original request with the old snapshot. An unreplayable duplicate,
lost accepted response, consumed/stale authority, or durable-host-commit versus
ledger-acceptance split uses the same non-replay recovery.

Rejected output retains current DOM according to its validation/recovery data.
Refresh-required requests a fresh authorized island. Fatal stops Live for that
island unless its closed recovery directs ordinary navigation. Child parameter
failure refreshes/remounts the child alone; parent and child application are
not transactionally atomic. Redirect always wins over morph, events, effects,
child delivery, and URL reflection.
