//! The violating example that binds `par-inertia-ssr` (PAR-057): removed
//! once the fail receipt is recorded.

#[test]
fn inssr_ssr_is_on_by_default() {
    assert!(
        suprnova::SsrConfig::default().enabled,
        "PAR-057: SSR is on by default, gated by bundle detection"
    );
}
