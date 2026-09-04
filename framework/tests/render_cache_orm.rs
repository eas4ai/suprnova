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

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::attrs;
use suprnova::eloquent::{MassPrunable, prune_one};
use suprnova::payments::{
    MockPaymentProvider, PaymentProvider, PaymentProviderRegistry, SubscribeRequest, Subscription,
    webhook_routes,
};
use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::ledger::SqlGenerationLedger;
use suprnova::testing::TestDatabase;
use suprnova::{
    DB, FrameworkError, MiddlewareRegistry, Model, Persistable, Router, handle_request,
};
use suprnova_live::render_cache::generation::GenerationLedger;

mod render_cache_support;
use render_cache_support::{Author, Book, Post, Tag, Trashable, Widget, boot};

/// fix2 item 4: `MassPrunable`'s bulk DELETE bypasses `Builder::delete_all`
/// entirely - it renders its own `DELETE FROM ... WHERE ...` and runs it
/// through `ExecutorChoice::resolve()` directly. Registered at module
/// scope like every other `#[suprnova::prunable]` fixture in this
/// codebase; exercised only via `prune_one("Post", ..)`, never
/// `prune_all()`, so it cannot interfere with any other test in this
/// binary (see the cross-test isolation note in `eloquent_soft_deletes.rs`).
#[suprnova::prunable]
#[async_trait::async_trait]
impl MassPrunable for Post {
    fn prunable() -> suprnova::Builder<Post> {
        Post::query().filter("title", "stale")
    }
}

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
    let item_id = item.id;
    let record = DependencyIdentity::record("trashables", item_id.to_string().as_bytes());
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
        .filter("id", item_id)
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

// ---- fix2: the write-path sweep's remaining critical paths --------------

/// fix2 item 2 (Critical): `Model::increment` / `decrement` issue a raw
/// `UPDATE` through `exec.run` directly - the canonical view-counter,
/// stock-level, balance write, exactly the field a cached page renders.
#[tokio::test]
async fn increment_and_decrement_advance_the_record_and_table() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    let post = Post::create(attrs! { title: "counter" })
        .await
        .expect("create");
    let record = DependencyIdentity::record("posts", post.id.to_string().as_bytes());
    let after_create = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(after_create.get(&table), Some(1));
    assert_eq!(after_create.get(&record), Some(1));

    post.increment("views", 5).await.expect("increment");
    let after_increment = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(
        after_increment.get(&table),
        Some(2),
        "increment must advance the table generation"
    );
    assert_eq!(
        after_increment.get(&record),
        Some(2),
        "and the specific row's record generation"
    );

    post.decrement("views", 2).await.expect("decrement");
    let after_decrement = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(
        after_decrement.get(&table),
        Some(3),
        "decrement (sugar over increment) must also advance"
    );
    assert_eq!(after_decrement.get(&record), Some(3));
}

/// fix2 item 2 (Critical): `Builder::increment_each` / `decrement_each`
/// and `Builder::upsert` each issue their own `exec.run` write, thirteen
/// and eighty-some lines below `delete_all`, which round 1 already
/// instrumented.
#[tokio::test]
async fn increment_each_and_upsert_advance_the_table() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    Post::create(attrs! { title: "a" }).await.expect("create");
    let after_create = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);

    Post::query()
        .increment_each([("views", 3)])
        .await
        .expect("increment_each");
    let after_increment_each = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after_increment_each,
        after_create.map(|g| g + 1),
        "increment_each must advance the table generation"
    );

    Post::query()
        .decrement_each([("views", 1)])
        .await
        .expect("decrement_each");
    let after_decrement_each = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(after_decrement_each, after_increment_each.map(|g| g + 1));

    Post::query()
        .upsert(
            vec![attrs! { id: 999, title: "upserted" }],
            vec!["id"],
            Some(vec!["title"]),
        )
        .await
        .expect("upsert");
    let after_upsert = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after_upsert,
        after_decrement_each.map(|g| g + 1),
        "upsert must advance the table generation"
    );
}

