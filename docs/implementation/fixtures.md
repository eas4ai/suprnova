# Shared Live conformance fixtures

## v1 and v2 conformance fixtures

`fixtures/v1/` and `fixtures/v2/` are the single reviewed Rust/TypeScript
sources of truth for their whole protocol versions. v1 contains:

- `canonical-success.json` and `canonical-failure.json`;
- `snapshot-success.json` and `snapshot-failure.json`;
- `protocol-success.json` and `protocol-failure.json`;
- `response-ordering.json`;
- `compatibility.json`; and
- `manifest.sha256`.

Every fixture case has an explicit kind. Both consumers enumerate the complete
case set and reject unknown kinds, missing expectations, and unconsumed cases.
Expected failures use stable closed codes rather than language-specific error
text. Fixed roots are labelled `PUBLIC TEST VECTOR - NOT FOR PRODUCTION` and
must never be copied into application configuration.

The v1 manifest is the lowercase SHA-256 of the exact ordered bytes of all eight
JSON files. Its current value is
`a5c8748dd8bd160656d596973cdb9f6f436d2ae46dce9dca49e801360d659037`.
v2 contains `protocol-success.json`, `protocol-failure.json`,
`compatibility.json`, and `manifest.sha256`; its current manifest is
`a3ff70ca458e6d0ad6fa61e836d38dfe0937d59aa8bc338e5721d8899413c4d7`.

Rust validates both corpora in `tests/golden_fixtures.rs`; TypeScript validates
the same repository-relative bytes and digests in
`browser/tests/golden-fixtures.test.ts`. v2 cases cover signed child delivery,
`params_changed`, `lazy_complete`, `fresh_render`, URL intent, lifecycle batch
exclusivity, and v1/v2 compatibility. Server-only execution cases remain Rust
tests and do not create a parallel browser fixture truth.

The TypeScript implementation independently performs bounded duplicate-aware
parsing, RFC 8785-compatible canonicalization, Web Crypto HKDF/HMAC checks,
protocol validation, compatibility classification, and response ordering. The
reviewed adversarial cases include JSON-only whitespace, lone surrogates, depth
and failure-precedence boundaries, prototype-named object keys, noncanonical
base64url signatures, typed identity limits, unsafe redirect forms, invalid
child authority, and incompatible lifecycle batches.

v3 adds the browser-facing `directive-grammar.json` catalog and manifest. It is
the single source for directive name, owner, value kind, modifiers, conflicts,
phase, and fallback. Rust checks the catalog against the checker contract and
the generator emits `browser/src/generated/directive-contract.ts`; browser
generation check rejects any byte drift. v3 is a contract layer, not a new
snapshot or wire-protocol version.

## Parser, property, and fuzz regressions

Typed state, proposal, snapshot, child-envelope, protocol, checker, endpoint,
host-context, lifecycle, transaction, and concurrency tests retain minimized
regressions at the narrowest owning boundary. Property tests prove canonical
round trips, failed-proposal nonmutation, signed child verification, and bounded
hostile parser behavior.

Nightly fuzz targets cover canonical input, v1 request/response, signed
snapshots, v2 request/response, child parameters, the template checker, the
directive catalog boundary, and inert browser island metadata.
Persisted crashes become deterministic regression tests; coverage-discovery
corpora are not normative fixture data. The fixed 1-, 10-, and 100-component
compile fixtures exercise final macro paths, UI diagnostics, expansion size,
and isolated `cargo check` scaling.

## Updating the corpus

1. Add or change the smallest case that expresses the reviewed contract.
2. Update both exhaustive consumers only when a new cross-language case kind is
   intentional.
3. Format the JSON deterministically.
4. Recompute `manifest.sha256` from the same fixed filename order.
5. Run the Rust golden test and the complete browser test suite.
6. Run browser format, lint, typecheck, build, and budget checks.
7. Explain the compatibility or security reason in the owning specification or
   implementation contract.

Do not create a separate expected-value table in Rust or TypeScript. Fuzz
crashes and property-test regressions are minimized and retained at their owning
test boundary; coverage-discovery corpus output is not normative fixture data.
