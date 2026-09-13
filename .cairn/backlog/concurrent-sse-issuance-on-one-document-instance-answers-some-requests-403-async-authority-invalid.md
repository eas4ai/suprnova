# Concurrent SSE issuance on one document instance answers some requests 403 async_authority_invalid

Surfaced from: LIVE-018
Captured: 2026-09-13T21:26:58.202Z

Observed 2026-09-13 while porting the audit's ASTRA-07 probe (LIVE-018): with 513 concurrent issuances for one scope and one SSE document instance held at a delayed authorizer, 39 of the 512 admitted requests answered 403 with error async_authority_invalid (AuthorityInvalid: ScopeMismatch or InvalidCredential from the subscription service's connect step), the rest 201. The per-scope cap itself holds (exactly one 409). Reproduce with framework/tests/live/hardening.rs issuance_cap_holds_under_concurrency and inspect its status histogram. Not part of the hardening commitment; a finding for the owner.