/// fix2 item 3 (Critical, "the single most valuable item"):
/// `#[model(touches = [...])]` exists precisely to bust a parent's cached
/// representation when a child changes; `__touch_owners_via` ran that
/// UPDATE and advanced nothing at all before this fix.
#[tokio::test]
async fn touch_owners_advances_the_parent_table_and_record() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let authors_table = DependencyIdentity::table("authors");

    let author = Author::create(attrs! { name: "Ada" })
        .await
        .expect("create author");
    let author_id = author.id;
    let author_record = DependencyIdentity::record("authors", author_id.to_string().as_bytes());

    let before = ledger
        .current(&[authors_table.digest(), author_record.digest()])
        .await
        .expect("current");
    assert_eq!(before.get(&authors_table), Some(1));
    assert_eq!(before.get(&author_record), Some(1));

    Book::create(attrs! { author_id: author_id, title: "Book One" })
        .await
        .expect("create book");

    let after = ledger
        .current(&[authors_table.digest(), author_record.digest()])
        .await
        .expect("current");
    assert_eq!(
        after.get(&authors_table),
        Some(2),
        "creating a child with #[model(touches = [...])] must advance the parent's table \
         generation"
    );
    assert_eq!(
        after.get(&author_record),
        Some(2),
        "and the parent's specific record generation"
    );
}

/// fix2 item 3: `touch_owners_with_tx` is the only `*_with_tx` function in
/// the framework that executes SQL and, before this fix, advanced no
/// generation for it - same defect shape as ruling R47.
///
/// Per the reviewer's caution: the committed-case assertion alone is not a
/// reliable discriminator (a buggy implementation that opens a *separate*,
/// immediately-committed transaction for the advance would still pass it
/// on a pool with more than one connection - only round 1's single-
/// connection SQLite database turned that bug into a pool deadlock rather
/// than a silently-wrong number). The rollback assertion is what actually
/// proves the advance shares the caller's transaction, so it is the one
/// this test leans on.
#[tokio::test]
async fn touch_owners_with_tx_advances_only_on_commit_never_on_rollback() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let authors_table = DependencyIdentity::table("authors");

    let author = Author::create(attrs! { name: "Ada" })
        .await
        .expect("create author");
    let book = Book::create(attrs! { author_id: author.id, title: "Book One" })
        .await
        .expect("create book");

    let after_create = ledger
        .current(&[authors_table.digest()])
        .await
        .expect("current")
        .get(&authors_table);

    let tx = DB::begin_transaction().await.expect("begin");
    let mut committed_book = book.clone();
    committed_book.title = "Book One (v2)".to_owned();
    committed_book
        .save_with_tx(&tx)
        .await
        .expect("save_with_tx");
    tx.commit().await.expect("commit");
    let after_commit = ledger
        .current(&[authors_table.digest()])
        .await
        .expect("current")
        .get(&authors_table);
    assert_eq!(
        after_commit,
        after_create.map(|g| g + 1),
        "a committed with_tx touch must advance the parent's table generation"
    );

    let tx = DB::begin_transaction().await.expect("begin");
    let mut rolled_back_book = book.clone();
    rolled_back_book.title = "Book One (v3)".to_owned();
    rolled_back_book
        .save_with_tx(&tx)
        .await
        .expect("save_with_tx");
    tx.rollback().await.expect("rollback");
    let after_rollback = ledger
        .current(&[authors_table.digest()])
        .await
        .expect("current")
        .get(&authors_table);
    assert_eq!(
        after_rollback, after_commit,
        "a rolled back with_tx touch must advance the parent's generation by exactly zero - \
         this equality, not whether the committed case above advanced it, is what actually \
         discriminates the fix: a buggy separately-committed advance would still pass the \
         committed-case assertion"
    );
}

/// fix2 item 4: relation pivot `attach` / `detach` / `sync` write through
/// `ExecutorChoice` directly, bypassing every path this task otherwise
/// instruments. The pivot row's key is composite
/// (`pivot_foreign_key`, `pivot_related_key`), not addressable by
/// `DependencyIdentity::record`, so only the table identity advances -
/// over-invalidating a pivot table on every attach is the safe direction.
#[tokio::test]
async fn belongs_to_many_attach_detach_sync_advance_the_pivot_table() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let pivot_table = DependencyIdentity::table("post_tags");

    let post = Post::create(attrs! { title: "tagged" })
        .await
        .expect("create post");
    let tag_one = Tag::create(attrs! { name: "rust" })
        .await
        .expect("create tag");
    let tag_two = Tag::create(attrs! { name: "async" })
        .await
        .expect("create tag");

    let before = ledger
        .current(&[pivot_table.digest()])
        .await
        .expect("current")
        .get(&pivot_table);
    assert_eq!(before, Some(0));

    post.tags().attach(tag_one.id).await.expect("attach");
    let after_attach = ledger
        .current(&[pivot_table.digest()])
        .await
        .expect("current")
        .get(&pivot_table);
    assert_eq!(
        after_attach,
        Some(1),
        "attach must advance the pivot table generation"
    );

    post.tags().sync([tag_two.id]).await.expect("sync");
    let after_sync = ledger
        .current(&[pivot_table.digest()])
        .await
        .expect("current")
        .get(&pivot_table);
    assert_eq!(
        after_sync,
        Some(2),
        "sync (one attach, one detach) must advance the pivot table generation exactly once"
    );

    post.tags().detach(tag_two.id).await.expect("detach");
    let after_detach = ledger
        .current(&[pivot_table.digest()])
        .await
        .expect("current")
        .get(&pivot_table);
    assert_eq!(
        after_detach,
        Some(3),
        "detach must advance the pivot table generation"
    );
}

