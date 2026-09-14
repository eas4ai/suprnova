DECISION

Question:   The LIVE-022 baseline (.cairn/evidence/LIVE-022/20260914T132842262Z) shows a second defect beside the 403s: 1 of 512 admitted issuances answered 503 async_unavailable because AsyncState.constructing is one shared slot (framework/src/live/async_updates.rs:1075-1100) that a concurrent issuance overwrites between set and read, so the membership registry (line 2260) rejects the envelope context. Does this race fold into live-issuance-credentials as a new requirement, or go to the backlog with the LIVE-022 probe narrowed to no 403?
Recommend:  Fold it: add LIVE-023 (an issuance's envelope context is constructed from that issuance's own claims under concurrency) to this commitment, checked by the same probe, and fix it by keying the constructing claims by subscription id instead of one Option.
Because:    The requirement you confirmed is that every admitted issuance answers with a usable subscription, and the probe asserts exactly that; a narrowed probe would pass while 1 in 512 admitted requests still fails, and the fix is a few lines in the same file.
If wrong:   The commitment grows by one requirement and one small change in async_updates.rs; the credential fix itself is unchanged either way.
Instead:    Record the race with cairn backlog, narrow the probe to assert no 403, and close the commitment on the credential fix alone.

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: LIVE-022
Status: open
Raised: 2026-09-14T13:30:02.181Z
Raised after: LIVE-022=1
