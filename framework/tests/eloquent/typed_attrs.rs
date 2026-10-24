//! P2-10 - a malformed attribute value must name its field, not become
//! that field's `Default`.
//!
//! `#[model]` generated both `fill` and `from_attrs_unsaved` (the engine
//! behind `first_or_new`) with a direct
//! `serde_json::from_value(v).unwrap_or_default()` for every non-mutator
//! field. A value of the wrong type was therefore not an error: it was
//! silently replaced by `Default::default()` and the call returned `Ok`.
//!
//! `user.fill(attrs! { age: "not a number" })` set `age = 0` and reported
//! success. That is the same failure class as P2-09(a)'s silently dropped
//! eager loads - wrong data, no error, no way to notice - and it is
//! reachable anywhere attrs are built from request input, which is the
//! ordinary case for `fill`.
//!
//! Both generated functions already returned `Result<_, FrameworkError>`,
//! and the sibling mutator arms beside the broken ones already used `?`.
//! The error had somewhere to go the whole time.
//!
//! Note the asymmetry these tests pin: an *unknown* column is still
//! skipped silently (Laravel's `$model->fill()` parity, deliberate), while
//! a *known* column carrying an undecodable value is now an error. Those
//! are different questions and they get different answers.
//!
//! Models are declared at module scope - `#[suprnova::model]` emits an
//! inner module whose `use super::*` only sees this file's top-level
//! imports, so a model inside a test fn breaks SeaORM type resolution.

use suprnova::eloquent::FirstOrCreate;
use suprnova::testing::TestDatabase;
use suprnova::{attrs, model};

// ---- Models -------------------------------------------------------------

#[model(
    table = "p2_10_profiles",
    timestamps = false,
    fillable = ["name", "age", "score", "active"]
)]
pub struct P210Profile {
    pub id: i64,
    pub name: String,
    pub age: i32,
    pub score: f64,
    pub active: bool,
}

// ---- Migrations ---------------------------------------------------------

async fn migrate(db: &TestDatabase) {
    db.execute_unprepared(
        "CREATE TABLE p2_10_profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            age INTEGER NOT NULL,
            score REAL NOT NULL,
            active BOOLEAN NOT NULL
        )",
    )
    .await
    .expect("create p2_10_profiles");
}

fn blank() -> P210Profile {
    P210Profile {
        id: 0,
        name: String::new(),
        age: 7,
        score: 1.5,
        active: true,
        ..Default::default()
    }
}

// ---- fill ---------------------------------------------------------------

/// The headline regression. A string where an integer belongs used to
/// yield `age = 0` and `Ok(())`.
#[tokio::test]
async fn fill_rejects_a_value_of_the_wrong_type_instead_of_defaulting_it() {
    let mut p = blank();

    let err = p
        .fill(attrs! { age: "not a number" })
        .expect_err("a string is not an i32; this must not silently become 0");

    let msg = format!("{err}");
    assert!(
        msg.contains("age"),
        "the error must name the offending field - that is the whole \
         point of a typed field error: {msg}"
    );
    assert_eq!(
        p.age, 7,
        "and the field must be left alone. Overwriting it with `Default` \
         is precisely the defect; writing 0 here would be the old \
         behaviour wearing an error message"
    );
}

/// Not just integers - the same hole existed for every non-mutator field
/// of every type.
#[tokio::test]
async fn fill_rejects_wrong_types_across_field_kinds() {
    for (label, bad) in [
        ("i32 from string", attrs! { age: "twelve" }),
        ("f64 from bool", attrs! { score: true }),
        ("bool from string", attrs! { active: "yes" }),
        ("String from number", attrs! { name: 42 }),
    ] {
        let mut p = blank();
        let before = (p.name.clone(), p.age, p.score, p.active);

        assert!(
            p.fill(bad).is_err(),
            "{label}: a wrong-typed value must be an error, not a default"
        );
        assert_eq!(
            (p.name.clone(), p.age, p.score, p.active),
            before,
            "{label}: a rejected fill must not have modified the model"
        );
    }
}

/// The fix must not have broken the ordinary path.
#[tokio::test]
async fn fill_still_applies_well_formed_values() {
    let mut p = blank();

    p.fill(attrs! { name: "Ada", age: 36, score: 9.5, active: false })
        .expect("well-formed attrs must still apply");

    assert_eq!(p.name, "Ada");
    assert_eq!(p.age, 36);
    assert_eq!(p.score, 9.5);
    assert!(!p.active);
}

/// serde's numeric flexibility is not "wrong type" - an integer JSON
/// value decoding into an `f64` field is a legitimate coercion and must
/// keep working. Erring here would make the fix a regression.
#[tokio::test]
async fn fill_accepts_a_json_integer_for_a_float_field() {
    let mut p = blank();

    p.fill(attrs! { score: 10 })
        .expect("an integer is a valid f64");

    assert_eq!(p.score, 10.0);
}

/// The deliberate asymmetry: unknown columns stay silent. This is
/// Laravel's `$model->fill()` behaviour and the fix must not have
/// widened into it.
#[tokio::test]
async fn fill_still_skips_unknown_columns_silently() {
    let mut p = blank();

    p.fill(attrs! { name: "Ada", nonexistent_column: "whatever" })
        .expect("an unknown column is skipped, not an error (Laravel parity)");

    assert_eq!(p.name, "Ada");
}

/// A guarded field is dropped by the mass-assignment filter before any
/// decoding happens, so a malformed value for a field the caller may not
/// set must not turn into an error either - that would leak which fields
/// exist to a caller who is not allowed to touch them.
#[tokio::test]
async fn fill_ignores_malformed_values_for_non_fillable_fields() {
    let mut p = blank();

    p.fill(attrs! { id: "not an integer" })
        .expect("`id` is not fillable, so it never reaches decoding");

    assert_eq!(p.id, 0);
}

// ---- first_or_new -------------------------------------------------------

/// `from_attrs_unsaved` carried the identical defect, on the path
/// `first_or_new` uses to build an unsaved instance.
#[tokio::test]
async fn first_or_new_rejects_a_value_of_the_wrong_type() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;

    let err = P210Profile::first_or_new(attrs! {
        name: "Ghost",
        age: "not a number",
    })
    .await
    .expect_err("a string is not an i32");

    assert!(
        format!("{err}").contains("age"),
        "the error must name the field: {err}"
    );
}

#[tokio::test]
async fn first_or_new_still_builds_from_well_formed_attrs() {
    let db = TestDatabase::sqlite_memory().await.expect("sqlite");
    migrate(&db).await;

    let p = P210Profile::first_or_new(attrs! {
        name: "Ghost",
        age: 41,
        score: 2.5,
        active: true,
    })
    .await
    .expect("well-formed attrs must still build an unsaved instance");

    assert_eq!(p.name, "Ghost");
    assert_eq!(p.age, 41);
    assert_eq!(p.id, 0, "unsaved, so no primary key yet");
}
