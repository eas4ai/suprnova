DECISION

Question:   Building the debounce promotion found live:check proving a view it never checked: an empty {% call %}{% endcall %} to a macro that uses caller() renders zero branches, so nothing after it is checked and the component reads as proved; widen the promoted commitment to the checker proving what the runtime accepts?
Recommend:  One commitment, checker-proves-what-runtime-accepts: an empty call renders one empty caller branch and a view that renders no branches fails closed (new requirement, reproduced in crates/suprnova-live/tests with a live:click to an undeclared action proving clean); the debounce vocabulary shared by checker and runtime; and the 24 errors the fix exposes in the form gallery fixed (the validation summary's live:error names an action, the debounce 250 vs the field's 300, the group fields).
Because:    The false proof violates LIVE-008's own falsifier (an undeclared action reports clean), it hid the very debounce mismatch the promotion names, and every earlier ui-live-check receipt over the form gallery proved nothing; the debounce fix cannot be checked while the checker skips the view.
If wrong:   The commitment grows from a vocabulary fix to a checker soundness fix plus a form-gallery repair, about a day instead of hours, and the upload hang waits longer.
Instead:    Keep the promoted commitment to the debounce vocabulary alone and promote the false proof as its own commitment next, accepting that the debounce fix lands before live:check can verify it on the form gallery.

Reply: ok | instead | ask. If this isn't clear, ask me to explain it another way before you decide.

Concerns: LIVE-008
Raised: 2026-09-16T15:17:15.550Z
Raised after: LIVE-008=17