/// fix2 item 4: `MassPrunable`'s bulk DELETE bypasses every path this task
/// otherwise instruments.
#[tokio::test]
async fn mass_prunable_bulk_delete_advances_the_table() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    Post::create(attrs! { title: "stale" })
        .await
        .expect("create");
    let before = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);

    let pruned = prune_one("Post", false)
        .await
        .expect("prune_one")
        .expect("Post is registered via #[suprnova::prunable]");
    assert_eq!(pruned, 1);

    let after = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after,
        before.map(|g| g + 1),
        "MassPrunable's bulk DELETE must advance the table generation"
    );
}

/// fix2 item 4: the raw facade siblings the brief never named beside
/// `DB::statement` - `DB::insert` / `update` / `delete` /
/// `affecting_statement` / `unprepared` / `statement_on` /
/// `affecting_statement_on` - all collapse to the broad authority, the
/// same as `DB::statement` does for a statement whose table isn't known.
#[tokio::test]
async fn db_raw_facade_write_siblings_advance_the_broad_authority() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let broad = DependencyIdentity::broad();

    let id = Post::create(attrs! { title: "raw" })
        .await
        .expect("create")
        .id;
    let mut expected: u64 = 0;
    macro_rules! assert_broad {
        () => {{
            expected += 1;
            assert_eq!(
                ledger
                    .current(&[broad.digest()])
                    .await
                    .expect("current")
                    .get(&broad),
                Some(expected)
            );
        }};
    }

    DB::insert("INSERT INTO posts (title) VALUES ('via-insert')", vec![])
        .await
        .expect("DB::insert");
    assert_broad!();

    DB::update(
        &format!("UPDATE posts SET title = 'x' WHERE id = {id}"),
        vec![],
    )
    .await
    .expect("DB::update");
    assert_broad!();

    DB::delete("DELETE FROM posts WHERE title = 'via-insert'", vec![])
        .await
        .expect("DB::delete");
    assert_broad!();

    DB::affecting_statement(
        &format!("UPDATE posts SET title = 'y' WHERE id = {id}"),
        vec![],
    )
    .await
    .expect("DB::affecting_statement");
    assert_broad!();

    DB::unprepared(&format!("UPDATE posts SET title = 'z' WHERE id = {id}"))
        .await
        .expect("DB::unprepared");
    assert_broad!();

    DB::statement_on(
        "__primary__",
        &format!("UPDATE posts SET title = 'w' WHERE id = {id}"),
        vec![],
    )
    .await
    .expect("DB::statement_on");
    assert_broad!();

    DB::affecting_statement_on(
        "__primary__",
        &format!("UPDATE posts SET title = 'v' WHERE id = {id}"),
        vec![],
    )
    .await
    .expect("DB::affecting_statement_on");
    assert_broad!();

    // DDL through `unprepared` is not a SELECT, so it still advances.
    DB::unprepared("CREATE INDEX IF NOT EXISTS ix_posts_title ON posts (title)")
        .await
        .expect("DB::unprepared ddl");
    assert_broad!();

    // A SELECT through the same raw escapes is a read, not a write - it
    // must never advance the broad authority. Discriminates `statement_on`
    // specifically: unlike `affecting_statement`, it accepts arbitrary SQL
    // and must tell a read from a write itself.
    DB::statement_on("__primary__", "SELECT COUNT(*) FROM posts", vec![])
        .await
        .expect("DB::statement_on select");
    assert_eq!(
        ledger
            .current(&[broad.digest()])
            .await
            .expect("current")
            .get(&broad),
        Some(expected),
        "a SELECT through `statement_on` must not advance the broad authority - `expected` was \
         not bumped for it, unlike every write above"
    );
}

