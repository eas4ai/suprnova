# A model proposal queued behind an in-flight action is sent with the revision and snapshot captured when it was queued

Surfaced from: FORM-008
Captured: 2026-09-15T15:13:28.021Z

Observed on the live-native gallery (2026-09-15): typing into a debounced model input while its previous proposal was still in flight queued the next proposal; the runtime later sent it with the earlier base revision and the consumed snapshot, the server answered 409 signature_invalid with refresh_island, and the runtime reloaded the document. The queued send also waited for the next stream tick rather than the in-flight response. Expected: a queued intent takes the island's authority at send time and flushes when the in-flight response commits. The dogfood case now waits for the round-trip's results before selecting, which is the intended FORM-008 flow, so the race is not covered by a mechanism.
