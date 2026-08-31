//! Compile-time contracts for Live component authoring through `suprnova::live`.

#[test]
fn live_component_authoring_contract() {
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/live/pass/*.rs");
    tests.compile_fail("tests/ui/live/fail/*.rs");
}
