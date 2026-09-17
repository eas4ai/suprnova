# A model proposal that fails to decode is dropped silently and the action reports success

Surfaced from: unstated
Captured: 2026-09-17T11:53:47.541Z
Promoted to: live-library-review-remediation

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. suprnova-macros/src/live/component.rs binds each apply_optional and apply_required result to `_application` and discards it, and framework/src/live/action.rs prepares the ProposalBatch without reading batch.issues(). Live spec 03 (crates/suprnova-live/docs/specs/suprnova-live/03-component-state-and-binding.md) requires an invalid conversion to produce a field-level binding error and says an action cannot silently consume a fallback value. Reproduced on the dogfood form gallery: model proposals quantity null (a u64 field) and topics false (a Vec<String> field) returned outcome accepted with empty validation and the old state. A user who clears a number field and saves keeps the old number and sees success. The discarded result predates 2.0.2.
