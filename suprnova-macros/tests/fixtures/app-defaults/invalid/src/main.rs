//! The code is fine; only the manifest is wrong.

use chrono::{DateTime, Utc};
use suprnova::model;

#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[suprnova::main(flavor = "current_thread")]
async fn main() {}