/// fix2 item 5: seven call sites this task instrumented in the base
/// commit had no assertions at all. `Model::delete` and `force_delete` as
/// trait defaults are two of them.
#[tokio::test]
async fn model_delete_and_force_delete_trait_defaults_advance() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    let post_a = Post::create(attrs! { title: "a" }).await.expect("create a");
    let post_b = Post::create(attrs! { title: "b" }).await.expect("create b");
    let record_a = DependencyIdentity::record("posts", post_a.id.to_string().as_bytes());
    let record_b = DependencyIdentity::record("posts", post_b.id.to_string().as_bytes());
    let before = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);

    post_a.delete().await.expect("delete");
    let after_delete = ledger
        .current(&[table.digest(), record_a.digest()])
        .await
        .expect("current");
    assert_eq!(
        after_delete.get(&table),
        before.map(|g| g + 1),
        "Model::delete (trait default) must advance the table"
    );
    assert_eq!(
        after_delete.get(&record_a),
        Some(2),
        "create bumped it to 1, delete to 2"
    );

    post_b.force_delete().await.expect("force_delete");
    let after_force_delete = ledger
        .current(&[table.digest(), record_b.digest()])
        .await
        .expect("current");
    assert_eq!(
        after_force_delete.get(&table),
        before.map(|g| g + 2),
        "Model::force_delete (trait default) must advance the table"
    );
    assert_eq!(
        after_force_delete.get(&record_b),
        Some(2),
        "create bumped it to 1, force_delete to 2"
    );
}

/// fix2 item 5: the four `_with_tx` shims other than `save_with_tx` (which
/// round 1 already covered) had no assertions at all.
#[tokio::test]
async fn all_with_tx_shims_advance_on_commit() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    let tx = DB::begin_transaction().await.expect("begin");
    let created = Post::create_with_tx(&tx, attrs! { title: "created" })
        .await
        .expect("create_with_tx");
    tx.commit().await.expect("commit");
    let record = DependencyIdentity::record("posts", created.id.to_string().as_bytes());
    let after_create = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(after_create.get(&table), Some(1));
    assert_eq!(after_create.get(&record), Some(1));

    let tx = DB::begin_transaction().await.expect("begin");
    let updated = created
        .update_with_tx(&tx, attrs! { title: "updated" })
        .await
        .expect("update_with_tx");
    tx.commit().await.expect("commit");
    let after_update = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(after_update.get(&table), Some(2));
    assert_eq!(after_update.get(&record), Some(2));

    let tx = DB::begin_transaction().await.expect("begin");
    updated.delete_with_tx(&tx).await.expect("delete_with_tx");
    tx.commit().await.expect("commit");
    let after_delete = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(after_delete.get(&table), Some(3));
    assert_eq!(after_delete.get(&record), Some(3));

    let another = Post::create(attrs! { title: "another" })
        .await
        .expect("create");
    let record_another = DependencyIdentity::record("posts", another.id.to_string().as_bytes());
    let before_force = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    let tx = DB::begin_transaction().await.expect("begin");
    another
        .force_delete_with_tx(&tx)
        .await
        .expect("force_delete_with_tx");
    tx.commit().await.expect("commit");
    let after_force = ledger
        .current(&[table.digest(), record_another.digest()])
        .await
        .expect("current");
    assert_eq!(after_force.get(&table), before_force.map(|g| g + 1));
    assert_eq!(
        after_force.get(&record_another),
        Some(2),
        "create bumped it to 1, force_delete_with_tx to 2"
    );
}

