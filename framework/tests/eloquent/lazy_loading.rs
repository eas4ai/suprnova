//! Lazy-loading prevention (`suprnova::eloquent::prevent_lazy_loading`).
//!
//! With the switch on, a relation read on a model of a query that
//! returned more than one row, without the relation loaded, is refused:
//! it runs no query, and the error names the model and the relation. A
//! registered handler is called instead and the read goes on. A model
//! from a one-row read, a model built in the process, and a loaded
//! relation read as before, and with the switch off nothing changes.
//!
//! The switch is process-wide, and the tests of this binary run in
//! parallel in one process. Every test here holds [`SwitchGuard`], which
//! serialises them and turns the switch off again when it drops, on a
//! panic too. A test elsewhere in this binary that reads a relation
//! lazily on a model of a multi-row query holds the same guard with the
//! switch off, so it never runs while the switch is on. The tests here
//! are also `#[serial]`, because the query log they count with is
//! process-wide as well; they count the reads of the `lz_*` tables only.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serial_test::serial;
use suprnova::eloquent::{
    LazyLoadingViolation, clear_lazy_loading_violation_handler, handle_lazy_loading_violation,
    prevent_lazy_loading, preventing_lazy_loading,
};
use suprnova::testing::TestDatabase;
use suprnova::{DB, FrameworkError, Model, attrs, model};

// ---- The switch ----------------------------------------------------------

/// Held by every test that turns lazy-loading prevention on, and by
/// every test that must not run while it is on. A panicking holder
/// poisons it; the next holder goes on, because [`SwitchGuard`] turned
/// the switch off before the lock was released.
static SWITCH: Mutex<()> = Mutex::new(());

/// Holds [`SWITCH`]. Dropping it turns lazy-loading prevention off and
/// removes the violation handler, on a panic of the test too.
pub struct SwitchGuard {
    _held: MutexGuard<'static, ()>,
}

impl SwitchGuard {
    /// Take the lock and turn prevention on.
    pub fn on() -> Self {
        let guard = Self::off();
        prevent_lazy_loading(true);
        guard
    }

    /// Take the lock and leave prevention off: for a test that reads a
    /// relation lazily on a model of a multi-row query, which a test
    /// with the switch on at the same time would make fail.
    pub fn off() -> Self {
        Self {
            _held: SWITCH.lock().unwrap_or_else(PoisonError::into_inner),
        }
    }
}

impl Drop for SwitchGuard {
    fn drop(&mut self) {
        prevent_lazy_loading(false);
        // A panic here, during the unwind of a failed test, would abort
        // the whole binary, so the error is reported instead.
        if let Err(error) = clear_lazy_loading_violation_handler() {
            eprintln!("the lazy loading violation handler was not removed: {error}");
        }
    }
}

// ---- Fixtures: one model family with every relation kind ---------------

#[model(table = "lz_authors", relations = {
    posts: HasMany<LzPost>,
    profile: HasOne<LzProfile>,
    comments: HasManyThrough<LzPost, LzComment>,
    first_comment: HasOneThrough<LzPost, LzComment>,
})]
pub struct LzAuthor {
    pub id: i64,
    pub name: String,
}

#[model(table = "lz_posts", morph_type = "lz_post", relations = {
    author: BelongsTo<LzAuthor>,
    comments: HasMany<LzComment>,
    tags: BelongsToMany<LzTag, LzPostTag>,
    images: MorphMany<LzImage> { name = "imageable" },
    cover: MorphOne<LzImage> { name = "imageable" },
    labels: MorphToMany<LzLabel, LzLabelable> { name = "labelable" },
})]
pub struct LzPost {
    pub id: i64,
    pub lz_author_id: i64,
    pub title: String,
}

#[model(table = "lz_comments", relations = {
    post: BelongsTo<LzPost>,
})]
pub struct LzComment {
    pub id: i64,
    pub lz_post_id: i64,
    pub body: String,
}

