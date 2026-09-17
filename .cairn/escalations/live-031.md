DECISION

Question:   live-library-review-remediation is Done and pushed (origin/main 397559ca, gate green); one backlog item remains: after a server render replaces a control's value, retyping the value the browser last proposed sends nothing. Promote it as the next commitment?
Recommend:  Promote it as a small commitment: the browser treats a render that changes a bound control as the field's new baseline, so the next edit is always sent, with a dogfood case that clears the seat count, resets, and clears it again.
Because:    It is user-visible: the control shows an empty count with no error while the island still holds 1, and the mismatch only appears at the next submit; the reset case in the dogfood suite currently works around it.
If wrong:   A commitment goes to a rare sequence (a refused edit, a server replacement, then the same edit again) while 2.1.0 waits.
Instead:    Remove the backlog item and close the loop, or keep it in the backlog and cut 2.1.0 first.

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: LIVE-031
Raised: 2026-09-17T16:34:56.026Z
Raised after: LIVE-031=4
