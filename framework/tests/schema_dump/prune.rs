//! PAR-040: pruning the migrations a dump covers.

use crate::cases::{self, COMMENTS, POSTS, USERS};
use std::fs;
use std::path::Path;
use suprnova::SchemaDump;

/// `src/migrations/` as `suprnova make:migration` writes it, with three
/// migrations.
fn migrations(root: &Path) -> std::path::PathBuf {
    let dir = root.join("src/migrations");
    fs::create_dir_all(&dir).expect("mkdir");
    for name in [USERS, POSTS, COMMENTS] {
        fs::write(dir.join(format!("{name}.rs")), "// a migration\n").expect("write");
    }
    fs::write(
        dir.join("mod.rs"),
        format!(
            "pub use sea_orm_migration::prelude::*;\n\nmod {USERS};\nmod {POSTS};\nmod {COMMENTS};\n\npub struct Migrator;\n\n#[async_trait::async_trait]\nimpl MigratorTrait for Migrator {{\n    fn migrations() -> Vec<Box<dyn MigrationTrait>> {{\n        vec![\n            Box::new({USERS}::Migration),\n            Box::new({POSTS}::Migration),\n            Box::new({COMMENTS}::Migration),\n        ]\n    }}\n}}\n"
        ),
    )
    .expect("write mod.rs");
    dir
}

/// A dump whose ledger records the first two migrations.
async fn dump(root: &Path) -> std::path::PathBuf {
    suprnova::use_database_path(root);
    let url = format!("sqlite://{}?mode=rwc", root.join("app.sqlite").display());
    SchemaDump::migrate::<cases::First>(&url, None)
        .await
        .expect("migrate");
    let path = root.join("dump.sql");
    SchemaDump::dump::<cases::First>(&url, &path)
        .await
        .expect("dump");
    path
}

#[tokio::test]
async fn prune_replaces_the_dumped_migrations_and_keeps_the_rest() {
    let _lock = cases::exclusive().await;
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = migrations(root.path());
    let dump = dump(root.path()).await;

    let pruned = SchemaDump::prune(&dir, &dump).expect("prune");
    assert_eq!(pruned, [USERS, POSTS]);

    assert!(!dir.join(format!("{USERS}.rs")).exists());
    assert!(!dir.join(format!("{POSTS}.rs")).exists());
    assert!(
        dir.join(format!("{COMMENTS}.rs")).exists(),
        "a migration the ledger does not record stays"
    );
    let module = fs::read_to_string(dir.join("mod.rs")).expect("read mod.rs");
    for name in [USERS, POSTS] {
        assert!(!module.contains(&format!("mod {name};")), "{module}");
        assert!(
            module.contains(&format!(
                "Box::new(suprnova::PrunedMigration::new(\"{name}\"))"
            )),
            "{module}"
        );
    }
    assert!(module.contains(&format!("mod {COMMENTS};")), "{module}");
    assert!(
        module.contains(&format!("Box::new({COMMENTS}::Migration)")),
        "{module}"
    );
}

#[tokio::test]
async fn a_dump_with_no_ledger_prunes_nothing() {
    let _lock = cases::exclusive().await;
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = migrations(root.path());
    let before = fs::read_to_string(dir.join("mod.rs")).expect("read mod.rs");
    let dump = root.path().join("dump.sql");
    fs::write(&dump, "CREATE TABLE sd_users (id INTEGER PRIMARY KEY);\n").expect("write");

    assert!(SchemaDump::prune(&dir, &dump).expect("prune").is_empty());
    assert_eq!(
        fs::read_to_string(dir.join("mod.rs")).expect("read"),
        before
    );
    for name in [USERS, POSTS, COMMENTS] {
        assert!(dir.join(format!("{name}.rs")).exists());
    }
}

#[tokio::test]
async fn a_failed_dump_prunes_nothing() {
    let _lock = cases::exclusive().await;
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = migrations(root.path());
    let before = fs::read_to_string(dir.join("mod.rs")).expect("read mod.rs");
    let empty = root.path().join("bin");
    fs::create_dir(&empty).expect("mkdir");
    let path_before = std::env::var_os("PATH");
    // SAFETY: every test in this binary holds `cases::exclusive`, so no
    // other test thread reads PATH while it changes.
    unsafe { std::env::set_var("PATH", &empty) };

    SchemaDump::dump_and_prune::<cases::First>(
        "postgres://dumper:hunter2@127.0.0.1:1/absent",
        &root.path().join("dump.sql"),
        &dir,
    )
    .await
    .expect_err("no pg_dump on PATH");
    // SAFETY: as above.
    unsafe { std::env::set_var("PATH", path_before.unwrap_or_default()) };
    assert_eq!(
        fs::read_to_string(dir.join("mod.rs")).expect("read"),
        before
    );
    for name in [USERS, POSTS, COMMENTS] {
        assert!(dir.join(format!("{name}.rs")).exists());
    }
}

/// A recorded migration with no file of its name, and no `PrunedMigration`
/// entry, is one whose `name()` differs from its file: prune refuses and
/// changes nothing rather than leaving it behind.
#[tokio::test]
async fn a_recorded_migration_without_its_file_stops_the_prune() {
    let _lock = cases::exclusive().await;
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = migrations(root.path());
    fs::rename(
        dir.join(format!("{POSTS}.rs")),
        dir.join("m20260101_000002_posts_named_otherwise.rs"),
    )
    .expect("rename");
    let before = fs::read_to_string(dir.join("mod.rs")).expect("read mod.rs");
    let dump = dump(root.path()).await;

    let err = SchemaDump::prune(&dir, &dump).expect_err("a recorded name with no file");
    assert!(err.to_string().contains(POSTS), "{err}");
    assert_eq!(
        fs::read_to_string(dir.join("mod.rs")).expect("read"),
        before
    );
    assert!(
        dir.join(format!("{USERS}.rs")).exists(),
        "nothing was pruned"
    );
}

/// A second prune leaves the names an earlier one kept.
#[tokio::test]
async fn a_second_prune_keeps_the_earlier_names() {
    let _lock = cases::exclusive().await;
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = migrations(root.path());
    let dump = dump(root.path()).await;
    SchemaDump::prune(&dir, &dump).expect("the first prune");
    let after_first = fs::read_to_string(dir.join("mod.rs")).expect("read");

    assert!(
        SchemaDump::prune(&dir, &dump)
            .expect("the second prune")
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(dir.join("mod.rs")).expect("read"),
        after_first
    );
}

/// A migration written as a directory module is pruned with its directory.
#[tokio::test]
async fn a_directory_module_is_pruned_with_its_directory() {
    let _lock = cases::exclusive().await;
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = migrations(root.path());
    fs::remove_file(dir.join(format!("{USERS}.rs"))).expect("remove");
    fs::create_dir(dir.join(USERS)).expect("mkdir");
    fs::write(dir.join(USERS).join("mod.rs"), "// a migration\n").expect("write");
    let dump = dump(root.path()).await;

    assert_eq!(
        SchemaDump::prune(&dir, &dump).expect("prune"),
        [USERS, POSTS]
    );
    assert!(!dir.join(USERS).exists());
}
