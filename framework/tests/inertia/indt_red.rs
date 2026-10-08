//! Temporary violating example for rebinding PAR-071 and PAR-074 after their rewording: names a type that does not exist, so the inertia binary does not compile.
#[test]
fn indt_red_rebinding_example() {
    let _config: suprnova::DevToolsConfigThatDoesNotExist = unreachable!();
}
