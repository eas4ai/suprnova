//! Regression coverage for `suprnova workflow:install` upgrades.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::{Builder, TempDir};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");
const WORKFLOWS_MIGRATION: &str = "m20240101_000003_create_workflows_table";
const WORKFLOW_STEPS_MIGRATION: &str = "m20240101_000004_create_workflow_steps_table";
const NORMALIZE_DATETIME_MIGRATION: &str = "m20260901_000001_normalize_workflow_datetime_columns";

fn project_dir() -> TempDir {
    let root = fixture_root();
    Builder::new()
        .prefix("workflow-install-")
        .tempdir_in(root)
        .expect("create workflow:install fixture")
}

fn fixture_root() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../target"));
    let root = target.join("test-workspaces");
    fs::create_dir_all(&root).expect("create CLI test workspace");
    root
}

fn run_install(root: &Path) -> Output {
    Command::new(BIN)
        .arg("workflow:install")
        .current_dir(root)
        .output()
        .expect("spawn suprnova workflow:install")
}

#[test]
fn workflow_install_preserves_create_migrations_and_registers_datetime_normalizer_once() {
    let project = project_dir();
    let migrations = project.path().join("src/migrations");
    fs::create_dir_all(&migrations).expect("create migrations directory");

    let workflows_path = migrations.join(format!("{WORKFLOWS_MIGRATION}.rs"));
    let steps_path = migrations.join(format!("{WORKFLOW_STEPS_MIGRATION}.rs"));
    let workflows_sentinel = "// sentinel: preserve workflows migration\n";
    let steps_sentinel = "// sentinel: preserve workflow steps migration\n";
    fs::write(&workflows_path, workflows_sentinel).expect("seed workflows migration");
    fs::write(&steps_path, steps_sentinel).expect("seed workflow steps migration");

    let mod_path = migrations.join("mod.rs");
    fs::write(
        &mod_path,
        format!(
            r#"pub use sea_orm_migration::prelude::*;

mod {WORKFLOWS_MIGRATION};
mod {WORKFLOW_STEPS_MIGRATION};

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {{
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {{
        vec![
            Box::new({WORKFLOWS_MIGRATION}::Migration),
            Box::new({WORKFLOW_STEPS_MIGRATION}::Migration),
        ]
    }}
}}
"#
        ),
    )
    .expect("seed migrations module");

    let first = run_install(project.path());
    assert!(
        first.status.success(),
        "first workflow:install failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    let canonical_entry =
        format!("            Box::new({NORMALIZE_DATETIME_MIGRATION}::Migration),");
    let reindented_entry = format!("\tBox::new({NORMALIZE_DATETIME_MIGRATION}::Migration),");
    let module_after_first = fs::read_to_string(&mod_path).expect("read first module update");
    assert!(
        module_after_first.contains(&canonical_entry),
        "first install must register the normalizer:\n{module_after_first}"
    );
    fs::write(
        &mod_path,
        module_after_first.replace(&canonical_entry, &reindented_entry),
    )
    .expect("reindent normalizer entry");

    let second = run_install(project.path());
    assert!(
        second.status.success(),
        "second workflow:install failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr),
    );

    assert_eq!(
        fs::read_to_string(&workflows_path).expect("read workflows migration"),
        workflows_sentinel,
        "workflow:install must not replace an existing workflows migration"
    );
    assert_eq!(
        fs::read_to_string(&steps_path).expect("read workflow steps migration"),
        steps_sentinel,
        "workflow:install must not replace an existing workflow steps migration"
    );

    let normalizer_path = migrations.join(format!("{NORMALIZE_DATETIME_MIGRATION}.rs"));
    assert!(
        normalizer_path.is_file(),
        "workflow:install must add {}",
        normalizer_path.display()
    );
    let normalizer = fs::read_to_string(&normalizer_path).expect("read datetime normalizer");
    assert!(
        normalizer.contains("use sea_orm::{ConnectionTrait, DbBackend};"),
        "generated normalizer must import every SeaORM type it names:\n{normalizer}"
    );

    let module = fs::read_to_string(&mod_path).expect("read migrations module");
    let declaration = format!("mod {NORMALIZE_DATETIME_MIGRATION};");
    let entry = format!("Box::new({NORMALIZE_DATETIME_MIGRATION}::Migration),");
    assert_eq!(
        module.matches(&declaration).count(),
        1,
        "normalizer module declaration must occur exactly once:\n{module}"
    );
    assert_eq!(
        module.matches(&entry).count(),
        1,
        "normalizer migrator entry must occur exactly once:\n{module}"
    );
}
