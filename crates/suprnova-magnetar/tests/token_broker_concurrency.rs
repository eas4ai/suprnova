//! Two-pod convergence suite (Task 5:
//! `docs/specs/suprnova-magnetar/11-token-broker.md`'s "Every concurrency
//! property is proven on all three backends plus in-memory SQLite"):
//! two independent `TokenBrokerService` instances share one database, race
//! concurrent refresh/access calls with `single_flight = false`, and must
//! converge on exactly one committed provider call with zero double
//! spends. SQLite always runs; Postgres and MySQL run against the live
//! backends named by `MAGNETAR_POSTGRES_TEST_URL`/`MAGNETAR_MYSQL_TEST_URL`
//! whenever the corresponding Cargo feature is compiled in, mirroring
//! `tests/storage_tokens.rs`'s existing live-backend convention.

#![cfg(all(
    feature = "oauth",
    any(
        feature = "seaorm-sqlite",
        feature = "seaorm-postgres",
        feature = "seaorm-mysql"
    )
))]

#[path = "fixtures/broker_harness.rs"]
mod broker_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use magnetar::broker::{BrokerConfig, TokenBroker, TokenBrokerService};
use magnetar::crypto::{AeadEncryptor, CryptoPurpose, Encryptor};
use magnetar::oauth::OAuthProviderRegistry;
use magnetar::storage::{CommitProviderToken, NewProviderToken, ProviderTokenStore, SeaOrmStorage};
use sea_orm::sea_query::{MysqlQueryBuilder, PostgresQueryBuilder, SqliteQueryBuilder};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Schema, Statement};
use secrecy::ExposeSecret;

use broker_harness::{BrokerMockProvider, DelayedScriptedHttpTransport, RecordingReuseHook};
use storage_schema::{StorageSchema, provider_tokens};

/// Bootstrap a `provider_tokens`-only database against a live backend: no
/// precedent in this repo runs real business-logic suites against
/// Postgres/MySQL with full schema creation (only `SELECT 1`-style
/// connectivity checks exist), so this renders the shared fixture entity's
/// `CREATE TABLE` per-backend from `sea_orm::Schema` directly, exactly as
/// `storage_schema::database()` already does for SQLite.
async fn provider_token_database(
    url: &str,
    backend: DbBackend,
) -> magnetar::Result<DatabaseConnection> {
    let db = Database::connect(url)
        .await
        .map_err(|error| magnetar::Error::Internal {
            message: format!("connect to {backend:?} target: {error}"),
        })?;
    let schema = Schema::new(backend);
    let mut create = schema.create_table_from_entity(provider_tokens::Entity);
    create.if_not_exists();
    let stmt = match backend {
        DbBackend::Postgres => create.to_string(PostgresQueryBuilder),
        DbBackend::MySql => create.to_string(MysqlQueryBuilder),
        DbBackend::Sqlite => create.to_string(SqliteQueryBuilder),
        _ => {
            return Err(magnetar::Error::DependencyUnavailable {
                dependency: "database backend".to_owned(),
                message: format!("unsupported SeaORM database backend: {backend:?}"),
            });
        }
    };
    db.execute_raw(Statement::from_string(backend, stmt))
        .await
        .map_err(|error| magnetar::Error::Internal {
            message: format!("create provider_tokens table on {backend:?}: {error}"),
        })?;
    Ok(db)
}

/// A record id unique to this process/run, so repeated suite runs against
/// a persistent live container never collide on a primary key.
fn unique_id(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    let salt: u64 = rand::random();
    format!("two-pod:{label}:{nanos}:{salt}")
}

