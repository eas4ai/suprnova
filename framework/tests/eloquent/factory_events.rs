//! A factory insert fires the lifecycle events `Model::create` fires, so an
//! observer sees a factory's rows exactly as it sees the application's.
//!
//! The observer registry is process-wide, so the observer below runs for
//! the widgets of every test. Its counters are per thread: a
//! `#[tokio::test]` runs its body, and the observers its inserts fire, on
//! its own thread, so a test counts its own rows even while another test
//! inserts widgets beside it in one process under plain `cargo test`. The
//! counters are still read as differences, so the tests hold however
//! tests are given threads.

use std::cell::Cell;

use async_trait::async_trait;
use suprnova::eloquent::attrs::Attrs;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::observers::Observer;
use suprnova::testing::TestDatabase;
use suprnova::{Factory, FrameworkError, Model as _, model};

#[model(table = "fe_widgets", fillable = ["name"])]
pub struct FeWidget {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

pub struct FeWidgetFactory;

impl Factory for FeWidgetFactory {
    type Model = FeWidget;

    fn definition() -> FeWidget {
        FeWidget {
            id: 0,
            name: "Widget".into(),
            slug: String::new(),
            ..Default::default()
        }
    }
}

thread_local! {
    static CREATED: Cell<usize> = const { Cell::new(0) };
    static SAVED: Cell<usize> = const { Cell::new(0) };
}

pub struct FeWidgetObserver;

#[suprnova::observer(FeWidget)]
#[async_trait]
impl Observer<FeWidget> for FeWidgetObserver {
    /// Derives the slug, and refuses a widget named `forbidden`.
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        let name = attrs
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_owned();
        if name == "forbidden" {
            return EventResult::cancel("that name is not allowed");
        }
        attrs.insert("slug", name.to_lowercase().replace(' ', "-"));
        EventResult::ok()
    }

    async fn created(&self, _widget: &FeWidget) -> Result<(), FrameworkError> {
        CREATED.set(CREATED.get() + 1);
        Ok(())
    }

    async fn saved(&self, _widget: &FeWidget) -> Result<(), FrameworkError> {
        SAVED.set(SAVED.get() + 1);
        Ok(())
    }
}

async fn widgets() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE fe_widgets (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            name TEXT NOT NULL, \
            slug TEXT NOT NULL\
         )",
    )
    .await
    .unwrap();
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .unwrap();
    db
}

#[tokio::test]
async fn a_factory_insert_fires_the_events_create_fires() {
    let _db = widgets().await;
    let (created, saved) = (CREATED.get(), SAVED.get());

    let rows = FeWidgetFactory::new()
        .count(3)
        .with(|widget| widget.name = "Blue Widget".into())
        .create_many()
        .await
        .unwrap();

    assert_eq!(rows.len(), 3);
    assert_eq!(CREATED.get() - created, 3);
    assert_eq!(SAVED.get() - saved, 3);
    for row in &rows {
        assert_eq!(
            row.slug, "blue-widget",
            "what the creating observer set is what was written"
        );
    }
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    assert_eq!(ids, [1, 2, 3], "the database assigned each key");
    let stored = FeWidget::find(1).await.unwrap().expect("the row is stored");
    assert_eq!(stored.slug, "blue-widget");
}

#[tokio::test]
async fn a_creating_observer_that_cancels_aborts_the_factory_insert() {
    let _db = widgets().await;
    let created = CREATED.get();

    let refused = FeWidgetFactory::new()
        .with(|widget| widget.name = "forbidden".into())
        .create()
        .await;

    assert!(
        refused.is_err(),
        "the insert is refused as create refuses it"
    );
    assert_eq!(CREATED.get(), created);
    assert_eq!(FeWidget::query().count().await.unwrap(), 0);
}

#[tokio::test]
async fn create_quietly_inserts_without_any_observer() {
    let _db = widgets().await;
    let (created, saved) = (CREATED.get(), SAVED.get());

    let row = FeWidgetFactory::new()
        .with(|widget| widget.name = "forbidden".into())
        .create_quietly()
        .await
        .expect("no observer runs, so nothing refuses the name");

    assert_eq!(row.slug, "", "no observer derived a slug");
    assert_eq!(CREATED.get(), created);
    assert_eq!(SAVED.get(), saved);
    assert_eq!(FeWidget::query().count().await.unwrap(), 1);
}
