# One data-derived key with a byte outside the key alphabet fails the whole island render

Surfaced from: LIVE-024
Captured: 2026-09-17T11:53:47.748Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. The live_key filter (crates/suprnova-live/src/view/live_key.rs) returns an Askama error for any byte outside ASCII letters, digits, _, -, . and :, so a datatable row, list item, feed item, toast or combobox option keyed by an email address, a name with a space or a non-ASCII value makes the island unrenderable for every viewer of that data. manual/live.md says loop keys pass through live_key, but not that a refused key fails the render or how to derive a key that always passes.
