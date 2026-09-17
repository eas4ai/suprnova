# A toast times out under the pointer and can hide while its dismiss button has focus

Surfaced from: FDB-004
Captured: 2026-09-17T11:53:47.663Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. crates/suprnova-live/components/toast/toast.js pauses on a captured pointerenter and resumes on a captured pointerleave from any descendant, so moving from the toast's text to its padding resumes the timers with the pointer still inside, and a pointer leaving while focus stays on the dismiss button resumes them too, hiding the toast and dropping focus. Reproduced in Chromium: with the pointer kept on the text the toast stays; moved from the text to the padding, it is dismissed while it still matches :hover.
