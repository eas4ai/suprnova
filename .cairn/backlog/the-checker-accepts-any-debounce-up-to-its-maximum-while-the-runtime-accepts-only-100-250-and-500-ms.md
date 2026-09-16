# The checker accepts any debounce up to its maximum while the runtime accepts only 100, 250 and 500 ms

Surfaced from: LIVE-024
Outside because: LIVE-024 names the stable-key vocabulary; the debounce modifier set is the model timing grammar (Live spec 03 and the generated directive contract), which no requirement in this commitment covers.
Captured: 2026-09-16T13:47:19.247Z

Found 2026-09-15 building live-key-vocabulary: suprnova.search-input wrote live:model.debounce.300ms. live:check proved it, because crates/suprnova-live/src/checker/directive.rs accepts any debounce.<n>ms with 0 < n <= MAX_DEBOUNCE_MILLIS (crates/suprnova-live/src/state/timing.rs), while the browser runtime's parseModelTiming (crates/suprnova-live/browser/src/models/timing.ts) and the generated directive contract list only debounce.100ms, 250ms and 500ms and throw model_timing_invalid for anything else. The component now uses 250ms; the checker and the runtime still disagree on the vocabulary, the same class of defect as LIVE-024.
