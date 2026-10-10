//! The concurrency facade's test binary (PAR-189, PAR-190): the manual
//! chapter the commitment delivers is linked from the documentation index.

#[test]
fn the_concurrency_chapter_is_linked_from_the_documentation_index() {
    let index = std::fs::read_to_string("../manual/documentation.md").expect("the documentation index");
    assert!(
        index.contains("concurrency.md"),
        "manual/documentation.md links the concurrency chapter, as every subsystem's chapter is linked"
    );
}
