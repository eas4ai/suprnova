DECISION

Question:   Should a plain Auth::logout, which clears authentication but keeps the session row, also end the Live streams that session opened, given that the scaffold's logout controller calls it and today those streams keep delivering to the logged-out browser?
Recommend:  Yes, in the framework: treat a session losing its authenticated user as revocation for its identity-bound memberships, so plain logout and invalidation both end streams, and leave the scaffold's controller as it is.
Because:    A user who clicks the scaffold's logout expects their streams to stop; fixing only the scaffold template leaves every existing application on plain logout with the leak, while the framework hook covers them all.
If wrong:   A session that switches users without logging out loses its streams at the switch and reconnects, and one more small requirement joins this work before the library resumes.
Instead:    Change the scaffold's logout controller to Auth::logout_and_invalidate and document that plain logout keeps streams alive; existing applications keep the leak until they change their own controller.

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: LIVE-019
Status: open
Raised: 2026-09-14T04:05:34.130Z
Raised after: LIVE-019=2
