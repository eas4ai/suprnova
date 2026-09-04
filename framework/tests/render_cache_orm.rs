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
use suprnova::eloquent::{MassPrunable, prune_one};
use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::ledger::SqlGenerationLedger;
use suprnova::{DB, FrameworkError, Model};
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
