# A render that changes a bound control becomes the baseline its next edit is compared with

Level: Consequential
Decided by: developer
Promotes: the-browser-runtime-drops-an-edit-equal-to-its-last-proposal-after-a-server-render-changed-the-control
Rests on: LIVE-031 FORM-009
Would be wrong if: the edit a render replaced is already sent again by the runtime on some path the backlog item missed, or the fix needs Live spec 11's Agreed text changed, which belongs to a specification phase

## Decision

Chosen 2026-09-17 by the developer on escalation live-031 (ok), after live-library-review-remediation reached Done and was pushed at 397559ca. The browser runtime compares each model edit with the value it last proposed (ModelState.propose in crates/suprnova-live/browser/src/models/state.ts), and nothing moves that value when a render gives the control a different one: after a refused empty seat count and a reset that renders 1, clearing the count again sends nothing, and the control shows an empty count with no error while the island holds 1. One commitment, live-model-render-baseline, drafts LIVE-037: the runtime sends an edit whose value differs from what its control held after the island's last applied render. Its falsifier is the dogfood sequence itself. Rejected: removing the backlog item, which leaves a control and its island silently out of step; and cutting 2.1.0 first, which ships the mismatch.

## Realized by

(none yet: recorded, not built)
