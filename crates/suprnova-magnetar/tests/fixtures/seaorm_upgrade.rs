use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend};

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
use futures_util::future::FutureExt;
#[cfg(feature = "seaorm-postgres")]
use sea_orm::Statement;
#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
use std::panic::AssertUnwindSafe;

#[derive(Copy, Clone, Debug)]
pub enum SeaOrm11Fixture {
    #[cfg(feature = "seaorm-sqlite")]
    Sqlite,
    #[cfg(feature = "seaorm-postgres")]
    Postgres,
    #[cfg(feature = "seaorm-mysql")]
    MySql,
}

impl SeaOrm11Fixture {
    pub fn sql(&self) -> &'static str {
        match self {
            #[cfg(feature = "seaorm-sqlite")]
            Self::Sqlite => include_str!("seaorm_1_1/sqlite.sql"),
            #[cfg(feature = "seaorm-postgres")]
            Self::Postgres => include_str!("seaorm_1_1/postgres.sql"),
            #[cfg(feature = "seaorm-mysql")]
            Self::MySql => include_str!("seaorm_1_1/mysql.sql"),
        }
    }

    pub fn backend(&self) -> DbBackend {
        match self {
            #[cfg(feature = "seaorm-sqlite")]
            Self::Sqlite => DbBackend::Sqlite,
            #[cfg(feature = "seaorm-postgres")]
            Self::Postgres => DbBackend::Postgres,
            #[cfg(feature = "seaorm-mysql")]
            Self::MySql => DbBackend::MySql,
        }
    }

    #[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
    pub fn backend_label(&self) -> &'static str {
        match self {
            #[cfg(feature = "seaorm-sqlite")]
            Self::Sqlite => "sqlite",
            #[cfg(feature = "seaorm-postgres")]
            Self::Postgres => "postgres",
            #[cfg(feature = "seaorm-mysql")]
            Self::MySql => "mysql",
        }
    }
}

pub type ImportResult<T> = Result<T, String>;

pub struct ImportedDatabase {
    pub connection: DatabaseConnection,
    backend: DbBackend,
    #[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
    admin_url: Option<String>,
    #[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
    database_name: Option<String>,
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
fn random_name(prefix: &str, fixture: &SeaOrm11Fixture) -> String {
    format!(
        "magnetar_{prefix}_{}_{}",
        fixture.backend_label(),
        rand::random::<u64>()
    )
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
fn database_url(admin_url: &str, name: &str) -> ImportResult<String> {
    let (prefix, _) = admin_url
        .rsplit_once('/')
        .ok_or_else(|| "backend URL must include a database name".to_owned())?;
    Ok(format!("{prefix}/{name}"))
}

async fn connect_single_connection(url: &str) -> ImportResult<DatabaseConnection> {
    let mut options = ConnectOptions::new(url.to_owned());
    options.max_connections(1).min_connections(1);
    Database::connect(options)
        .await
        .map_err(|error| format!("fixture connection failed for {url}: {error}"))
}

async fn execute_fixture_sql(
    database: &DatabaseConnection,
    fixture: SeaOrm11Fixture,
) -> ImportResult<()> {
    database
        .execute_unprepared(fixture.sql())
        .await
        .map_err(|error| {
            format!(
                "fixture statement failed under {:?}: {error}",
                fixture.backend()
            )
        })?;

    #[cfg(feature = "seaorm-postgres")]
    if fixture.backend() == DbBackend::Postgres {
        database
            .execute_unprepared("SET search_path TO public")
            .await
            .map_err(|error| {
                format!("fixture PostgreSQL import must restore schema search_path: {error}")
            })?;

        let current_schema = database
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT current_schema()".to_owned(),
            ))
            .await
            .map_err(|error| {
                format!("fixture PostgreSQL import must expose current_schema(): {error}")
            })?
            .ok_or_else(|| {
                "fixture PostgreSQL import must return one current_schema() row".to_owned()
            })?
            .try_get_by_index::<String>(0)
            .map_err(|error| {
                format!("fixture PostgreSQL current_schema value must be readable: {error}")
            })?
            .to_lowercase();

        if current_schema != "public" {
            return Err(format!(
                "fixture PostgreSQL search_path must be public, got {current_schema}"
            ));
        }
    }

