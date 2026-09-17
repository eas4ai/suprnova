# Element ids inside an island are validated by the runtime and never by the checker

Surfaced from: LIVE-025
Captured: 2026-09-17T11:53:47.776Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. crates/suprnova-live/browser/src/morph/keys.ts runs the SAFE_KEY check on every id inside an island before a morph, while crates/suprnova-live/src/checker/html.rs only records ids for teleport targets. Reproduced: the runtime identity planner refuses id "_top" and id "user[email]" with morph_identity_key_invalid, so a view that passes live:check disconnects its island at the first morph. Predates 2.0.2.
