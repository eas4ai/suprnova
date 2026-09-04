//! RenderCache Tier 0, Task 12 - the write side that connects Task 10's
//! request-scoped collector to Task 11's database-authoritative ledger.
//!
//! Every supported ORM and query-builder write path advances the
//! generation of what it changed, inside the transaction that made the
//! change - a model write advances its table and record, a bulk or
//! model-less table write advances its table, and a raw statement of
//! unknown shape advances the broad authority. All of it only survives a
//! commit: a rollback, ambient or through an explicit `_with_tx` handle,
//! must advance nothing.

use suprnova::attrs;
use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::ledger::SqlGenerationLedger;
use suprnova::{DB, FrameworkError, Model};
use suprnova_live::render_cache::generation::GenerationLedger;

mod render_cache_support;
use render_cache_support::{Post, Trashable, Widget, boot};

#[tokio::test]
async fn a_model_save_advances_the_record_and_table_generations_atomically() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    let post = Post::create(attrs! { title: "hello" })
        .await
        .expect("create");
    let key = post.id.to_string();
    let record = DependencyIdentity::record("posts", key.as_bytes());
    let after = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(after.get(&table), Some(1));
    assert_eq!(after.get(&record), Some(1));

    let aborted: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async move {
            let mut post = Post::find(post.id).await?.expect("post");
            post.title = "changed".to_owned();
            post.save().await?;
            Err(FrameworkError::internal("abort"))
        })
    })
    .await;
    assert!(aborted.is_err());
    let still = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(
        still.get(&table),
        Some(1),
        "a rolled back save advances nothing"
    );
    assert_eq!(still.get(&record), Some(1), "nor does it touch the record");

    // A genuine (non-aborted) save must still advance both - proves `save`
    // itself is instrumented, not merely that a rollback discards whatever
    // it did. The rollback assertion above would pass just as well if
    // `save` advanced nothing at all, ever.
    let mut post = Post::find(post.id).await.expect("find").expect("post");
    post.title = "changed for real".to_owned();
    post.save().await.expect("save");
    let after_save = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(after_save.get(&table), Some(2));
    assert_eq!(after_save.get(&record), Some(2));
}

#[tokio::test]
async fn bulk_builder_and_unknown_raw_writes_collapse_to_broader_authority() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");
    let broad = DependencyIdentity::broad();

    Post::create(attrs! { title: "a" }).await.expect("create");
    Post::query()
        .update_all(attrs! { title: "b" })
        .await
        .expect("bulk");
    let after_bulk = ledger.current(&[table.digest()]).await.expect("current");
    assert_eq!(
        after_bulk.get(&table),
        Some(2),
        "bulk writes advance the table"
    );

    DB::table("posts")
        .update(attrs! { title: "c" })
        .await
        .expect("builder");
    assert_eq!(
        ledger
            .current(&[table.digest()])
            .await
            .expect("current")
            .get(&table),
        Some(3)
    );

    DB::statement("UPDATE posts SET title = 'd'", vec![])
        .await
        .expect("raw");
    assert_eq!(
        ledger
            .current(&[broad.digest()])
            .await
            .expect("current")
            .get(&broad),
        Some(1),
        "an unparsed raw write advances the broad authority"
    );

    // A raw SELECT is a read, not a write: it must not advance the broad
    // authority. This is the discriminating half of the assertion above -
    // without the `SELECT` carve-out in `DB::statement`, this would also
    // read back `Some(2)`.
    DB::statement("SELECT COUNT(*) FROM posts", vec![])
        .await
        .expect("select");
    assert_eq!(
        ledger
            .current(&[broad.digest()])
            .await
            .expect("current")
            .get(&broad),
        Some(1),
        "a raw SELECT must not advance the broad authority"
    );
}

/// Ruling R45: the read side (`observe_record_read_json` /
/// `record_identity`) encodes a record's primary key as its JSON
/// `Display` form, quotes included - `"widget-1"` for the string
/// `"widget-1"`, not `widget-1`. An integer key never exposes a write
/// side that trims those quotes, because there are none to trim; a
/// string key does. This model's primary key is a `String`, so a write
/// side that (incorrectly) stripped quotes would advance a *different*
/// digest than the one a read observed, and record-level invalidation
/// would silently never fire for this table.
#[tokio::test]
async fn a_string_primary_key_advances_the_correctly_quoted_record_identity() {
    boot().await;
    let ledger = SqlGenerationLedger::new();

    Widget::create(attrs! { id: "widget-1", name: "gizmo" })
        .await
        .expect("create");

    let correctly_quoted = DependencyIdentity::record("widgets", b"\"widget-1\"");
    let after = ledger
        .current(&[correctly_quoted.digest()])
        .await
        .expect("current");
    assert_eq!(
        after.get(&correctly_quoted),
        Some(1),
        "the write side must encode the key exactly as the read side does, quotes included"
    );

    let trimmed = DependencyIdentity::record("widgets", b"widget-1");
    let trimmed_after = ledger.current(&[trimmed.digest()]).await.expect("current");
    assert_eq!(
        trimmed_after.get(&trimmed),
        Some(0),
        "the trimmed (unquoted) encoding must stay untouched - it is not what the read side observes"
    );
}

