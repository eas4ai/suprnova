# Shared v1 conformance fixtures

`fixtures/v1/` is the single reviewed Rust/TypeScript source of truth for the
iteration 001 wire contract. The corpus contains:

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

The manifest is the lowercase SHA-256 of the exact ordered bytes of all eight
JSON files. Its current value is
`a5c8748dd8bd160656d596973cdb9f6f436d2ae46dce9dca49e801360d659037`.
Rust validates it in `tests/golden_fixtures.rs`; TypeScript validates the same
repository-relative files and digest in `browser/tests/golden-fixtures.test.ts`.
The TypeScript implementation independently performs bounded duplicate-aware
parsing, RFC 8785-compatible canonicalization, Web Crypto HKDF/HMAC checks,
protocol validation, compatibility classification, and response ordering.
The reviewed adversarial cases include JSON-only whitespace, lone surrogates,
depth and failure-precedence boundaries, prototype-named object keys,
noncanonical base64url signatures, typed identity limits, and unsafe redirect
forms.

## Updating the corpus

1. Add or change the smallest case that expresses the reviewed contract.
2. Update both exhaustive consumers only when a new case kind is intentional.
3. Format the JSON deterministically.
4. Recompute `manifest.sha256` from the same fixed filename order.
5. Run the Rust golden test and the complete browser test suite.
6. Run browser format, lint, typecheck, build, and budget checks.
7. Explain the compatibility or security reason in the owning specification.

Do not create a separate expected-value table in Rust or TypeScript. Fuzz
crashes and property-test regressions are minimized and retained in
`tests/fuzz_regressions.rs`; coverage-discovery corpus output is not normative
fixture data.
