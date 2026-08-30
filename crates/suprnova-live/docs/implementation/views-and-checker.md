# Live views and checked Askama templates

Iteration 002 makes Askama 0.16 the normative checked external-template
substrate behind the framework-owned view abstraction. Component and handler
contracts do not expose an interchangeable unchecked renderer.

## Rendering contract

Askama templates implement the sealed `ViewTemplate` contract and render into a
framework-owned bounded buffer. Success exists only after the complete output,
assets, mounts, children, metadata, and byte limits validate. A render error
cannot publish partial HTML.

Document and island authority are deliberately different:

| Result | Authority |
| --- | --- |
| `DocumentRender` | Complete HTML bytes, typed status/header/cache/media intent, assets, and inert initial mount metadata |
| `IslandRender` | One complete island boundary, assets, and independently owned children |

An island has no status, header, cookie, cache, redirect, or media-type field.
The engine owns and escapes the island wrapper and accepts exactly one bounded
root. Mount metadata contains validated component/slot identity and signed data,
never executable script or handler attributes.

`CanonicalDocumentConformance` proves GET, HEAD, conditional validator, body
suppression, and representation-length behavior. It is a standalone
conformance adapter, not a registered Suprnova route.

## Askama checker

`TemplateChecker` consumes the immutable component registry and a bounded
template catalog. `askama_parser` supplies the normative AST and spans for
expressions, `if`, `match`, loops, includes, blocks, macros, and inheritance;
`html5ever` tokenizes every reachable HTML branch. The checker joins branch
stack state, enforces nesting/attribute/token/source/diagnostic limits, and
reports stable source-oriented diagnostic codes.

The closed directive grammar checks registered action, model, validation,
event, effect, URL, nested-component, and stable-key identities. It also checks
binding timing modifiers and accessibility constraints. Dynamic tag names,
attribute names, or identity-bearing values are reported as unproved instead of
receiving false static proof. Repeated children require stable checked keys and
cannot reach into an ancestor component's actions or state.

The checker recognizes server-visible local-signal and browser directives only
as grammar. Iteration 002 does not implement timers, history APIs, DOM morphing,
Stimulus controllers, or effect execution.

## Trusted markup and escaping

Ordinary Askama interpolation uses HTML escaping. The checker rejects raw
Askama `safe`, including qualified or whitespace-varied forms. Deliberately
unescaped HTML must be a bounded `TrustedHtml` created from compile-time
framework markup or output from a `RegisteredSanitizer`. Both paths require an
explicit bounded `TrustedMarkupReason`; sanitizer output also carries a stable
sanitizer identity.

Only the Suprnova-owned `trusted_html` Askama filter can unwrap `TrustedHtml`.
There is no `String` conversion or generic application-provided view renderer
that bypasses this boundary. Debug output exposes provenance class only, never
trusted markup bytes.

## Failure and recovery

Askama syntax/data failures, HTML parse or branch-stack failures, raw `safe`,
missing templates, unknown identities, invalid keys/directives, executable
mount metadata, multiple roots, and configured limit violations fail before a
successful render is constructed. Diagnostics contain registered identities,
view locations, and closed codes, not state, proposals, arguments, signed
snapshots, or rendered payloads.

On an action render failure, the action coordinator follows the accepted
authority rules in [actions and validation](actions-and-validation.md). A
browser morph failure after server acceptance follows the fresh-render path in
[protocol v2](protocol-v2.md); it never retries the original action.