/// Ruling R48: `restore` is macro-generated for `soft_deletes` models and
/// runs its own UPDATE through `ExecutorChoice::resolve_write` directly,
/// bypassing `Builder::update_all`, `DB::table(...)`, and `DB::statement`
/// alike. Under the wiring the task text describes, restoring a row would
/// advance nothing at all, and a page that omitted the restored row would
/// keep being served after the restore commits. The same macro also
/// generates `delete` and `force_delete` overrides for a `soft_deletes`
/// model, which equally bypass the `Model` trait defaults this task
/// instruments in `model.rs` - this test exercises all three.
#[tokio::test]
async fn soft_delete_delete_restore_and_force_delete_all_advance_the_row() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("trashables");

    let item = Trashable::create(attrs! { title: "x" })
        .await
        .expect("create");
    let record = DependencyIdentity::record("trashables", item.id.to_string().as_bytes());
    let both = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(both.get(&table), Some(1));
    assert_eq!(both.get(&record), Some(1));

    item.delete().await.expect("soft delete");
    let both = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(
        both.get(&table),
        Some(2),
        "the soft-delete override's own UPDATE must advance the table generation"
    );
    assert_eq!(both.get(&record), Some(2));

    let trashed = Trashable::with_trashed()
        .filter("id", 1)
        .first()
        .await
        .expect("query")
        .expect("row still present, tombstoned");
    trashed.clone().restore().await.expect("restore");
    let both = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(
        both.get(&table),
        Some(3),
        "restore must advance the table generation - it is the exact gap ruling R48 fixes"
    );
    assert_eq!(both.get(&record), Some(3));

    trashed.force_delete().await.expect("force delete");
    let both = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(
        both.get(&table),
        Some(4),
        "force_delete's soft-delete override must also advance the table generation"
    );
    assert_eq!(both.get(&record), Some(4));
}

/// Ruling R47: `save_with_tx` (and its four `_with_tx` siblings) route
/// their row write through an explicit `ExecutorChoice::from_tx(tx)` and
/// bypass the ambient `CURRENT_TX` task-local by design - the explicit
/// handle is authoritative. A generation advance that instead checked
/// only the ambient task-local would find none active, fall back to
/// opening a transaction of its own, and commit independently of `tx`: a
/// caller that rolls back `tx` would then undo the row write while that
/// separately-committed advance stood.
///
/// This also pins ruling R51's assumption: the soft-delete `find`
/// override observes only the table (never the record) on the read side,
/// which over-invalidates rather than under-invalidates *only* because
/// every row-write path - including a `_with_tx` one - always advances
/// the table alongside the record. If a `_with_tx` path silently stopped
/// doing that, `find`'s coarser read would go from "safe but wasteful" to
/// "unsafe": a soft-delete model changed through `save_with_tx` would
/// stop invalidating pages that depended on its table.
#[tokio::test]
async fn with_tx_writes_advance_only_on_commit_never_on_rollback() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("trashables");

    let item = Trashable::create(attrs! { title: "x" })
        .await
        .expect("create");
    assert_eq!(
        ledger
            .current(&[table.digest()])
            .await
            .expect("current")
            .get(&table),
        Some(1)
    );

    // Committed `_with_tx` write: advances the table.
    let tx = DB::begin_transaction().await.expect("begin");
    let mut committed = item.clone();
    committed.title = "y".to_owned();
    committed.save_with_tx(&tx).await.expect("save_with_tx");
    tx.commit().await.expect("commit");
    assert_eq!(
        ledger
            .current(&[table.digest()])
            .await
            .expect("current")
            .get(&table),
        Some(2),
        "a committed _with_tx write must advance the table generation"
    );

    // Rolled-back `_with_tx` write: advances nothing.
    let tx = DB::begin_transaction().await.expect("begin");
    let mut rolled_back = item.clone();
    rolled_back.title = "z".to_owned();
    rolled_back.save_with_tx(&tx).await.expect("save_with_tx");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        ledger
            .current(&[table.digest()])
            .await
            .expect("current")
            .get(&table),
        Some(2),
        "a rolled back _with_tx write must advance nothing - the generation must not move \
         independently of the transaction that rolled back the row write it describes"
    );
}

/// fix1 item 3: `Builder::with_tx(&tx)` sets `tx_override`, which
/// `resolve_write` honours without installing the ambient `CURRENT_TX`
/// task-local - the identical defect ruling R47 fixed for the model
/// `_with_tx` shims, still present on the bulk-write path. Before the fix,
/// `M::query().with_tx(&tx).update_all(..)` would land its row write on
/// the caller's explicit transaction while the advance opened a *separate*
/// one, so this test's rollback half would have found the table
/// generation advanced anyway - exactly the same discriminator the
/// `with_tx_writes_advance_only_on_commit_never_on_rollback` test above
/// uses for the model path.
#[tokio::test]
async fn with_tx_bulk_writes_advance_only_on_commit_never_on_rollback() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    Post::create(attrs! { title: "a" }).await.expect("create");
    assert_eq!(
        ledger
            .current(&[table.digest()])
            .await
            .expect("current")
            .get(&table),
        Some(1)
    );

    // Committed bulk write through an explicit builder transaction
    // override: advances the table.
    let tx = DB::begin_transaction().await.expect("begin");
    Post::query()
        .with_tx(&tx)
        .update_all(attrs! { title: "b" })
        .await
        .expect("bulk with_tx");
    tx.commit().await.expect("commit");
    assert_eq!(
        ledger
            .current(&[table.digest()])
            .await
            .expect("current")
            .get(&table),
        Some(2),
        "a committed with_tx bulk write must advance the table generation"
    );

    // Rolled-back bulk write through an explicit builder transaction
    // override: advances nothing.
    let tx = DB::begin_transaction().await.expect("begin");
    Post::query()
        .with_tx(&tx)
        .update_all(attrs! { title: "c" })
        .await
        .expect("bulk with_tx");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        ledger
            .current(&[table.digest()])
            .await
            .expect("current")
            .get(&table),
        Some(2),
        "a rolled back with_tx bulk write must advance nothing"
    );
}
