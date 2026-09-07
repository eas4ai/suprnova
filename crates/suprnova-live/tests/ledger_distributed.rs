//! Two nodes over one record store: the instance-ledger kernel seen from
//! outside the engine.
//!
//! Every test here builds two independent [`DistributedInstanceLedger`]
//! handles over one [`MemoryRecordStore`], which is what a Tier 1 or Tier 2
//! deployment looks like from the engine's side: two processes, one backend,
//! no shared memory between them. The last three run the provider conformance
//! suite over the kernel and over `MemoryInstanceLedger`, which is now the
//! same kernel over an in-process store, so Tier 0 and the distributed tiers
//! are proved against one suite.

mod ledger_support;

use std::sync::Arc;

use ledger_support::{digest, idempotency, instance, scope};
use suprnova_live::clock::Clock;
use suprnova_live::identity::{Revision, UnixMillis};
use suprnova_live::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, ClaimGrant, ClaimOutcome, ClaimRequest,
    DistributedInstanceLedger, InstanceRecordKey, InstanceRecordStore, LedgerErrorKind,
    LiveInstanceLedger, MemoryInstanceLedger, MemoryRecordStore, MountInstanceRecord,
    RefreshReason,
};
use suprnova_live_test_support::ControlledClock;
use suprnova_live_test_support::ledger_conformance::{conformance_limits, run_all, run_two_node};

/// One deployment: two ledger handles over one shared record store.
struct Deployment {
    first: DistributedInstanceLedger<MemoryRecordStore>,
    second: DistributedInstanceLedger<MemoryRecordStore>,
    store: Arc<MemoryRecordStore>,
    clock: Arc<ControlledClock>,
}

fn deployment() -> Deployment {
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(1_000)));
    let store = Arc::new(MemoryRecordStore::new(Arc::clone(&clock) as Arc<dyn Clock>));
    Deployment {
        first: DistributedInstanceLedger::new(
            Arc::clone(&store),
            Arc::clone(&clock) as Arc<dyn Clock>,
            conformance_limits(),
        ),
        second: DistributedInstanceLedger::new(
            Arc::clone(&store),
            Arc::clone(&clock) as Arc<dyn Clock>,
            conformance_limits(),
        ),
        store,
        clock,
    }
}

fn mount(instance_start: u8) -> MountInstanceRecord {
    MountInstanceRecord::new(
        scope(0x10),
        instance(instance_start),
        digest(0x30),
        Revision::new(0),
        UnixMillis::new(5_000),
    )
}

fn claim_request(instance_start: u8, base: u64, retry: u8) -> ClaimRequest {
    ClaimRequest::new(
        scope(0x10),
        instance(instance_start),
        Revision::new(base),
        idempotency(retry),
        digest(retry.wrapping_add(0x80)),
    )
}

fn granted(outcome: ClaimOutcome) -> ClaimGrant {
    match outcome {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("the claim was expected to be granted, not {other:?}"),
    }
}

#[tokio::test]
async fn an_instance_one_node_mounts_is_current_authority_on_the_other() {
    let nodes = deployment();
    let record = mount(0x20);

    let authority = nodes
        .first
        .mount_instance(record.clone())
        .await
        .expect("the first node creates the mount");

    assert_eq!(authority.revision(), Revision::new(0));
    assert_eq!(
        nodes
            .second
            .current_accepted_revision(record.scope(), record.instance_id())
            .await
            .expect("the second node reads the shared store"),
        Some(Revision::new(0)),
        "authority lives in the store, not in the node that created it"
    );
    assert_eq!(
        nodes
            .second
            .mount_instance(record)
            .await
            .expect_err("a mount is create-only across nodes too")
            .kind(),
        LedgerErrorKind::InstanceConflict
    );
}

#[tokio::test]
async fn two_nodes_racing_one_base_revision_grant_exactly_one_token() {
    let nodes = deployment();
    nodes
        .first
        .mount_instance(mount(0x21))
        .await
        .expect("the mount is created");
    let request = claim_request(0x21, 0, 0x50);

    let (first, second) = tokio::join!(
        nodes.first.claim(request.clone()),
        nodes.second.claim(request)
    );

    let outcomes = [
        first.expect("the first claim classifies"),
        second.expect("the second claim classifies"),
    ];
    let granted = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, ClaimOutcome::Granted(_)))
        .count();
    let in_progress = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, ClaimOutcome::InProgress { .. }))
        .count();
    assert_eq!(granted, 1, "exactly one node claims the successor");
    assert_eq!(
        in_progress, 1,
        "the loser observes the exact request already executing"
    );
}

