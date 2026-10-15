//! Native Postgres coverage uses a disposable database only when requested.

use super::database_support::{
    create, descriptor, document, engine, live_database, query, vectors,
};
use serial_test::serial;
use std::sync::Arc;
use suprnova::{
    DB, FrameworkError,
    search::{LanguageConfig, SearchEngine, testing::conformance},
};

#[tokio::test]
#[serial(search_database_postgres)]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_database_obeys_the_shared_driver_contract() -> Result<(), FrameworkError> {
    let _guard = live_database("PG_TEST_URL").await?;
    vectors("database_pg_conformance");
    conformance::run(
        Arc::new(engine()),
        conformance::Options {
            index_name: "database_pg_conformance",
            ..Default::default()
        },
    )
    .await
}

#[tokio::test]
#[serial(search_database_postgres)]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_setweight_rank_headline_and_ts_stat_are_native() -> Result<(), FrameworkError> {
    let _guard = live_database("PG_TEST_URL").await?;
    let index = descriptor("database_pg_native");
    let generation = create(&index).await?;
    let driver = engine();
    driver
        .index(
            &index,
            &generation,
            &[
                document(&index, "a", "orchard", "garden"),
                document(&index, "b", "garden", "orchard"),
            ],
        )
        .await?;
    let mut q = query("orchard");
    q.highlights = true;
    let result = driver.query(&index, &generation, &q).await?;
    assert_eq!(result.candidates[0].document.key, "a");
    assert!(result.candidates[0].score > result.candidates[1].score);
    assert!(
        result.candidates[0]
            .highlights
            .as_ref()
            .expect("native headline")["title"]
            .contains("<em>orchard</em>")
    );
    let vector = DB::select_one(
        "SELECT search_vector::text AS vector FROM search_database_pg_native_native WHERE key = $1",
        vec!["a".into()],
    )
    .await?
    .expect("stored vector");
    assert!(vector.get_string("vector")?.contains("A"));
    let rank = DB::select_one("SELECT ts_rank(search_vector, to_tsquery($1::regconfig, $2), 1)::float8 AS rank FROM search_database_pg_native_native WHERE key = $3", vec!["\"search_database_pg_native_native_ts\"".into(), "orchard".into(), "a".into()]).await?.expect("native rank");
    assert!(rank.get_float("rank")? > 0.0);
    let terms = DB::select(
        "SELECT word, ndoc FROM ts_stat($1)",
        vec!["SELECT search_vector FROM search_database_pg_native_native".into()],
    )
    .await?;
    assert!(terms.iter().any(
        |row| row.get_string("word").is_ok_and(|word| word == "orchard")
            && row.get_int("ndoc").is_ok_and(|count| count == 2)
    ));
    let vocabulary = driver.vocabulary(&index, &generation).await?;
    assert_eq!(vocabulary.words.get("orchard"), Some(&2));
    driver.drop_generation(&index, &generation).await
}

#[tokio::test]
#[serial(search_database_postgres)]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_simple_configuration_preserves_unstemmed_terms() -> Result<(), FrameworkError> {
    let _guard = live_database("PG_TEST_URL").await?;
    let index = descriptor("database_pg_simple");
    let generation = suprnova::search::GenerationId::new("simple")?;
    let mut settings = index.index_settings();
    settings.index_time.language = LanguageConfig::Postgres {
        configuration: "simple".into(),
    };
    let driver = engine();
    driver
        .create_generation(&index, &settings, &generation)
        .await?;
    let mut doc = document(&index, "a", "running", "gardens");
    doc.content_hash = doc.document.content_hash(&settings.index_time.digest());
    driver.index(&index, &generation, &[doc]).await?;
    assert_eq!(
        driver
            .count(&index, &generation, &query("running"))
            .await?
            .value(),
        1
    );
    assert_eq!(
        driver
            .count(&index, &generation, &query("run"))
            .await?
            .value(),
        0
    );
    assert_eq!(driver.read_settings(&index, &generation).await?, settings);
    driver.drop_generation(&index, &generation).await
}

#[tokio::test]
#[serial(search_database_postgres)]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_english_configuration_uses_native_stems_and_stopwords()
-> Result<(), FrameworkError> {
    let _guard = live_database("PG_TEST_URL").await?;
    let index = descriptor("database_pg_english");
    let generation = suprnova::search::GenerationId::new("english")?;
    let mut settings = index.index_settings();
    settings.index_time.language = LanguageConfig::Postgres {
        configuration: "english".into(),
    };
    let driver = engine();
    driver
        .create_generation(&index, &settings, &generation)
        .await?;
    let mut doc = document(&index, "a", "running", "the gardens");
    doc.content_hash = doc.document.content_hash(&settings.index_time.digest());
    driver.index(&index, &generation, &[doc]).await?;
    assert_eq!(
        driver
            .count(&index, &generation, &query("run"))
            .await?
            .value(),
        1
    );
    assert_eq!(
        driver
            .count(&index, &generation, &query("the"))
            .await?
            .value(),
        0
    );
    let vocabulary = driver.vocabulary(&index, &generation).await?;
    assert_eq!(vocabulary.words.get("run"), Some(&1));
    assert!(!vocabulary.words.contains_key("running"));
    assert!(!vocabulary.words.contains_key("the"));
    driver.drop_generation(&index, &generation).await
}

#[tokio::test]
#[serial(search_database_postgres)]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_server_language_pins_a_copy_of_the_server_configuration()
-> Result<(), FrameworkError> {
    let _guard = live_database("PG_TEST_URL").await?;
    let index = descriptor("database_pg_pinned");
    let generation = create(&index).await?;
    let pinned = "\"search_database_pg_pinned_native_ts\"";
    // The copy tokenizes exactly as the server default did when the generation began.
    let sample = "The gardeners were running through the orchards";
    let row = DB::select_one(
        "SELECT to_tsvector($1::regconfig, $2)::text AS pinned, to_tsvector(current_setting('default_text_search_config')::regconfig, $2)::text AS server",
        vec![pinned.into(), sample.into()],
    )
    .await?
    .expect("both vectors");
    assert_eq!(row.get_string("pinned")?, row.get_string("server")?);
    let driver = engine();
    driver
        .index(
            &index,
            &generation,
            &[document(&index, "a", "orchards", "gardeners were running")],
        )
        .await?;
    let stored = DB::select_one(
        "SELECT search_vector::text AS stored, (setweight(to_tsvector($1::regconfig, 'orchards'), 'A') || setweight(to_tsvector($1::regconfig, 'gardeners were running'), 'B'))::text AS expected FROM search_database_pg_pinned_native WHERE key = $2",
        vec![pinned.into(), "a".into()],
    )
    .await?
    .expect("stored vector");
    assert_eq!(stored.get_string("stored")?, stored.get_string("expected")?);
    assert_eq!(
        driver
            .count(&index, &generation, &query("orchards"))
            .await?
            .value(),
        1
    );
    driver.drop_generation(&index, &generation).await?;
    let left = DB::select_one(
        "SELECT count(*)::int8 AS left FROM pg_ts_config WHERE cfgname = $1",
        vec!["search_database_pg_pinned_native_ts".into()],
    )
    .await?
    .expect("count");
    assert_eq!(left.get_int("left")?, 0);
    Ok(())
}
