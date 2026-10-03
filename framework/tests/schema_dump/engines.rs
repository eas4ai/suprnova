//! Postgres, MySQL and MariaDB, each against a throwaway database its URL
//! names. Ignored by default; the `par-schema-dump` mechanism runs them.

use crate::cases;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The URL in `var`; a run that asks for these tests must provide it.
fn url(var: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| panic!("{var} must name a throwaway database"))
}

/// Puts a wrapper for `tool` first on PATH that writes its arguments to
/// `args.log` and then runs the real tool, and returns the log's path.
fn record_arguments(dir: &Path, tool: &str) -> std::path::PathBuf {
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
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let log = dir.join("args.log");
    let wrapper = bin.join(tool);
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\nexec '{real}' \"$@\"\n",
            log.display()
        ),
    )
    .expect("write the wrapper");
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    // SAFETY: nextest runs each test in its own process, so no other
    // thread reads PATH while it changes.
    unsafe { std::env::set_var("PATH", path) };
    log
}

async fn every_case(url: &str, engine: &str, tool: &str) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    suprnova::use_database_path(dir.path());
    let log = record_arguments(dir.path(), tool);

    cases::dump_holds_the_schema_and_the_ledger(url, dir.path(), engine).await;
    let password = url::Url::parse(url)
        .expect("a URL")
        .password()
        .map(str::to_owned)
        .expect("the test URL has a password");
    let arguments = std::fs::read_to_string(&log).expect("the tool ran");
    assert!(
        !arguments.contains(&password),
        "the password stays out of {tool}'s arguments"
    );

    cases::the_dump_loads_before_newer_migrations(url, dir.path()).await;
    cases::only_an_empty_ledger_loads(url, dir.path()).await;
    cases::pruned_migrations_keep_their_names(url, dir.path()).await;
}

#[tokio::test]
#[ignore = "needs PG_TEST_URL"]
async fn postgres_dumps_loads_and_prunes() {
    every_case(&url("PG_TEST_URL"), "postgres", "pg_dump").await;
}

#[tokio::test]
#[ignore = "needs MYSQL_TEST_URL, a MySQL server"]
async fn mysql_dumps_loads_and_prunes() {
    every_case(&url("MYSQL_TEST_URL"), "mysql", "mysqldump").await;
}

#[tokio::test]
#[ignore = "needs MARIADB_TEST_URL, a MariaDB server"]
async fn mariadb_dumps_loads_and_prunes() {
    every_case(&url("MARIADB_TEST_URL"), "mariadb", "mariadb-dump").await;
}