#[tokio::test]
async fn a_commit_on_one_node_is_the_accepted_revision_on_the_other() {
    let nodes = deployment();
    let record = mount(0x22);
    nodes
        .first
        .mount_instance(record.clone())
        .await
        .expect("the mount is created");
    let grant = granted(
        nodes
            .first
            .claim(claim_request(0x22, 0, 0x51))
            .await
            .expect("the claim classifies"),
    );

    assert_eq!(
        nodes
            .second
            .current_accepted_revision(record.scope(), record.instance_id())
            .await
            .expect("the second node reads the shared store"),
        Some(Revision::new(0)),
        "a claimed successor is not accepted authority on any node"
    );

    nodes
        .first
        .commit(
            &grant.into_token(),
            AcceptedOutcome::new(AcceptedOutcomeKind::Rendered, digest(0x70)),
        )
        .await
        .expect("the matching claim commits");

    assert_eq!(
        nodes
            .second
            .current_accepted_revision(record.scope(), record.instance_id())
            .await
            .expect("the second node reads the shared store"),
        Some(Revision::new(1))
    );
}

#[tokio::test]
async fn a_released_claim_crosses_nodes_once_the_queue_drains() {
    let nodes = deployment();
    let record = mount(0x23);
    nodes
        .first
        .mount_instance(record.clone())
        .await
        .expect("the mount is created");
    let grant = granted(
        nodes
            .first
            .claim(claim_request(0x23, 0, 0x52))
            .await
            .expect("the claim classifies"),
    );
    assert!(
        matches!(
            nodes
                .second
                .claim(claim_request(0x23, 0, 0x53))
                .await
                .expect("a different retry identity classifies"),
            ClaimOutcome::IdempotencyConflict
        ),
        "a pending claim owns its base revision on every node"
    );

    nodes.first.abandon_on_drop(grant.into_token());
    nodes
        .first
        .current_accepted_revision(record.scope(), record.instance_id())
        .await
        .expect("the next operation drains the queued release");

    let retry = nodes
        .second
        .claim(claim_request(0x23, 0, 0x53))
        .await
        .expect("the released base revision classifies");
    assert!(
        matches!(retry, ClaimOutcome::Granted(_)),
        "the release restored the base revision for every node, got {retry:?}"
    );
}

#[tokio::test]
async fn flush_cleanup_applies_a_queued_release_without_another_operation() {
    let nodes = deployment();
    let record = mount(0x24);
    nodes
        .first
        .mount_instance(record.clone())
        .await
        .expect("the mount is created");
    let grant = granted(
        nodes
            .first
            .claim(claim_request(0x24, 0, 0x54))
            .await
            .expect("the claim classifies"),
    );

    nodes.first.abandon_on_drop(grant.into_token());
    nodes
        .first
        .flush_cleanup()
        .await
        .expect("the queued release is applied");

    assert!(
        matches!(
            nodes
                .second
                .claim(claim_request(0x24, 0, 0x55))
                .await
                .expect("the released base revision classifies"),
            ClaimOutcome::Granted(_)
        ),
        "flush_cleanup applies the release without another ledger operation"
    );
}

#[tokio::test]
async fn a_fenced_claim_never_restores_base_revision_authority_on_either_node() {
    let nodes = deployment();
    let record = mount(0x25);
    nodes
        .first
        .mount_instance(record.clone())
        .await
        .expect("the mount is created");
    let grant = granted(
        nodes
            .first
            .claim(claim_request(0x25, 0, 0x56))
            .await
            .expect("the claim classifies"),
    );

    nodes.first.fence_on_drop(grant.into_token());
    nodes
        .first
        .flush_cleanup()
        .await
        .expect("the queued fence is applied");

    assert!(matches!(
        nodes
            .second
            .claim(claim_request(0x25, 0, 0x56))
            .await
            .expect("the fenced instance classifies"),
        ClaimOutcome::RefreshRequired(RefreshReason::Consumed)
    ));
    assert_eq!(
        nodes
            .second
            .current_accepted_revision(record.scope(), record.instance_id())
            .await
            .expect("the second node reads the shared store"),
        None,
        "a fenced claim leaves no accepted authority anywhere"
    );
}