/// fix3 item 4: `all_with_tx_shims_advance_on_commit` above proved only the
/// committed half for `create_with_tx`/`update_with_tx`/`delete_with_tx`/
/// `force_delete_with_tx`. `save_with_tx` already had the rollback
/// discriminator (`with_tx_writes_advance_only_on_commit_never_on_rollback`
/// above) - this gives the other four the same proof: not "did the
/// committed case work" but "did a rolled back one leave the generation
/// exactly where it started."
#[tokio::test]
async fn all_with_tx_shims_advance_only_on_commit_never_on_rollback() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    // create_with_tx, rolled back: no row ever commits, so the table
    // generation must be exactly what it was before the attempt.
    let before_create = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    let tx = DB::begin_transaction().await.expect("begin");
    Post::create_with_tx(&tx, attrs! { title: "rolled back create" })
        .await
        .expect("create_with_tx");
    tx.rollback().await.expect("rollback");
    let after_create = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after_create, before_create,
        "a rolled back create_with_tx must advance nothing"
    );

    // Committed fixture row for the remaining three shims.
    let base = Post::create(attrs! { title: "base" })
        .await
        .expect("create");
    let record = DependencyIdentity::record("posts", base.id.to_string().as_bytes());
    let baseline = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    let (table_gen, record_gen) = (baseline.get(&table), baseline.get(&record));

    // update_with_tx, rolled back: neither generation moves.
    let tx = DB::begin_transaction().await.expect("begin");
    let updated = base
        .clone()
        .update_with_tx(&tx, attrs! { title: "updated then rolled back" })
        .await
        .expect("update_with_tx");
    tx.rollback().await.expect("rollback");
    let after_update = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(
        after_update.get(&table),
        table_gen,
        "a rolled back update_with_tx must advance nothing"
    );
    assert_eq!(
        after_update.get(&record),
        record_gen,
        "a rolled back update_with_tx must advance nothing"
    );

    // delete_with_tx, rolled back: neither generation moves, and the
    // DELETE itself is undone, leaving the row in place for the next shim.
    let tx = DB::begin_transaction().await.expect("begin");
    updated
        .clone()
        .delete_with_tx(&tx)
        .await
        .expect("delete_with_tx");
    tx.rollback().await.expect("rollback");
    let after_delete = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(
        after_delete.get(&table),
        table_gen,
        "a rolled back delete_with_tx must advance nothing"
    );
    assert_eq!(
        after_delete.get(&record),
        record_gen,
        "a rolled back delete_with_tx must advance nothing"
    );

    // force_delete_with_tx, rolled back: neither generation moves.
    let tx = DB::begin_transaction().await.expect("begin");
    updated
        .clone()
        .force_delete_with_tx(&tx)
        .await
        .expect("force_delete_with_tx");
    tx.rollback().await.expect("rollback");
    let after_force_delete = ledger
        .current(&[table.digest(), record.digest()])
        .await
        .expect("current");
    assert_eq!(
        after_force_delete.get(&table),
        table_gen,
        "a rolled back force_delete_with_tx must advance nothing"
    );
    assert_eq!(
        after_force_delete.get(&record),
        record_gen,
        "a rolled back force_delete_with_tx must advance nothing"
    );
}

/// fix2 item 5: `delete_one_or_fail` in both branches had no assertions.
/// `delete_or_fail` picks the branch on whether it is already inside an
/// ambient transaction - a bare call takes the explicit-tx branch (it
/// opens one itself); a call already inside `DB::transaction` takes the
/// ambient branch.
#[tokio::test]
async fn delete_one_or_fail_advances_in_both_branches() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    let post_a = Post::create(attrs! { title: "a" }).await.expect("create a");
    let record_a = DependencyIdentity::record("posts", post_a.id.to_string().as_bytes());
    let before = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    post_a
        .delete_or_fail()
        .await
        .expect("delete_or_fail (explicit-tx branch)");
    let after_explicit = ledger
        .current(&[table.digest(), record_a.digest()])
        .await
        .expect("current");
    assert_eq!(after_explicit.get(&table), before.map(|g| g + 1));
    assert_eq!(
        after_explicit.get(&record_a),
        Some(2),
        "create bumped it to 1, delete_or_fail to 2"
    );

    let post_b = Post::create(attrs! { title: "b" }).await.expect("create b");
    let record_b = DependencyIdentity::record("posts", post_b.id.to_string().as_bytes());
    let before_ambient = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    DB::transaction(|_tx| Box::pin(async move { post_b.delete_or_fail().await }))
        .await
        .expect("commit");
    let after_ambient = ledger
        .current(&[table.digest(), record_b.digest()])
        .await
        .expect("current");
    assert_eq!(after_ambient.get(&table), before_ambient.map(|g| g + 1));
    assert_eq!(
        after_ambient.get(&record_b),
        Some(2),
        "create bumped it to 1, delete_or_fail to 2"
    );
}

/// fix2 item 5: `Builder::delete_all` (ambient) had no assertions - only
/// `update_all` was covered.
#[tokio::test]
async fn builder_delete_all_advances_the_table() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    Post::create(attrs! { title: "to-delete" })
        .await
        .expect("create");
    let before = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);

    let deleted = Post::query()
        .filter("title", "to-delete")
        .delete_all()
        .await
        .expect("delete_all");
    assert_eq!(deleted, 1);
    let after = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after,
        before.map(|g| g + 1),
        "Builder::delete_all must advance the table generation"
    );
}

