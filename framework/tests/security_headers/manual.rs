//! The manual is part of the security contract, not an optional appendix.

#[test]
fn sec_006_manual_names_every_header_preset_nonces_and_divergence() {
    let manual = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../manual/security-headers.md"
    ))
    .expect("security headers chapter");
    for required in [
        "SecurityHeaders",
        "strict",
        "balanced",
        "relaxed",
        "Content-Security-Policy",
        "Content-Security-Policy-Report-Only",
        "Strict-Transport-Security",
        "X-Frame-Options",
        "X-Content-Type-Options",
        "Referrer-Policy",
        "Permissions-Policy",
        "Cross-Origin-Opener-Policy",
        "Cross-Origin-Embedder-Policy",
        "Cross-Origin-Resource-Policy",
        "Reporting-Endpoints",
        "nonce",
        "Complete",
        "### Why Suprnova diverges",
        "FrameGuard",
        "Vite::useCspNonce",
        "spatie/laravel-csp",
        "## Next",
    ] {
        assert!(manual.contains(required), "missing {required}");
    }
}
