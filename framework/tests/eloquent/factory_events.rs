//! A factory insert fires the lifecycle events `Model::create` fires, so an
//! observer sees a factory's rows exactly as it sees the application's.
//!
//! The observer registry is process-wide. Under nextest each test is its
//! own process; the counters are still read as differences so the tests
//! also hold in one shared process.

use std::sync::atomic::{AtomicUsize, Ordering};

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

static CREATED: AtomicUsize = AtomicUsize::new(0);
static SAVED: AtomicUsize = AtomicUsize::new(0);

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
        CREATED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn saved(&self, _widget: &FeWidget) -> Result<(), FrameworkError> {
        SAVED.fetch_add(1, Ordering::SeqCst);
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
    let (created, saved) = (CREATED.load(Ordering::SeqCst), SAVED.load(Ordering::SeqCst));

    let rows = FeWidgetFactory::new()
        .count(3)
        .with(|widget| widget.name = "Blue Widget".into())
        .create_many()
        .await
        .unwrap();

    assert_eq!(rows.len(), 3);
    assert_eq!(CREATED.load(Ordering::SeqCst) - created, 3);
    assert_eq!(SAVED.load(Ordering::SeqCst) - saved, 3);
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
    let created = CREATED.load(Ordering::SeqCst);

    let refused = FeWidgetFactory::new()
        .with(|widget| widget.name = "forbidden".into())
        .create()
        .await;

    assert!(
        refused.is_err(),
        "the insert is refused as create refuses it"
    );
    assert_eq!(CREATED.load(Ordering::SeqCst), created);
    assert_eq!(FeWidget::query().count().await.unwrap(), 0);
}

#[tokio::test]
async fn create_quietly_inserts_without_any_observer() {
    let _db = widgets().await;
    let (created, saved) = (CREATED.load(Ordering::SeqCst), SAVED.load(Ordering::SeqCst));

    let row = FeWidgetFactory::new()
        .with(|widget| widget.name = "forbidden".into())
        .create_quietly()
        .await
        .expect("no observer runs, so nothing refuses the name");

    assert_eq!(row.slug, "", "no observer derived a slug");
    assert_eq!(CREATED.load(Ordering::SeqCst), created);
    assert_eq!(SAVED.load(Ordering::SeqCst), saved);
    assert_eq!(FeWidget::query().count().await.unwrap(), 1);
}