/// fix2 item 5: `DbTableBuilder::insert` and `::delete` had no assertions
/// - only `::update` was covered.
#[tokio::test]
async fn db_table_builder_insert_and_delete_advance_the_table() {
    boot().await;
    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("posts");

    let before = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    let id = DB::table("posts")
        .insert(attrs! { title: "table-builder" })
        .await
        .expect("insert");
    let after_insert = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after_insert,
        before.map(|g| g + 1),
        "DbTableBuilder::insert must advance the table generation"
    );

    let deleted = DB::table("posts")
        .filter("id", id)
        .delete()
        .await
        .expect("delete");
    assert_eq!(deleted, 1);
    let after_delete = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after_delete,
        after_insert.map(|g| g + 1),
        "DbTableBuilder::delete must advance the table generation"
    );
}

// ---- fix3 item 1: both `Persistable` implementations ----------------------
//
// `factory/persist.rs` carries two `Persistable` impls: a per-struct one
// the `#[suprnova::model]` macro emits for Eloquent-facing structs (already
// instrumented before this fix - `derive_eloquent.rs`), and a second,
// blanket impl over any raw SeaORM `ModelTrait` type with no Suprnova
// `Model` at all, which reached `ActiveModelTrait::insert` on the bare
// connection and advanced nothing. This raw entity is the target for the
// blanket path; `Widget` (already imported above) stands in for the macro
// path so both are proven from the same test.
mod raw_widget {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "raw_widgets")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = true)]
        pub id: i64,
        pub name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[tokio::test]
async fn both_persistable_implementations_advance_their_table() {
    boot().await;
    DB::unprepared(
        "CREATE TABLE raw_widgets (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            name TEXT NOT NULL\
         )",
    )
    .await
    .expect("create raw_widgets table");

    let ledger = SqlGenerationLedger::new();

    // Blanket impl: a raw SeaORM `ModelTrait` type, reached only through
    // `ActiveModelTrait::insert` on the bare `DB::connection()` - no
    // `Model` trait, no `M::TABLE`, no ambient transaction routing.
    let raw_table = DependencyIdentity::table("raw_widgets");
    let before_raw = ledger
        .current(&[raw_table.digest()])
        .await
        .expect("current")
        .get(&raw_table);
    let raw = raw_widget::Model {
        id: 0,
        name: "raw".to_owned(),
    };
    let raw = raw.persist().await.expect("blanket Persistable::persist");
    assert!(raw.id > 0, "sqlite assigned the id: {}", raw.id);
    let after_raw = ledger
        .current(&[raw_table.digest()])
        .await
        .expect("current")
        .get(&raw_table);
    assert_eq!(
        after_raw,
        Some(before_raw.unwrap_or(0) + 1),
        "the blanket Persistable impl over a raw SeaORM model must advance its table"
    );

    // Macro-generated impl: a Suprnova `#[model]` struct, called directly
    // rather than through `create`/`Factory` so this proves the same
    // `Persistable::persist` entry point `direct_persistable_call_on_eloquent_struct_works`
    // (`eloquent_factory_persist.rs`) exercises, side by side with the
    // blanket impl above. `Tag` (auto-increment integer PK), not `Widget`
    // (`String` PK) - `persist_via_seaorm` flips every primary-key column
    // to `NotSet` before inserting so the database can assign it, which
    // only a database-generated (auto-increment) key can satisfy.
    let tag_table = DependencyIdentity::table("tags");
    let before_tag = ledger
        .current(&[tag_table.digest()])
        .await
        .expect("current")
        .get(&tag_table);
    let tag = Tag {
        id: 0,
        name: "tag".to_owned(),
        ..Default::default()
    };
    tag.persist().await.expect("macro Persistable::persist");
    let after_tag = ledger
        .current(&[tag_table.digest()])
        .await
        .expect("current")
        .get(&tag_table);
    assert_eq!(
        after_tag,
        Some(before_tag.unwrap_or(0) + 1),
        "the macro-generated Persistable impl must advance its table"
    );
}

