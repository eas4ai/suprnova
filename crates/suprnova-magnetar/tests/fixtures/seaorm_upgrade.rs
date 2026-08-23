use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement,
};

pub enum SeaOrm11Fixture {
    Sqlite,
    Postgres,
    MySql,
}

impl SeaOrm11Fixture {
    pub fn sql(&self) -> &'static str {
        match self {
            Self::Sqlite => include_str!("seaorm_1_1/sqlite.sql"),
            Self::Postgres => include_str!("seaorm_1_1/postgres.sql"),
            Self::MySql => include_str!("seaorm_1_1/mysql.sql"),
        }
    }

    pub fn backend(&self) -> DbBackend {
        match self {
            Self::Sqlite => DbBackend::Sqlite,
            Self::Postgres => DbBackend::Postgres,
            Self::MySql => DbBackend::MySql,
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

fn normalized_statements(sql: &str, _backend: DbBackend) -> Vec<String> {
    let mut normalized_sql = String::new();
    for raw_line in sql.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("--")
            || (trimmed.starts_with("/*") && trimmed.ends_with("*/"))
        {
            continue;
        }

        normalized_sql.push_str(trimmed);
        normalized_sql.push('\n');
    }

    normalized_sql
        .split(';')
        .map(|statement| statement.trim().to_owned())
        .filter(|statement| !statement.is_empty())
        .collect()
}

async fn connect_single_connection(url: &str) -> DatabaseConnection {
    let mut options = ConnectOptions::new(url.to_owned());
    options.max_connections(1).min_connections(1);
    Database::connect(options)
        .await
        .expect("fixture connection should be created with single pool lane")
}

async fn execute_fixture_sql(database: &DatabaseConnection, fixture: SeaOrm11Fixture) {
    for statement in normalized_statements(fixture.sql(), fixture.backend()) {
        database
            .execute_raw(Statement::from_string(fixture.backend(), statement.clone()))
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "fixture statement must execute under {:?}: {statement} -> {error}",
                    fixture.backend()
                )
            });
    }

    if fixture.backend() == DbBackend::Postgres {
        database
            .execute_raw(Statement::from_string(
                DbBackend::Postgres,
                "SET search_path TO public".to_owned(),
            ))
            .await
            .expect("PostgreSQL fixture import must restore schema search path");

        let current_schema = database
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT current_schema()".to_owned(),
            ))
            .await
            .expect("PostgreSQL fixture must expose current_schema()")
            .expect("PostgreSQL fixture import must run current_schema() query")
            .try_get_by_index::<String>(0)
            .expect("PostgreSQL fixture current_schema() must be readable")
            .to_lowercase();

        assert_eq!(
            current_schema, "public",
            "PostgreSQL fixture session must be reset to public search_path"
        );
    }
}

pub async fn import_fixture(fixture: SeaOrm11Fixture) -> ImportedDatabase {
    match fixture.backend() {
        DbBackend::Sqlite => {
            let connection = connect_single_connection("sqlite::memory:").await;
            execute_fixture_sql(&connection, fixture).await;
            ImportedDatabase {
                connection,
                backend: DbBackend::Sqlite,
                admin_url: None,
                database_name: None,
            }
        }
        DbBackend::Postgres => {
            let admin_url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
                .expect("MAGNETAR_POSTGRES_TEST_URL is required");
            let admin = Database::connect(&admin_url)
                .await
                .expect("PostgreSQL admin connection must be available");
            let database_name = random_name("upgrade", &fixture);
            admin
                .execute_raw(Statement::from_string(
                    DbBackend::Postgres,
                    format!("CREATE DATABASE \"{database_name}\""),
                ))
                .await
                .expect("isolated PostgreSQL fixture database must be created");
            let database_url = database_url(&admin_url, &database_name);
            let connection = connect_single_connection(&database_url).await;
            execute_fixture_sql(&connection, fixture).await;
            ImportedDatabase {
                connection,
                backend: DbBackend::Postgres,
                admin_url: Some(admin_url),
                database_name: Some(database_name),
            }
        }
        DbBackend::MySql => {
            let admin_url = std::env::var("MAGNETAR_MYSQL_TEST_URL")
                .expect("MAGNETAR_MYSQL_TEST_URL is required");
            let admin = Database::connect(&admin_url)
                .await
                .expect("MySQL admin connection must be available");
            let database_name = random_name("upgrade", &fixture);
            admin
                .execute_raw(Statement::from_string(
                    DbBackend::MySql,
                    format!("CREATE DATABASE `{database_name}`"),
                ))
                .await
                .expect("isolated MySQL fixture database must be created");
            let database_url = database_url(&admin_url, &database_name);
            let connection = connect_single_connection(&database_url).await;
            execute_fixture_sql(&connection, fixture).await;
            ImportedDatabase {
                connection,
                backend: DbBackend::MySql,
                admin_url: Some(admin_url),
                database_name: Some(database_name),
            }
        }
        _ => unreachable!("only sqlite, postgres, and mysql fixtures are supported"),
    }
}

impl ImportedDatabase {
    pub async fn cleanup(self) {
        match self.backend {
            DbBackend::Sqlite => {}
            DbBackend::Postgres => {
                let admin_url = self
                    .admin_url
                    .expect("fixture cleanup must preserve PostgreSQL admin URL");
                let database_name = self
                    .database_name
                    .expect("fixture cleanup must preserve PostgreSQL database name");
                let admin = Database::connect(&admin_url)
                    .await
                    .expect("PostgreSQL cleanup must connect to admin URL");
                let statement = format!("DROP DATABASE IF EXISTS \"{database_name}\" WITH (FORCE)");
                admin
                    .execute_raw(Statement::from_string(DbBackend::Postgres, statement))
                    .await
                    .expect("fixture PostgreSQL database must be dropped");
            }
            DbBackend::MySql => {
                let admin_url = self
                    .admin_url
                    .expect("fixture cleanup must preserve MySQL admin URL");
                let database_name = self
                    .database_name
                    .expect("fixture cleanup must preserve MySQL database name");
                let admin = Database::connect(&admin_url)
                    .await
                    .expect("MySQL cleanup must connect to admin URL");
                admin
                    .execute_raw(Statement::from_string(
                        DbBackend::MySql,
                        format!("DROP DATABASE IF EXISTS `{database_name}`"),
                    ))
                    .await
                    .expect("fixture MySQL database must be dropped");
            }
            _ => {}
        }
    }
}
