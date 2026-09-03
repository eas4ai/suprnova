//! Typed dependency identities, bounded observation, and coherence checks.

use suprnova_live::render_cache::generation::{
    CoherenceCheck, DependencyIdentity, GenerationLedger, MAX_OBSERVATIONS, MemoryGenerationLedger,
    ObservationWindow,
};

fn users_table() -> DependencyIdentity {
    DependencyIdentity::table("users")
}
fn user_7() -> DependencyIdentity {
    DependencyIdentity::record("users", b"7")
}

#[tokio::test]
async fn identities_are_typed_versioned_and_digest_stably() {
    assert_eq!(
        DependencyIdentity::table("users").digest(),
        users_table().digest()
    );
    assert_ne!(users_table().digest(), user_7().digest());
    assert_ne!(
        user_7().digest(),
        DependencyIdentity::record("users", b"8").digest()
    );
    assert_ne!(
        DependencyIdentity::query_class("users", "active").digest(),
        DependencyIdentity::query_class("users", "all").digest()
    );
    assert_eq!(
        DependencyIdentity::broad().digest(),
        DependencyIdentity::broad().digest()
    );
    assert!(
        DependencyIdentity::try_table("").is_err(),
        "names are bounded and non-empty"
    );
    assert!(DependencyIdentity::try_table(&"t".repeat(129)).is_err());
    assert!(
        DependencyIdentity::try_record("users", b"").is_err(),
        "an empty record key is rejected"
    );
    assert!(
        DependencyIdentity::try_record("users", &vec![0_u8; 513]).is_err(),
        "a record key past the maximum length is rejected"
    );
    assert!(
        DependencyIdentity::try_query_class("users", &"c".repeat(129)).is_err(),
        "an over-long query class name is rejected"
    );
    assert!(
        DependencyIdentity::try_config(&"c".repeat(129)).is_err(),
        "an over-long config name is rejected"
    );
    assert!(
        DependencyIdentity::try_feature(&"f".repeat(129)).is_err(),
        "an over-long feature name is rejected"
    );
}

#[tokio::test]
async fn a_memory_ledger_advances_only_committed_identities_and_the_broad_authority() {
    let ledger = MemoryGenerationLedger::new();
    let before = ledger
        .current(&[users_table().digest(), user_7().digest()])
        .await
        .expect("current");
    assert_eq!(before.get(&users_table()), Some(0));
    ledger.advance(&[user_7()]).await.expect("advance");
    let after = ledger
        .current(&[users_table().digest(), user_7().digest()])
        .await
        .expect("current");
    assert_eq!(after.get(&user_7()), Some(1));
    assert_eq!(
        after.get(&users_table()),
        Some(0),
        "advancing a record does not advance its table"
    );
    ledger
        .advance(&[DependencyIdentity::broad()])
        .await
        .expect("broad");
    assert_eq!(
        ledger
            .current(&[DependencyIdentity::broad().digest()])
            .await
            .expect("c")
            .get(&DependencyIdentity::broad()),
        Some(1)
    );
    assert_eq!(ledger.epoch().await.expect("epoch"), 1);
}

#[tokio::test]
async fn an_observation_window_detects_any_moved_generation() {
    let ledger = MemoryGenerationLedger::new();
    let mut window = ObservationWindow::open(1);
    window.observe(users_table()).expect("observe");
    window.observe(user_7()).expect("observe");
    window
        .observe(user_7())
        .expect("duplicate observation is idempotent");
    let window_epoch = window.epoch();
    let observed = window.close(&ledger).await.expect("close");
    assert_eq!(observed.len(), 3, "the broad authority is always observed");
    ledger.advance(&[user_7()]).await.expect("advance");
    let current = ledger.current(&observed.digests()).await.expect("current");
    let current_epoch = ledger.epoch().await.expect("epoch");
    match CoherenceCheck::compare(&observed, &current, current_epoch, window_epoch) {
        CoherenceCheck::Moved(moved) => assert_eq!(moved, vec![user_7().digest()]),
        CoherenceCheck::Coherent => panic!("a moved generation must be visible"),
    }
    let mut full = ObservationWindow::open(1);
    // The broad authority seeded by `open` already occupies one slot, so
    // only `MAX_OBSERVATIONS - 1` more distinct identities fit before the
    // window holds exactly `MAX_OBSERVATIONS` in total.
    for index in 0..(MAX_OBSERVATIONS - 1) {
        full.observe(DependencyIdentity::record(
            "t",
            index.to_string().as_bytes(),
        ))
        .expect("within bound");
    }
    assert!(
        full.observe(DependencyIdentity::record("t", b"overflow"))
            .is_err(),
        "observations are bounded"
    );
}

#[tokio::test]
async fn an_epoch_change_reports_the_broad_authority_moved_with_no_generation_change() {
    let ledger = MemoryGenerationLedger::new();
    let window = ObservationWindow::open(ledger.epoch().await.expect("epoch"));
    let window_epoch = window.epoch();
    let observed = window.close(&ledger).await.expect("close");
    ledger.advance_epoch();
    let current = ledger.current(&observed.digests()).await.expect("current");
    let current_epoch = ledger.epoch().await.expect("epoch");
    assert_ne!(
        current_epoch, window_epoch,
        "the ledger epoch must actually have advanced"
    );
    match CoherenceCheck::compare(&observed, &current, current_epoch, window_epoch) {
        CoherenceCheck::Moved(moved) => {
            assert_eq!(
                moved,
                vec![DependencyIdentity::broad().digest()],
                "an epoch change alone reports only the broad authority as moved"
            );
        }
        CoherenceCheck::Coherent => panic!("an epoch change must be visible"),
    }
}

#[tokio::test]
async fn a_full_observation_window_closes_to_exactly_the_bound() {
    let ledger = MemoryGenerationLedger::new();
    let mut full = ObservationWindow::open(1);
    // The broad authority seeded by `open` already occupies one slot, so
    // only `MAX_OBSERVATIONS - 1` more distinct identities fit before the
    // window holds exactly `MAX_OBSERVATIONS` in total.
    for index in 0..(MAX_OBSERVATIONS - 1) {
        full.observe(DependencyIdentity::record(
            "t",
            index.to_string().as_bytes(),
        ))
        .expect("within bound");
    }
    let observed = full
        .close(&ledger)
        .await
        .expect("a window filled to the bound must still close");
    assert_eq!(
        observed.len(),
        MAX_OBSERVATIONS,
        "the closed set holds exactly the bound, including the broad authority"
    );
}
