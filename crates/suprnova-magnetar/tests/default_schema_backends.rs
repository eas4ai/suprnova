#![cfg(all(
    feature = "migration",
    any(feature = "seaorm-postgres", feature = "seaorm-mysql")
))]

use magnetar::default_migration::DefaultMigrationBindings;
use magnetar::default_schema::DefaultAuthSchema;
use magnetar::migration::{MigrationEngine, MigrationRunner, ShapeConfirmation, SourceShape};
use magnetar::storage::{NewUser, SeaOrmStorage, UserStore};
use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

async fn verify(url: &str) {
    let database = Database::connect(url).await.expect("connect live backend");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("create default auth tables");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("default migration is replay-safe");
    let runner = MigrationEngine::new(
        database.clone(),
        DefaultMigrationBindings::new(database.clone()).sharing_source_database(),
    );
    assert_eq!(
        runner.detect_shape().await.expect("detect default schema"),
        SourceShape::Magnetar
    );
    let store = SeaOrmStorage::<DefaultAuthSchema>::new(database);
    let email = format!("default-schema-{}@example.test", rand::random::<u64>());
    let created = store
        .create_user(NewUser {
            email: email.clone(),
            password_hash: Some("fixture-hash".to_owned()),
        })
        .await
        .expect("create canonical i64 app user");
    assert_eq!(created.email, email);
    assert!(created.user_id.parse::<i64>().is_ok());
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
async fn postgres_default_schema_is_replay_safe() {
    let url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    verify(&url).await;
}

#[cfg(feature = "seaorm-postgres")]
#[tokio::test]
async fn postgres_api_import_advances_the_default_user_sequence() {
    let server_url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
        .expect("MAGNETAR_POSTGRES_TEST_URL is required");
    let admin = Database::connect(&server_url)
        .await
        .expect("connect PostgreSQL admin database");
    let database_name = format!("magnetar_sequence_{}", rand::random::<u64>());
    admin
        .execute(Statement::from_string(
            DbBackend::Postgres,
            format!("CREATE DATABASE \"{database_name}\""),
        ))
        .await
        .expect("create isolated PostgreSQL database");
    let prefix = server_url
        .rsplit_once('/')
        .expect("PostgreSQL URL contains a database path")
        .0;
    let database_url = format!("{prefix}/{database_name}");
    let database = Database::connect(&database_url)
        .await
        .expect("connect isolated PostgreSQL database");
    database
        .execute(Statement::from_string(
            DbBackend::Postgres,
            "CREATE TABLE app_users (
                id BIGSERIAL PRIMARY KEY,
                email TEXT NOT NULL UNIQUE
            )"
            .to_owned(),
        ))
        .await
        .expect("create API source users table");
    database
        .execute(Statement::from_string(
            DbBackend::Postgres,
            "INSERT INTO app_users (id, email)
             VALUES (4242, 'imported@example.test')"
                .to_owned(),
        ))
        .await
        .expect("insert API source user");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("create default destination schema");
    let runner = MigrationEngine::new(
        database.clone(),
        DefaultMigrationBindings::new(database.clone()).sharing_source_database(),
    );
    let plan = runner
        .dry_run(ShapeConfirmation {
            detected: SourceShape::SuprnovaApi,
            operator_selected: SourceShape::SuprnovaApi,
        })
        .await
        .expect("plan API migration");
    runner.apply(&plan).await.expect("apply API migration");
    let imported_max = database
        .query_one(Statement::from_string(
            DbBackend::Postgres,
            "SELECT MAX(id) AS max_id FROM app_users".to_owned(),
        ))
        .await
        .expect("read imported user IDs")
        .expect("maximum row");
    assert_eq!(
        imported_max.try_get::<Option<i64>>("", "max_id").unwrap(),
        Some(4242)
    );

    let created = database
        .query_one(Statement::from_string(
            DbBackend::Postgres,
            "INSERT INTO app_users (email)
             VALUES ('after-import@example.test')
             RETURNING id"
                .to_owned(),
        ))
        .await
        .expect("insert application user after migration")
        .expect("inserted user ID");
    assert_eq!(created.try_get::<i64>("", "id").unwrap(), 4243);

    drop(runner);
    drop(database);
    admin
        .execute(Statement::from_string(
            DbBackend::Postgres,
            format!("DROP DATABASE \"{database_name}\""),
        ))
        .await
        .expect("drop isolated PostgreSQL database");
}

#[cfg(feature = "seaorm-mysql")]
#[tokio::test]
async fn mysql_default_schema_is_replay_safe() {
    let url =
        std::env::var("MAGNETAR_MYSQL_TEST_URL").expect("MAGNETAR_MYSQL_TEST_URL is required");
    verify(&url).await;
}
