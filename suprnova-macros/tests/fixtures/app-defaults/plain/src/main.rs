//! Without the tables nothing changes: date-times store as text and
//! `id()` stays signed. The key still follows the `id` field.

use chrono::{DateTime, Utc};
use suprnova::eloquent::EloquentModel;
use suprnova::model;

#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[model(table = "orders")]
pub struct Order {
    pub id: u64,
}

pub fn keys(post: <Post as EloquentModel>::Key, order: <Order as EloquentModel>::Key) -> (i64, u64) {
    (post, order)
}

pub fn storage(row: post::Model) -> (String, String) {
    (row.created_at, row.updated_at)
}

#[suprnova::main(flavor = "current_thread")]
async fn main() {
    assert!(!suprnova::boot::unsigned_ids(), "no table, no unsigned ids");
}
