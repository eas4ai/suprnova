# The combobox never suppresses a listbox rendered for an older query, and its stale-result mechanism tests a path the shipped macro never takes

Surfaced from: FORM-008
Captured: 2026-09-17T11:53:47.635Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. crates/suprnova-live/components/combobox/combobox.js applies acceptsResults only when the listbox carries data-sn-remote, which combobox.html never renders and nothing sets; crates/suprnova-live/browser/tests/combobox-stale-results.test.ts proves acceptsResults and QuerySequence in isolation, and the element never calls QuerySequence.accepts. The view's own comment says the element refuses an older listbox. Reproduced: a listbox rendered with data-sn-query "ca" is shown while the input says "c".
