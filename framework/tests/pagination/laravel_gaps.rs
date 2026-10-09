use suprnova::{LengthAwarePaginator, Paginated};

#[test]
fn an_empty_page_has_page_one_and_links_to_it() {
    let page = LengthAwarePaginator::<i32>::new(vec![], 0, 10, 1);
    assert_eq!(page.last_page, 1);
    let body = serde_json::to_value(&page).unwrap();
    assert_eq!(body["last_page_url"], "?page=1");
    assert_eq!(body["links"][1]["page"], 1);
    assert_eq!(body["links"][1]["active"], true);
    assert!(
        page.links_iter()
            .any(|(rel, url)| rel == "last" && url == "?page=1")
    );
}

use suprnova::testing::TestDatabase;
use suprnova::{Context, Cursor, CursorDirection, CursorPaginator, Model, Paginator, attrs};

#[suprnova::model(table = "gap_page_rows", timestamps = false)]
struct PageRow {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
}
async fn fixture() -> TestDatabase {
    Context::test_clear_query();
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared("CREATE TABLE gap_page_rows (id INTEGER PRIMARY KEY, name TEXT NOT NULL, created_at INTEGER NOT NULL)").await.unwrap();
    for (id, created_at) in [(1, 2), (2, 1), (3, 1), (4, 2), (5, 1)] {
        PageRow::create(attrs! { id: id, name: format!("row-{id}"), created_at: created_at })
            .await
            .unwrap();
    }
    db
}

#[test]
fn through_preserves_every_pages_metadata() {
    let page = LengthAwarePaginator::new(vec!["a", "b"], 22, 2, 3)
        .with_path("/rows")
        .with_page_name("p");
    let before = serde_json::to_value(&page).unwrap();
    let page = page.through(|item| item.to_uppercase());
    let mut expected = before;
    expected["data"] = serde_json::json!(["A", "B"]);
    assert_eq!(serde_json::to_value(page).unwrap(), expected);
    let page = Paginator::new(vec!["a", "b"], 3, 2, true)
        .with_path("/rows")
        .with_page_name("p");
    let before = serde_json::to_value(&page).unwrap();
    assert_eq!(
        serde_json::to_value(page.through(String::from)).unwrap(),
        before
    );
    let page = CursorPaginator::new(vec!["a"], 2, Some("next".into()), Some("prev".into()))
        .with_path("/rows")
        .with_cursor_name("after")
        .with_current_cursor("now");
    let before = serde_json::to_value(&page).unwrap();
    let page = page.through(String::from);
    assert_eq!(page.current_cursor.as_deref(), Some("now"));
    assert_eq!(page.cursor_name.as_deref(), Some("after"));
    assert_eq!(serde_json::to_value(page).unwrap(), before);
    let page = LengthAwarePaginator::<i32>::new(vec![], 0, 10, 1)
        .through(|_| panic!("empty pages never transform"));
    assert!(page.data.is_empty());
}

#[tokio::test]
async fn cursor_pages_keep_tied_order_columns_in_both_directions() {
    let _db = fixture().await;
    let query = || {
        PageRow::query()
            .order_by_asc("created_at")
            .order_by_asc("id")
    };
    let first = query().cursor_paginate(2).await.unwrap();
    assert_eq!(first.data.iter().map(|r| r.id).collect::<Vec<_>>(), [2, 3]);
    let wire = first.next_cursor.as_ref().unwrap();
    let cursor = Cursor::decode(wire).unwrap();
    assert_eq!(
        cursor.parameter("id"),
        Some(&sea_orm::Value::BigInt(Some(3)))
    );
    assert_eq!(
        cursor.parameter("created_at"),
        Some(&sea_orm::Value::BigInt(Some(1)))
    );
    assert!(cursor.parameter("absent").is_none());
    Context::test_set_query("cursor", wire);
    let second = query().cursor_paginate(2).await.unwrap();
    assert_eq!(second.data.iter().map(|r| r.id).collect::<Vec<_>>(), [5, 1]);
    Context::test_set_query("cursor", second.prev_cursor.as_ref().unwrap());
    let back = query().cursor_paginate(2).await.unwrap();
    assert_eq!(back.data.iter().map(|r| r.id).collect::<Vec<_>>(), [2, 3]);
    Context::test_set_query("cursor", second.next_cursor.as_ref().unwrap());
    let last = query().cursor_paginate(2).await.unwrap();
    assert_eq!(last.data.iter().map(|r| r.id).collect::<Vec<_>>(), [4]);
    assert!(last.next_cursor.is_none());
    Context::test_clear_query();
}

#[tokio::test]
async fn invalid_request_pages_fall_back_to_one_and_empty_results_have_one_page() {
    let _db = fixture().await;
    for value in ["0", "abc", "-7", "", "1.5"] {
        Context::test_set_query("page", value);
        let page = PageRow::query().paginate(2).await.unwrap();
        assert_eq!(page.current_page, 1);
        assert_eq!(page.data.len(), 2);
    }
    Context::test_clear_query();
    let page = PageRow::query().filter("id", 99).paginate(2).await.unwrap();
    assert_eq!(page.last_page, 1);
    assert_eq!(page.total, 0);
    assert!(PageRow::query().paginate(0).await.is_err());
}

