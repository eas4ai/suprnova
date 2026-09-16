# The runtime reads live:key, the checker's vocabulary, for morph identity

Level: Consequential
Decided by: Shawn
Promotes: a-template-s-live-key-never-reaches-the-runtime-which-reads-data-suprnova-live-key
Rests on: LIVE-024, OVL-006
Would be wrong if: a template's live:key still has no effect on morph identity, or an element carrying both attributes with different values morphs under one of them silently

## Decision

Chosen 2026-09-15 on escalation ovl-006 after session-request-serialization reached Done with no roadmap item left. The checker validates live:key and the manual names it as a directive, but the browser runtime's morph identity, controls and preservation read only data-suprnova-live-key, so every library component writes the key twice and a user's keyed control has no effect. The runtime reads live:key as the stable key; data-suprnova-live-key stays the engine's spelling on roots the engine renders; both present and disagreeing fails morph validation. Rejected: teaching the checker the data attribute (it would leave the manual's vocabulary dead) and emitting the data attribute from the render pipeline (a second spelling in every document for one reader). Alternatives left in the backlog: the upload provider's cancellation hang under load; the form gallery's .prevent rides along here.

## Realized by

- 159dd219  live: the runtime reads live:key for morph identity, controls and preservation, and the library writes the key once (LIVE-024)