    Ok(())
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
async fn drop_temporary_database(
    admin_url: &str,
    backend: DbBackend,
    database_name: &str,
) -> ImportResult<()> {
    #[cfg(feature = "seaorm-sqlite")]
    if matches!(backend, DbBackend::Sqlite) {
        return Err("temporary sqlite database cleanup should not connect to admin DB".to_owned());
    }

    #[cfg(feature = "seaorm-postgres")]
    if matches!(backend, DbBackend::Postgres) {
        let admin = connect_single_connection(admin_url).await?;
        let statement = format!("DROP DATABASE IF EXISTS \"{database_name}\" WITH (FORCE)");
        admin
            .execute_unprepared(&statement)
            .await
            .map_err(|error| {
                format!("fixture database `{database_name}` must be dropped: {error}")
            })?;
        return Ok(());
    }

    #[cfg(feature = "seaorm-mysql")]
    if matches!(backend, DbBackend::MySql) {
        let admin = connect_single_connection(admin_url).await?;
        let statement = format!("DROP DATABASE IF EXISTS `{database_name}`");
        admin
            .execute_unprepared(&statement)
            .await
            .map_err(|error| {
                format!("fixture database `{database_name}` must be dropped: {error}")
            })?;
        return Ok(());
    }

    Err(format!(
        "temporary fixture database drop is unsupported for backend {backend:?}"
    ))
}

#[cfg(feature = "seaorm-sqlite")]
async fn import_temporary_fixture_sqlite(
    fixture: SeaOrm11Fixture,
) -> ImportResult<ImportedDatabase> {
    let connection = connect_single_connection("sqlite::memory:").await?;
    execute_fixture_sql(&connection, fixture).await?;
    Ok(ImportedDatabase {
        connection,
        backend: DbBackend::Sqlite,
        #[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
        admin_url: None,
        #[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
        database_name: None,
    })
}

#[cfg(any(feature = "seaorm-postgres", feature = "seaorm-mysql"))]
async fn import_temporary_fixture_remote(
    fixture: SeaOrm11Fixture,
    admin_url: String,
    database_name: String,
    backend: DbBackend,
    create_sql: String,
) -> ImportResult<ImportedDatabase> {
    let admin = connect_single_connection(&admin_url).await?;
    let mut imported: Option<ImportedDatabase> = None;

    let import = AssertUnwindSafe(async {
        admin
            .execute_unprepared(&create_sql)
            .await
            .map_err(|error| {
                format!(
                    "isolated {backend:?} fixture database `{database_name}` must be created: {error}"
                )
            })?;

        let database_url = database_url(&admin_url, &database_name)?;
        let connection = connect_single_connection(&database_url).await?;
        imported = Some(ImportedDatabase {
            connection,
            backend,
            admin_url: Some(admin_url.clone()),
            database_name: Some(database_name.clone()),
        });

        if let Some(handle) = imported.as_ref() {
            execute_fixture_sql(&handle.connection, fixture).await?;
        } else {
            return Err("successful fixture import lost its cleanup handle before fixture execution".to_owned());
        }

        imported
            .take()
            .ok_or_else(|| "successful fixture import lost its cleanup handle".to_owned())
    })
    .catch_unwind()
    .await;

    match import {
        Ok(Ok(handle)) => Ok(handle),
        Ok(Err(error)) => {
            let cleanup = if let Some(handle) = imported {
                handle.cleanup().await
            } else {
                drop_temporary_database(&admin_url, backend, &database_name).await
            };

            if let Err(cleanup_error) = cleanup {
                Err(format!("{error}; cleanup failed: {cleanup_error}"))
            } else {
                Err(error)
            }
        }
        Err(panic) => {
            let cleanup_error = if let Some(handle) = imported {
                handle.cleanup().await.err()
            } else {
                drop_temporary_database(&admin_url, backend, &database_name)
                    .await
                    .err()
            };

            if let Some(cleanup_error) = cleanup_error {
                eprintln!(
                    "fixture cleanup failed during panic for {backend:?} database `{database_name}`: {cleanup_error}"
                );
            }

            std::panic::resume_unwind(panic)
        }
    }
}

pub async fn import_fixture(fixture: SeaOrm11Fixture) -> ImportResult<ImportedDatabase> {
    let backend = fixture.backend();

    #[cfg(feature = "seaorm-sqlite")]
    if matches!(backend, DbBackend::Sqlite) {
        return import_temporary_fixture_sqlite(fixture).await;
    }

    #[cfg(feature = "seaorm-postgres")]
    if matches!(backend, DbBackend::Postgres) {
        let admin_url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
            .map_err(|error| format!("MAGNETAR_POSTGRES_TEST_URL is required: {error}"))?;
        let database_name = random_name("upgrade", &fixture);
        let create_sql = format!("CREATE DATABASE \"{database_name}\"");
        return import_temporary_fixture_remote(
            fixture,
            admin_url,
            database_name,
            DbBackend::Postgres,
            create_sql,
        )
        .await;
    }

    #[cfg(feature = "seaorm-mysql")]
    if matches!(backend, DbBackend::MySql) {
        let admin_url = std::env::var("MAGNETAR_MYSQL_TEST_URL")
            .map_err(|error| format!("MAGNETAR_MYSQL_TEST_URL is required: {error}"))?;
        let database_name = random_name("upgrade", &fixture);
        let create_sql = format!("CREATE DATABASE `{database_name}`");
        return import_temporary_fixture_remote(
            fixture,
            admin_url,
            database_name,
            DbBackend::MySql,
            create_sql,
        )
        .await;
    }

    Err(format!(
        "temporary fixture import is unsupported for backend {backend:?}"
    ))
}

impl ImportedDatabase {
    pub async fn cleanup(self) -> ImportResult<()> {
        let close_error =
            self.connection.close().await.err().map(|error| {
                format!("fixture database connection could not close cleanly: {error}")
            });

        let drop_error = match self.backend {
            #[cfg(feature = "seaorm-sqlite")]
            DbBackend::Sqlite => Ok(()),

            #[cfg(feature = "seaorm-postgres")]
            DbBackend::Postgres => {
                let admin_url = self
                    .admin_url
                    .expect("fixture cleanup must preserve PostgreSQL admin URL");
                let database_name = self
                    .database_name
                    .expect("fixture cleanup must preserve PostgreSQL database name");
                drop_temporary_database(&admin_url, DbBackend::Postgres, &database_name).await
            }

            #[cfg(feature = "seaorm-mysql")]
            DbBackend::MySql => {
                let admin_url = self
                    .admin_url
                    .expect("fixture cleanup must preserve MySQL admin URL");
                let database_name = self
                    .database_name
                    .expect("fixture cleanup must preserve MySQL database name");
                drop_temporary_database(&admin_url, DbBackend::MySql, &database_name).await
            }

            _ => Err(format!(
                "fixture cleanup is unsupported for backend {:?}",
                self.backend
            )),
        };

        match (close_error, drop_error) {
            (None, Ok(())) => Ok(()),
            (None, Err(error)) => Err(error),
            (Some(error), Ok(())) => Err(error),
            (Some(close_error), Err(drop_error)) => Err(format!(
                "{close_error}; fallback database-drop attempt also ran: {drop_error}"
            )),
        }
    }
}
