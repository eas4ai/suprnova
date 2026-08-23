use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};

#[derive(Clone, Debug)]
pub enum SeaOrm11Fixture {
    Sqlite,
    Postgres,
    MySql,
}

impl SeaOrm11Fixture {
    pub fn sql(&self) -> &'static str {
        match self {
            #[cfg(feature = "seaorm-sqlite")]
            Self::Sqlite => include_str!("seaorm_1_1/sqlite.sql"),
            #[cfg(not(feature = "seaorm-sqlite"))]
            Self::Sqlite => unreachable!("sqlite fixture requires feature `seaorm-sqlite`"),

            #[cfg(feature = "seaorm-postgres")]
            Self::Postgres => include_str!("seaorm_1_1/postgres.sql"),
            #[cfg(not(feature = "seaorm-postgres"))]
            Self::Postgres => unreachable!("postgres fixture requires feature `seaorm-postgres`"),

            #[cfg(feature = "seaorm-mysql")]
            Self::MySql => include_str!("seaorm_1_1/mysql.sql"),
            #[cfg(not(feature = "seaorm-mysql"))]
            Self::MySql => unreachable!("mysql fixture requires feature `seaorm-mysql`"),
        }
    }

    pub fn backend(&self) -> DbBackend {
        match self {
            #[cfg(feature = "seaorm-sqlite")]
            Self::Sqlite => DbBackend::Sqlite,
            #[cfg(not(feature = "seaorm-sqlite"))]
            Self::Sqlite => unreachable!("sqlite fixture requires feature `seaorm-sqlite`"),

            #[cfg(feature = "seaorm-postgres")]
            Self::Postgres => DbBackend::Postgres,
            #[cfg(not(feature = "seaorm-postgres"))]
            Self::Postgres => unreachable!("postgres fixture requires feature `seaorm-postgres`"),

            #[cfg(feature = "seaorm-mysql")]
            Self::MySql => DbBackend::MySql,
            #[cfg(not(feature = "seaorm-mysql"))]
            Self::MySql => unreachable!("mysql fixture requires feature `seaorm-mysql`"),
        }
    }

    pub fn backend_label(&self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
            Self::MySql => "mysql",
        }
    }
}

pub type ImportResult<T> = Result<T, String>;

pub struct ImportedDatabase {
    pub connection: DatabaseConnection,
    backend: DbBackend,
    admin_url: Option<String>,
    database_name: Option<String>,
}

fn random_name(prefix: &str, fixture: &SeaOrm11Fixture) -> String {
    format!("magnetar_{prefix}_{}_{}", fixture.backend_label(), rand::random::<u64>())
}

fn database_url(admin_url: &str, name: &str) -> String {
    let (prefix, _) = admin_url
        .rsplit_once('/')
        .expect("backend URL must include a database name");
    format!("{prefix}/{name}")
}

