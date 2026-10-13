//! Postgres, MySQL and MariaDB, each against a throwaway database its URL
//! names. Ignored by default; the `par-schema-dump` mechanism runs them.

use crate::cases;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The URL in `var` with `tls` added to its query; a run that asks for
/// these tests must provide the URL.
fn url(var: &str, tls: &str) -> String {
    let url = std::env::var(var).unwrap_or_else(|_| panic!("{var} must name a throwaway database"));
    let joiner = if url.contains('?') { '&' } else { '?' };
    format!("{url}{joiner}{tls}")
}

/// PATH and HOME as they were, put back on drop.
struct Environment {
    path: Option<OsString>,
    home: Option<OsString>,
}

impl Drop for Environment {
    fn drop(&mut self) {
        // SAFETY: every test in this binary holds `cases::exclusive`, so no
        // other test thread reads the environment while it changes.
        unsafe {
            match &self.path {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
            match &self.home {
                Some(home) => std::env::set_var("HOME", home),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}

/// Puts a wrapper for each tool first on PATH that appends its arguments
/// to `args.log` and its `PG*` environment to `env.log`, then runs the
/// real tool; and points HOME at a directory whose `.my.cnf` holds a wrong
/// password, which the tools must not prefer to the URL's.
fn record_tools(dir: &Path, tools: &[&str]) -> (Environment, PathBuf, PathBuf) {
    let saved = Environment {
        path: std::env::var_os("PATH"),
        home: std::env::var_os("HOME"),
    };
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let args = dir.join("args.log");
    let env = dir.join("env.log");
    for tool in tools {
        let real = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {tool}"))
            .output()
            .expect("look up the tool");
        let real = String::from_utf8(real.stdout)
            .expect("a path")
            .trim()
            .to_owned();
        assert!(!real.is_empty(), "{tool} must be installed");
        let wrapper = bin.join(tool);
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\nenv | grep '^PG' >> '{}'\nexec '{real}' \"$@\"\n",
                args.display(),
                env.display()
            ),
        )
        .expect("write the wrapper");
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("mkdir");
    std::fs::write(
        home.join(".my.cnf"),
        "[client]\npassword=not-the-password\n",
    )
    .expect("write .my.cnf");
    let path = format!(
        "{}:{}",
        bin.display(),
        saved.path.clone().unwrap_or_default().to_string_lossy()
    );
    // SAFETY: every test in this binary holds `cases::exclusive`, so no
    // other test thread reads the environment while it changes.
    unsafe {
        std::env::set_var("PATH", path);
        std::env::set_var("HOME", &home);
    }
    (saved, args, env)
}

async fn every_case(url: &str, engine: &str, tools: &[&str]) {
    let _lock = cases::exclusive().await;
    let dir = tempfile::tempdir().expect("a temporary directory");
    suprnova::use_database_path(dir.path());
    let (_environment, args, env) = record_tools(dir.path(), tools);

    cases::dump_holds_the_schema_and_the_ledger(url, dir.path(), engine).await;
    let password = url::Url::parse(url)
        .expect("a URL")
        .password()
        .map(str::to_owned)
        .expect("the test URL has a password");
    let arguments = std::fs::read_to_string(&args).expect("the tool ran");
    assert!(
        !arguments.contains(&password),
        "the password stays out of the tools' arguments"
    );
    if engine == "postgres" {
        let environment = std::fs::read_to_string(&env).expect("the tool ran");
        assert!(
            environment.contains("PGSSLMODE=require"),
            "the URL's sslmode reaches pg_dump: {environment}"
        );
    } else {
        assert!(
            arguments.lines().any(|a| a.starts_with("--ssl")),
            "the URL's ssl-mode reaches the tool: {arguments}"
        );
    }

    cases::the_dump_loads_before_newer_migrations(url, dir.path()).await;
    cases::only_an_empty_ledger_loads(url, dir.path()).await;
    cases::pruned_migrations_keep_their_names(url, dir.path()).await;
    cases::tables_without_a_ledger_are_not_loaded_over(url, dir.path()).await;
    cases::a_missing_schema_path_is_an_error(url, dir.path()).await;
    cases::fresh_reloads_views_and_routines(url, dir.path(), engine).await;
}

#[tokio::test]
#[ignore = "needs PG_TEST_URL"]
async fn postgres_dumps_loads_and_prunes() {
    every_case(
        &url("PG_TEST_URL", "sslmode=require"),
        "postgres",
        &["pg_dump", "psql"],
    )
    .await;
}

#[tokio::test]
#[ignore = "needs MYSQL_TEST_URL, a MySQL server"]
async fn mysql_dumps_loads_and_prunes() {
    every_case(
        &url("MYSQL_TEST_URL", "ssl-mode=REQUIRED"),
        "mysql",
        &["mysqldump", "mysql"],
    )
    .await;
}

#[tokio::test]
#[ignore = "needs MARIADB_TEST_URL, a MariaDB server"]
async fn mariadb_dumps_loads_and_prunes() {
    every_case(
        &url("MARIADB_TEST_URL", "ssl-mode=REQUIRED"),
        "mariadb",
        &["mariadb-dump", "mariadb"],
    )
    .await;
}

/// The URL in `var` read the way SQLx reads it but spelled differently: the
/// password moved to a `password=` query parameter that SQLx applies after
/// a stale one left in the authority, and `query` appended. The framework
/// connects with it, so the tools must too.
fn respelled_url(var: &str, query: &str) -> String {
    let mut url = url::Url::parse(
        &std::env::var(var).unwrap_or_else(|_| panic!("{var} must name a throwaway database")),
    )
    .expect("a URL");
    let password = url
        .password()
        .map(|password| {
            percent_encoding::percent_decode_str(password)
                .decode_utf8_lossy()
                .into_owned()
        })
        .expect("the test URL has a password");
    url.set_password(Some("not-the-password"))
        .expect("the URL takes a password");
    url.query_pairs_mut().append_pair("password", &password);
    let joiner = if url.query().is_some() { '&' } else { '?' };
    format!("{url}{joiner}{query}")
}

#[tokio::test]
#[ignore = "needs PG_TEST_URL"]
async fn postgres_tools_read_the_url_like_the_connection() {
    let url = respelled_url("PG_TEST_URL", "ssl-mode=require");
    let _lock = cases::exclusive().await;
    let dir = tempfile::tempdir().expect("a temporary directory");
    suprnova::use_database_path(dir.path());
    let (_environment, _args, env) = record_tools(dir.path(), &["pg_dump", "psql"]);

    // The connection reads `password=` over the stale authority password and
    // `ssl-mode` as `sslmode`; pg_dump has to reach the same server the
    // same way, or the dump fails to authenticate.
    cases::dump_holds_the_schema_and_the_ledger(&url, dir.path(), "postgres").await;
    let environment = std::fs::read_to_string(&env).expect("the tool ran");
    assert!(
        environment.contains("PGSSLMODE=require"),
        "the URL's ssl-mode reaches pg_dump: {environment}"
    );
}

#[tokio::test]
#[ignore = "needs MYSQL_TEST_URL, a MySQL server"]
async fn mysql_tools_read_the_unhyphenated_tls_spelling() {
    let url = url("MYSQL_TEST_URL", "sslmode=REQUIRED");
    let _lock = cases::exclusive().await;
    let dir = tempfile::tempdir().expect("a temporary directory");
    suprnova::use_database_path(dir.path());
    let (_environment, args, _env) = record_tools(dir.path(), &["mysqldump", "mysql"]);

    cases::dump_holds_the_schema_and_the_ledger(&url, dir.path(), "mysql").await;
    // A MySQL client takes the mode as `--ssl-mode`; a MariaDB client,
    // which may answer to the same name, as `--ssl`.
    let arguments = std::fs::read_to_string(&args).expect("the tool ran");
    assert!(
        arguments
            .lines()
            .any(|a| a == "--ssl-mode=REQUIRED" || a == "--ssl"),
        "the URL's sslmode reaches the tool: {arguments}"
    );
}