// ---- fix4 item 1: assert payments writes advance ---------------------------
//
// Round 3 instrumented all thirteen raw-SeaORM write sites in
// `payments::webhook_route` via `advance_mirror_table`, but nothing asserted
// that any of them actually advances anything - the existing hydration and
// idempotency suites would catch a panic or a wrong entity type, but a
// silently removed call, a wrong branch guard, or the wrong entity would pass
// every test in the repository. Payments was the one confirmed live
// under-invalidation bug on this branch (fix3's whole reason for existing),
// so this closes that gap against the real webhook route.
//
// This test proves sites 1 and 13 (the `payments_webhook_events` audit-row
// insert and `mark_failed`'s update), NOT sites 3-12 (the
// `payments_subscriptions`/`payments_transactions`/`payments_customers`
// mirror-table upserts `upsert_subscription` etc. perform). That narrowing is
// deliberate, not an oversight: driving a successful `subscription.created`
// webhook through the real route with RenderCache installed deadlocks under
// SQLite, unconditionally, regardless of connection pool size - see the
// finding recorded in the round 4 report. Sites 1 and 13 both run on the bare
// `db: &DatabaseConnection` OUTSIDE `try_hydrate`'s transaction (the doc
// comment above `handle_webhook_inner`'s hydration call spells this out:
// provider HTTP calls, and therefore this event's audit bookkeeping, run
// before the transaction opens), so they are exactly the two sites this
// harness CAN exercise safely - one insert, one update, both against a real
// payments table, both able to fail exactly as fix4 describes (a silently
// removed call, a wrong branch guard, or the wrong entity would go undetected
// otherwise).
//
// The scenario: post a `subscription.created` webhook for a `sub_id` that
// was never registered with the mock provider. `try_hydrate` pre-fetches the
// provider's state (`Subscription::get`) BEFORE opening its transaction; an
// unknown id makes that fetch fail with `NotFound`, so `try_hydrate` returns
// `Err` without ever calling `db.begin()`. `handle_webhook_inner` still (a)
// inserts the audit row before dispatching to `try_hydrate` (site 1) and (b)
// calls `mark_failed` on the `Err` branch (site 13) - both real writes to
// `payments_webhook_events`, both outside any transaction, from one HTTP
// call.

/// Combined migrator: the payments schema (for the webhook route's mirror
/// tables) plus the RenderCache migration (so `advance_mirror_table`'s
/// `SqlGenerationLedger` calls have `suprnova_render_epochs` to write to).
struct PaymentsRenderCacheMigrator;

#[async_trait::async_trait]
impl MigratorTrait for PaymentsRenderCacheMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations = suprnova::payments::migrations::migrations();
        migrations.push(Box::new(suprnova::render_cache::migration::Migration));
        migrations
    }
}

/// Same shape as `payments_webhook_hydration.rs`'s `spawn_server`: accept
/// `accepts` sequential connections against the webhook router, each served
/// on its own spawned task.
async fn spawn_payments_server(router: Router, accepts: usize) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn send_payments_webhook(
    addr: SocketAddr,
    path: &str,
    body: Bytes,
) -> (hyper::http::StatusCode, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let content_len = body.len().to_string();
    let req = hyper::Request::builder()
        .method("POST")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Type", "application/json")
        .header("Content-Length", content_len)
        .body(Full::new(body))
        .expect("build request");

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send timeout")
        .expect("send_request");
    let (parts, resp_body) = resp.into_parts();
    let collected = resp_body.collect().await.expect("collect body").to_bytes();
    (parts.status, collected)
}

#[tokio::test]
async fn payments_webhook_insert_and_update_advance_the_table_generation() {
    suprnova::render_cache::mark_installed();
    let db = TestDatabase::fresh::<PaymentsRenderCacheMigrator>()
        .await
        .expect("payments + render-cache migrations should apply cleanly");
    let conn = Arc::new(db.conn().clone());

    let provider_name: &'static str = "render-cache-payments-probe";
    let mock = Arc::new(MockPaymentProvider::new());
    let as_trait: Arc<dyn PaymentProvider> = mock.clone();
    PaymentProviderRegistry::bind(provider_name, as_trait);

    let router = webhook_routes(conn.clone());
    let addr = spawn_payments_server(router, 1).await;
    let path = format!("/webhooks/payments/{provider_name}");

    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("payments_webhook_events");
    let before = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);

    // A `subscription.created` webhook for a `sub_id` the mock provider has
    // never seen: `try_hydrate` pre-fetches `Subscription::get(sub_id)`
    // before opening its transaction, that lookup fails `NotFound`, so
    // `try_hydrate` returns `Err` without ever calling `db.begin()`. One
    // request therefore drives both: the audit-row insert (site 1) before
    // dispatch, and `mark_failed`'s update (site 13) on the error branch.
    let body = Bytes::from(
        serde_json::json!({
            "id": "evt_render_cache_probe",
            "type": "subscription.created",
            "data": { "object": { "id": "sub_never_registered", "customer": "cus_never_registered" } }
        })
        .to_string(),
    );
    let (status, resp) = send_payments_webhook(addr, &path, body).await;
    assert_eq!(
        status.as_u16(),
        503,
        "an unknown subscription id must fail hydration (503), not succeed: {}",
        String::from_utf8_lossy(&resp)
    );

    let after = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after,
        Some(before.unwrap_or(0) + 2),
        "the audit-row insert (site 1) and mark_failed's update (site 13) must each advance \
         the payments_webhook_events table generation - one request, two writes, two advances"
    );
}

