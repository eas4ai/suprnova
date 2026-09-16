# A declared debounce is limited to the grammar's durations, and the checker renders an empty caller as empty content

Level: Consequential
Decided by: Shawn
Promotes: the-checker-accepts-any-debounce-up-to-its-maximum-while-the-runtime-accepts-only-100-250-and-500-ms
Rests on: LIVE-025, LIVE-026, LIVE-027
Would be wrong if: an application needs a debounce the grammar does not list, or a component the fixed checker proves still references something the runtime refuses

## Decision

Chosen 2026-09-16 on escalations live-024 and live-008. The backlog record said the checker accepts any debounce; the build showed the checker's grammar is closed like the runtime's (100, 250, 500 ms) and the only wider vocabulary was the model macro and BindingTiming (1 to 60000 ms), so a field declaring 300 ms could never be bound by a template the checker accepts. The form gallery's 300 ms looked accepted only because an empty call block to the validation summary macro made the checker render zero branches and prove the view unchecked. Rejected for the debounce: widening the grammar and runtime to any duration, which would reopen the reviewed v4 directive fixture and its generated contracts for a vocabulary no shipped component needs. The live:error target follows the runtime, which resolves a field, the island, or an action, the same resolution 2dc59f5a gave the loading and success directives.

## Realized by

(none yet: recorded, not built)