#[tokio::test]
async fn a_record_evicted_from_the_store_is_no_authority_on_either_node() {
    let nodes = deployment();
    let record = mount(0x26);
    nodes
        .first
        .mount_instance(record.clone())
        .await
        .expect("the mount is created");
    let key = InstanceRecordKey {
        scope: record.scope().clone(),
        instance_id: record.instance_id().clone(),
    };

    nodes.store.remove(&key).await.expect("the store answers");

    for node in [&nodes.first, &nodes.second] {
        assert_eq!(
            node.current_accepted_revision(record.scope(), record.instance_id())
                .await
                .expect("an evicted record reads cleanly"),
            None
        );
        assert!(matches!(
            node.claim(claim_request(0x26, 0, 0x57))
                .await
                .expect("an evicted record classifies"),
            ClaimOutcome::RefreshRequired(RefreshReason::Missing)
        ));
    }
}

#[tokio::test]
async fn an_elapsed_instance_is_distinguishable_from_one_that_never_existed() {
    let nodes = deployment();
    let record = mount(0x27);
    nodes
        .first
        .mount_instance(record.clone())
        .await
        .expect("the mount is created");

    nodes.clock.set(UnixMillis::new(5_000));

    assert!(matches!(
        nodes
            .second
            .claim(claim_request(0x27, 0, 0x58))
            .await
            .expect("an elapsed instance classifies"),
        ClaimOutcome::RefreshRequired(RefreshReason::InstanceExpired)
    ));
    assert!(matches!(
        nodes
            .second
            .claim(claim_request(0x9f, 0, 0x58))
            .await
            .expect("an instance that never existed classifies"),
        ClaimOutcome::RefreshRequired(RefreshReason::Missing)
    ));
}

#[tokio::test]
async fn a_claim_token_from_one_node_is_never_accepted_by_the_other() {
    let nodes = deployment();
    nodes
        .first
        .mount_instance(mount(0x28))
        .await
        .expect("the mount is created");
    let grant = granted(
        nodes
            .first
            .claim(claim_request(0x28, 0, 0x59))
            .await
            .expect("the claim classifies"),
    );

    assert_eq!(
        nodes
            .second
            .commit(
                &grant.into_token(),
                AcceptedOutcome::new(AcceptedOutcomeKind::NoRender, digest(0x71)),
            )
            .await
            .expect_err("a token cannot cross providers")
            .kind(),
        LedgerErrorKind::ClaimMismatch
    );
}

#[tokio::test]
async fn the_conformance_suite_passes_over_the_distributed_kernel() {
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(1_000)));
    let store = Arc::new(MemoryRecordStore::new(Arc::clone(&clock) as Arc<dyn Clock>));
    let ledger: Arc<dyn LiveInstanceLedger> = Arc::new(DistributedInstanceLedger::new(
        store,
        Arc::clone(&clock) as Arc<dyn Clock>,
        conformance_limits(),
    ));

    run_all(ledger, clock).await.expect("the kernel conforms");
}

#[tokio::test]
async fn the_conformance_suite_passes_over_the_tier_zero_ledger() {
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(1_000)));
    let ledger: Arc<dyn LiveInstanceLedger> = Arc::new(MemoryInstanceLedger::new(
        Arc::clone(&clock) as Arc<dyn Clock>,
        conformance_limits(),
    ));

    run_all(ledger, clock)
        .await
        .expect("the Tier 0 provider conforms");
}

#[tokio::test]
async fn the_two_node_conformance_suite_passes_over_one_store() {
    let nodes = deployment();
    let first: Arc<dyn LiveInstanceLedger> = Arc::new(nodes.first);
    let second: Arc<dyn LiveInstanceLedger> = Arc::new(nodes.second);

    run_two_node(first, second, nodes.clock)
        .await
        .expect("two nodes over one store conform");
}
