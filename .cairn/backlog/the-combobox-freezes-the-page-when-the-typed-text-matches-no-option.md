# The combobox freezes the page when the typed text matches no option

Surfaced from: FORM-008
Captured: 2026-09-17T11:53:47.475Z
Promoted to: live-library-review-remediation

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. crates/suprnova-live/components/combobox/combobox.js observes its listbox's hidden attribute with a MutationObserver, and render() writes listbox.hidden again through setExpanded(false). Setting an attribute queues a mutation record even when the value does not change, so while the input has focus and no option matches, the observer re-renders inside microtasks forever. Reproduced in Chromium with one sn-combobox that never moved: typing "zz", or dispatching one input event with a value no option contains, left the renderer unresponsive (page.evaluate(() => 1) timed out after 3 s), while "ca" stayed responsive. A moved combobox also froze on Enter. The dogfood browser case only types a matching query.