/// The convergence proof itself, backend-agnostic: seed one record via the
/// store's own claim/commit CAS with an already-expired access token, then
/// race N concurrent `access_token` calls split across two independent
/// `TokenBrokerService` instances sharing `db`, with `single_flight =
/// false` on both -- the storage layer alone must arbitrate to exactly one
/// provider call, one committed generation, and zero racer left holding a
/// stale or discarded result.
async fn run_two_pod_convergence(db: DatabaseConnection) {
    let store: Arc<dyn ProviderTokenStore> = Arc::new(SeaOrmStorage::<StorageSchema>::new(db));
    let encryptor = Arc::new(AeadEncryptor::new([11; 32]));
    let record_id = unique_id("convergence");

    store
        .create_if_missing(NewProviderToken {
            id: record_id.clone(),
            provider: "mock".to_owned(),
        })
        .await
        .expect("create_if_missing");
    let now = Utc::now();
    let seed_claim = "seed";
    assert!(
        store
            .claim(
                &record_id,
                0,
                seed_claim,
                now + chrono::Duration::seconds(30),
                now
            )
            .await
            .expect("seed claim")
    );
    let access_ciphertext = encryptor
        .encrypt(CryptoPurpose::ProviderToken, b"seed-access")
        .unwrap();
    let refresh_ciphertext = encryptor
        .encrypt(CryptoPurpose::RefreshToken, b"seed-refresh")
        .unwrap();
    let raw_payload_ciphertext = encryptor
        .encrypt(CryptoPurpose::ProviderToken, b"{}")
        .unwrap();
    assert!(
        store
            .commit(
                &record_id,
                seed_claim,
                0,
                CommitProviderToken {
                    access_ciphertext,
                    refresh_ciphertext: Some(refresh_ciphertext),
                    raw_payload_ciphertext,
                    token_type: "Bearer".to_owned(),
                    scopes: String::new(),
                    // Already expired: every racer must observe a need to
                    // refresh, never the fast path.
                    access_expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
                    new_generation: 0,
                },
            )
            .await
            .expect("seed commit")
    );

    let provider = Arc::new(BrokerMockProvider::new("mock", "https://mock.test/token"));
    let transport = Arc::new(DelayedScriptedHttpTransport::default());
    transport.push_json_after(
        Duration::from_millis(50),
        200,
        r#"{"access_token":"converged-access","token_type":"Bearer","expires_in":3600,"refresh_token":"converged-refresh"}"#,
    );

    let config = BrokerConfig {
        // Correctness must hold with no in-process coordination between
        // the two pods: the storage CAS write is the only arbiter.
        single_flight: false,
        provider_call_timeout: Duration::from_millis(1000),
        lease_grace: Duration::from_millis(300),
        poll_interval: Duration::from_millis(10),
        ..BrokerConfig::default()
    };

    let mut registry_a = OAuthProviderRegistry::new();
    registry_a.register(provider.clone()).expect("registers");
    let mut registry_b = OAuthProviderRegistry::new();
    registry_b.register(provider.clone()).expect("registers");

    let hook_a = Arc::new(RecordingReuseHook::default());
    let hook_b = Arc::new(RecordingReuseHook::default());

    let pod_a = Arc::new(
        TokenBrokerService::new(
            store.clone(),
            encryptor.clone(),
            transport.clone(),
            Arc::new(registry_a),
            config.clone(),
        )
        .with_reuse_hook(hook_a.clone()),
    );
    let pod_b = Arc::new(
        TokenBrokerService::new(
            store.clone(),
            encryptor.clone(),
            transport.clone(),
            Arc::new(registry_b),
            config,
        )
        .with_reuse_hook(hook_b.clone()),
    );

    // Racers split across both pods, both talking to one shared database
    // with no in-process coordination between the two `TokenBrokerService`
    // instances -- the literal "two-pod" deployment shape.
    const RACERS: usize = 8;
    let mut handles = Vec::with_capacity(RACERS);
    for i in 0..RACERS {
        let record_id = record_id.clone();
        let pod: Arc<TokenBrokerService> = if i % 2 == 0 {
            pod_a.clone()
        } else {
            pod_b.clone()
        };
        handles.push(tokio::spawn(
            async move { pod.access_token(&record_id).await },
        ));
    }

    let mut tokens = Vec::with_capacity(RACERS);
    for handle in handles {
        tokens.push(
            handle
                .await
                .expect("racer task must not panic")
                .expect("every racer, on either pod, must converge on a valid token"),
        );
    }
    for token in &tokens {
        assert_eq!(
            token.value.expose_secret(),
            "converged-access",
            "no racer may observe a stale or discarded result"
        );
    }
    assert_eq!(
        transport.request_count(),
        1,
        "exactly one provider call across both pods sharing one database"
    );
    assert_eq!(
        hook_a.count() + hook_b.count(),
        0,
        "a healthy concurrent refresh must never be mistaken for reuse"
    );

    let row = store.read(&record_id).await.unwrap().expect("row exists");
    assert_eq!(
        row.generation, 1,
        "the rotating provider response must advance the generation exactly once"
    );
    assert!(
        row.claim_id.is_none(),
        "the lease must be released after convergence"
    );

    store
        .delete(&record_id)
        .await
        .expect("cleanup seeded record");
}

#[cfg(feature = "seaorm-sqlite")]
#[tokio::test]
async fn two_pod_convergence_sqlite() {
    let db = provider_token_database("sqlite::memory:", DbBackend::Sqlite)
        .await
        .unwrap();
    run_two_pod_convergence(db).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
async fn two_pod_convergence_postgres() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL must be configured for the live two-pod suite");
    let db = provider_token_database(&url, DbBackend::Postgres)
        .await
        .unwrap();
    run_two_pod_convergence(db).await;
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
async fn two_pod_convergence_mysql() {
    let url = std::env::var("MAGNETAR_MYSQL_TEST_URL")
        .expect("MAGNETAR_MYSQL_TEST_URL must be configured for the live two-pod suite");
    let db = provider_token_database(&url, DbBackend::MySql)
        .await
        .unwrap();
    run_two_pod_convergence(db).await;
}
