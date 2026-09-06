# Cached segments should be able to contain cached segments -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-09-06
Target domain: `16-cache-variance-privacy-and-stitching.md`

## What it is

Composite stitching caches exactly one level. A `PublicShellStitched` route
stores a shared shell with typed slots in it, and every slot is re-rendered
on every hit; nothing inside a slot is cached, and a slot's island cannot
itself be a shell with slots of its own. The graph is flat by construction:
`Segment` has a `Slot` variant whose output is supplied by the host, never a
segment graph of its own, and `SegmentGraph::validate` proves the literal
segments partition one shell.

Two things that a real application wants therefore have no expression. The
first is a fragment that is expensive, identical for a large group, and
appears inside a part of the page that varies per principal: today it is
re-rendered inside its island on every hit, because the only thing the cache
can hold is the outermost shell. The second is a fragment shared by several
documents, such as a navigation tree or a pricing table that appears on
twenty routes: today each route caches its own copy inside its own shell, so
one change invalidates twenty entries and each is rebuilt separately.

An inner cached segment would need an identity and a version of its own,
independent of any document that includes it, so that one stored copy can be
included from many shells and invalidated once. It would need ownership to
stay acyclic and bounded in depth, so assembly still terminates and its cost
is still known from typed facts before a byte is copied, the way
`assembled_len` knows a flat graph's cost today. It would need a privacy
rule that composes: an inner segment SHALL NOT be servable under a policy
wider than the one the including document was published under, so no
composition can widen sharing by nesting. And it would need a framework API
for declaring an inner segment - the counterpart of `LiveMount` and
`LiveMount::on_stitch_failure` for something that is not an island - because
today the only thing that can produce a slot is an identity-bound Live
mount.

This note captures the gap. It proposes no design: whether an inner segment
is a second entry kind, a recursive `Segment` variant, or an entry the
assembler resolves through the store is exactly the question the next
iteration would answer.

## Acceptance criteria

- A cached segment has an identity and a stored version that no including
  document owns, so one stored copy MAY be included from several documents
  and is invalidated once rather than once per including route.
- Ownership is acyclic and depth-bounded by a named constant, and a graph
  that would exceed the depth bound, or that would include itself directly
  or transitively, is declined at publication rather than at assembly.
- The exact assembled length of a nested graph is computable from typed
  facts before any byte is copied, so the body bound is enforced before
  allocation exactly as it is for a flat graph today.
- An inner segment SHALL NOT be served under a representation class wider,
  or a freshness window longer, than the document including it; a policy
  that would relax its parent is refused where it is declared.
- Every inner segment is reauthorized per request in the same way an island
  slot is, or is provably identity-free; a nested inclusion never skips a
  check that the same content would have had at the top level.
- A failure inside an inner segment resolves through a declared policy with
  the same closed set of outcomes a slot has today (`fail_document`, `omit`,
  `fallback`), and a failure that reaches the outermost document leaves the
  route serving its own uncached render.
- The framework offers one typed way to declare an inner cached segment and
  its policy, and a declaration that names a segment the running build no
  longer has is drift: it fails the segment, never substitutes another.
- Telemetry distinguishes an inner segment's outcomes from an island slot's,
  under closed low-cardinality labels, so an operator can tell which level
  of a nested document actually assembled.
- The conformance corpus carries a nested case, and the cross-language
  fixtures reject an unknown nesting depth or an unknown segment kind rather
  than ignoring it.