fn normalized_statements(sql: &str) -> Vec<String> {
    let mut cleaned = String::new();
    for line in sql.lines() {
        let line = line.trim_end();
        if line.trim_start().starts_with("--") {
            continue;
        }
        cleaned.push_str(line);
        cleaned.push('\n');
    }
    cleaned
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
async fn connect_single_connection(url: &str) -> ImportResult<DatabaseConnection> {
    let mut options = ConnectOptions::new(url.to_owned());
    options.max_connections(1).min_connections(1);
    Database::connect(options)
        .await
        .map_err(|error| format!("fixture connection failed for {url}: {error}"))
}

async fn execute_fixture_sql(database: &DatabaseConnection, fixture: SeaOrm11Fixture) -> ImportResult<()> {
    for statement in normalized_statements(fixture.sql()) {
        database
            .execute_raw(Statement::from_string(fixture.backend(), statement.clone()))
            .await
            .map_err(|error| {
                format!("fixture statement failed under {:?}: {statement} -> {error}", fixture.backend())
            })?;
    }

    if fixture.backend() == DbBackend::Postgres {
        database
            .execute_raw(Statement::from_string(
                DbBackend::Postgres,
                "SET search_path TO public".to_owned(),
            ))
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
            .ok_or_else(|| "fixture PostgreSQL import must return one current_schema() row".to_owned())?
            .try_get_by_index::<String>(0)
            .map_err(|error| format!("fixture PostgreSQL current_schema value must be readable: {error}"))?
            .to_lowercase();

        if current_schema != "public" {
            return Err(format!(
                "fixture PostgreSQL search_path must be public, got {current_schema}"
            ));
        }
    }

    Ok(())
}

async fn drop_temporary_database(
    admin_url: &str,
    backend: DbBackend,
    database_name: &str,
) -> ImportResult<()> {
    let admin = connect_single_connection(admin_url).await?;
    let statement = match backend {
        DbBackend::Postgres => format!("DROP DATABASE IF EXISTS \"{database_name}\" WITH (FORCE)"),
        DbBackend::MySql => format!("DROP DATABASE IF EXISTS `{database_name}`"),
        DbBackend::Sqlite => {
            return Err("temporary sqlite database cleanup should not connect to admin DB".to_owned());
        }
        _ => unreachable!("only sqlite/postgres/mysql fixture DB cleanup is required"),
    };
    admin
        .execute_raw(Statement::from_string(backend, statement))
        .await
        .map_err(|error| format!("fixture database `{database_name}` must be dropped: {error}"))?;
    Ok(())
}
pub async fn import_fixture(fixture: SeaOrm11Fixture) -> ImportResult<ImportedDatabase> {
    match fixture.backend() {
        DbBackend::Sqlite => {
            let connection = connect_single_connection("sqlite::memory:").await?;
            execute_fixture_sql(&connection, fixture).await?;
            Ok(ImportedDatabase {
                connection,
                backend: DbBackend::Sqlite,
                admin_url: None,
                database_name: None,
            })
        }
        DbBackend::Postgres => {
            let admin_url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
                .map_err(|error| format!("MAGNETAR_POSTGRES_TEST_URL is required: {error}"))?;
            let admin = connect_single_connection(&admin_url).await?;
            let database_name = random_name("upgrade", &fixture);
            admin
                .execute_raw(Statement::from_string(
                    DbBackend::Postgres,
                    format!("CREATE DATABASE \"{database_name}\""),
                ))
                .await
                .map_err(|error| {
                    format!("isolated PostgreSQL fixture database `{database_name}` must be created: {error}")
                })?;

            let database_url = database_url(&admin_url, &database_name);
            let connection = match connect_single_connection(&database_url).await {
                Ok(connection) => connection,
                Err(error) => {
                    let cleanup = drop_temporary_database(
                        &admin_url,
                        DbBackend::Postgres,
                        &database_name,
                    )
                    .await;
                    return match cleanup {
                        Ok(()) => Err(error),
                        Err(drop_error) => {
                            Err(format!("failed to connect to fixture database `{database_name}`: {error}; cleanup failed: {drop_error}"))
                        }
                    };
                }
            };

            let imported = ImportedDatabase {
                connection,
                backend: DbBackend::Postgres,
                admin_url: Some(admin_url),
                database_name: Some(database_name),
            };
            if let Err(error) = execute_fixture_sql(&imported.connection, fixture).await {
                let cleanup = imported.cleanup().await;
                return cleanup.map_or_else(
                    |cleanup_error| {
                        Err(format!("fixture SQL execution failed: {error}; cleanup failed: {cleanup_error}"))
                    },
                    |_| Err(format!("fixture SQL execution failed: {error}")),
                );
            }

            Ok(imported)
        }
        DbBackend::MySql => {
            let admin_url = std::env::var("MAGNETAR_MYSQL_TEST_URL")
                .map_err(|error| format!("MAGNETAR_MYSQL_TEST_URL is required: {error}"))?;
            let admin = connect_single_connection(&admin_url).await?;
            let database_name = random_name("upgrade", &fixture);
            admin
                .execute_raw(Statement::from_string(
                    DbBackend::MySql,
                    format!("CREATE DATABASE `{database_name}`"),
                ))
                .await
                .map_err(|error| {
                    format!("isolated MySQL fixture database `{database_name}` must be created: {error}")
                })?;

            let database_url = database_url(&admin_url, &database_name);
            let connection = match connect_single_connection(&database_url).await {
                Ok(connection) => connection,
                Err(error) => {
                    let cleanup = drop_temporary_database(
                        &admin_url,
                        DbBackend::MySql,
                        &database_name,
                    )
                    .await;
                    return match cleanup {
                        Ok(()) => Err(error),
                        Err(drop_error) => {
                            Err(format!("failed to connect to fixture database `{database_name}`: {error}; cleanup failed: {drop_error}"))
                        }
                    };
                }
            };

            let imported = ImportedDatabase {
                connection,
                backend: DbBackend::MySql,
                admin_url: Some(admin_url),
                database_name: Some(database_name),
            };
            if let Err(error) = execute_fixture_sql(&imported.connection, fixture).await {
                let cleanup = imported.cleanup().await;
                return cleanup.map_or_else(
                    |cleanup_error| {
                        Err(format!("fixture SQL execution failed: {error}; cleanup failed: {cleanup_error}"))
                    },
                    |_| Err(format!("fixture SQL execution failed: {error}")),
                );
            }

            Ok(imported)
        }
        _ => unreachable!("only sqlite, postgres, and mysql fixtures are supported"),
    }
}

impl ImportedDatabase {
    pub async fn cleanup(self) -> ImportResult<()> {
        let close_error = self.connection.close().await.err().map(|error| {
            format!("fixture database connection could not close cleanly: {error}")
        });

        let drop_error = match self.backend {
            DbBackend::Sqlite => Ok(()),
            DbBackend::Postgres => {
                let admin_url = self
                    .admin_url
                    .expect("fixture cleanup must preserve PostgreSQL admin URL");
                let database_name = self
                    .database_name
                    .expect("fixture cleanup must preserve PostgreSQL database name");
                drop_temporary_database(&admin_url, DbBackend::Postgres, &database_name).await
            }
            DbBackend::MySql => {
                let admin_url = self
                    .admin_url
                    .expect("fixture cleanup must preserve MySQL admin URL");
                let database_name = self
                    .database_name
                    .expect("fixture cleanup must preserve MySQL database name");
                drop_temporary_database(&admin_url, DbBackend::MySql, &database_name).await
            }
            _ => unreachable!("only sqlite/postgres/mysql cleanup is supported"),
        };

        match (close_error, drop_error) {
            (None, Ok(())) => Ok(()),
            (None, Err(error)) => Err(error),
            (Some(error), Ok(())) => Err(error),
            (Some(close_error), Err(drop_error)) => {
                Err(format!("{close_error}; fallback database-drop attempt also ran: {drop_error}"))
            }
        }
    }
}
