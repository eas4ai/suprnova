DECISION

Question:   Should LIVE-016's session and revocation clause be built now, so a user who logs out or whose session is revoked stops receiving async Live events, or should the requirement be narrowed to the Gate re-check that shipped?
Recommend:  Build it before the library resumes, as one follow-up commitment: store a session identity on each membership, add a revocation version the host bumps on logout or credential change, and retire memberships whose session no longer holds.
Because:    ASTRA-01 was about delivery outliving authorization; the shipped Gate re-check covers policy changes, but a user who logs out today keeps receiving async updates until the stream itself closes, which is the same class of leak.
If wrong:   The library waits one more commitment and the host carries a revocation version it may not otherwise need.
Instead:    Supersede LIVE-016 to cover the Gate re-check alone and capture session binding in the backlog for a later commitment; the current receipt then fully satisfies it.

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: LIVE-016
Status: open
Raised: 2026-09-14T01:16:27.311Z
Raised after: LIVE-016=3
Answer: ok
Answered: 2026-09-14T01:44:21.956Z
Answered after: LIVE-016=3
Answered order: 1
