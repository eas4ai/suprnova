DECISION

Question:   The live-native review records two open findings on defects outside the commitment, and LOOP-033 blocks Done until they are resolved: (1) the runtime sends a model proposal queued behind an in-flight action with the revision and snapshot captured when it was queued, and only on the next stream tick, so the server answers 409 and the document reloads; (2) the session middleware writes concurrent requests back last-writer-wins, so a flash set by a redirect is lost to a request that started earlier. How should they be resolved?
Recommend:  Fix (1) now under this commitment: the scheduler takes the island's authority when it sends and flushes when the in-flight response commits, proved by a browser case that types while a proposal is in flight. For (2), write an Agreed requirement for per-session request serialization (the cache lock driver, Laravel's session blocking, opt-in per route or global) and name it in a new commitment after this one, and resolve the review finding by that reference.
Because:    (1) is a bounded runtime defect the family's own flow exposes and the Live gate can prove; (2) is a framework-wide design choice with multi-node consequences that deserves its own requirement, mechanism and commitment rather than a fix folded into a component review.
If wrong:   Both stay open and the roadmap does not complete until both are fixed inside this commitment, or (1) is also deferred and the review stays open.
Instead:    fix both now in this commitment | defer both to the backlog and resolve the findings by reference | another split

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: FORM-008
Status: open
Raised: 2026-09-15T18:44:42.770Z
Raised after: FORM-008=3
Answer: ok
Answered: 2026-09-15T18:45:50.972Z
Answered after: FORM-008=3
Answered order: 9
