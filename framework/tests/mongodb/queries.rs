//! PAR-184: `DocumentQuery<M>`, the builder a document model's `query()`
//! answers. Every rendering is asserted without a server through
//! `to_filter()` (the filter with sort, skip, limit and projection) and
//! `to_pipeline()` (the aggregation stages); a chain the server cannot run
//! as written is an error naming the call, raised before any connection is
//! needed. The `mongodb_` tests run the builder against `MONGODB_TEST_URL`
//! and are ignored without it.

use chrono::NaiveDate;
use suprnova::bson::oid::ObjectId;
use suprnova::bson::{Bson, Document, Regex, doc};
use suprnova::{Direction, DocumentModel, FrameworkError, Mongo, MongoConfig};

use crate::support::{database_of, test_url};

/// A person: the builder's subject in the rendering tests.
#[suprnova::document(collection = "m2_people", fillable = ["name", "age", "role", "tags", "team", "born_at", "score"])]
pub struct Person {
    pub name: String,
    pub age: i32,
    pub role: Option<String>,
    pub tags: Vec<String>,
    pub team: Option<String>,
    pub born_at: Option<suprnova::bson::DateTime>,
    pub score: Option<i64>,
    pub created_at: Option<suprnova::bson::DateTime>,
    pub updated_at: Option<suprnova::bson::DateTime>,
}

/// A soft-deleting ticket.
#[suprnova::document(collection = "m2_tickets", fillable = ["title"], soft_deletes)]
pub struct Ticket {
    pub title: String,
    pub deleted_at: Option<suprnova::bson::DateTime>,
}

/// A code whose key is a string the application gives.
#[suprnova::document(collection = "m2_codes", primary_key = "code", fillable = ["code", "label"])]
pub struct Code {
    pub code: String,
    pub label: String,
}

fn names(error: &FrameworkError, what: &str) -> bool {
    error.to_string().contains(what)
}

/// The case-insensitive regular expression `like` renders.
fn insensitive(pattern: &str) -> Regex {
    Regex {
        pattern: pattern.try_into().expect("a pattern without NUL"),
        options: "i".try_into().expect("options"),
    }
}

// --- Rendering -------------------------------------------------------------

#[test]
fn the_requirement_chain_renders_its_filter_sort_skip_and_limit() {
    let rendered = Person::query()
        .where_("age", ">=", 18)
        .where_in("role", ["a", "b"])
        .order_by("name", Direction::Asc)
        .skip(10)
        .take(5)
        .to_filter()
        .expect("render");
    assert_eq!(
        rendered.filter,
        doc! { "age": { "$gte": 18 }, "role": { "$in": ["a", "b"] } }
    );
    assert_eq!(rendered.sort, Some(doc! { "name": 1 }));
    assert_eq!(rendered.skip, Some(10));
    assert_eq!(rendered.limit, Some(5));
    assert_eq!(rendered.projection, None);
}

#[test]
fn each_operator_renders_its_query_operator() {
    let filter = |op: &str| {
        Person::query()
            .where_("age", op, 30)
            .to_filter()
            .expect("render")
            .filter
    };
    assert_eq!(filter("="), doc! { "age": 30 });
    assert_eq!(filter("!="), doc! { "age": { "$ne": 30 } });
    assert_eq!(filter("<>"), doc! { "age": { "$ne": 30 } });
    assert_eq!(filter("<"), doc! { "age": { "$lt": 30 } });
    assert_eq!(filter("<="), doc! { "age": { "$lte": 30 } });
    assert_eq!(filter(">"), doc! { "age": { "$gt": 30 } });
    assert_eq!(filter(">="), doc! { "age": { "$gte": 30 } });
}