#[tokio::test]
async fn cursors_refuse_missing_parameters_null_boundaries_and_tampering() {
    let _db = fixture().await;
    let cursor = Cursor::new(
        std::collections::BTreeMap::from([("other".into(), sea_orm::Value::BigInt(Some(1)))]),
        CursorDirection::Next,
    );
    Context::test_set_query("cursor", cursor.encode().unwrap());
    assert_eq!(
        PageRow::query()
            .cursor_paginate(2)
            .await
            .unwrap_err()
            .status_code(),
        400
    );
    Context::test_set_query("cursor", "garbage");
    assert_eq!(
        PageRow::query()
            .cursor_paginate(2)
            .await
            .unwrap_err()
            .status_code(),
        400
    );
    Context::test_clear_query();
    assert!(
        Cursor::new(
            std::collections::BTreeMap::from([("id".into(), sea_orm::Value::BigInt(None))]),
            CursorDirection::Next
        )
        .encode()
        .is_err()
    );
}

#[tokio::test]
async fn mixed_direction_cursors_match_the_facade_and_reach_every_row() {
    use sea_orm::{EntityTrait, QueryOrder};
    type Entity = <PageRow as suprnova::EloquentModel>::Entity;
    type Column = <Entity as EntityTrait>::Column;
    let _db = fixture().await;
    let query = || {
        let mut query = Entity::find()
            .order_by_desc(Column::CreatedAt)
            .order_by_asc(Column::Id);
        sea_orm::QueryTrait::query(&mut query).expr_window_as(
            sea_orm::sea_query::Expr::cust("row_number()"),
            sea_orm::sea_query::WindowStatement::new()
                .order_by(Column::Name, sea_orm::Order::Asc)
                .to_owned(),
            sea_orm::sea_query::Alias::new("position"),
        );
        query
    };
    let first = suprnova::Pagination::cursor(query(), None, 2, Column::Id)
        .await
        .unwrap();
    assert_eq!(
        first.data.iter().map(|row| row.id).collect::<Vec<_>>(),
        [1, 4]
    );
    let cursor = Cursor::decode(first.next_cursor.as_ref().unwrap()).unwrap();
    assert_eq!(
        cursor.parameter("created_at"),
        Some(&sea_orm::Value::BigInt(Some(2)))
    );
    assert!(
        cursor.parameter("name").is_none(),
        "window orders are not cursor boundaries"
    );
    let second = suprnova::Pagination::cursor(query(), first.next_cursor.as_deref(), 2, Column::Id)
        .await
        .unwrap();
    assert_eq!(
        second.data.iter().map(|row| row.id).collect::<Vec<_>>(),
        [2, 3]
    );
    let back = suprnova::Pagination::cursor(query(), second.prev_cursor.as_deref(), 2, Column::Id)
        .await
        .unwrap();
    assert_eq!(back.data, first.data);
    let last = suprnova::Pagination::cursor(query(), second.next_cursor.as_deref(), 2, Column::Id)
        .await
        .unwrap();
    assert_eq!(last.data.iter().map(|row| row.id).collect::<Vec<_>>(), [5]);
    assert!(last.next_cursor.is_none());
    Context::test_set_query("cursor", second.next_cursor.as_ref().unwrap());
    let builder = PageRow::query()
        .order_by_desc("created_at")
        .order_by_asc("id")
        .cursor_paginate(2)
        .await
        .unwrap();
    assert_eq!(
        builder.data.iter().map(|row| row.id).collect::<Vec<_>>(),
        [5]
    );
    Context::test_clear_query();
    let empty = PageRow::query()
        .filter("id", 99)
        .order_by_desc("created_at")
        .order_by_asc("id")
        .cursor_paginate(2)
        .await
        .unwrap();
    assert!(empty.data.is_empty() && empty.next_cursor.is_none() && empty.prev_cursor.is_none());
    assert_eq!(
        PageRow::query()
            .order_by_raw("length(name)")
            .order_by_asc("id")
            .cursor_paginate(2)
            .await
            .unwrap_err()
            .status_code(),
        400
    );
}

#[test]
fn named_cursors_keep_types_and_legacy_cursors_remain_readable() {
    suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    let values = std::collections::BTreeMap::from([
        ("id".into(), sea_orm::Value::BigInt(Some(8))),
        ("name".into(), sea_orm::Value::String(Some("row-8".into()))),
    ]);
    let cursor = Cursor::new(values.clone(), CursorDirection::Prev);
    let decoded = Cursor::decode(&cursor.encode().unwrap()).unwrap();
    assert_eq!(decoded.direction(), CursorDirection::Prev);
    for (name, value) in &values {
        assert_eq!(decoded.parameter(name), Some(value));
    }
    let legacy = CursorPaginator::<()>::encode_value(
        &sea_orm::Value::BigInt(Some(8)),
        CursorDirection::Next,
    )
    .unwrap();
    assert_eq!(
        CursorPaginator::<()>::decode_value(&legacy).unwrap(),
        (sea_orm::Value::BigInt(Some(8)), CursorDirection::Next)
    );
    assert!(CursorPaginator::<()>::decode_value(&cursor.encode().unwrap()).is_err());
    assert!(
        Cursor::new(std::collections::BTreeMap::new(), CursorDirection::Next)
            .encode()
            .is_err()
    );
}

#[tokio::test]
async fn a_cursor_boundary_constrains_every_branch_of_a_filter() {
    let _db = fixture().await;
    let query = || {
        PageRow::query()
            .filter("id", 2)
            .or_filter("id", 1)
            .order_by_asc("created_at")
            .order_by_asc("id")
    };
    let first = query().cursor_paginate(1).await.unwrap();
    assert_eq!(first.data[0].id, 2);
    Context::test_set_query("cursor", first.next_cursor.as_ref().unwrap());
    let second = query().cursor_paginate(1).await.unwrap();
    assert_eq!(second.data[0].id, 1);
    assert!(second.next_cursor.is_none());
    Context::test_clear_query();
}
