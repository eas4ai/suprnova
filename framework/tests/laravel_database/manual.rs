//! LDB-012: the manual's chapter on running on a Laravel database, and the
//! scaffold's comment on `unsigned_ids`.

use std::path::PathBuf;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the framework sits in the repository")
        .to_path_buf()
}

fn read(path: &str) -> String {
    let path = repository().join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The chapter's text with line breaks folded to spaces, so a phrase
/// matches wherever the prose wraps.
fn folded(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn ldb_012_the_chapter_is_linked_and_has_the_manuals_shape() {
    let index = read("manual/documentation.md");
    assert!(
        index.contains("(laravel-database.md)"),
        "manual/documentation.md does not link the chapter"
    );
    let chapter = read("manual/laravel-database.md");
    assert!(chapter.contains("```rust"), "the chapter has no runnable example");
    assert!(chapter.contains("\n### Why Suprnova diverges\n"));
    let last_section = chapter
        .lines()
        .rfind(|line| line.starts_with("## "))
        .expect("a section");
    assert_eq!(last_section, "## Next", "the chapter does not close with ## Next");
}

#[test]
fn ldb_012_the_chapter_names_every_table_and_its_layout() {
    let chapter = read("manual/laravel-database.md");
    for (table, layout) in [
        ("`jobs`", "Laravel's `jobs`"),
        ("`job_batches`", "Laravel's `job_batches`"),
        ("`failed_jobs`", "Laravel's `failed_jobs`"),
        ("`sessions`", "the Laravel 13 skeleton's `sessions`"),
        ("`notifications`", "Laravel's `notifications`"),
        ("`users`", "the Laravel 13 skeleton's `users`"),
        ("`features`", "laravel/pennant's `features`"),
        ("`roles`, `permissions`", "spatie/laravel-permission's"),
    ] {
        let row = chapter
            .lines()
            .find(|line| line.starts_with(&format!("| {table} |")))
            .unwrap_or_else(|| panic!("the chapter's table has no row for {table}"));
        assert!(row.contains(layout), "{table} is not given {layout}: {row}");
    }
}

#[test]
fn ldb_012_the_chapter_covers_the_upgrade_setting_and_aliases() {
    let chapter = folded(&read("manual/laravel-database.md"));
    for phrase in [
        // The upgrade of LDB-010 and what it moves.
        "## Upgrading an existing Suprnova application",
        "A queued job stays queued, a delayed job stays delayed, and a reserved job stays reserved",
        "each batch keeps its total, pending and failed counts",
        "Every session keeps its id, its user, its data and its CSRF token",
        "with every notification, flag, role, permission and assignment",
        "This migration moves an earlier scaffold's `users`",
        // The setting of LDB-004 and what it gives up.
        "LARAVEL_SHARED_DATABASE=true",
        "The setting gives up two things",
        "Argon2id",
        // Morph aliases.
        "morph_aliases = [\"post\"]",
        // Unsigned keys.
        "unsigned_ids = true",
        // Laravel's queue:retry.
        "`queue:retry all` stops at the first failed job Suprnova wrote",
        "`queue:retry --queue=<queue>` with a queue only Laravel uses, and Laravel retries its own failed jobs",
        // What is not supported.
        "Sanctum tokens Laravel issued",
        "Columns Laravel's encrypter wrote, such as Fortify's two-factor secrets and recovery codes, and any column with an `encrypted` cast",
        "Remember-me cookies Laravel issued",
        "whose time zone is not UTC",
    ] {
        assert!(chapter.contains(phrase), "the chapter does not say: {phrase}");
    }
}

/// The `users` migration the chapter gives is the one `upgrade.rs` runs on
/// every engine.
#[test]
fn ldb_012_the_chapters_users_migration_is_the_tested_one() {
    let chapter = read("manual/laravel-database.md");
    let tested = read("framework/tests/laravel_database/users_upgrade.rs");
    assert!(
        chapter.contains(tested.trim_end()),
        "the chapter's users migration differs from users_upgrade.rs"
    );
}

#[test]
fn ldb_012_the_scaffolds_unsigned_ids_comment_says_users_id_is_unsigned() {
    let manifest = folded(&read(
        "suprnova-cli/src/templates/files/backend/Cargo.toml.tpl",
    ))
    .replace("# ", "");
    assert!(
        manifest.contains("This scaffold's `users.id` is already unsigned on MySQL"),
        "the scaffold's comment on unsigned_ids does not say users.id is unsigned"
    );
    assert!(manifest.contains("unsigned_ids = true"));
}
