//! LDB-006: a model over a table Laravel created reads and writes rows
//! whose timestamps are NULL, soft-deletes through `deleted_at`, and reads
//! and writes Laravel's `json` columns; the scaffold's `User` finds,
//! updates and creates rows in the `users` table Laravel 13's skeleton
//! creates.

use suprnova::{Model, attrs};

use crate::models::LdbPost;
use crate::models::note::Note;
use crate::on_every_engine;
use crate::scaffold::user::User;
use crate::support::{self, Engine};

/// The fixture's posts: 1 with a `json` value and timestamps, 2 trashed,
/// 3 with NULL everywhere Laravel allows it.
async fn laravel_rows_read_and_write(engine: Engine) {
    let (db, _) = support::laravel(engine).await;
    let _bound = support::bind(&db.conn);

    // Reads: NULL timestamps, a `json` value, Laravel's trashed row.
    let first = LdbPost::find(1u64).await.expect("find").expect("post 1");
    assert_eq!(
        first.meta,
        Some(serde_json::json!({ "tags": ["a", "b"], "draft": false })),
        "{engine:?}: the json column"
    );
    assert!(first.created_at.is_some() && first.updated_at.is_some());
    let bare = LdbPost::find(3u64).await.expect("find").expect("post 3");
    assert_eq!(
        (
            bare.meta.clone(),
            bare.created_at,
            bare.updated_at,
            bare.deleted_at
        ),
        (None, None, None, None),
        "{engine:?}: NULL json and timestamps"
    );
    let visible: Vec<u64> = LdbPost::query()
        .order_by_asc("id")
        .get()
        .await
        .expect("posts")
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(
        visible,
        [1, 3],
        "{engine:?}: Laravel's trashed post 2 is hidden"
    );
    let trashed = LdbPost::query()
        .only_trashed()
        .get()
        .await
        .expect("trashed posts");
    assert_eq!(trashed.iter().map(|p| p.id).collect::<Vec<_>>(), [2]);

    // Writes: a row with NULL timestamps updates, a json value round-trips.
    let meta = serde_json::json!({ "tags": ["x"], "nested": { "n": 1.5, "ok": true } });
    let updated = bare
        .update(attrs! { title: "Edited", meta: meta.clone() })
        .await
        .expect("update a row with NULL timestamps");
    assert_eq!(updated.title, "Edited");
    let reread = LdbPost::find(3u64).await.expect("find").expect("post 3");
    assert_eq!(reread.title, "Edited");
    assert_eq!(
        reread.meta,
        Some(meta.clone()),
        "{engine:?}: json round trip"
    );
    assert!(
        reread.created_at.is_none(),
        "{engine:?}: created_at stays NULL"
    );
    assert!(reread.updated_at.is_some(), "{engine:?}: updated_at is set");
    let stored = support::rows(&db.conn, "SELECT meta FROM posts WHERE id = 3").await;
    let raw = &stored[0]["meta"];
    let decoded = match raw {
        serde_json::Value::String(text) => serde_json::from_str(text).expect("JSON text"),
        other => other.clone(),
    };
    assert_eq!(
        decoded, meta,
        "{engine:?}: the column holds the JSON itself"
    );

    let created = <LdbPost as Model>::create(attrs! { title: "New", meta: meta.clone() })
        .await
        .expect("create");
    let created = LdbPost::find(created.id)
        .await
        .expect("find")
        .expect("created");
    assert_eq!(
        created.meta,
        Some(meta),
        "{engine:?}: json written on create"
    );
    assert!(created.created_at.is_some());

    // A soft delete of a Laravel row sets `deleted_at`.
    let first = LdbPost::find(1u64).await.expect("find").expect("post 1");
    first.delete().await.expect("soft delete");
    let stored = support::rows(&db.conn, "SELECT deleted_at FROM posts WHERE id = 1").await;
    assert!(
        !stored[0]["deleted_at"].is_null(),
        "{engine:?}: the soft delete left deleted_at NULL"
    );
    assert!(LdbPost::find(1u64).await.expect("find").is_none());
    assert_eq!(support::count(&db.conn, "posts", "id = 1").await, 1);
}

on_every_engine!(laravel_rows_read_and_write =>
    ldb_006_null_timestamps_json_and_soft_deletes_sqlite,
    ldb_006_null_timestamps_json_and_soft_deletes_postgres,
    ldb_006_null_timestamps_json_and_soft_deletes_mysql);

