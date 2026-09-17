# The tooltip cannot be dismissed without moving the pointer or focus while OVL-002 keeps it CSS only

Changes: OVL-002
Captured: 2026-09-17T12:00:49.225Z
Promoted to: tooltip-dismissal

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0, and split from the backlog item the-css-tooltip-cannot-be-hovered-or-dismissed when it was promoted into live-library-review-remediation, which builds the hoverable half as OVL-007. Live spec 23 (crates/suprnova-live/docs/specs/suprnova-live/23-overlay-and-disclosure-components.md) says tooltips respond to focus and hover with bounded delays and are dismissible, and WCAG 2.2 success criterion 1.4.13 requires a way to dismiss content that appears on hover or focus without moving the pointer or focus, unless it obscures nothing. Dismissal with Escape needs script or a declarative popover hint that the qualified engines do not all support, while OVL-002 requires the tooltip to be CSS only. A specification phase decides between a script enhancement, a native popover hint once every qualified engine supports it, or a tooltip placement that never obscures content.
