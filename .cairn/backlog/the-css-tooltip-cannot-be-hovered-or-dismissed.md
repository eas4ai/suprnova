# The CSS tooltip cannot be hovered or dismissed

Surfaced from: OVL-002
Captured: 2026-09-17T11:53:47.933Z
Promoted to: live-library-review-remediation

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. crates/suprnova-live/components/tooltip/tooltip.css sets pointer-events none on the bubble and shows it only for :hover and :has(:focus-visible), with no Escape path and no delay. Live spec 23 (crates/suprnova-live/docs/specs/suprnova-live/23-overlay-and-disclosure-components.md) says tooltips respond to focus and hover with bounded delays and are dismissible, as WCAG 1.4.13 requires. Reproduced in Chromium: the bubble is visible with the pointer on the trigger and hidden once the pointer moves onto the bubble.