#[test]
fn like_renders_an_anchored_case_insensitive_regular_expression() {
    let filter = Person::query()
        .where_("name", "like", "jo%")
        .to_filter()
        .expect("render")
        .filter;
    assert_eq!(filter, doc! { "name": { "$regex": insensitive("^jo.*$") } });

    // `_` is one character; the pattern's other characters match literally.
    let filter = Person::query()
        .where_("name", "like", "a.b_c%")
        .to_filter()
        .expect("render")
        .filter;
    assert_eq!(
        filter,
        doc! { "name": { "$regex": insensitive(r"^a\.b.c.*$") } }
    );

    let filter = Person::query()
        .where_("name", "not like", "jo%")
        .to_filter()
        .expect("render")
        .filter;
    assert_eq!(filter, doc! { "name": { "$not": insensitive("^jo.*$") } });
}

#[test]
fn an_unknown_operator_or_a_pattern_that_is_no_string_is_an_error_naming_where() {
    let error = Person::query()
        .where_("age", "~", 3)
        .to_filter()
        .expect_err("no such operator");
    assert!(names(&error, "where_") && names(&error, "~"), "{error}");
    let error = Person::query()
        .where_("age", "like", 3)
        .to_filter()
        .expect_err("like needs a string");
    assert!(names(&error, "where_") && names(&error, "like"), "{error}");
}

#[test]
fn the_where_family_renders_in_nin_null_between_date_exists_and_raw() {
    let rendered = Person::query()
        .where_not_in("role", ["x"])
        .where_null("team")
        .where_not_null("born_at")
        .where_between("score", 10..=20)
        .where_exists("tags")
        .where_raw(doc! { "$expr": { "$gt": ["$age", "$score"] } })
        .to_filter()
        .expect("render");
    assert_eq!(
        rendered.filter,
        doc! {
            "role": { "$nin": ["x"] },
            "team": null,
            "born_at": { "$ne": null },
            "score": { "$gte": 10, "$lte": 20 },
            "tags": { "$exists": true },
            "$expr": { "$gt": ["$age", "$score"] },
        }
    );

    let day = NaiveDate::from_ymd_opt(2026, 10, 10).expect("a date");
    let rendered = Person::query()
        .where_date("born_at", day)
        .to_filter()
        .expect("render");
    let start = suprnova::bson::DateTime::parse_rfc3339_str("2026-10-10T00:00:00Z").expect("start");
    let end = suprnova::bson::DateTime::parse_rfc3339_str("2026-10-11T00:00:00Z").expect("end");
    assert_eq!(
        rendered.filter,
        doc! { "born_at": { "$gte": start, "$lt": end } }
    );
}

#[test]
fn or_where_starts_a_new_group_and_and_binds_tighter() {
    let filter = Person::query()
        .where_("age", ">", 60)
        .or_where("role", "=", "admin")
        .where_("team", "=", "core")
        .to_filter()
        .expect("render")
        .filter;
    assert_eq!(
        filter,
        doc! { "$or": [
            { "age": { "$gt": 60 } },
            { "role": "admin", "team": "core" },
        ] }
    );
    // A leading or_where is a plain where.
    let filter = Person::query()
        .or_where("role", "=", "admin")
        .to_filter()
        .expect("render")
        .filter;
    assert_eq!(filter, doc! { "role": "admin" });
}

#[test]
fn two_conditions_on_one_field_render_as_and() {
    let filter = Person::query()
        .where_("age", ">", 1)
        .where_("age", "<", 5)
        .to_filter()
        .expect("render")
        .filter;
    assert_eq!(
        filter,
        doc! { "$and": [ { "age": { "$gt": 1 } }, { "age": { "$lt": 5 } } ] }
    );
}

#[test]
fn the_key_field_is_queried_sorted_and_projected_as_underscore_id() {
    let id = ObjectId::new();
    let rendered = Person::query()
        .where_("id", "=", id)
        .order_by("id", Direction::Desc)
        .project(["name", "id"])
        .to_filter()
        .expect("render");
    assert_eq!(rendered.filter, doc! { "_id": id });
    assert_eq!(rendered.sort, Some(doc! { "_id": -1 }));
    assert_eq!(rendered.projection, Some(doc! { "name": 1, "_id": 1 }));
}

