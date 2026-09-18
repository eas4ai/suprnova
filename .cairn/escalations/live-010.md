DECISION

Question:   tooltip-dismissal is Done and pushed (origin/main 138080f3, gate green); the one backlog item is the upload file provider's retry tests, which fail about one whole-file run in fifteen and turned the Live gate red four times this evening. Promote it as the next commitment?
Recommend:  Promote it: make the provider's state reads deterministic under the controlled store's pause points, or serialize the tests that share it, and prove it with a repeated run of the whole file.
Because:    It is measured, not suspected: 2 of 30 consecutive runs failed on an idle machine, in two different tests, and the Live gate needed five attempts tonight, which costs about forty minutes of machine time per commitment.
If wrong:   A commitment goes to test timing while no user-facing work is scheduled, and 2.1.0 waits another loop.
Instead:    Remove the item and keep rerunning the gate, or cut 2.1.0 first and take this after.

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: LIVE-010
Raised: 2026-09-17T23:34:52.468Z
Raised after: LIVE-010=31
Answer: instead fix the fucking problem (Shawn, in chat, 2026-09-17 21:46 EDT): the flake is fixed, not handed back
Answered: 2026-09-18T01:47:15.694Z
Answered after: LIVE-010=31
Answered order: 17
