#[test]
fn auth_drift_review_records_sources_result_and_architecture_changes() {
    let review = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../docs/integration-auth-drift-review.md"),
    )
    .expect("auth drift review must exist");
    for required in [
        "27f7ddf4bb6c523c4ffa42fa12e4a568a7990f88",
        "968b0be66b1d49f60a2bcb1ab28b5f1b93fa3a5d",
        "4f13499097eece1f445236ad10572e7d4ed91183",
        "crates/suprnova-magnetar",
        "magnetar_integration",
        "No Torii package remains",
    ] {
        assert!(
            review.contains(required),
            "drift review is missing: {required}"
        );
    }
}
