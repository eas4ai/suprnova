use futures_util::future::FutureExt;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
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

#[derive(Clone)]
enum SplitState {
    Normal,
    LineComment,
    BlockComment,
    SingleQuote,
    DoubleQuote,
    Backtick,
    Dollar(Vec<u8>),
}

fn is_dollar_tag_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn dollar_quote_delimiter_at(sql: &[u8], start: usize) -> Option<usize> {
    if sql.get(start) != Some(&b'$') {
        return None;
    }

    let mut cursor = start + 1;
    while let Some(byte) = sql.get(cursor) {
        if is_dollar_tag_byte(*byte) {
            cursor += 1;
        } else {
            break;
        }
    }

    let has_tag = cursor > start + 1;
    if let Some(byte) = sql.get(cursor) {
        if *byte == b'$' && (has_tag || cursor == start + 1) {
            Some(cursor - start + 1)
        } else {
            None
        }
    } else {
        None
    }
}

/// Split SQL into executable statements, preserving all bytes.
///
/// The splitter respects SQL statement boundaries while ignoring semicolons that are part
/// of comments or quoted literals.
fn normalized_statements(sql: &str) -> Vec<String> {
    let sql = sql.as_bytes();
    let mut start = 0usize;
    let mut cursor = 0usize;
    let mut state = SplitState::Normal;
    let mut statements = Vec::new();

    while cursor < sql.len() {
        match state {
            SplitState::Normal => {
                if sql[cursor] == b'-' && sql.get(cursor + 1) == Some(&b'-') {
                    state = SplitState::LineComment;
                    cursor += 2;
                    continue;
                }

                if sql[cursor] == b'/' && sql.get(cursor + 1) == Some(&b'*') {
                    state = SplitState::BlockComment;
                    cursor += 2;
                    continue;
                }

                if sql[cursor] == b'\'' {
                    state = SplitState::SingleQuote;
                    cursor += 1;
                    continue;
                }

                if sql[cursor] == b'"' {
                    state = SplitState::DoubleQuote;
                    cursor += 1;
                    continue;
                }

                if sql[cursor] == b'`' {
                    state = SplitState::Backtick;
                    cursor += 1;
                    continue;
                }

                if sql[cursor] == b'$' {
                    if let Some(dollar_len) = dollar_quote_delimiter_at(sql, cursor) {
                        state = SplitState::Dollar(sql[cursor..cursor + dollar_len].to_vec());
                        cursor += dollar_len;
                        continue;
                    }
                }

                if sql[cursor] == b';' {
                    let statement = core::str::from_utf8(&sql[start..cursor])
                        .expect("SQL fixture bytes must be UTF-8")
                        .trim();
                    if !statement.is_empty() {
                        statements.push(statement.to_owned());
                    }
                    cursor += 1;
                    start = cursor;
                    continue;
                }

                cursor += 1;
            }

            SplitState::LineComment => {
                if sql[cursor] == b'\n' {
                    state = SplitState::Normal;
                }
                cursor += 1;
            }

            SplitState::BlockComment => {
                if sql[cursor] == b'*' && sql.get(cursor + 1) == Some(&b'/') {
                    state = SplitState::Normal;
                    cursor += 2;
                    continue;
                }
                cursor += 1;
            }

            SplitState::SingleQuote => {
                if sql[cursor] == b'\'' {
                    if sql.get(cursor + 1) == Some(&b'\'') {
                        cursor += 2;
                        continue;
                    }
                    state = SplitState::Normal;
                    cursor += 1;
                    continue;
                }
                cursor += 1;
            }

            SplitState::DoubleQuote => {
                if sql[cursor] == b'"' {
                    if sql.get(cursor + 1) == Some(&b'"') {
                        cursor += 2;
                        continue;
                    }
                    state = SplitState::Normal;
                    cursor += 1;
                    continue;
                }
                cursor += 1;
            }

            SplitState::Backtick => {
                if sql[cursor] == b'`' {
                    if sql.get(cursor + 1) == Some(&b'`') {
                        cursor += 2;
                        continue;
                    }
                    state = SplitState::Normal;
                    cursor += 1;
                    continue;
                }
                cursor += 1;
            }

            SplitState::Dollar(ref delimiter) => {
                let delimiter = delimiter.clone();
                if sql[cursor..].starts_with(&delimiter[..]) {
                    state = SplitState::Normal;
                    cursor += delimiter.len();
                    continue;
                }
                cursor += 1;
            }
        }
    }

    let final_statement = core::str::from_utf8(&sql[start..])
        .expect("SQL fixture bytes must be UTF-8")
        .trim();
    if !final_statement.is_empty() {
        statements.push(final_statement.to_owned());
    }

    statements
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
                format!(
                    "fixture statement failed under {:?}: {statement} -> {error}",
                    fixture.backend()
                )
            })?;
    }

    #[cfg(feature = "seaorm-postgres")]
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

    #[cfg(not(feature = "seaorm-postgres"))]
    let _ = fixture;

    Ok(())
}

