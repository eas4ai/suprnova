# Actual-browser qualification

Playwright conformance and actual-browser qualification are separate release requirements.
Playwright provides deterministic Chromium, Firefox, and WebKit regression coverage. It does not
prove Chrome, Edge, Firefox, or Safari product compatibility, and Playwright WebKit must never be
reported as Safari.

`matrix.json` defines eight mandatory qualification slots: the documented minimum Chrome 111, Edge
111, Firefox 128, and Safari 16.4 floors, plus the current stable channel for each product.
`schema.json` defines the persisted evidence envelope. The checked-in `results/` directory is empty
by design; this repository does not contain fabricated passing evidence.

## Qualification states

```sh
npm run compatibility:check
```

The checker prints one of three states and uses it as a release boundary:

- `qualified` means every matrix slot has fresh, passing evidence for the current production runtime
  bytes and fixture manifest;
- `failed` means submitted evidence is malformed, identifies the wrong product or floor, contains a
  failing conformance result, or otherwise violates the evidence contract;
- `unqualified` means required evidence is absent or stale. It exits with status 2 and blocks a
  release qualification claim.

Ordinary local implementation commands do not claim product qualification and may continue while the
matrix is unqualified. To display the state without failing such an informational workflow, run
`node scripts/check-compatibility.mjs --allow-unqualified`. Never use that option in a release gate.

## Provider adapters

The core runner is provider-neutral. A provider adapter is an external ECMAScript module exporting
`runCompatibility(input)`. Credentials stay in that adapter's environment or secret store; the
runner neither accepts credential arguments nor copies environment values into evidence.

```sh
npm run compatibility:run -- \
  --target safari-minimum-16-4 \
  --adapter /secure/adapters/webdriver.mjs
```

By default the runner builds the production assets and starts the same conformance host used by the
Playwright catalog. For a remote browser, expose that host through an authenticated tunnel and pass
its public origin with `--base-url`. The runner verifies the host marker and runtime artifact hash
before invoking the adapter.

The adapter receives:

- the exact matrix target and required conformance case IDs;
- the verified conformance-host base URL;
- a fresh 256-bit test-run nonce, an authenticated challenge value, the runtime SHA-256, and the
  fixture-manifest SHA-256.

It must exercise every case in the named actual browser and return one receipt per case. Every
receipt repeats the authenticated nonce and both artifact identities. Evidence is written only after
the complete receipt set validates. The adapter must derive product and version from CDP or
WebDriver capabilities as allowed for that product; user-agent-only and simulated identity claims
are rejected. `chromium` cannot stand in for Chrome or Edge, and `webkit` cannot stand in for
Safari.

## Safari and minimum-version providers

Safari qualification requires a real Safari product on macOS and identity reported by Safari's
WebDriver capabilities. A WebKit build on another operating system is useful regression coverage,
not Safari evidence.

Minimum Chrome, Edge, and Firefox floors may be supplied by an internal browser lab, virtual
machine, or remote WebDriver/CDP service. No commercial service is normative. Whichever provider is
used must expose the actual product/version and operating system, run the complete catalog against
the current artifact challenge, and return a durable provider attestation. Current-stable targets
must request the provider's stable channel at execution time and record the resulting numeric
product version.

Result files are named after their matrix target, for example `results/firefox-minimum-128.json`. Do
not hand-author them. Provider attestations, screenshots, videos, and HAR files can contain
sensitive material; retain those outside the repository and place only the bounded evidence envelope
in `results/`.
