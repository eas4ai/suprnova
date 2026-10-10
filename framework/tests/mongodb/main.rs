//! The MongoDB backend's test binary (PAR-182 to PAR-188): the manual chapter
//! the commitment delivers is linked from the documentation index.

#[test]
fn the_mongodb_chapter_is_linked_from_the_documentation_index() {
    let index = std::fs::read_to_string("../manual/documentation.md").expect("the documentation index");
    assert!(
        index.contains("mongodb.md"),
        "manual/documentation.md links the MongoDB chapter, as every subsystem's chapter is linked"
    );
}