#[model(table = "lz_profiles")]
pub struct LzProfile {
    pub id: i64,
    pub lz_author_id: i64,
    pub bio: String,
}

#[model(table = "lz_tags")]
pub struct LzTag {
    pub id: i64,
    pub label: String,
}

#[model(table = "lz_post_tag", primary_key = "id", timestamps = false)]
pub struct LzPostTag {
    pub id: i64,
    pub lz_post_id: i64,
    pub lz_tag_id: i64,
}

#[model(table = "lz_images", relations = {
    imageable: MorphTo { targets = [LzPost] },
})]
pub struct LzImage {
    pub id: i64,
    pub imageable_id: i64,
    pub imageable_type: String,
    pub url: String,
}

#[model(table = "lz_labels", relations = {
    posts: MorphedByMany<LzPost, LzLabelable> {
        name = "labelable",
        target_morph_type = "lz_post",
    },
})]
pub struct LzLabel {
    pub id: i64,
    pub name: String,
}

#[model(table = "lz_labelables", primary_key = "id", timestamps = false)]
pub struct LzLabelable {
    pub id: i64,
    pub lz_label_id: i64,
    pub labelable_id: i64,
    pub labelable_type: String,
}

/// Two authors, `ada` and `grace`, each with one post (`quasar` and
/// `nebula`), and each post with one comment, one image, one tag and one
/// label; each author has one profile. Every table holds two rows, so a
/// query over all of one model returns more than one row.
async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for ddl in [
        "CREATE TABLE lz_authors (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE lz_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         lz_author_id INTEGER NOT NULL, title TEXT NOT NULL)",
        "CREATE TABLE lz_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         lz_post_id INTEGER NOT NULL, body TEXT NOT NULL)",
        "CREATE TABLE lz_profiles (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         lz_author_id INTEGER NOT NULL, bio TEXT NOT NULL)",
        "CREATE TABLE lz_tags (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL)",
        "CREATE TABLE lz_post_tag (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         lz_post_id INTEGER NOT NULL, lz_tag_id INTEGER NOT NULL)",
        "CREATE TABLE lz_images (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         imageable_id INTEGER NOT NULL, imageable_type TEXT NOT NULL, url TEXT NOT NULL)",
        "CREATE TABLE lz_labels (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE lz_labelables (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         lz_label_id INTEGER NOT NULL, labelable_id INTEGER NOT NULL, \
         labelable_type TEXT NOT NULL)",
    ] {
        db.execute_unprepared(ddl).await.unwrap();
    }
    for (name, title) in [("ada", "quasar"), ("grace", "nebula")] {
        let author = LzAuthor::create(attrs! { name: name }).await.unwrap();
        LzProfile::create(attrs! { lz_author_id: author.id, bio: format!("{name} bio") })
            .await
            .unwrap();
        let post = LzPost::create(attrs! { lz_author_id: author.id, title: title })
            .await
            .unwrap();
        LzComment::create(attrs! { lz_post_id: post.id, body: format!("on {title}") })
            .await
            .unwrap();
        LzImage::create(attrs! {
            imageable_id: post.id,
            imageable_type: "lz_post",
            url: format!("{title}.png"),
        })
        .await
        .unwrap();
        let tag = LzTag::create(attrs! { label: title }).await.unwrap();
        post.tags().attach(tag.id).await.unwrap();
        let label = LzLabel::create(attrs! { name: title }).await.unwrap();
        post.labels().attach(label.id).await.unwrap();
    }
    db
}

/// Starts the query log empty.
fn start_query_log() {
    DB::enable_query_log().unwrap();
    DB::flush_query_log().unwrap();
}

/// The SELECTs logged since [`start_query_log`] that read an `lz_*`
/// table, then the log switched off and emptied, so a failed assertion
/// after this call does not leave it running for the next test.
fn stop_query_log_counting_reads() -> usize {
    let reads = DB::get_query_log()
        .unwrap()
        .into_iter()
        .map(|q| q.sql.to_uppercase())
        .filter(|sql| sql.trim_start().starts_with("SELECT"))
        .filter(|sql| sql.contains("LZ_"))
        .count();
    DB::disable_query_log().unwrap();
    DB::flush_query_log().unwrap();
    reads
}

