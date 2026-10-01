//! Where `inertia_response!` looks for a page component at compile time.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const ISSUE_LOOKUP: &str = r#"
[package]
name = "app"
version = "0.1.0"

[package.metadata.suprnova.inertia]
pages_dir = "resources/angular/pages"
page_file = "{dir}/{name|lower}.page.ts"
"#;

    fn pattern(text: &str) -> PagePattern {
        match PagePattern::parse(text) {
            Ok(pattern) => pattern,
            Err(problem) => panic!("`{text}` should parse: {problem}"),
        }
    }

    fn pattern_error(text: &str) -> String {
        match PagePattern::parse(text) {
            Ok(_) => panic!("`{text}` should be rejected"),
            Err(problem) => problem,
        }
    }

    fn lookup(manifest: &str) -> PageLookup {
        match PageLookup::from_manifest(manifest) {
            Ok(Some(lookup)) => lookup,
            Ok(None) => panic!("the manifest should configure a lookup"),
            Err(problem) => panic!("the manifest should be accepted: {problem}"),
        }
    }

    fn manifest_error(manifest: &str) -> String {
        match PageLookup::from_manifest(manifest) {
            Ok(_) => panic!("the manifest should be rejected"),
            Err(problem) => problem,
        }
    }

    fn with_table(body: &str) -> String {
        format!("[package]\nname = \"app\"\n\n[package.metadata.suprnova.inertia]\n{body}\n")
    }

    // ---- page_file patterns ----

    #[test]
    fn issue_pattern_maps_nested_components_to_lowercase_files() {
        let pattern = pattern("{dir}/{name|lower}.page.ts");
        assert_eq!(
            pattern.render("Tramits/BaixaMatricula/Create"),
            "Tramits/BaixaMatricula/create.page.ts"
        );
        assert_eq!(pattern.render("Tramits/Index"), "Tramits/index.page.ts");
    }

    #[test]
    fn top_level_component_leaves_no_leading_slash() {
        let pattern = pattern("{dir}/{name|lower}.page.ts");
        assert_eq!(pattern.render("Home"), "home.page.ts");
    }

    #[test]
    fn placeholders_without_filters_keep_the_component_spelling() {
        let pattern = pattern("{dir}/{name}/index.tsx");
        assert_eq!(pattern.render("Admin/Users"), "Admin/Users/index.tsx");
        assert_eq!(pattern.render("Home"), "Home/index.tsx");
    }

    #[test]
    fn kebab_and_snake_split_words_at_case_changes() {
        assert_eq!(
            pattern("{name|kebab}.ts").render("BaixaMatricula"),
            "baixa-matricula.ts"
        );
        assert_eq!(
            pattern("{name|snake}.ts").render("BaixaMatricula"),
            "baixa_matricula.ts"
        );
        assert_eq!(
            pattern("{name|kebab}.ts").render("HTMLReport"),
            "html-report.ts"
        );
        assert_eq!(
            pattern("{name|snake}.ts").render("Create2FA"),
            "create2_fa.ts"
        );
        assert_eq!(
            pattern("{name|kebab}.ts").render("already_snake-or-kebab"),
            "already-snake-or-kebab.ts"
        );
        assert_eq!(pattern("{name|snake}.ts").render("Index"), "index.ts");
    }

    #[test]
    fn filters_on_dir_apply_to_every_segment() {
        let pattern = pattern("{dir|kebab}/{name|snake}.component.ts");
        assert_eq!(
            pattern.render("AdminArea/UserProfile/EditForm"),
            "admin-area/user-profile/edit_form.component.ts"
        );
        assert_eq!(
            self::pattern("{dir|lower}/{name}.vue").render("Admin/Reports/Index"),
            "admin/reports/Index.vue"
        );
        assert_eq!(
            self::pattern("{dir|snake}/{name}.vue").render("Index"),
            "Index.vue"
        );
    }

    #[test]
    fn a_placeholder_may_repeat() {
        let pattern = pattern("{dir}/{name}/{name|kebab}.page.ts");
        assert_eq!(
            pattern.render("Users/EditProfile"),
            "Users/EditProfile/edit-profile.page.ts"
        );
    }

    #[test]
    fn malformed_patterns_name_the_key_and_the_problem() {
        let cases: &[(&str, &[&str])] = &[
            ("", &["must not be empty"]),
            ("{dir}/{name.page.ts", &["unclosed `{`"]),
            ("{dir}/name}.page.ts", &["`}`", "no opening `{`"]),
            ("{}/{name}.ts", &["empty placeholder `{}`"]),
            ("{file}.ts", &["unknown placeholder `{file}`", "`{dir}`", "`{name}`"]),
            (
                "{name|upper}.ts",
                &["unknown filter `upper`", "`{name|upper}`", "lower", "kebab", "snake"],
            ),
            ("{name|}.ts", &["empty filter", "`{name|}`"]),
            ("{name|lower|kebab}.ts", &["one filter", "`{name|lower|kebab}`"]),
            ("{dir}/index.page.ts", &["must contain `{name}`"]),
            ("/{name}.ts", &["relative"]),
        ];
        for (text, expected) in cases {
            let problem = pattern_error(text);
            assert!(
                problem.contains("`page_file`"),
                "`{text}`: the error must name the key, got: {problem}"
            );
            for fragment in *expected {
                assert!(
                    problem.contains(fragment),
                    "`{text}`: expected `{fragment}` in: {problem}"
                );
            }
        }
    }

    // ---- the manifest table ----

    #[test]
    fn manifest_without_the_table_keeps_the_starter_lookup() {
        assert_eq!(
            PageLookup::from_manifest("[package]\nname = \"app\"\n"),
            Ok(None)
        );
        assert_eq!(
            PageLookup::from_manifest(
                "[package]\nname = \"app\"\n\n[package.metadata.docs.rs]\nall-features = true\n"
            ),
            Ok(None)
        );
        assert_eq!(
            PageLookup::from_manifest(
                "[package]\nname = \"app\"\n\n[package.metadata.suprnova.other]\nkey = 1\n"
            ),
            Ok(None)
        );
    }

    #[test]
    fn starter_lookup_tries_the_four_extensions_under_frontend_src_pages() {
        assert_eq!(
            PageLookup::starter().candidates("Users/Index"),
            vec![
                "frontend/src/pages/Users/Index.svelte",
                "frontend/src/pages/Users/Index.tsx",
                "frontend/src/pages/Users/Index.jsx",
                "frontend/src/pages/Users/Index.vue",
            ]
        );
    }

    #[test]
    fn issue_table_resolves_one_file_per_component() {
        let lookup = lookup(ISSUE_LOOKUP);
        assert_eq!(
            lookup.candidates("Tramits/BaixaMatricula/Create"),
            vec!["resources/angular/pages/Tramits/BaixaMatricula/create.page.ts"]
        );
        assert_eq!(
            lookup.candidates("Tramits/Index"),
            vec!["resources/angular/pages/Tramits/index.page.ts"]
        );
    }

    #[test]
    fn pages_dir_alone_keeps_the_starter_extensions() {
        let lookup = lookup(&with_table("pages_dir = \"resources/js/Pages/\""));
        assert_eq!(
            lookup.candidates("Admin/Dashboard"),
            vec![
                "resources/js/Pages/Admin/Dashboard.svelte",
                "resources/js/Pages/Admin/Dashboard.tsx",
                "resources/js/Pages/Admin/Dashboard.jsx",
                "resources/js/Pages/Admin/Dashboard.vue",
            ]
        );
    }

    #[test]
    fn page_file_alone_uses_the_starter_directory() {
        let lookup = lookup(&with_table("page_file = \"{dir}/{name|lower}.page.ts\""));
        assert_eq!(
            lookup.candidates("Users/Index"),
            vec!["frontend/src/pages/Users/index.page.ts"]
        );
    }

    #[test]
    fn an_empty_table_resolves_like_the_starter_lookup() {
        let lookup = lookup(&with_table(""));
        assert_eq!(
            lookup.candidates("Home"),
            PageLookup::starter().candidates("Home")
        );
    }

    #[test]
    fn malformed_tables_name_the_key_and_the_problem() {
        let cases: Vec<(String, &[&str])> = vec![
            (
                with_table("pagesdir = \"x\""),
                &["unknown key `pagesdir`", "`pages_dir`", "`page_file`"],
            ),
            (with_table("pages_dir = 3"), &["`pages_dir`", "must be a string"]),
            (with_table("page_file = true"), &["`page_file`", "must be a string"]),
            (with_table("pages_dir = \"\""), &["`pages_dir`", "must not be empty"]),
            (
                with_table("pages_dir = \"/srv/pages\""),
                &["`pages_dir`", "relative to the crate directory"],
            ),
            (
                with_table("page_file = \"{dir}/{name|upper}.page.ts\""),
                &["`page_file`", "unknown filter `upper`"],
            ),
            (
                "[package]\nname = \"app\"\n\n[package.metadata.suprnova]\ninertia = \"pages\"\n"
                    .to_string(),
                &["`package.metadata.suprnova.inertia`", "must be a table"],
            ),
            (
                "[package]\nname = \"app\"\n\n[package.metadata]\nsuprnova = 1\n".to_string(),
                &["`package.metadata.suprnova`", "must be a table"],
            ),
            ("[package\nname = ".to_string(), &["could not parse"]),
        ];
        for (manifest, expected) in cases {
            let problem = manifest_error(&manifest);
            for fragment in expected {
                assert!(
                    problem.contains(fragment),
                    "expected `{fragment}` in: {problem}\nfor manifest:\n{manifest}"
                );
            }
        }
    }

    // ---- error messages ----

    #[test]
    fn starter_message_is_unchanged_with_suggestions() {
        let available = vec!["Dashboard".to_string(), "Home".to_string()];
        assert_eq!(
            starter_not_found_message("Hom", &available),
            "Inertia component 'Hom' not found.\n\
             Looked in: frontend/src/pages/\n\
             Tried extensions: .svelte, .tsx, .jsx, .vue\n\
             \n\
             Available components:\n  - Dashboard\n  - Home\n\
             \n\
             Did you mean 'Home'?"
        );
    }

    #[test]
    fn starter_message_is_unchanged_without_components() {
        assert_eq!(
            starter_not_found_message("Home", &[]),
            "Inertia component 'Home' not found.\n\
             Looked in: frontend/src/pages/\n\
             Tried extensions: .svelte, .tsx, .jsx, .vue\n\
             \n\
             No components found in frontend/src/pages/.\n\
             Make sure your frontend directory structure is set up correctly."
        );
    }

    #[test]
    fn configured_message_names_the_resolved_path() {
        let lookup = lookup(ISSUE_LOOKUP);
        let available = vec![
            "Tramits/BaixaMatricula/create.page.ts".to_string(),
            "Tramits/index.page.ts".to_string(),
        ];
        let message = lookup.not_found_message("Tramits/BaixaMatricula/Crate", &available);
        assert!(
            message.starts_with("Inertia component 'Tramits/BaixaMatricula/Crate' not found.\n"),
            "{message}"
        );
        assert!(
            message.contains(
                "Looked for: resources/angular/pages/Tramits/BaixaMatricula/crate.page.ts"
            ),
            "{message}"
        );
        assert!(
            message.contains("[package.metadata.suprnova.inertia] in Cargo.toml"),
            "{message}"
        );
        assert!(
            message.contains(
                "Page files under resources/angular/pages/:\n  - Tramits/BaixaMatricula/create.page.ts\n  - Tramits/index.page.ts"
            ),
            "{message}"
        );
        assert!(
            message.contains("Did you mean 'Tramits/BaixaMatricula/create.page.ts'?"),
            "{message}"
        );
        assert!(!message.contains("frontend/src/pages"), "{message}");
    }

    #[test]
    fn configured_message_without_pages_says_so() {
        let lookup = lookup(ISSUE_LOOKUP);
        let message = lookup.not_found_message("Home", &[]);
        assert!(
            message.contains("Looked for: resources/angular/pages/home.page.ts"),
            "{message}"
        );
        assert!(
            message.contains("No page files found under resources/angular/pages/."),
            "{message}"
        );
        assert!(!message.contains("Did you mean"), "{message}");
    }

    #[test]
    fn configured_message_for_pages_dir_lists_every_candidate() {
        let lookup = lookup(&with_table("pages_dir = \"resources/js/Pages\""));
        let message = lookup.not_found_message("Admin/Dashbord", &["Admin/Dashboard".to_string()]);
        assert!(
            message.contains(
                "Looked for: resources/js/Pages/Admin/Dashbord.svelte, \
                 resources/js/Pages/Admin/Dashbord.tsx, \
                 resources/js/Pages/Admin/Dashbord.jsx, \
                 resources/js/Pages/Admin/Dashbord.vue"
            ),
            "{message}"
        );
        assert!(
            message.contains("Available components:\n  - Admin/Dashboard"),
            "{message}"
        );
        assert!(message.contains("Did you mean 'Admin/Dashboard'?"), "{message}");
    }

    // ---- the filesystem ----

    fn crate_dir(files: &[&str]) -> tempfile::TempDir {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(error) => panic!("create a scratch crate directory: {error}"),
        };
        for file in files {
            let path = dir.path().join(file);
            if let Some(parent) = path.parent()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                panic!("create {}: {error}", parent.display());
            }
            if let Err(error) = std::fs::write(&path, "export {};\n") {
                panic!("write {}: {error}", path.display());
            }
        }
        dir
    }

    fn write(dir: &Path, file: &str, body: &str) {
        if let Err(error) = std::fs::write(dir.join(file), body) {
            panic!("write {file}: {error}");
        }
    }

    #[test]
    fn find_accepts_exactly_the_file_at_the_resolved_path() {
        let dir = crate_dir(&[
            "resources/angular/pages/Tramits/BaixaMatricula/create.page.ts",
            "resources/angular/pages/Tramits/index.page.ts",
            "frontend/src/pages/Home.svelte",
        ]);
        let lookup = lookup(ISSUE_LOOKUP);
        assert_eq!(
            lookup.find(dir.path(), "Tramits/BaixaMatricula/Create"),
            Some(
                dir.path()
                    .join("resources/angular/pages/Tramits/BaixaMatricula/create.page.ts")
            )
        );
        assert_eq!(
            lookup.find(dir.path(), "Tramits/Index"),
            Some(dir.path().join("resources/angular/pages/Tramits/index.page.ts"))
        );
        assert_eq!(lookup.find(dir.path(), "Tramits/BaixaMatricula/Edit"), None);
        // The starter location no longer counts once a lookup is set.
        assert_eq!(lookup.find(dir.path(), "Home"), None);
    }

    #[test]
    fn starter_find_accepts_any_of_the_four_extensions() {
        let dir = crate_dir(&["frontend/src/pages/Home.vue", "frontend/src/pages/Users/Index.tsx"]);
        let lookup = PageLookup::starter();
        assert_eq!(
            lookup.find(dir.path(), "Home"),
            Some(dir.path().join("frontend/src/pages/Home.vue"))
        );
        assert_eq!(
            lookup.find(dir.path(), "Users/Index"),
            Some(dir.path().join("frontend/src/pages/Users/Index.tsx"))
        );
        assert_eq!(lookup.find(dir.path(), "Missing"), None);
    }

    #[test]
    fn available_lists_the_files_the_pattern_can_name() {
        let dir = crate_dir(&[
            "resources/angular/pages/Tramits/BaixaMatricula/create.page.ts",
            "resources/angular/pages/Tramits/index.page.ts",
            "resources/angular/pages/Tramits/shared.service.ts",
            "resources/angular/pages/README.md",
        ]);
        assert_eq!(
            lookup(ISSUE_LOOKUP).available(dir.path()),
            vec![
                "Tramits/BaixaMatricula/create.page.ts".to_string(),
                "Tramits/index.page.ts".to_string(),
            ]
        );
    }

    #[test]
    fn available_lists_components_for_the_extension_lookup() {
        let dir = crate_dir(&[
            "frontend/src/pages/Home.svelte",
            "frontend/src/pages/Users/Index.tsx",
            "frontend/src/pages/styles.css",
        ]);
        assert_eq!(
            PageLookup::starter().available(dir.path()),
            vec!["Home".to_string(), "Users/Index".to_string()]
        );
    }

    #[test]
    fn read_lookup_without_a_manifest_keeps_the_starter_lookup() {
        let dir = crate_dir(&[]);
        assert_eq!(read_lookup(dir.path()), Ok(None));
    }

    #[test]
    fn read_lookup_reads_the_crate_manifest() {
        let dir = crate_dir(&[]);
        write(dir.path(), "Cargo.toml", ISSUE_LOOKUP);
        assert_eq!(read_lookup(dir.path()), Ok(Some(lookup(ISSUE_LOOKUP))));
    }

    #[test]
    fn read_lookup_errors_name_the_table_and_the_manifest() {
        let dir = crate_dir(&[]);
        write(
            dir.path(),
            "Cargo.toml",
            &with_table("page_file = \"{dir}/{name|upper}.page.ts\""),
        );
        let problem = match read_lookup(dir.path()) {
            Ok(_) => panic!("an unknown filter must be rejected"),
            Err(problem) => problem,
        };
        assert!(
            problem.starts_with("[package.metadata.suprnova.inertia] in Cargo.toml: "),
            "{problem}"
        );
        assert!(problem.contains("unknown filter `upper`"), "{problem}");
    }
}
