//! Relations whose key types cannot match: each one fails the build with
//! an error that names both models.

use suprnova::bson::oid::ObjectId;

/// Keyed by an `ObjectId`; its posts hold a `String`.
#[suprnova::document(collection = "writers", relations = { posts: HasMany<Post> })]
pub struct Writer {
    pub name: String,
}

#[suprnova::document(collection = "posts", relations = { writer: BelongsTo<Writer> })]
pub struct Post {
    pub writer_id: String,
}

/// Its members keep its key in an array of strings.
#[suprnova::document(collection = "groups", relations = { members: BelongsToMany<Member> })]
pub struct Group {
    pub member_ids: Vec<ObjectId>,
}

#[suprnova::document(collection = "members")]
pub struct Member {
    pub group_ids: Vec<String>,
}

/// An SQL model keyed by an `i64`; its notes hold an `ObjectId`.
#[suprnova::model(table = "accounts", relations = { notes: HasManyDocuments<Note> })]
pub struct Account {
    pub id: i64,
}

#[suprnova::document(collection = "notes", relations = { account: BelongsToModel<Account> })]
pub struct Note {
    pub account_id: ObjectId,
}

fn main() {}