/// Both posts, in the order of their ids: a query that returns two rows.
async fn both_posts() -> suprnova::Collection<LzPost> {
    let posts = LzPost::query().order_by_asc("id").get().await.unwrap();
    assert_eq!(posts.len(), 2, "the fixture has two posts");
    posts
}

// ---- The switch off ------------------------------------------------------

#[tokio::test]
#[serial]
async fn with_the_switch_off_a_lazy_read_runs_its_query() {
    let _switch = SwitchGuard::off();
    let _db = fixture().await;
    assert!(!preventing_lazy_loading(), "the switch is off by default");
    let posts = both_posts().await;

    start_query_log();
    let author = posts[0].author().first().await;
    let reads = stop_query_log_counting_reads();

    let author = author.unwrap().expect("the post has an author");
    assert_eq!(author.name, "ada");
    assert_eq!(reads, 1, "the lazy read runs its query");
}

// ---- The switch on, no handler -------------------------------------------

#[tokio::test]
#[serial]
async fn a_lazy_read_on_each_row_of_a_multi_row_query_is_refused_without_a_query() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let posts = both_posts().await;

    start_query_log();
    let mut refusals = Vec::new();
    for post in posts.iter() {
        refusals.push(post.author().first().await);
    }
    let reads = stop_query_log_counting_reads();

    assert_eq!(reads, 0, "a refused read runs no query");
    for refusal in refusals {
        let message = refusal
            .expect_err("a lazy read on a row of a two-row query is refused")
            .to_string();
        assert!(message.contains("`LzPost`"), "names the model: {message}");
        assert!(
            message.contains("`author`"),
            "names the relation: {message}"
        );
        assert!(
            message.contains(".with([\"author\"])") && message.contains(".load([\"author\"])"),
            "says how to load it eagerly: {message}"
        );
        assert!(
            !message.chars().any(|c| c.is_ascii_digit()) && !message.contains("quasar"),
            "holds no key and no value of the row: {message}"
        );
    }
}

#[tokio::test]
#[serial]
async fn a_clone_keeps_the_mark_and_a_replica_drops_it() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let posts = both_posts().await;

    let cloned = posts[0].clone();
    assert!(
        cloned.author().first().await.is_err(),
        "a clone of a row of a two-row query is still that row"
    );
    let replica = posts[0].replicate().await.unwrap();
    let author = replica
        .author()
        .first()
        .await
        .expect("a replica is built in the process and loads lazily");
    assert_eq!(author.map(|a| a.name), Some("ada".to_string()));
}

