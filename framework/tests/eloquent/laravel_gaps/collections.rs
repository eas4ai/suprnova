//! Loaded collections compare models by identity and constrain eager reads.

use crate::query_fixture::Fixture;
use suprnova::{Builder, Collection, Model, model};

/// A parent fixture lets constrained loading run on an existing collection.
#[model(table = "gap_collection_owners", timestamps = false, relations = {
    posts: HasMany<CollectionPost>,
})]
pub struct CollectionOwner {
    pub id: i64,
    pub name: String,
}

/// Both published and unpublished rows exercise the constraint.
#[model(table = "gap_collection_posts", timestamps = false)]
pub struct CollectionPost {
    pub id: i64,
    pub collection_owner_id: i64,
    pub published: bool,
}

/// A custom text key checks that lookup does not assume an integer id.
#[model(table = "gap_collection_codes", timestamps = false, primary_key = "code", auto_increment = false, hidden = ["code"])]
pub struct CollectionCode {
    pub code: String,
    pub label: String,
}

fn owner(id: i64, name: &str) -> CollectionOwner {
    CollectionOwner {
        id,
        name: name.into(),
        ..Default::default()
    }
}

#[test]
fn lookup_uses_keys_and_keeps_collection_order() {
    let rows = Collection::from(vec![owner(3, "first"), owner(1, "one"), owner(3, "last")]);
    assert_eq!(rows.find(3).unwrap().name, "first");
    assert_eq!(
        rows.find_model(&owner(3, "another load")).unwrap().name,
        "first"
    );
    assert_eq!(rows.find_many([1, 3, 3, 99]).model_keys(), vec![3, 1, 3]);
    assert!(rows.find(99).is_none());
    let fallback = owner(7, "default");
    assert_eq!(rows.find_or(3, &fallback).name, "first");
    assert_eq!(rows.find_or(99, &fallback).name, "default");
    assert!(rows.find_many(Vec::<i64>::new()).is_empty());
    let empty = Collection::<CollectionOwner>::new();
    assert!(empty.find(3).is_none());
    assert!(empty.find_many([1, 3]).is_empty());
}

#[test]
fn text_primary_keys_are_supported() {
    let rows = Collection::from(vec![CollectionCode {
        code: "alpha".into(),
        label: "A".into(),
        ..Default::default()
    }]);
    assert!(rows[0].to_array().get("code").is_none());
    assert_eq!(rows.find("alpha").unwrap().label, "A");
    assert_eq!(rows.clone().unique_models().len(), 1);
    assert!(rows.clone().diff_models(&rows).is_empty());
    assert_eq!(rows.find_many(["alpha", "missing"]).len(), 1);
    assert!(rows.find("missing").is_none());
}

#[test]
fn unique_keeps_last_copy_in_first_key_order_and_diff_uses_identity() {
    let rows = Collection::from(vec![owner(3, "first"), owner(1, "one"), owner(3, "last")]);
    let unique = rows.clone().unique_models();
    assert_eq!(unique.model_keys(), vec![3, 1]);
    assert_eq!(unique[0].name, "last");
    assert_eq!(
        rows.clone()
            .diff_models(&Collection::from(vec![owner(3, "changed")]))
            .model_keys(),
        vec![1]
    );
    assert_eq!(rows.diff_models(&Collection::new()).len(), 3);
    assert!(
        Collection::<CollectionOwner>::new()
            .unique_models()
            .is_empty()
    );
    assert_eq!(
        Collection::from(vec![1, 1, 2]).unique().into_vec(),
        vec![1, 2]
    );
    assert_eq!(
        Collection::from(vec![1, 2])
            .diff(Collection::from(vec![2]))
            .into_vec(),
        vec![1]
    );
}

async fn fixture() -> Fixture {
    let fixture = Fixture::sqlite().await;
    fixture
        .exec("CREATE TABLE gap_collection_owners (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await;
    fixture.exec("CREATE TABLE gap_collection_posts (id INTEGER PRIMARY KEY, collection_owner_id INTEGER NOT NULL, published BOOLEAN NOT NULL)").await;
    fixture
        .exec("INSERT INTO gap_collection_owners VALUES (1, 'one'), (3, 'three')")
        .await;
    fixture
        .exec("INSERT INTO gap_collection_posts VALUES (1, 1, 1), (2, 1, 0), (3, 3, 1), (4, 3, 0)")
        .await;
    fixture
}

#[tokio::test]
async fn load_with_filters_existing_models_and_replaces_loaded_relations() {
    let _fixture = fixture().await;
    let mut rows = CollectionOwner::query()
        .order_by("id", suprnova::Direction::Asc)
        .get()
        .await
        .unwrap();
    rows.load(["posts"]).await.unwrap();
    assert_eq!(rows[0].posts_loaded().len(), 2);
    rows.load_with("posts", |q: Builder<CollectionPost>| {
        q.filter("published", true)
    })
    .await
    .unwrap();
    assert_eq!(rows.model_keys(), vec![1, 3]);
    for row in &rows {
        assert_eq!(row.posts_loaded().len(), 1);
        assert!(row.posts_loaded()[0].published);
    }
    rows.load_with("posts", |q: Builder<CollectionPost>| q.filter("id", 99))
        .await
        .unwrap();
    assert!(rows.iter().all(|row| row.posts_loaded().is_empty()));
    let mut empty = Collection::<CollectionOwner>::new();
    empty
        .load_with("posts", |q: Builder<CollectionPost>| {
            q.filter("published", true)
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn load_with_reports_unknown_relations_and_wrong_target_types() {
    let mut empty = Collection::<CollectionOwner>::new();
    empty
        .load_with("missing", |_: Builder<CollectionPost>| {
            panic!("an empty collection must not evaluate its constraint")
        })
        .await
        .unwrap();
    let _fixture = fixture().await;
    let mut rows = CollectionOwner::all().await.unwrap();
    assert!(
        rows.load_with("missing", |q: Builder<CollectionPost>| q)
            .await
            .is_err()
    );
    assert!(
        rows.load_with("posts", |q: Builder<CollectionOwner>| q)
            .await
            .is_err()
    );
}
