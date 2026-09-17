# The select chevron is invisible in the dark color scheme

Surfaced from: UI-002
Captured: 2026-09-17T11:53:47.870Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. The base layer draws the select chevron as a data-URI SVG with stroke currentColor (crates/suprnova-live/browser/src/styles/suprnova-ui.css, the select rule). An SVG image does not inherit the document's color, so the stroke is black, and appearance: none removed the native arrow. Reproduced in Chromium with prefers-color-scheme dark: the chevron region's luminance spans 0 to 24 on the dark surface, against 0 to 255 in the light scheme.
