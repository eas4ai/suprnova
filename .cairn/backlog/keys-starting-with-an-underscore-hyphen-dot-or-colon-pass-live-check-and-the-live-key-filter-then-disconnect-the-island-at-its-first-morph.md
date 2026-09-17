# Keys starting with an underscore, hyphen, dot or colon pass live:check and the live_key filter, then disconnect the island at its first morph

Surfaced from: LIVE-024
Captured: 2026-09-17T11:53:47.574Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. The checker (crates/suprnova-live/src/checker/html.rs, validate_keys) and check_live_key (crates/suprnova-live/src/view/live_key.rs) accept any key of ASCII letters, digits, _, -, . and :, while the runtime's SAFE_KEY (crates/suprnova-live/browser/src/morph/keys.ts, controls.ts, teleport.ts and uploads/morph.ts) requires a letter or digit first. scanOwnedTree validates every key before a morph and throws morph_identity_key_invalid; recovery requests a fresh render, which fails the same way, and the island disconnects. Reproduced: check_live_key accepts "-1", "_draft", ".x" and ":flash"; the runtime identity planner refuses "-1" and "_draft" and plans "a-1". A datatable keyed by a negative id is enough.
