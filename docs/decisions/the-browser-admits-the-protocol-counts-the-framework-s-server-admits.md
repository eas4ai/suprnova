# The browser admits the protocol counts the framework's server admits

Level: Consequential
Decided by: Shawn
Promotes: a-live-form-with-more-than-eight-model-bound-controls-cannot-submit-and-the-runtime-drops-the-submit-with-only-a-diagnostic
Rests on: LIVE-028, LIVE-029, LIVE-030
Would be wrong if: the lower browser counts were a deliberate client-side bound some specification or threat model relies on, or a shipped host configures ProtocolLimits below 128

## Decision

Chosen 2026-09-16 on escalation live-027. The promotion recommended that a submit fit the protocol's proposal bound or fail visibly; building it showed the eight-proposal bound existed only in the browser's validators (8 proposals, operations, events, effects, extensions; 16 arguments and validation entries), while the framework configures ProtocolLimits at 128 for each and the browser scheduler already admits 128 proposals, and no Live specification sets the lower counts. So the bound a form must fit is the framework's, the browser is aligned to it, the checker enforces it for submit forms, and an oversized request fails as a resource limit. Rejected: keeping eight and only failing visibly, which leaves any form of more than seven fields unable to submit against a server that accepts them.

## Realized by
- dc031353  live: the browser admits the protocol counts the framework's server admits, the checker bounds a submit form, and a refused oversized request is a resource limit (LIVE-028 to LIVE-030)