// ---- fix5 item 3: assert a mirror-table site now that the deadlock is fixed
//
// Round 4 could only test the audit-row bookkeeping sites (1 and 13) - the
// actual mirror-table upserts (sites 3-12) deadlocked under SQLite when
// exercised through the real webhook route, because `advance_mirror_table`
// ran from inside `try_hydrate`'s own open transaction. Round 5 hoists that
// advance to run once, after the transaction commits, which removes the
// deadlock; this test is the proof - the same insert/update scenario round 4
// could not safely drive.
//
// Shared by the unconditional SQLite test below and the two `#[ignore]`d
// live-Postgres/live-MySQL tests further down, so the same scenario proves
// the fix on all three backends rather than three independently-written
// (and independently-driftable) copies.
async fn payments_webhook_mirror_scenario(
    conn: Arc<sea_orm::DatabaseConnection>,
    provider_name: &'static str,
) {
    let mock = Arc::new(MockPaymentProvider::new());
    let as_trait: Arc<dyn PaymentProvider> = mock.clone();
    PaymentProviderRegistry::bind(provider_name, as_trait);

    // Seed a subscription in the mock provider so the webhook has a known
    // sub_id/cust_id `Subscription::get` can resolve - same setup
    // `payments_webhook_hydration.rs` uses.
    let sub = mock
        .subscribe(SubscribeRequest {
            customer_ref: "cus_render_cache_mirror".into(),
            price_refs: vec!["price_a".into()],
            trial_days: None,
            idempotency_key: None,
            metadata: None,
        })
        .await
        .expect("mock subscribe");
    let sub_id = sub.provider_subscription_id.clone();
    let cust_id = sub.provider_customer_id.clone();

    let router = webhook_routes(conn.clone());
    let addr = spawn_payments_server(router, 2).await;
    let path = format!("/webhooks/payments/{provider_name}");

    let ledger = SqlGenerationLedger::new();
    let table = DependencyIdentity::table("payments_subscriptions");
    let before = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);

    // Insert path: subscription.created has no existing mirror row, so
    // `upsert_subscription`'s `None` branch runs `am.insert(db)` inside
    // `try_hydrate`'s transaction - the site that deadlocked before the
    // hoist.
    let insert_body = Bytes::from(
        serde_json::json!({
            "id": "evt_render_cache_mirror_created",
            "type": "subscription.created",
            "data": { "object": { "id": sub_id, "customer": cust_id } }
        })
        .to_string(),
    );
    let (status, resp) = send_payments_webhook(addr, &path, insert_body).await;
    assert_eq!(
        status.as_u16(),
        200,
        "insert webhook failed: {}",
        String::from_utf8_lossy(&resp)
    );
    let after_insert = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after_insert,
        Some(before.unwrap_or(0) + 1),
        "a subscription.created webhook insert must advance the payments_subscriptions table \
         generation, once, after the hydration transaction commits"
    );

    // Update path: subscription.updated finds the mirror row just inserted,
    // so `upsert_subscription`'s `Some` branch runs `am.update(db)`, again
    // inside the transaction.
    let update_body = Bytes::from(
        serde_json::json!({
            "id": "evt_render_cache_mirror_updated",
            "type": "subscription.updated",
            "data": { "object": { "id": sub_id, "customer": cust_id } }
        })
        .to_string(),
    );
    let (status, resp) = send_payments_webhook(addr, &path, update_body).await;
    assert_eq!(
        status.as_u16(),
        200,
        "update webhook failed: {}",
        String::from_utf8_lossy(&resp)
    );
    let after_update = ledger
        .current(&[table.digest()])
        .await
        .expect("current")
        .get(&table);
    assert_eq!(
        after_update,
        Some(after_insert.unwrap_or(0) + 1),
        "a subscription.updated webhook update must advance the payments_subscriptions table \
         generation again"
    );
}

#[tokio::test]
async fn payments_webhook_subscription_insert_and_update_advance_the_mirror_table() {
    suprnova::render_cache::mark_installed();
    let db = TestDatabase::fresh::<PaymentsRenderCacheMigrator>()
        .await
        .expect("payments + render-cache migrations should apply cleanly");
    let conn = Arc::new(db.conn().clone());
    payments_webhook_mirror_scenario(conn, "render-cache-payments-mirror-probe").await;
}