#[tokio::test]
#[serial]
async fn every_relation_kind_goes_through_the_check() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let authors = LzAuthor::query().order_by_asc("id").get().await.unwrap();
    let posts = both_posts().await;
    let images = LzImage::query().order_by_asc("id").get().await.unwrap();
    let labels = LzLabel::query().order_by_asc("id").get().await.unwrap();
    assert_eq!(
        (authors.len(), images.len(), labels.len()),
        (2, 2, 2),
        "every model of the fixture comes from a two-row query"
    );

    start_query_log();
    let reads: Vec<(&str, Result<(), FrameworkError>)> = vec![
        ("posts", authors[0].posts().get().await.map(drop)),
        ("posts", authors[0].posts().first().await.map(drop)),
        ("profile", authors[0].profile().first().await.map(drop)),
        ("profile", authors[0].profile().get().await.map(drop)),
        ("comments", authors[0].comments().get().await.map(drop)),
        ("comments", authors[0].comments().first().await.map(drop)),
        (
            "first_comment",
            authors[0].first_comment().first().await.map(drop),
        ),
        (
            "first_comment",
            authors[0].first_comment().get().await.map(drop),
        ),
        ("author", posts[0].author().first().await.map(drop)),
        ("comments", posts[0].comments().get().await.map(drop)),
        ("comments", posts[0].comments().first().await.map(drop)),
        ("tags", posts[0].tags().get().await.map(drop)),
        ("tags", posts[0].tags().first().await.map(drop)),
        ("images", posts[0].images().get().await.map(drop)),
        ("images", posts[0].images().first().await.map(drop)),
        ("cover", posts[0].cover().first().await.map(drop)),
        ("labels", posts[0].labels().get().await.map(drop)),
        ("labels", posts[0].labels().first().await.map(drop)),
        ("imageable", images[0].imageable().get().await.map(drop)),
        ("posts", labels[0].posts().get().await.map(drop)),
        ("posts", labels[0].posts().first().await.map(drop)),
    ];
    let queries = stop_query_log_counting_reads();

    assert_eq!(queries, 0, "no refused read runs a query");
    for (relation, read) in reads {
        match read {
            Ok(()) => panic!("the lazy read of `{relation}` was not refused"),
            Err(error) => {
                let message = error.to_string();
                assert!(
                    message.contains(&format!("the relation `{relation}`")),
                    "the refusal names `{relation}`: {message}"
                );
            }
        }
    }

    // A count loads no model, so it is no lazy load.
    assert_eq!(authors[0].posts().count().await.unwrap(), 1);
}

// ---- Every source of the mark ---------------------------------------------

/// Asserts that a lazy read was refused for the lazy-loading switch.
fn assert_refused<T>(read: Result<T, FrameworkError>, source: &str) {
    match read {
        Ok(_) => panic!("a lazy read on a row of {source} was not refused"),
        Err(error) => assert!(
            error.to_string().contains("lazy"),
            "the refusal of {source} is the lazy loading one: {error}"
        ),
    }
}

#[tokio::test]
#[serial]
async fn a_row_of_all_is_marked() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let posts = LzPost::all().await.unwrap();
    assert_eq!(posts.len(), 2);
    assert_refused(posts[0].author().first().await, "all()");
}

#[tokio::test]
#[serial]
async fn a_row_of_find_many_is_marked() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let posts = LzPost::find_many([1_i64, 2]).await.unwrap();
    assert_eq!(posts.len(), 2);
    assert_refused(posts[0].author().first().await, "find_many of two keys");
}

#[tokio::test]
#[serial]
async fn a_row_of_a_page_of_two_is_marked() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let page = LzPost::query()
        .order_by_asc("id")
        .paginate(2)
        .await
        .unwrap();
    assert_eq!(page.data.len(), 2);
    assert_refused(page.data[0].author().first().await, "a page of two");
}

#[tokio::test]
#[serial]
async fn a_row_of_a_chunk_of_two_is_marked() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let outcomes: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&outcomes);
    LzPost::query()
        .order_by_asc("id")
        .chunk(2, move |batch| {
            let sink = Arc::clone(&sink);
            async move {
                assert_eq!(batch.len(), 2, "the chunk holds both rows");
                let refused = batch[0].author().first().await.is_err();
                sink.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(refused);
                Ok(())
            }
        })
        .await
        .unwrap();
    assert_eq!(
        *outcomes.lock().unwrap_or_else(PoisonError::into_inner),
        [true],
        "one chunk, and the read on its row was refused"
    );
}

#[tokio::test]
#[serial]
async fn a_row_of_lazy_is_marked() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let mut stream = LzPost::query().order_by_asc("id").lazy();
    let post = stream.next().await.expect("a row").unwrap();
    assert_refused(post.author().first().await, "lazy()");
}

#[tokio::test]
#[serial]
async fn a_row_of_each_over_two_rows_is_marked() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let outcomes: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&outcomes);
    LzPost::query()
        .order_by_asc("id")
        .each(move |post| {
            let sink = Arc::clone(&sink);
            async move {
                let refused = post.author().first().await.is_err();
                sink.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(refused);
                Ok(())
            }
        })
        .await
        .unwrap();
    assert_eq!(
        *outcomes.lock().unwrap_or_else(PoisonError::into_inner),
        [true, true],
        "the read was refused on both rows, the last one too"
    );
}