/// The scaffold's `User` over the `users` Laravel 13's skeleton created:
/// `id` unsigned on MySQL, nullable timestamps.
async fn scaffold_user_on_laravels_users(engine: Engine) {
    let (db, _) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);

    let taylor = User::find(1u64).await.expect("find").expect("user 1");
    assert_eq!(
        (taylor.name.as_str(), taylor.email.as_str()),
        ("Taylor", "taylor@example.com")
    );
    let jeffrey = User::find_by_email("jeffrey@example.com")
        .await
        .expect("find by email")
        .expect("jeffrey");
    assert_eq!(jeffrey.id, 4);
    assert!(jeffrey.created_at.is_none() && jeffrey.updated_at.is_none());

    let renamed = jeffrey
        .update(attrs! { name: "Jeffrey Way" })
        .await
        .expect("update a user with NULL timestamps");
    assert_eq!(renamed.name, "Jeffrey Way");
    let reread = User::find(4u64).await.expect("find").expect("user 4");
    assert_eq!(reread.name, "Jeffrey Way", "{engine:?}");

    // The SeaORM types the scaffold's model re-exports read and write the
    // same table.
    {
        use crate::scaffold::user::{ActiveModel, Column, Entity};
        use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
        let row = Entity::find()
            .filter(Column::Email.eq("abigail@example.com"))
            .one(&db.conn)
            .await
            .expect("query the entity")
            .expect("abigail");
        assert_eq!(row.id, suprnova::StoredU64(2), "{engine:?}");
        let mut active: ActiveModel = row.into();
        active.name = Set("Abigail Otwell".to_owned());
        active
            .update(&db.conn)
            .await
            .expect("update through the entity");
        let reread = User::find(2u64).await.expect("find").expect("user 2");
        assert_eq!(reread.name, "Abigail Otwell", "{engine:?}");
    }

    let created = User::create("Nuno", "nuno@example.com", "created-by-suprnova")
        .await
        .expect("create a user");
    assert_eq!(
        created.id, 5,
        "{engine:?}: the next id after Laravel's rows"
    );
    let found = User::find_by_email("nuno@example.com")
        .await
        .expect("find")
        .expect("nuno");
    assert!(
        found
            .verify_password("created-by-suprnova")
            .expect("verify")
    );
    assert!(found.created_at.is_some());
}

on_every_engine!(scaffold_user_on_laravels_users =>
    ldb_006_scaffold_user_finds_updates_and_creates_sqlite,
    ldb_006_scaffold_user_finds_updates_and_creates_postgres,
    ldb_006_scaffold_user_finds_updates_and_creates_mysql);

/// PAR-080 on a Laravel database: the scaffold's notes migration creates
/// `notes` with its key into the `users` table Laravel's skeleton created
/// (`id` unsigned on MySQL), and the scaffold's `Note` reads one user's
/// notes and no other user's.
async fn scaffold_notes_on_laravels_users(engine: Engine) {
    let (db, _) = support::laravel(engine).await;
    crate::scaffold::migrate(&db.conn).await.expect("migrate");
    let _bound = support::bind(&db.conn);

    for (user_id, title) in [(1u64, "Taylor's note"), (2u64, "Abigail's note")] {
        <Note as Model>::create(attrs! {
            user_id: user_id,
            title: title,
            body: Option::<String>::None,
        })
        .await
        .unwrap_or_else(|e| panic!("{engine:?}: create a note for user {user_id}: {e}"));
    }
    let taylors: Vec<String> = Note::owned_by(1)
        .get()
        .await
        .expect("Taylor's notes")
        .iter()
        .map(|note| note.title.clone())
        .collect();
    assert_eq!(taylors, ["Taylor's note"], "{engine:?}");
    assert!(
        Note::owned_by(1)
            .filter("title", "Abigail's note")
            .first()
            .await
            .expect("query")
            .is_none(),
        "{engine:?}: another user's note is not Taylor's"
    );
}

on_every_engine!(scaffold_notes_on_laravels_users =>
    kit_scaffold_notes_belong_to_laravels_users_sqlite,
    kit_scaffold_notes_belong_to_laravels_users_postgres,
    kit_scaffold_notes_belong_to_laravels_users_mysql);

/// An application's existing notes schema and rows survive the kit migration.
async fn scaffold_keeps_existing_notes(engine: Engine) {
    use sea_orm::ConnectionTrait;

    let (db, _) = support::laravel(engine).await;
    db.conn
        .execute_unprepared("CREATE TABLE notes (legacy_title VARCHAR(255) NOT NULL)")
        .await
        .expect("create the application's notes table");
    db.conn
        .execute_unprepared("INSERT INTO notes (legacy_title) VALUES ('Keep this note')")
        .await
        .expect("insert the application's note");
    crate::scaffold::migrate(&db.conn)
        .await
        .expect("migrate without replacing the existing notes table");
    let rows = db
        .conn
        .query_all_raw(sea_orm::Statement::from_string(
            db.conn.get_database_backend(),
            "SELECT legacy_title FROM notes".to_owned(),
        ))
        .await
        .expect("the application's column survives");
    assert_eq!(rows.len(), 1, "{engine:?}: the application's row survives");
    assert_eq!(
        rows[0].try_get::<String>("", "legacy_title").unwrap(),
        "Keep this note",
        "{engine:?}"
    );
}

on_every_engine!(scaffold_keeps_existing_notes =>
    kit_scaffold_existing_notes_survive_sqlite,
    kit_scaffold_existing_notes_survive_postgres,
    kit_scaffold_existing_notes_survive_mysql);
