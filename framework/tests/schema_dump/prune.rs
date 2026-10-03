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
    let root = tempfile::tempdir().expect("a temporary directory");
    let dir = migrations(root.path());
    let before = fs::read_to_string(dir.join("mod.rs")).expect("read mod.rs");
    let empty = root.path().join("bin");
    fs::create_dir(&empty).expect("mkdir");
    // SAFETY: nextest runs each test in its own process, so no other
    // thread reads PATH while it changes.
    unsafe { std::env::set_var("PATH", &empty) };

    SchemaDump::dump_and_prune::<cases::First>(
        "postgres://dumper:hunter2@127.0.0.1:1/absent",
        &root.path().join("dump.sql"),
        &dir,
    )
    .await
    .expect_err("no pg_dump on PATH");
    assert_eq!(
        fs::read_to_string(dir.join("mod.rs")).expect("read"),
        before
    );
    for name in [USERS, POSTS, COMMENTS] {
        assert!(dir.join(format!("{name}.rs")).exists());
    }
}