#[tokio::test]
#[serial]
async fn a_row_of_each_over_one_row_loads_lazily() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let outcomes: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&outcomes);
    LzPost::query()
        .filter("title", "nebula")
        .each(move |post| {
            let sink = Arc::clone(&sink);
            async move {
                let loaded = post.author().first().await.is_ok();
                sink.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(loaded);
                Ok(())
            }
        })
        .await
        .unwrap();
    assert_eq!(
        *outcomes.lock().unwrap_or_else(PoisonError::into_inner),
        [true],
        "a walk of one row marks nothing"
    );
}

#[tokio::test]
#[serial]
async fn a_row_of_a_through_relation_is_marked() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    // A second comment on ada's post, so the through read returns two rows.
    LzComment::create(attrs! { lz_post_id: 1, body: "second" })
        .await
        .unwrap();
    let ada = LzAuthor::find(1).await.unwrap().expect("ada");
    let comments = ada
        .comments()
        .get()
        .await
        .expect("ada came from find and loads lazily");
    assert_eq!(comments.len(), 2);
    assert_refused(comments[0].post().first().await, "a through relation");
}

#[tokio::test]
#[serial]
async fn a_read_after_with_count_is_still_refused() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let authors = LzAuthor::with_count(["posts"])
        .order_by_asc("id")
        .get()
        .await
        .unwrap();
    assert_eq!(authors.len(), 2);
    assert_eq!(authors[0].posts_count(), 1, "with_count loaded the count");
    assert_refused(
        authors[0].posts().get().await,
        "a list loaded with with_count",
    );
}

// ---- What is never a violation ------------------------------------------

#[tokio::test]
#[serial]
async fn a_relation_loaded_with_with_load_or_load_missing_reads() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;

    let eager = LzPost::query()
        .order_by_asc("id")
        .with(["author"])
        .get()
        .await
        .unwrap();
    assert_eq!(eager.len(), 2);
    for post in eager.iter() {
        assert!(post.author_loaded().is_some(), "with() loaded the author");
        post.author()
            .first()
            .await
            .expect("a relation loaded with with() is no lazy load");
    }

    let mut loaded = both_posts().await;
    loaded.load(["author"]).await.unwrap();
    for post in loaded.iter() {
        assert!(post.author_loaded().is_some(), "load() loaded the author");
        post.author()
            .first()
            .await
            .expect("a relation loaded with load() is no lazy load");
    }

    let mut one_row_loaded = both_posts().await;
    one_row_loaded[1].load_missing(["author"]).await.unwrap();
    let author = one_row_loaded[1]
        .author()
        .first()
        .await
        .expect("a relation loaded with load_missing() on the row is no lazy load");
    assert_eq!(author.map(|a| a.name), Some("grace".to_string()));
    assert!(
        one_row_loaded[0].author().first().await.is_err(),
        "the other row still has the relation unloaded"
    );
}

#[tokio::test]
#[serial]
async fn a_model_from_a_one_row_read_or_built_in_the_process_loads_lazily() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;

    let found = LzPost::find(1).await.unwrap().expect("post 1");
    let author = found
        .author()
        .first()
        .await
        .expect("a model from find loads lazily");
    assert_eq!(author.map(|a| a.name), Some("ada".to_string()));

    let first = LzPost::query()
        .order_by_asc("id")
        .first()
        .await
        .unwrap()
        .expect("a post");
    first
        .author()
        .first()
        .await
        .expect("a model from first loads lazily");

    let one = LzPost::query()
        .filter("title", "nebula")
        .get()
        .await
        .unwrap();
    assert_eq!(one.len(), 1, "the get returned one row");
    let author = one[0]
        .author()
        .first()
        .await
        .expect("a model from a get of one row loads lazily");
    assert_eq!(author.map(|a| a.name), Some("grace".to_string()));

    let created = LzPost::create(attrs! { lz_author_id: 2, title: "pulsar" })
        .await
        .unwrap();
    created
        .author()
        .first()
        .await
        .expect("a model created in the process loads lazily");
}

