#[test]
fn external_plugins_cannot_mint_sessions_or_access_issuer() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/plugin_cannot_issue_session.rs");
}
