mod fixtures;

#[test]
fn manifest_owns_every_required_fixture_and_checksum() {
    use std::collections::BTreeSet;

    let manifest = fixtures::manifest();
    assert!(manifest.contains("\"schema_version\": 1"));
    assert!(manifest.contains("suprnova-framework-bcrypt"));
    assert!(manifest.contains("torii-password-auth-argon2"));
    assert!(manifest.contains("\"torii\""));
    assert!(manifest.contains("suprnova-web"));
    assert!(manifest.contains("suprnova-api"));
    for (id, backend, fixture_path) in [
        (
            "seaorm-1-1-sqlite",
            "sqlite",
            "tests/fixtures/seaorm_1_1/sqlite.sql",
        ),
        (
            "seaorm-1-1-postgres",
            "postgres",
            "tests/fixtures/seaorm_1_1/postgres.sql",
        ),
        (
            "seaorm-1-1-mysql",
            "mysql",
            "tests/fixtures/seaorm_1_1/mysql.sql",
        ),
    ] {
        let line = manifest
            .lines()
            .find(|line| line.contains(&format!("\"id\":\"{id}\"")))
            .expect("SeaORM 1.1 fixture must have a manifest record");
        assert!(line.contains("\"source_commit\":\"11af547c\""));
        assert!(line.contains("\"sea_orm\":\"1.1.20\""));
        assert!(line.contains(&format!("\"backend\":\"{backend}\"")));
        assert_eq!(
            fixtures::json_string(line, "fixture_path").as_deref(),
            Some(fixture_path)
        );
        assert_eq!(
            fixtures::json_string(line, "canonical_catalog_sha256")
                .expect("SeaORM 1.1 fixture must record its catalog digest")
                .len(),
            64
        );

        let fixture = std::fs::read_to_string(fixtures::repository_path(fixture_path))
            .expect("SeaORM 1.1 fixture must be readable");
        for representative in [
            "seaorm11@example.test",
            "seaorm11-session",
            "password-reset",
        ] {
            assert!(
                fixture.contains(representative),
                "{id} fixture is missing representative row value {representative}"
            );
        }
    }
    for label in [
        "mixed_case_email_collision",
        "passwordless_user",
        "passkey_envelope",
        "linked_account",
        "verification_timestamp",
        "two_factor_secret_ciphertext",
        "session",
        "app_owned_i64_foreign_key",
    ] {
        assert!(
            manifest.contains(label),
            "manifest is missing shape label {label}"
        );
    }

    let mut checked = 0;
    for line in manifest.lines() {
        if !line.contains("\"status\":\"generated\"") {
            continue;
        }
        let path = fixtures::json_string(line, "fixture_path")
            .expect("every generated fixture must have a path");
        assert!(
            path.starts_with("tests/fixtures/"),
            "generated fixture path must stay under tests/fixtures: {path}"
        );
        let fixture = fixtures::repository_path(&path);
        assert!(
            fixture.is_file(),
            "manifest-owned fixture is missing: {path}"
        );
        let checksum =
            fixtures::json_string(line, "sha256").expect("generated fixture checksum is quoted");
        assert_eq!(
            checksum.len(),
            64,
            "fixture checksum must be SHA-256: {path}"
        );
        assert_eq!(
            fixtures::sha256(&path),
            checksum,
            "fixture checksum drifted: {path}"
        );
        checked += 1;
    }

    let generated_paths: BTreeSet<_> = fixtures::manifest_generated_fixture_paths(&manifest)
        .into_iter()
        .collect();
    let generated_hash_paths: BTreeSet<_> = generated_paths
        .iter()
        .filter(|path| path.starts_with("tests/fixtures/hashes/"))
        .cloned()
        .collect();
    let hash_files: BTreeSet<_> = fixtures::files_under("tests/fixtures/hashes")
        .into_iter()
        .collect();
    assert_eq!(
        hash_files, generated_hash_paths,
        "every generated hash file must be manifest-owned and no extra hash file may pass"
    );
    assert_eq!(
        checked,
        generated_paths.len(),
        "every generated manifest record must have a path and checksum"
    );

    for path in fixtures::files_under("tests/fixtures") {
        if path == fixtures::MANIFEST
            || path == "tests/fixtures/mod.rs"
            || path == "tests/fixtures/databases/.gitkeep"
            || path == "tests/fixtures/README.md"
            || path.ends_with(".rs")
        {
            continue;
        }
        assert!(
            generated_paths.contains(&path),
            "fixture file is not manifest-owned: {path}"
        );
    }

    assert!(
        fixtures::repository_path("tests/fixtures/databases").is_dir(),
        "generated database fixture directory must exist"
    );
    let database_files: BTreeSet<_> = fixtures::files_under("tests/fixtures/databases")
        .into_iter()
        .filter(|path| path != "tests/fixtures/databases/.gitkeep")
        .collect();
    let generated_database_paths: BTreeSet<_> = generated_paths
        .iter()
        .filter(|path| path.starts_with("tests/fixtures/databases/"))
        .cloned()
        .collect();
    assert_eq!(
        database_files, generated_database_paths,
        "every generated database must be manifest-owned and no extra database file may pass"
    );

    for (id, fixture_path) in [
        ("torii", "tests/fixtures/databases/torii.sqlite"),
        (
            "suprnova-web",
            "tests/fixtures/databases/suprnova-web.sqlite",
        ),
        (
            "suprnova-api",
            "tests/fixtures/databases/suprnova-api.sqlite",
        ),
    ] {
        let fixture_line = manifest
            .lines()
            .find(|line| line.contains(&format!("\"id\":\"{id}\"")))
            .expect("database fixture must have a manifest record");
        assert!(
            fixture_line.contains("\"kind\":\"database\",\"fixture_path\"")
                && fixture_line.contains("\"status\":\"generated\""),
            "database fixture must be generated: {id}"
        );
        assert_eq!(
            fixtures::json_string(fixture_line, "fixture_path").as_deref(),
            Some(fixture_path),
            "database fixture path drifted: {id}"
        );
    }
}