async fn drop_temporary_database(
    admin_url: &str,
    backend: DbBackend,
    database_name: &str,
) -> ImportResult<()> {
    let admin = connect_single_connection(admin_url).await?;

    #[cfg(feature = "seaorm-sqlite")]
    if matches!(backend, DbBackend::Sqlite) {
        return Err("temporary sqlite database cleanup should not connect to admin DB".to_owned());
    }

    #[cfg(feature = "seaorm-postgres")]
    if matches!(backend, DbBackend::Postgres) {
        let statement = format!("DROP DATABASE IF EXISTS \"{database_name}\" WITH (FORCE)");
        admin
            .execute_raw(Statement::from_string(backend, statement))
            .await
            .map_err(|error| {
                format!("fixture database `{database_name}` must be dropped: {error}")
            })?;
        return Ok(());
    }

    #[cfg(feature = "seaorm-mysql")]
    if matches!(backend, DbBackend::MySql) {
        let statement = format!("DROP DATABASE IF EXISTS `{database_name}`");
        admin
            .execute_raw(Statement::from_string(backend, statement))
            .await
            .map_err(|error| {
                format!("fixture database `{database_name}` must be dropped: {error}")
            })?;
        return Ok(());
    }

    Err(format!("temporary fixture database drop is unsupported for backend {backend:?}"))
}

async fn import_temporary_fixture_sqlite(fixture: SeaOrm11Fixture) -> ImportResult<ImportedDatabase> {
    let connection = connect_single_connection("sqlite::memory:").await?;
    execute_fixture_sql(&connection, fixture).await?;
    Ok(ImportedDatabase {
        connection,
        backend: DbBackend::Sqlite,
        admin_url: None,
        database_name: None,
    })
}

async fn import_temporary_fixture_remote(
    fixture: SeaOrm11Fixture,
    admin_url: String,
    database_name: String,
    backend: DbBackend,
    create_sql: String,
) -> ImportResult<ImportedDatabase> {
    let admin = connect_single_connection(&admin_url).await?;
    admin
        .execute_raw(Statement::from_string(backend, create_sql))
        .await
        .map_err(|error| {
            format!("isolated {backend:?} fixture database `{database_name}` must be created: {error}")
        })?;

    let database_url = database_url(&admin_url, &database_name);
    let mut imported: Option<ImportedDatabase> = None;

    let import = AssertUnwindSafe(async {
        let connection = connect_single_connection(&database_url).await?;
        let active = ImportedDatabase {
            connection,
            backend,
            admin_url: Some(admin_url.clone()),
            database_name: Some(database_name.clone()),
        };
        imported = Some(active);
        execute_fixture_sql(&imported.as_ref().expect("fixture import handle must exist").connection, fixture)
            .await?;
        Ok::<_, String>(())
    })
    .catch_unwind()
    .await;

    match import {
        Ok(Ok(())) => imported.ok_or_else(|| {
            "fixture import unexpectedly completed without exposing a cleanup handle".to_owned()
        }),
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
            if let Some(handle) = imported {
                let cleanup = handle.cleanup().await;
                if let Err(cleanup_error) = cleanup {
                    let _ = cleanup_error;
                }
            } else {
                let _ = drop_temporary_database(&admin_url, backend, &database_name).await;
            }

            std::panic::resume_unwind(panic)
        }
    }
}

pub async fn import_fixture(fixture: SeaOrm11Fixture) -> ImportResult<ImportedDatabase> {
    match fixture.backend() {
        DbBackend::Sqlite => import_temporary_fixture_sqlite(fixture).await,

        #[cfg(feature = "seaorm-postgres")]
        DbBackend::Postgres => {
            let admin_url = std::env::var("MAGNETAR_POSTGRES_TEST_URL")
                .map_err(|error| format!("MAGNETAR_POSTGRES_TEST_URL is required: {error}"))?;
            let database_name = random_name("upgrade", &fixture);
            let create_sql = format!("CREATE DATABASE \"{database_name}\"");
            import_temporary_fixture_remote(
                fixture,
                admin_url,
                database_name,
                DbBackend::Postgres,
                create_sql,
            )
            .await
        }
        #[cfg(not(feature = "seaorm-postgres"))]
        DbBackend::Postgres => {
            unreachable!("postgres fixture requires feature `seaorm-postgres`")
        }

        #[cfg(feature = "seaorm-mysql")]
        DbBackend::MySql => {
            let admin_url = std::env::var("MAGNETAR_MYSQL_TEST_URL")
                .map_err(|error| format!("MAGNETAR_MYSQL_TEST_URL is required: {error}"))?;
            let database_name = random_name("upgrade", &fixture);
            let create_sql = format!("CREATE DATABASE `{database_name}`");
            import_temporary_fixture_remote(
                fixture,
                admin_url,
                database_name,
                DbBackend::MySql,
                create_sql,
            )
            .await
        }
        #[cfg(not(feature = "seaorm-mysql"))]
        DbBackend::MySql => {
            unreachable!("mysql fixture requires feature `seaorm-mysql`")
        }

        _ => unreachable!("only sqlite, postgres, and mysql fixtures are supported"),
    }
}

