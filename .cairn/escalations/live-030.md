DECISION

Question:   live-protocol-bounds is Done and one backlog item remains; promote it as the next commitment?
Recommend:  Promote upload-file-provider-canceled-verification-and-removal-awaits-remain-exactly-retryable-can-hang-under-load: reproduce the hang with the store pause points under contention, make the cancellation waits bounded, and prove the test settles under load.
Because:    It is the only recorded defect left, it killed one repository gate run at the step timeout, and a cancellation await that can hang is a correctness question for the upload provider, not only a test flake.
If wrong:   The loop spends a commitment on a rare flake while no user-facing work is scheduled.
Instead:    Close the loop here with no current commitment, or name a new item for the specification.

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: LIVE-030
Raised: 2026-09-16T19:48:54.865Z
Raised after: LIVE-030=1
