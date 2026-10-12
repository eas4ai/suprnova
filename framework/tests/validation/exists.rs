//! Laravel's `exists`: the value, or each element of an array, names a
//! row.

use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
use suprnova::testing::TestContainer;
use suprnova::{AsyncRule, DbConnection, Exists, Unique, ValidationErrors};

/// Two teams' tags. `TestContainer` is thread-local, so every test builds
/// its own in-memory database and installs it on its own thread.
async fn tags_db() -> DbConnection {
    let raw = Database::connect("sqlite::memory:").await.unwrap();
    for sql in [
        "CREATE TABLE tags (id INTEGER PRIMARY KEY, team_id INTEGER NOT NULL, slug TEXT NOT NULL)",
        "INSERT INTO tags (id, team_id, slug) VALUES (1, 7, 'rust'), (2, 7, 'laravel'), (3, 8, 'php')",
    ] {
        raw.execute_raw(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
            .await
            .unwrap();
    }
    DbConnection::from_raw(raw)
}

fn sorted_keys(errs: &ValidationErrors) -> Vec<String> {
    let mut keys: Vec<String> = errs.errors.keys().cloned().collect();
    keys.sort();
    keys
}

#[tokio::test]
async fn a_value_that_names_a_row_passes() {
    let _guard = TestContainer::fake();
    TestContainer::singleton(tags_db().await);

    assert!(Exists::new("tags", "slug").passes("rust").await.is_ok());
    let err = Exists::new("tags", "slug").passes("go").await.unwrap_err();
    assert_eq!(err.key, "validation-exists");
    assert_eq!(err.args.get("column"), Some(&"slug".into()));
}

#[tokio::test]
async fn where_eq_scopes_the_lookup() {
    let _guard = TestContainer::fake();
    TestContainer::singleton(tags_db().await);

    let team_seven = Exists::new("tags", "slug").where_eq("team_id", 7);
    assert!(team_seven.passes("laravel").await.is_ok());
    assert!(
        team_seven.passes("php").await.is_err(),
        "php exists, but for team 8"
    );
    let both = Exists::new("tags", "slug")
        .where_eq("team_id", 8)
        .where_eq("id", 3);
    assert!(both.passes("php").await.is_ok());
}

#[tokio::test]
async fn a_typed_value_binds_as_itself() {
    let _guard = TestContainer::fake();
    TestContainer::singleton(tags_db().await);

    let rule = Exists::new("tags", "id");
    let mut errs = ValidationErrors::new();
    rule.check_value(1i64, &mut errs, "tag_id").await;
    rule.check_value(4i64, &mut errs, "other_tag_id").await;
    assert_eq!(sorted_keys(&errs), ["other_tag_id"]);
}

#[tokio::test]
async fn check_each_reports_every_missing_element_under_its_index() {
    let _guard = TestContainer::fake();
    TestContainer::singleton(tags_db().await);

    let mut errs = ValidationErrors::new();
    Exists::new("tags", "id")
        .where_eq("team_id", 7)
        .check_each(&[1i64, 3, 2, 9], &mut errs, "tag_ids")
        .await;
    assert_eq!(sorted_keys(&errs), ["tag_ids.1", "tag_ids.3"]);

    let mut none = ValidationErrors::new();
    Exists::new("tags", "slug")
        .check_each::<String>(&[], &mut none, "slugs")
        .await;
    assert!(none.is_empty(), "an empty array has nothing to find");
}

/// An identifier the allowlist refuses fails the field without a query,
/// and the message says nothing about the SQL: a validation message is
/// rendered into the response body.
#[tokio::test]
async fn a_hostile_identifier_fails_without_saying_why() {
    let _guard = TestContainer::fake();
    TestContainer::singleton(tags_db().await);

    for rule in [
        Exists::new("tags; DROP TABLE tags", "slug"),
        Exists::new("tags", "slug) OR (1=1"),
        Exists::new("tags", "slug").where_eq("team_id = team_id OR 1", 7),
    ] {
        let err = rule.passes("rust").await.unwrap_err();
        assert_eq!(err.key, "validation-unchecked");
        for leaked in ["DROP", "OR", "identifier", "tags"] {
            assert!(!err.fallback.contains(leaked), "{leaked}: {}", err.fallback);
        }
    }
    assert!(
        Exists::new("tags", "slug").passes("rust").await.is_ok(),
        "the table is still there"
    );
}

#[tokio::test]
async fn a_database_failure_does_not_reach_the_message() {
    let _guard = TestContainer::fake();
    TestContainer::singleton(tags_db().await);

    let err = Exists::new("no_such_table", "id")
        .passes("1")
        .await
        .unwrap_err();
    assert_eq!(err.key, "validation-unchecked");
    assert!(!err.fallback.contains("no_such_table"), "{}", err.fallback);

    let unique = Unique::new("no_such_table", "email")
        .passes("a@example.com")
        .await
        .unwrap_err();
    assert_eq!(unique.key, "validation-unchecked");
    assert!(
        !unique.fallback.contains("no_such_table"),
        "{}",
        unique.fallback
    );
}

/// When the database fails, `check_each` stops: the field has failed, and
/// asking about every other value would repeat the failure once each.
#[tokio::test]
async fn check_each_stops_at_the_first_database_failure() {
    let _guard = TestContainer::fake();
    TestContainer::singleton(tags_db().await);

    let mut errs = ValidationErrors::new();
    Exists::new("no_such_table", "id")
        .check_each(&[1i64, 2, 1], &mut errs, "tag_ids")
        .await;
    assert_eq!(sorted_keys(&errs), ["tag_ids.0", "tag_ids.2"]);
    for messages in errs.errors.values() {
        assert!(messages.iter().all(|msg| msg.key == "validation-unchecked"));
    }
}

#[tokio::test]
async fn a_repeated_element_is_reported_under_every_index() {
    let _guard = TestContainer::fake();
    TestContainer::singleton(tags_db().await);

    let mut errs = ValidationErrors::new();
    Exists::new("tags", "id")
        .check_each(&[4i64, 1, 4, 9, 4], &mut errs, "tag_ids")
        .await;
    assert_eq!(
        sorted_keys(&errs),
        ["tag_ids.0", "tag_ids.2", "tag_ids.3", "tag_ids.4"]
    );
}

#[tokio::test]
async fn a_bound_value_cannot_widen_the_match() {
    let _guard = TestContainer::fake();
    TestContainer::singleton(tags_db().await);

    let rule = Exists::new("tags", "slug");
    for value in ["' OR '1'='1", "rust' --", "%"] {
        assert!(rule.passes(value).await.is_err(), "{value}");
    }
}

/// ROOT-26: inside a transaction, `exists` and `unique` read through it,
/// so they see the rows the transaction wrote and have not committed. On a
/// pool of two connections the check used to run on the other one and
/// miss them; on a pool of one it waited for the connection the
/// transaction held.
#[tokio::test]
async fn presence_rules_read_through_the_ambient_transaction() {
    use sea_orm::ConnectOptions;
    use suprnova::{DB, FrameworkError};

    let dir = tempfile::tempdir().expect("a temporary directory");
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("tags.db").display());
    let mut options = ConnectOptions::new(url);
    options.max_connections(2).min_connections(1);
    let raw = Database::connect(options).await.unwrap();
    raw.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        "CREATE TABLE tags (id INTEGER PRIMARY KEY, team_id INTEGER NOT NULL, slug TEXT NOT NULL)"
            .to_string(),
    ))
    .await
    .unwrap();
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(raw));

    let (exists, unique) = DB::transaction(|_tx| {
        Box::pin(async move {
            DB::statement(
                "INSERT INTO tags (id, team_id, slug) VALUES (1, 7, 'go')",
                Vec::<sea_orm::Value>::new(),
            )
            .await?;
            let exists = Exists::new("tags", "slug").passes("go").await.is_ok();
            let unique = Unique::new("tags", "slug").passes("go").await.is_ok();
            Ok::<_, FrameworkError>((exists, unique))
        })
    })
    .await
    .expect("the transaction commits");
    assert!(exists, "exists sees the row the transaction inserted");
    assert!(!unique, "unique sees it too, so the value is taken");
}
