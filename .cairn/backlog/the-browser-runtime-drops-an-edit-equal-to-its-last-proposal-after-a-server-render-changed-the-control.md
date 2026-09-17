# The browser runtime drops an edit equal to its last proposal after a server render changed the control

Surfaced from: LIVE-031
Outside because: LIVE-031 requires the framework to answer an undecodable proposal with a field error and not run the action; whether the browser sends an edit again after a render replaced the control's value is the runtime's model state, which no requirement of this commitment names.
Captured: 2026-09-17T14:11:02.662Z
Promoted to: live-model-render-baseline

Found 2026-09-17 while proving the dogfood form gallery's reset (FORM-009). ModelState.propose in crates/suprnova-live/browser/src/models/state.ts compares an edit with browserProposal, the last value the browser proposed, and nothing reconciles browserProposal when a server render replaces the control's value. Sequence on /live/forms: clear the seat count (proposes null, refused with a field error, the island keeps 1), click Reset (the render shows 1), clear the count again: propose(quantity, null) reports unchanged, so no request is sent, the control shows an empty count beside no error while the island holds 1, and the mismatch only surfaces at the next submit. The same holds for any control whose render changes after a refused or superseded proposal. crates/suprnova-live/browser/e2e/app-dogfood-forms.spec.ts works around it by making a valid edit before the second empty one.
