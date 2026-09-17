# Clicking an inner tab of nested local tabs hides the outer panel that holds it

Surfaced from: NAV-002
Captured: 2026-09-17T11:53:47.842Z
Promoted to: live-library-review-remediation

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. crates/suprnova-live/components/tabs/tabs.js selects every descendant role=tab and handles clicks and keys that bubble from an inner sn-tabs, so the outer element's selection runs for the inner tab. Reproduced in Chromium: clicking the second inner tab set every outer tab to aria-selected false and hid the outer panel that contains the inner tabs. Arrow keys index across both tab sets.
