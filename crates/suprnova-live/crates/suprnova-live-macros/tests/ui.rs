//! Compile-time contract tests for Live component authoring macros.

#[test]
fn live_component_authoring_contract() {
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/pass/*.rs");
    tests.compile_fail("tests/ui/fail/*.rs");
}
