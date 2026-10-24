//! The invalid inventory stays in its own process so other boot tests remain valid.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use suprnova::{Config, Router, Server, domain_event};

#[domain_event(name = "es.boot.duplicate", version = 1)]
#[derive(Serialize, Deserialize)]
struct FirstFact;
#[domain_event(name = "es.boot.duplicate", version = 1)]
#[derive(Serialize, Deserialize)]
struct SecondFact;

#[tokio::test]
async fn es_002_duplicate_registry_stops_boot_before_route_construction() {
    let _container = suprnova::testing::TestContainer::fake();
    Config::register(
        suprnova::config::AppConfig::builder()
            .environment(suprnova::config::Environment::Testing)
            .build(),
    );
    suprnova::testing::install_test_encryption_key();
    let called = Arc::new(AtomicBool::new(false));
    let route_called = called.clone();
    let result = Server::try_from_config_with_routes_async(move || async move {
        route_called.store(true, Ordering::SeqCst);
        Ok(Router::new())
    })
    .await;
    assert!(
        !called.load(Ordering::SeqCst),
        "the route constructor ran before registry validation"
    );
    let error = result
        .err()
        .expect("duplicate registry must refuse boot")
        .to_string();
    assert!(error.contains("es.boot.duplicate"), "{error}");
    assert!(
        error.contains(std::any::type_name::<FirstFact>()),
        "{error}"
    );
    assert!(
        error.contains(std::any::type_name::<SecondFact>()),
        "{error}"
    );
}