#[test]
fn the_soft_delete_scope_joins_the_filter() {
    let filter = Ticket::query()
        .where_("title", "=", "Bug")
        .to_filter()
        .expect("render")
        .filter;
    assert_eq!(filter, doc! { "deleted_at": null, "title": "Bug" });
    let filter = Ticket::query()
        .where_("title", "=", "Bug")
        .or_where("title", "=", "Task")
        .to_filter()
        .expect("render")
        .filter;
    assert_eq!(
        filter,
        doc! { "deleted_at": null, "$or": [ { "title": "Bug" }, { "title": "Task" } ] }
    );
    let filter = Ticket::only_trashed()
        .where_("title", "=", "Bug")
        .to_filter()
        .expect("render")
        .filter;
    assert_eq!(
        filter,
        doc! { "deleted_at": { "$ne": null }, "title": "Bug" }
    );
}

#[test]
fn to_pipeline_renders_the_find_as_aggregation_stages() {
    let pipeline = Person::query()
        .where_("age", ">=", 18)
        .order_by("name", Direction::Asc)
        .skip(2)
        .take(3)
        .project(["name"])
        .to_pipeline()
        .expect("render");
    assert_eq!(
        pipeline,
        vec![
            doc! { "$match": { "age": { "$gte": 18 } } },
            doc! { "$sort": { "name": 1 } },
            doc! { "$skip": 2_i64 },
            doc! { "$limit": 3_i64 },
            doc! { "$project": { "name": 1 } },
        ]
    );
}

#[test]
fn group_by_with_count_renders_a_group_stage() {
    let pipeline = Person::query()
        .where_("age", ">=", 18)
        .group_by(["role"])
        .count()
        .to_pipeline()
        .expect("render");
    assert_eq!(
        pipeline,
        vec![
            doc! { "$match": { "age": { "$gte": 18 } } },
            doc! { "$group": { "_id": { "role": "$role" }, "count": { "$sum": 1 } } },
            doc! { "$project": { "_id": 0, "role": "$_id.role", "count": 1 } },
        ]
    );
}

#[test]
fn group_by_renders_sum_avg_min_and_max_and_then_sorts_skips_and_limits() {
    let pipeline = Person::query()
        .order_by("sum_score", Direction::Desc)
        .take(2)
        .group_by(["role", "team"])
        .sum("score")
        .avg("age")
        .min("age")
        .max("age")
        .to_pipeline()
        .expect("render");
    assert_eq!(
        pipeline,
        vec![
            doc! { "$group": {
                "_id": { "role": "$role", "team": "$team" },
                "sum_score": { "$sum": "$score" },
                "avg_age": { "$avg": "$age" },
                "min_age": { "$min": "$age" },
                "max_age": { "$max": "$age" },
            } },
            doc! { "$project": {
                "_id": 0, "role": "$_id.role", "team": "$_id.team",
                "sum_score": 1, "avg_age": 1, "min_age": 1, "max_age": 1,
            } },
            doc! { "$sort": { "sum_score": -1 } },
            doc! { "$limit": 2_i64 },
        ]
    );
}

#[test]
fn a_group_without_keys_aggregates_every_matching_document() {
    let pipeline = Person::query()
        .group_by(Vec::<String>::new())
        .sum("age")
        .to_pipeline()
        .expect("render");
    assert_eq!(
        pipeline,
        vec![
            doc! { "$group": { "_id": null, "sum_age": { "$sum": "$age" } } },
            doc! { "$project": { "_id": 0, "sum_age": 1 } },
        ]
    );
}

// --- Chains the server cannot run as written --------------------------------

#[tokio::test]
async fn a_write_after_skip_or_take_is_an_error_naming_the_call() {
    // No connection is registered: each error comes before one is needed.
    let error = Person::query()
        .take(1)
        .update(doc! { "name": "x" })
        .await
        .expect_err("update cannot take");
    assert!(names(&error, "update") && names(&error, "take"), "{error}");
    let error = Person::query()
        .skip(1)
        .delete()
        .await
        .expect_err("delete cannot skip");
    assert!(names(&error, "delete") && names(&error, "skip"), "{error}");
    let error = Person::query()
        .take(1)
        .increment("age", 1)
        .await
        .expect_err("increment cannot take");
    assert!(names(&error, "increment"), "{error}");
    let error = Person::query()
        .take(1)
        .push("tags", "a")
        .await
        .expect_err("push cannot take");
    assert!(names(&error, "push"), "{error}");
}

