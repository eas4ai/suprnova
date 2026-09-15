# A template's live:key never reaches the runtime, which reads data-suprnova-live-key

Surfaced from: OVL-006
Captured: 2026-09-15T00:36:53.574Z

The checker validates live:key (crates/suprnova-live/src/checker/html.rs, validate_keys) and the browser runtime lists key as a directive, but morph identity and morph controls read only data-suprnova-live-key (crates/suprnova-live/browser/src/morph/keys.ts, controls.ts, preserve.ts) and nothing on the server or in the runtime maps one to the other; the manual names live:key as a directive. A keyed morph control written as the checker requires has no effect at runtime. The overlay components write both attributes until the vocabulary is unified: live:key for the checker, data-suprnova-live-key for the runtime. Resolve by having the runtime accept live:key for identity (or the render pipeline emit the data attribute), with a checker fixture and a browser case proving a keyed control from a template.
