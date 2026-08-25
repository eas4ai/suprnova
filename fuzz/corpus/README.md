# Suprnova Live fuzz corpus

Protocol and snapshot targets start from libFuzzer's empty input. Browser
directive and metadata targets also carry small named v3 success/failure seeds
so a smoke campaign immediately crosses the reviewed grammar and engine-emitted
metadata shapes. The v4 media-header target carries named truncated PNG/GIF/WebP
and malformed JPEG-marker inputs so every smoke campaign enters the bounded
format-specific parser paths. Every campaign remains deterministic under a fixed `-seed`.
Any crashing, timing-out, or otherwise interesting input discovered by a longer
campaign must be minimized with `cargo fuzz cmin`, copied into the matching
target corpus directory, and added as a small named case in
`tests/fuzz_regressions.rs` before the fix lands.

All targets deliberately use strict tiny limits and fixed public non-production
keys. They exercise only boundary classification and must never dispatch
application code.