impl ImportedDatabase {
    pub async fn cleanup(self) -> ImportResult<()> {
        let close_error = self
            .connection
            .close()
            .await
            .err()
            .map(|error| format!("fixture database connection could not close cleanly: {error}"));

        let drop_error = match self.backend {
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
                    .expect("fixture cleanup must preserve MySQL database name");
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
            (Some(close_error), Err(drop_error)) => {
                Err(format!("{close_error}; fallback database-drop attempt also ran: {drop_error}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{dollar_quote_delimiter_at, normalized_statements};

    fn legacy_split(sql: &str) -> Vec<String> {
        sql.split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    #[test]
    fn split_ignores_pg_dump_comment_semicolons() {
        let sql =
            "-- Name: app_users; Type: TABLE; Schema: public\nCREATE TABLE public.app_users (id INT);\n-- Name: users_id_seq; Type: SEQUENCE;\nCREATE TABLE public.users_id_seq (id INT);";

        let statements = normalized_statements(sql);
        assert_eq!(statements.len(), 2, "comment semicolons must not split statements");
        assert!(statements[0].contains("CREATE TABLE public.app_users (id INT)"));
        assert!(statements[1].contains("CREATE TABLE public.users_id_seq (id INT)"));
    }

    #[test]
    fn split_fails_to_split_quoted_and_escaped_semicolons() {
        let sql = "INSERT INTO t(v) VALUES ('a;b;c', 'd'';e', \"x;y\", `z;z`);\nUPDATE t SET v='ok';";
        let statements = normalized_statements(sql);
        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("INSERT INTO t(v) VALUES ('a;b;c', 'd'';e', \"x;y\", `z;z`)"));
        assert_eq!(statements[1], "UPDATE t SET v='ok'");
    }

    #[test]
    fn split_ignores_semicolons_in_block_comments() {
        let sql = "CREATE TABLE t (id INT); /* comment with ; should be ignored */\nINSERT INTO t VALUES (1);";
        let statements = normalized_statements(sql);
        assert_eq!(statements.len(), 2);
        assert_eq!(statements[0], "CREATE TABLE t (id INT)");
        assert!(
            statements[1].contains("INSERT INTO t VALUES (1)"),
            "statement after block comment must still execute as a full SQL statement"
        );
    }

    #[test]
    fn split_ignores_backticks_and_dollar_quotes() {
        let sql = "CREATE TABLE `t;a` (id INT)\n;\nDO $$ BEGIN\n  RAISE NOTICE 'a;b';\nEND; $$;\nCREATE TABLE t2 (v text);";
        let statements = normalized_statements(sql);
        assert_eq!(statements.len(), 3);
        assert!(statements[0].contains("CREATE TABLE `t;a` (id INT)"));
        assert!(statements[1].starts_with("DO $$ BEGIN\n  RAISE NOTICE 'a;b';"));
    }

    #[test]
    fn split_supports_tagged_dollar_quotes() {
        let sql = "DO $body$ SELECT 'a;b;c'; RAISE NOTICE '\"x;y\"'; $body$;\nSELECT 1;";
        let statements = normalized_statements(sql);
        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("DO $body$"));
        assert_eq!(statements[1], "SELECT 1");
    }

    #[test]
    fn legacy_splitter_fails_pg_comment_semicolon_case() {
        let sql = "CREATE TABLE public.app_users (id INT);\n-- Name: app_users; Type: TABLE; Schema: public\nINSERT INTO public.app_users VALUES (1);";

        let legacy = legacy_split(sql);
        assert!(legacy.len() > 2, "legacy splitter should split on comment semicolons");
    }

    #[test]
    fn dollar_quote_delimiter_parser_detects_tag_or_empty() {
        assert_eq!(dollar_quote_delimiter_at(b"$$ body $$", 0), Some(2));
        assert_eq!(dollar_quote_delimiter_at(b"$tag$ body $tag$", 0), Some(5));
        assert_eq!(dollar_quote_delimiter_at(b"$bad body", 0), None);
    }
}
