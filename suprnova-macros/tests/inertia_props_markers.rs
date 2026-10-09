//! PAR-068: `#[derive(InertiaProps)]` takes the two markers
//! `suprnova generate-types` reads, `#[inertia_props(shared)]` and
//! `#[inertia_props(flash)]`, and refuses any other use of the attribute.

#[test]
fn intt_inertia_props_markers_compile_and_wrong_markers_fail_the_build() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/inertia-props/pass/*.rs");
    cases.compile_fail("tests/ui/inertia-props/fail/*.rs");
}