// ---- Models the eager loader loaded --------------------------------------

#[tokio::test]
#[serial]
async fn a_nested_lazy_read_on_eager_loaded_models_is_refused() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;

    // One IN-query loads the two posts of the two authors: two rows.
    let authors = LzAuthor::query()
        .order_by_asc("id")
        .with(["posts"])
        .get()
        .await
        .unwrap();
    let post = &authors[0].posts_loaded()[0];
    start_query_log();
    let refused = post.comments().get().await;
    let reads = stop_query_log_counting_reads();
    let message = refused
        .expect_err("the posts came from an eager load of two rows")
        .to_string();
    assert!(message.contains("`comments`"), "{message}");
    assert_eq!(reads, 0, "a refused read runs no query");

    // Loaded along the whole path, the nested relation reads.
    let authors = LzAuthor::query()
        .order_by_asc("id")
        .with(["posts.comments"])
        .get()
        .await
        .unwrap();
    let post = &authors[0].posts_loaded()[0];
    assert_eq!(post.comments_loaded().len(), 1);
    post.comments()
        .get()
        .await
        .expect("a nested relation loaded with with() is no lazy load");

    // An eager load that fetched one row marks nothing.
    let mut ada = LzAuthor::find(1).await.unwrap().expect("ada");
    ada.load(["posts"]).await.unwrap();
    let comments = ada.posts_loaded()[0]
        .comments()
        .get()
        .await
        .expect("the one post of one author came from a one-row read");
    assert_eq!(comments.len(), 1);
}

// ---- The switch on, with a handler ---------------------------------------

#[tokio::test]
#[serial]
async fn a_handler_is_called_with_the_names_and_the_read_goes_on() {
    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    let seen: Arc<Mutex<Vec<LazyLoadingViolation>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    handle_lazy_loading_violation(move |violation| {
        sink.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(*violation);
    })
    .unwrap();
    let posts = both_posts().await;

    start_query_log();
    let mut authors = Vec::new();
    for post in posts.iter() {
        authors.push(post.author().first().await);
    }
    let reads = stop_query_log_counting_reads();

    let names: Vec<String> = authors
        .into_iter()
        .map(|author| {
            author
                .expect("with a handler the read goes on")
                .expect("the post has an author")
                .name
        })
        .collect();
    assert_eq!(names, ["ada", "grace"]);
    assert_eq!(reads, 2, "each read ran its query");
    let seen = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(seen.len(), 2, "the handler was called once per read");
    for violation in seen {
        assert_eq!(violation.model, "LzPost");
        assert_eq!(violation.relation, "author");
    }
}

#[tokio::test]
#[serial]
async fn a_handler_that_panics_poisons_nothing() {
    use futures::FutureExt as _;

    let _switch = SwitchGuard::on();
    let _db = fixture().await;
    handle_lazy_loading_violation(|_| panic!("a handler that panics")).unwrap();
    let posts = both_posts().await;

    // The panic unwinds through the read that called the handler.
    let panicked = std::panic::AssertUnwindSafe(posts[0].author().first())
        .catch_unwind()
        .await;
    assert!(panicked.is_err(), "the handler panicked inside the read");

    // The handler lock is whole: a new handler registers and is called.
    let seen: Arc<Mutex<Vec<LazyLoadingViolation>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    handle_lazy_loading_violation(move |violation| {
        sink.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(*violation);
    })
    .expect("the handler lock is not poisoned");
    posts[1]
        .author()
        .first()
        .await
        .expect("the read goes on with the new handler");
    assert_eq!(seen.lock().unwrap_or_else(PoisonError::into_inner).len(), 1);
}