#[tokio::test]
async fn upsert_after_a_where_is_an_error_naming_upsert() {
    let error = Person::query()
        .where_("age", ">", 1)
        .upsert(vec![doc! { "name": "a", "age": 1 }], &["name"])
        .await
        .expect_err("upsert matches by its own keys");
    assert!(names(&error, "upsert"), "{error}");
    let error = Person::query()
        .upsert(vec![doc! { "age": 1 }], &["name"])
        .await
        .expect_err("a value without its unique field");
    assert!(names(&error, "upsert") && names(&error, "name"), "{error}");
    let error = Code::query()
        .upsert(vec![doc! { "label": "x" }], &["label"])
        .await
        .expect_err("no key to insert with");
    assert!(names(&error, "upsert") && names(&error, "code"), "{error}");
}

#[tokio::test]
async fn distinct_sorted_by_another_field_is_an_error_naming_distinct() {
    let error = Person::query()
        .order_by("age", Direction::Asc)
        .distinct("role")
        .await
        .expect_err("the distinct values have no age");
    assert!(names(&error, "distinct") && names(&error, "age"), "{error}");
}

#[tokio::test]
async fn an_update_that_mixes_operators_and_fields_or_writes_the_key_is_an_error() {
    let error = Person::query()
        .update(doc! { "$set": { "name": "x" }, "age": 3 })
        .await
        .expect_err("mixed");
    assert!(names(&error, "update"), "{error}");
    let error = Person::query()
        .update(doc! { "id": ObjectId::new() })
        .await
        .expect_err("the key");
    assert!(names(&error, "update") && names(&error, "_id"), "{error}");
}

#[tokio::test]
async fn paginate_with_zero_per_page_is_an_error_naming_per_page() {
    let error = Person::query()
        .paginate(0, 1)
        .await
        .expect_err("zero per page");
    assert!(names(&error, "per_page"), "{error}");
    let error = Person::query()
        .simple_paginate(0, 1)
        .await
        .expect_err("zero per page");
    assert!(names(&error, "per_page"), "{error}");
}

#[tokio::test]
async fn a_rendering_error_is_also_the_terminal_error() {
    let error = Person::query()
        .where_("age", "~", 3)
        .get()
        .await
        .expect_err("no such operator");
    assert!(names(&error, "where_"), "{error}");
}

// --- Against a server ------------------------------------------------------

async fn connect() {
    let url = test_url();
    let config = MongoConfig::builder()
        .uri(url.clone())
        .database(database_of(&url))
        .build()
        .expect("the test configuration");
    Mongo::init_with(config).await.expect("connect");
}

/// Each engine test owns a team: it removes the team's people first and
/// queries only them, so tests can run at once against one server.
async fn seed(team: &str, people: &[(&str, i32, &str, &[&str])]) {
    Mongo::collection::<Document>(Person::COLLECTION)
        .expect("collection")
        .delete_many(doc! { "team": team })
        .await
        .expect("remove the team");
    for (name, age, role, tags) in people {
        Person::create(doc! {
            "name": *name,
            "age": *age,
            "role": *role,
            "tags": tags.to_vec(),
            "team": team,
            "score": i64::from(*age) * 2,
        })
        .await
        .expect("create a person");
    }
}

