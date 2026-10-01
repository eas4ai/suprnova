# Media and Encoding variance negotiate for real -- promoted to iteration 006

Status: Promoted (iteration 006, 2026-09-08)
Captured: 2026-09-08
Promoted: 2026-09-08
Target domain: `16-cache-variance-privacy-and-stitching.md`

## What it is

A route declaring `Media` or `Encoding` variance SHALL partition its stored
representations by the negotiated media type or content coding, drawn from a
closed set the route declares. The key, the `Vary` header, and the stored
representation SHALL agree on that negotiated value. A negotiation result
outside the declared set SHALL fall back to the declared default rather than
creating a variant, so an attacker-controlled `Accept` header cannot multiply
entries.

The `Media` and `Encoding` variance dimensions are declarable and are keyed
today, but the middleware resolves both to constants before keying: `Media`
becomes `"text/html"` and `Encoding` becomes `"identity"`
(`framework/src/render_cache/middleware.rs`, near lines 989 to 999). Declaring
either dimension therefore partitions nothing at all, and a route that declares
both stores exactly the one representation it would have stored declaring
neither. The manual states this plainly rather than implying that negotiation
already works.

## Acceptance criteria

- A route declaring `Media` or `Encoding` variance SHALL partition its
  representations by the negotiated media type or content coding drawn from a
  closed declared set.
- The key, the `Vary` header, and the stored representation SHALL agree; a
  negotiation result outside the declared set SHALL fall back to the declared
  default rather than creating a variant.
- Tests SHALL prove that two negotiations yield two representations and that a
  variant is never served to a request that did not negotiate it.
- The representations chapter SHALL replace the constants statement.
