//! Models over the tables of the fixture's Laravel application: `posts`
//! (nullable timestamps, soft deletes, a `json` column), `comments` and
//! `images` (polymorphic children of a post), and Laravel's `users` as a
//! polymorphic owner of spatie's assignments.
//!
//! `LdbPost` is `App\Models\Post` with the alias `post`, as a Laravel
//! application that adopted `Relation::morphMap` late stores it both ways.
//! `ldb_tags` and `ldb_taggables` are tables the test creates, for a
//! `MorphToMany` write.

use std::any::Any;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use suprnova::{Authenticatable, HasRoles, model};

#[model(
    table = "posts",
    morph_type = "App\\Models\\Post",
    morph_aliases = ["post"],
    fillable = ["title", "meta"],
    timestamps,
    soft_deletes,
    casts = {
        meta = suprnova::AsOptionalNativeJson<serde_json::Value>,
        created_at = suprnova::AsOptionalNaiveDateTime,
        updated_at = suprnova::AsOptionalNaiveDateTime,
        deleted_at = suprnova::AsOptionalNaiveDateTime,
    },
    relations = {
        comments: MorphMany<LdbComment> { name = "commentable" },
        image: MorphOne<LdbImage> { name = "imageable" },
        tags: MorphToMany<LdbTag, LdbTaggable> { name = "taggable" },
    },
)]
pub struct LdbPost {
    pub id: u64,
    pub title: String,
    pub meta: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(
    table = "comments",
    fillable = ["body", "commentable_type", "commentable_id"],
    timestamps,
    casts = {
        created_at = suprnova::AsOptionalNaiveDateTime,
        updated_at = suprnova::AsOptionalNaiveDateTime,
    },
    relations = {
        commentable: MorphTo { name = "commentable", targets = [LdbPost] },
    },
)]
pub struct LdbComment {
    pub id: u64,
    pub body: String,
    pub commentable_type: String,
    pub commentable_id: u64,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[model(
    table = "images",
    fillable = ["url", "imageable_type", "imageable_id"],
    timestamps,
    casts = {
        created_at = suprnova::AsOptionalNaiveDateTime,
        updated_at = suprnova::AsOptionalNaiveDateTime,
    },
    relations = {
        imageable: MorphTo { name = "imageable", targets = [LdbPost] },
    },
)]
pub struct LdbImage {
    pub id: u64,
    pub url: String,
    pub imageable_type: String,
    pub imageable_id: u64,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[model(table = "ldb_tags")]
pub struct LdbTag {
    pub id: u64,
    pub name: String,
}

#[model(table = "ldb_taggables", primary_key = "id")]
pub struct LdbTaggable {
    pub id: u64,
    pub ldb_tag_id: u64,
    pub taggable_id: u64,
    pub taggable_type: String,
}

/// Create `ldb_tags` and `ldb_taggables` on `conn`.
pub async fn create_tag_tables(conn: &sea_orm::DatabaseConnection) {
    use sea_orm::ConnectionTrait;
    let (key, reference) = match conn.get_database_backend() {
        sea_orm::DbBackend::Sqlite => ("INTEGER PRIMARY KEY AUTOINCREMENT", "INTEGER"),
        sea_orm::DbBackend::Postgres => ("BIGSERIAL PRIMARY KEY", "BIGINT"),
        _ => (
            "BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY",
            "BIGINT UNSIGNED",
        ),
    };
    for sql in [
        format!("CREATE TABLE ldb_tags (id {key}, name VARCHAR(255) NOT NULL)"),
        format!(
            "CREATE TABLE ldb_taggables (id {key}, ldb_tag_id {reference} NOT NULL, \
             taggable_id {reference} NOT NULL, taggable_type VARCHAR(255) NOT NULL)"
        ),
    ] {
        conn.execute_unprepared(&sql)
            .await
            .expect("create a tag table");
    }
}

/// Laravel's `users`, as the morph owner of spatie's assignments:
/// `App\Models\User`, with the alias `user` a morph map would give it.
#[model(
    table = "users",
    morph_type = "App\\Models\\User",
    morph_aliases = ["user"],
    timestamps,
    casts = {
        created_at = suprnova::AsOptionalNaiveDateTime,
        updated_at = suprnova::AsOptionalNaiveDateTime,
    },
)]
pub struct LdbUser {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl Authenticatable for LdbUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl HasRoles for LdbUser {}
