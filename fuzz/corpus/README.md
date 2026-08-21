# Suprnova Live fuzz corpus

Each iteration 001 fuzz target starts from libFuzzer's empty input and remains
deterministic under a fixed `-seed`. Any crashing, timing-out, or otherwise
interesting input discovered by a longer campaign must be minimized with
`cargo fuzz cmin`, copied into the matching target corpus directory, and added
as a small named case in `tests/fuzz_regressions.rs` before the fix lands.

The four targets deliberately use strict tiny limits and fixed public
non-production keys. They exercise only boundary classification and must never
dispatch application code.
