use super::*;
use crate::render_cache_middleware_support::{self as cache, Post};
use suprnova::render_cache::{DependencyIdentity, GenerationLedger, RenderCache};
use suprnova::{Aggregate, EventReplay, Model, Projector, attrs};

#[domain_event(name = "es.cache.title", version = 1)]
#[derive(Serialize, Deserialize)]
pub(super) struct Title {
    pub(super) id: i64,
    pub(super) title: String,
}

#[suprnova::projector]
#[derive(Default)]
struct Titles;
#[suprnova::async_trait]
impl Projector for Titles {
    fn name(&self) -> &'static str {
        "es.titles"
    }
    fn handles(&self, event: &StoredEvent) -> bool {
        event.event_type == "es.cache.title"
    }
    async fn project(&self, event: &StoredEvent) -> Result<(), FrameworkError> {
        if let Some(fact) = event.decode::<Title>()? {
            let mut post = Post::find(fact.id)
                .await?
                .ok_or_else(|| FrameworkError::internal("post missing"))?;
            post.title = fact.title;
            post.save().await?;
        }
        Ok(())
    }
    async fn reset(&self) -> Result<(), FrameworkError> {
        Post::query().update_all(attrs! { title: "" }).await?;
        Ok(())
    }
}

#[tokio::test]
#[serial_test::serial]
async fn es_003_replay_orm_writes_advance_generations_and_replace_a_cached_page() {
    let _db = database().await;
    let harness =
        cache::boot_with_render_cache_on_live_server_for_test(suprnova::DB::connection().unwrap())
            .await;
    let post = Post::create(attrs! { title: "initial" }).await.unwrap();
    let mut root = Aggregate::<Tally>::new(&uuid()).unwrap();
    root.record(Title {
        id: post.id,
        title: "from history".into(),
    })
    .await
    .unwrap();
    root.persist().await.unwrap();
    let mut corrupt = Post::find(post.id).await.unwrap().unwrap();
    corrupt.title = "incorrect projection".into();
    corrupt.save().await.unwrap();
    let path = format!("/write-atomicity/{}", post.id);
    let first = cache::dispatch_get(&harness, &path, &[]).await;
    assert_eq!(first.body.as_ref(), b"incorrect projection");
    let renders = cache::counting_route::renders();
    let hit = cache::dispatch_get(&harness, &path, &[]).await;
    assert_eq!(hit.body, first.body);
    assert_eq!(
        cache::counting_route::renders(),
        renders,
        "a stored entry served the second GET"
    );
    let ledger = cache::ledger();
    let identity = DependencyIdentity::record("posts", post.id.to_string().as_bytes());
    let before = ledger
        .current(&[identity.digest()])
        .await
        .unwrap()
        .get(&identity)
        .unwrap();
    EventReplay::projectors(["es.titles"])
        .page_size(1)
        .run()
        .await
        .unwrap();
    let after = ledger
        .current(&[identity.digest()])
        .await
        .unwrap()
        .get(&identity)
        .unwrap();
    assert!(after > before);
    let fresh = cache::dispatch_get(&harness, &path, &[]).await;
    assert_eq!(fresh.body.as_ref(), b"from history");
    assert_eq!(cache::counting_route::renders(), renders + 1);
    RenderCache::uninstall_for_test();
}