fn team(name: &str) -> suprnova::DocumentQuery<Person> {
    Person::query().where_("team", "=", name)
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_where_order_skip_take_first_count_exists_and_pluck() {
    connect().await;
    seed(
        "q-read",
        &[
            ("Ada", 36, "admin", &["math"]),
            ("Bob", 17, "user", &[]),
            ("Cy", 52, "user", &["ops"]),
            ("Joan", 28, "admin", &[]),
        ],
    )
    .await;

    let adults = team("q-read")
        .where_("age", ">=", 18)
        .where_in("role", ["admin", "user"])
        .order_by("name", Direction::Asc)
        .skip(1)
        .take(2)
        .get()
        .await
        .expect("get");
    let names: Vec<&str> = adults.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["Cy", "Joan"]);

    let first = team("q-read")
        .where_("name", "like", "jo%")
        .first()
        .await
        .expect("first")
        .expect("Joan");
    assert_eq!(first.name, "Joan");
    assert_eq!(team("q-read").count().await.expect("count"), 4);
    assert!(
        team("q-read")
            .where_("age", ">", 50)
            .exists()
            .await
            .expect("exists")
    );
    assert!(
        !team("q-read")
            .where_("age", ">", 90)
            .exists()
            .await
            .expect("exists")
    );
    let mut pluck = team("q-read")
        .order_by("age", Direction::Asc)
        .pluck("name")
        .await
        .expect("pluck");
    assert_eq!(pluck.remove(0), Bson::String("Bob".into()));
    let mut roles = team("q-read").distinct("role").await.expect("distinct");
    roles.sort_by_key(|role| role.to_string());
    assert_eq!(
        roles,
        vec![Bson::String("admin".into()), Bson::String("user".into())]
    );
    let day_people = team("q-read")
        .where_between("age", 20..=40)
        .order_by("age", Direction::Asc)
        .pluck("name")
        .await
        .expect("between");
    assert_eq!(
        day_people,
        vec![Bson::String("Joan".into()), Bson::String("Ada".into())]
    );
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_paginate_answers_the_second_page_and_the_total() {
    connect().await;
    let people: Vec<(String, i32)> = (1..=25).map(|n| (format!("P{n:02}"), n)).collect();
    let rows: Vec<(&str, i32, &str, &[&str])> = people
        .iter()
        .map(|(name, age)| (name.as_str(), *age, "user", &[][..]))
        .collect();
    seed("q-page", &rows).await;

    let page = team("q-page")
        .order_by("age", Direction::Asc)
        .paginate(10, 2)
        .await
        .expect("paginate");
    assert_eq!(page.total, 25);
    assert_eq!(page.current_page, 2);
    assert_eq!(page.last_page, 3);
    let ages: Vec<i32> = page.data.iter().map(|p| p.age).collect();
    assert_eq!(ages, (11..=20).collect::<Vec<_>>());

    let simple = team("q-page")
        .order_by("age", Direction::Asc)
        .simple_paginate(10, 3)
        .await
        .expect("simple_paginate");
    let ages: Vec<i32> = simple.data.iter().map(|p| p.age).collect();
    assert_eq!(ages, (21..=25).collect::<Vec<_>>());
    assert!(!simple.has_more_pages());
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_update_upsert_delete_and_the_array_operators_write_only_what_they_name() {
    connect().await;
    seed(
        "q-write",
        &[("Ada", 36, "admin", &["math"]), ("Bob", 17, "user", &["x"])],
    )
    .await;

    let changed = team("q-write")
        .where_("name", "=", "Ada")
        .update(doc! { "role": "owner" })
        .await
        .expect("update");
    assert_eq!(changed, 1);
    let ada = team("q-write")
        .where_("name", "=", "Ada")
        .first()
        .await
        .expect("first")
        .expect("Ada");
    assert_eq!(ada.role.as_deref(), Some("owner"));
    assert_eq!(ada.age, 36, "$set leaves the other fields as they were");
    assert_eq!(ada.tags, vec!["math".to_owned()]);

    team("q-write")
        .where_("name", "=", "Bob")
        .increment("age", 3)
        .await
        .expect("increment");
    team("q-write")
        .where_("name", "=", "Bob")
        .decrement("score", 4)
        .await
        .expect("decrement");
    team("q-write")
        .where_("name", "=", "Bob")
        .push("tags", "y")
        .await
        .expect("push");
    team("q-write")
        .where_("name", "=", "Bob")
        .pull("tags", "x")
        .await
        .expect("pull");
    team("q-write")
        .where_("name", "=", "Bob")
        .unset(["role"])
        .await
        .expect("unset");
    let bob = team("q-write")
        .where_("name", "=", "Bob")
        .first()
        .await
        .expect("first")
        .expect("Bob");
    assert_eq!(bob.age, 20);
    assert_eq!(bob.score, Some(30));
    assert_eq!(bob.tags, vec!["y".to_owned()]);
    assert_eq!(bob.role, None);

    let written = Person::query()
        .upsert(
            vec![
                doc! { "name": "Ada", "team": "q-write", "age": 37 },
                doc! { "name": "Cy", "team": "q-write", "age": 52 },
            ],
            &["name", "team"],
        )
        .await
        .expect("upsert");
    assert_eq!(written, 2);
    assert_eq!(team("q-write").count().await.expect("count"), 3);
    let ada = team("q-write")
        .where_("name", "=", "Ada")
        .first()
        .await
        .expect("first")
        .expect("Ada");
    assert_eq!(ada.age, 37);
    assert_eq!(
        ada.role.as_deref(),
        Some("owner"),
        "upsert sets only the given fields"
    );

    let removed = team("q-write")
        .where_("age", ">", 30)
        .delete()
        .await
        .expect("delete");
    assert_eq!(removed, 2);
    assert_eq!(team("q-write").count().await.expect("count"), 1);
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_group_by_and_the_aggregates_answer_per_group_and_overall() {
    connect().await;
    seed(
        "q-agg",
        &[
            ("Ada", 30, "admin", &[]),
            ("Bob", 20, "user", &[]),
            ("Cy", 40, "user", &[]),
        ],
    )
    .await;

    let rows = team("q-agg")
        .order_by("role", Direction::Asc)
        .group_by(["role"])
        .count()
        .sum("age")
        .avg("age")
        .min("age")
        .max("age")
        .get()
        .await
        .expect("group");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get_str("role").expect("role"), "admin");
    assert_eq!(rows[1].get_str("role").expect("role"), "user");
    assert_eq!(rows[1].get_i32("count").expect("count"), 2);
    assert_eq!(rows[1].get_i32("sum_age").expect("sum"), 60);
    assert_eq!(rows[1].get_f64("avg_age").expect("avg"), 30.0);
    assert_eq!(rows[1].get_i32("min_age").expect("min"), 20);
    assert_eq!(rows[1].get_i32("max_age").expect("max"), 40);

    assert_eq!(
        team("q-agg").sum("age").await.expect("sum"),
        Some(Bson::Int32(90))
    );
    assert_eq!(
        team("q-agg").avg("age").await.expect("avg"),
        Some(Bson::Double(30.0))
    );
    assert_eq!(
        team("q-agg").min("age").await.expect("min"),
        Some(Bson::Int32(20))
    );
    assert_eq!(
        team("q-agg").max("age").await.expect("max"),
        Some(Bson::Int32(40))
    );
    assert_eq!(
        team("q-agg")
            .where_("age", ">", 99)
            .max("age")
            .await
            .expect("max"),
        None,
        "no documents, no value"
    );
}

#[test]
fn a_group_sorts_by_its_result_fields_after_the_group() {
    let pipeline = Person::query()
        .group_by(["id"])
        .count()
        .order_by("count", Direction::Desc)
        .order_by("id", Direction::Asc)
        .skip(1)
        .take(2)
        .to_pipeline()
        .expect("render");
    assert_eq!(
        pipeline,
        vec![
            doc! { "$group": { "_id": { "id": "$_id" }, "count": { "$sum": 1 } } },
            doc! { "$project": { "_id": 0, "id": "$_id.id", "count": 1 } },
            doc! { "$sort": { "count": -1, "id": 1 } },
            doc! { "$skip": 1_i64 },
            doc! { "$limit": 2_i64 },
        ]
    );
}
