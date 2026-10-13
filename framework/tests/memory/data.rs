//! MEM-002 and MEM-003 on Eloquent: runtime casts, eager loading, pluck.

use suprnova::eloquent::casts::AsString;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, model};

use crate::support::{Heap, exclusive};

#[model(table = "mem_users", relations = {
    roles: BelongsToMany<MemRole, MemRoleUser>,
    posts: HasMany<MemPost>,
})]
pub struct MemUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "mem_roles", relations = {
    users: BelongsToMany<MemUser, MemRoleUser>,
})]
pub struct MemRole {
    pub id: i64,
    pub body: String,
}

#[model(table = "mem_role_user", primary_key = "id")]
pub struct MemRoleUser {
    pub id: i64,
    pub mem_user_id: i64,
    pub mem_role_id: i64,
}

#[model(table = "mem_posts")]
pub struct MemPost {
    pub id: i64,
    pub mem_user_id: i64,
    pub title: String,
    pub body: String,
}

const BODY: usize = 128 * 1024;
const ROWS: usize = 8;

/// Four users; eight roles and eight posts, each with a 128 KiB body;
/// every user holds every role and two posts.
async fn seed() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("a database");
    for sql in [
        "CREATE TABLE mem_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE mem_roles (id INTEGER PRIMARY KEY AUTOINCREMENT, body TEXT NOT NULL)",
        "CREATE TABLE mem_role_user (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         mem_user_id INTEGER NOT NULL, mem_role_id INTEGER NOT NULL)",
        "CREATE TABLE mem_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         mem_user_id INTEGER NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("a table");
    }
    let body = "b".repeat(BODY);
    let mut users = Vec::new();
    for i in 0..4 {
        users.push(
            MemUser::create(attrs! { name: format!("u{i}") })
                .await
                .expect("a user"),
        );
    }
    for i in 0..ROWS {
        let role = MemRole::create(attrs! { body: body.clone() })
            .await
            .expect("a role");
        for user in &users {
            user.roles().attach(role.id).await.expect("attach");
        }
        MemPost::create(attrs! {
            mem_user_id: users[i % users.len()].id,
            title: format!("t{i}"),
            body: body.clone(),
        })
        .await
        .expect("a post");
    }
    db
}

/// MEM-003: eager loading reads each key from its field, so it allocates
/// no more than a plain query of the same rows plus the attachments.
#[tokio::test]
async fn mem_audit_eager_loading_reads_keys_not_rows() {
    let _lock = exclusive().await;
    let _db = seed().await;
    MemUser::query()
        .with(["roles", "posts"])
        .get()
        .await
        .expect("a warm-up");

    let heap = Heap::start();
    let before = heap.bytes();
    let plain_roles = MemRole::query().get().await.expect("roles");
    let plain_posts = MemPost::query().get().await.expect("posts");
    let plain = heap.bytes() - before;
    drop((plain_roles, plain_posts));
    let before = heap.bytes();
    let users = MemUser::query()
        .with(["roles", "posts"])
        .get()
        .await
        .expect("eager");
    let eager = heap.bytes() - before;
    drop(heap);
    assert_eq!(users.len(), 4);
    // Each user gets its own copy of every role it holds (4 x 8 bodies)
    // and of its posts (8 bodies); the plain query reads 16 bodies.
    let attachments = (4 * ROWS + ROWS) as u64 * BODY as u64;
    assert!(
        eager < plain + attachments + (2 * ROWS * BODY) as u64,
        "eager loading allocated {eager} bytes; the plain rows {plain}, the attachments {attachments}"
    );
}

/// MEM-003: a runtime cast converts the column in place, without copying it.
#[tokio::test]
async fn mem_audit_a_runtime_cast_does_not_copy_its_column() {
    let _lock = exclusive().await;
    let _db = seed().await;
    MemPost::query()
        .with_casts(suprnova::casts! { body = AsString })
        .get()
        .await
        .expect("a warm-up");

    let heap = Heap::start();
    let before = heap.bytes();
    let posts = MemPost::query()
        .with_casts(suprnova::casts! { body = AsString })
        .get()
        .await
        .expect("cast body");
    let body_cast = heap.bytes() - before;
    drop(posts);
    let before = heap.bytes();
    let posts = MemPost::query()
        .with_casts(suprnova::casts! { title = AsString })
        .get()
        .await
        .expect("cast title");
    let title_cast = heap.bytes() - before;
    drop(posts);
    drop(heap);
    assert!(
        body_cast < title_cast + (ROWS * BODY / 2) as u64,
        "casting the body allocated {body_cast} bytes, casting the title {title_cast}"
    );
}

/// MEM-002: plucking a field few models have keeps no room for the rest.
#[tokio::test]
async fn mem_audit_plucking_a_missing_field_keeps_no_room() {
    let _lock = exclusive().await;
    let _db = seed().await;
    let posts = MemPost::query().get().await.expect("posts");
    let plucked = posts.pluck::<String>("no_such_column");
    assert!(plucked.0.is_empty());
    assert_eq!(plucked.0.capacity(), 0, "pluck kept room for every model");
}
