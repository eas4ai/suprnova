//! The starter kits on Inertia 3.8 (PAR-076 to PAR-080).
//!
//! Each kit's tests sit in their own module. They scaffold a project with
//! the CLI binary into a temporary directory and read the files it wrote, so
//! they see what `suprnova new` ships, not a template before substitution.
//! None of them runs npm: `kit_typecheck.rs` type-checks and builds a kit.

mod svelte {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Scaffold a svelte project into `tmp` and return its `frontend/`.
    fn scaffold(tmp: &tempfile::TempDir) -> PathBuf {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_suprnova"))
            .args([
                "new",
                "kitsvelte",
                "--no-interaction",
                "--no-git",
                "--frontend",
                "svelte",
            ])
            .current_dir(tmp.path())
            .output()
            .expect("run `suprnova new`");
        assert!(
            out.status.success(),
            "`suprnova new --frontend svelte` failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        tmp.path().join("kitsvelte/frontend")
    }

    fn read(frontend: &Path, rel: &str) -> String {
        fs::read_to_string(frontend.join(rel))
            .unwrap_or_else(|e| panic!("read frontend/{rel}: {e}"))
    }

    /// Every `.svelte` file under `frontend/src/<dir>`, as its path relative
    /// to that directory and its text, sorted by path.
    fn svelte_files(frontend: &Path, dir: &str) -> Vec<(String, String)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
            for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else if path.extension().is_some_and(|ext| ext == "svelte") {
                    let rel = path
                        .strip_prefix(root)
                        .expect("under root")
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push((rel, fs::read_to_string(&path).expect("read svelte file")));
                }
            }
        }
        let root = frontend.join("src").join(dir);
        let mut out = Vec::new();
        walk(&root, &root, &mut out);
        out.sort();
        out
    }

    /// The lines of a script or a template that are not comments.
    fn code_lines(body: &str) -> Vec<&str> {
        body.lines()
            .filter(|line| {
                let line = line.trim_start();
                !["//", "/*", "*", "<!--"]
                    .iter()
                    .any(|start| line.starts_with(start))
            })
            .collect()
    }

    /// `body` with every run of whitespace turned into one space, so a check
    /// reads a tag the same whether its attributes share a line or not.
    fn flatten(body: &str) -> String {
        body.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The text of every `<a ...>` opening tag in `body`.
    fn anchor_tags(body: &str) -> Vec<&str> {
        let mut tags = Vec::new();
        let mut rest = body;
        while let Some(at) = rest.find("<a") {
            let after = &rest[at + 2..];
            if after.starts_with(|c: char| c.is_whitespace() || c == '>') {
                let end = after.find('>').map_or(after.len(), |end| end + 1);
                tags.push(&rest[at..at + 2 + end]);
            }
            rest = after;
        }
        tags
    }

    #[test]
    fn kit_svelte_anchor_reader_sees_every_anchor_and_nothing_else() {
        assert_eq!(
            anchor_tags("<a href={`${root}/x`}>x</a> <abbr>y</abbr> <a\n  href=\"https://e.x\">"),
            vec!["<a href={`${root}/x`}>", "<a\n  href=\"https://e.x\">"]
        );
        assert!(anchor_tags("<Link href={`${root}/x`}>x</Link>").is_empty());
    }

    #[test]
    fn kit_svelte_manifest_pins_inertia_3_8_and_the_vite_plugin() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let manifest: serde_json::Value =
            serde_json::from_str(&read(&frontend, "package.json")).expect("package.json parses");
        for package in ["@inertiajs/svelte", "@inertiajs/core", "@inertiajs/vite"] {
            let pin = manifest["dependencies"][package]
                .as_str()
                .or_else(|| manifest["devDependencies"][package].as_str());
            assert_eq!(
                pin,
                Some("^3.8.0"),
                "package.json must declare {package} at ^3.8.0; got {manifest}"
            );
        }
        let check = manifest["scripts"]["check"].as_str().unwrap_or("");
        assert!(
            check.contains("svelte-check"),
            "package.json must have a `check` script that runs svelte-check; got {check:?}"
        );
        let build_ssr = manifest["scripts"]["build:ssr"].as_str().unwrap_or("");
        assert_eq!(build_ssr, "vite build --ssr src/ssr.ts");

        let vite = read(&frontend, "vite.config.ts");
        assert!(
            vite.contains("import inertia from '@inertiajs/vite'"),
            "vite.config.ts must import the Inertia plugin:\n{vite}"
        );
        assert!(
            vite.contains("inertia({ ssr: { entry: 'src/ssr.ts', sourcemap: false } })"),
            "vite.config.ts must register the plugin on the SSR entry `build:ssr` builds:\n{vite}"
        );
        for kept in [
            "base: './'",
            "outDir: 'bootstrap/ssr'",
            "entryFileNames: 'ssr.js'",
        ] {
            assert!(vite.contains(kept), "vite.config.ts lost `{kept}`:\n{vite}");
        }
    }

    #[test]
    fn kit_svelte_entries_resolve_pages_through_the_plugin() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        for entry in ["src/main.ts", "src/ssr.ts"] {
            let body = read(&frontend, entry);
            assert!(
                body.contains("pages: './pages',"),
                "{entry} must resolve pages through the plugin's `pages` shorthand:\n{body}"
            );
            assert!(
                !body.contains("import.meta.glob"),
                "{entry} must not glob the pages itself:\n{body}"
            );
        }
        // The plugin wraps a top-level `createInertiaApp(...)` statement in
        // an SSR build and adds the server itself
        // (packages/vite/src/ssrTransform.ts), so the SSR entry starts none.
        let ssr = read(&frontend, "src/ssr.ts");
        let code = code_lines(&ssr);
        assert!(
            code.iter()
                .any(|line| line.starts_with("createInertiaApp({")),
            "src/ssr.ts must be the top-level `createInertiaApp` statement the plugin wraps:\n{ssr}"
        );
        assert!(
            !code.iter().any(|line| line.contains("createServer")),
            "src/ssr.ts must leave `createServer` to the plugin:\n{ssr}"
        );
    }

    #[test]
    fn kit_svelte_entries_pass_title_server_head_nonce_and_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let main = read(&frontend, "src/main.ts");
        for option in [
            "title: pageTitle,",
            "serverHead: true,",
            "nonce,",
            "layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout),",
        ] {
            assert!(
                main.contains(option),
                "src/main.ts must pass `{option}`:\n{main}"
            );
        }
        assert!(
            main.contains(
                "document.querySelector<HTMLMetaElement>('meta[property=\"csp-nonce\"]')?.nonce"
            ),
            "src/main.ts must read the nonce from the csp-nonce meta element:\n{main}"
        );
        let ssr = read(&frontend, "src/ssr.ts");
        for option in [
            "title: pageTitle,",
            "serverHead: true,",
            "layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout),",
        ] {
            assert!(
                ssr.contains(option),
                "src/ssr.ts must pass `{option}`:\n{ssr}"
            );
        }
        let title = read(&frontend, "src/lib/title.ts");
        assert!(
            title.contains("export const appName = 'Kitsvelte'"),
            "src/lib/title.ts must name the application as the server titles it:\n{title}"
        );
    }

    #[test]
    fn kit_svelte_layouts_exist_with_their_chrome() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let app = flatten(&read(&frontend, "src/layouts/AppLayout.svelte"));
        for part in [
            "<Link href={`${root}/dashboard`}",
            "<Link href={`${root}/notes`}",
            "<Link href={`${root}/logout`} method=\"post\" as=\"button\"",
            "<FlashToast />",
            "preserveState={false}",
            "{#if auth.user}",
            "{auth.user.name}",
            "<Link href={`${root}/login`}",
            "<Link href={`${root}/register`}",
            "{#if heading}",
            "{@render children?.()}",
        ] {
            assert!(
                app.contains(part),
                "AppLayout.svelte lacks `{part}`:\n{app}"
            );
        }
        let guest = flatten(&read(&frontend, "src/layouts/GuestLayout.svelte"));
        for part in [
            "<Link href={`${root}/login`}",
            "<Link href={`${root}/register`}",
            "<FlashToast />",
            "{@render children?.()}",
        ] {
            assert!(
                guest.contains(part),
                "GuestLayout.svelte lacks `{part}`:\n{guest}"
            );
        }
    }

    #[test]
    fn kit_svelte_no_page_or_layout_links_an_application_route_with_an_anchor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let mut offenders = Vec::new();
        let mut files = 0usize;
        for dir in ["pages", "layouts", "components"] {
            for (rel, body) in svelte_files(&frontend, dir) {
                files += 1;
                for tag in anchor_tags(&body) {
                    let external = ["href=\"https://", "href='https://", "href={'https://"]
                        .iter()
                        .any(|href| tag.contains(href));
                    if !external {
                        offenders.push(format!("{dir}/{rel}: {tag}"));
                    }
                }
            }
        }
        assert!(files >= 14, "walked only {files} svelte files");
        assert!(
            offenders.is_empty(),
            "an application route is reached through an anchor, not a `Link`:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn kit_svelte_every_page_sets_its_title_with_head() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let pages = svelte_files(&frontend, "pages");
        let names: Vec<&str> = pages.iter().map(|(rel, _)| rel.as_str()).collect();
        assert_eq!(
            names,
            [
                "Dashboard.svelte",
                "Error.svelte",
                "Home.svelte",
                "Notes/Index.svelte",
                "Notes/Show.svelte",
                "auth/ForgotPassword.svelte",
                "auth/Login.svelte",
                "auth/Register.svelte",
                "auth/ResetPassword.svelte",
                "auth/VerifyEmail.svelte",
            ]
        );
        for (rel, body) in &pages {
            assert!(
                body.contains("components/Head.svelte'") && body.contains("<Head"),
                "pages/{rel} must set its title with `Head`:\n{body}"
            );
        }
        let head = read(&frontend, "src/components/Head.svelte");
        assert!(
            head.contains("<svelte:head>") && head.contains("<title>{pageTitle(title)}</title>"),
            "components/Head.svelte must title the document through `pageTitle`:\n{head}"
        );
    }

    #[test]
    fn kit_svelte_auth_pages_submit_through_the_form_component() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        for (page, action) in [
            ("Login", "login"),
            ("Register", "register"),
            ("ForgotPassword", "forgot-password"),
            ("ResetPassword", "reset-password"),
            ("VerifyEmail", "email/verification-notification"),
        ] {
            let body = read(&frontend, &format!("src/pages/auth/{page}.svelte"));
            let flat = flatten(&body);
            assert!(
                flat.contains(&format!(
                    "<Form action={{`${{root}}/{action}`}} method=\"post\""
                )),
                "auth/{page} must submit through `<Form action={{`${{root}}/{action}`}} method=\"post\">`:\n{body}"
            );
            assert!(
                body.contains("{#snippet children({ errors, processing })}"),
                "auth/{page} must read `errors` and `processing` from the form:\n{body}"
            );
            assert!(
                body.contains("disabled={processing}"),
                "auth/{page} must disable its button while processing:\n{body}"
            );
            assert!(
                !body.contains("useForm") && !body.contains("onsubmit"),
                "auth/{page} must not submit with `useForm` or a submit handler:\n{body}"
            );
        }
        let login = read(&frontend, "src/pages/auth/Login.svelte");
        assert!(
            login.contains("name=\"remember\"") && login.contains("type=\"checkbox\""),
            "auth/Login keeps the remember checkbox:\n{login}"
        );
    }

    #[test]
    fn kit_svelte_dashboard_uses_the_client_components() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let body = flatten(&read(&frontend, "src/pages/Dashboard.svelte"));
        for part in [
            "<Deferred data=\"stats\">",
            "{#snippet fallback()}",
            "<WhenVisible data=\"recent_notes\">",
            "usePoll(10000, { only: ['stats'] })",
            "useHttp(",
            "`${root}/profile/name`",
            "page.props.auth.user",
            "router.replaceProp('auth.user.name', ",
            "router.reload({ only: ['auth'] })",
            "errors.name",
            ": DashboardProps = $props()",
            "setLayoutProps({ heading: 'Dashboard' })",
        ] {
            assert!(
                body.contains(part),
                "Dashboard.svelte lacks `{part}`:\n{body}"
            );
        }
    }

    #[test]
    fn kit_svelte_notes_pages_use_the_client_components() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let index = flatten(&read(&frontend, "src/pages/Notes/Index.svelte"));
        for part in [
            "<Form action={`${root}/notes`} method=\"post\"",
            "name=\"title\"",
            "name=\"body\"",
            "useRemember(",
            "router.get(`${root}/notes`, { search: filters.search }",
            "reset: ['notes']",
            "<InfiniteScroll data=\"notes\"",
            "href={`${root}/notes/${note.id}`}",
            "prefetch",
            "component=\"Notes/Show\"",
            "pageProps={(_props, shared) => ({ ...shared, note })}",
            ": NotesIndexProps = $props()",
        ] {
            assert!(
                index.contains(part),
                "Notes/Index.svelte lacks `{part}`:\n{index}"
            );
        }
        let show = read(&frontend, "src/pages/Notes/Show.svelte");
        assert!(
            show.contains(": NotesShowProps = $props()") && show.contains("{note.title}"),
            "Notes/Show.svelte must render the note it is given:\n{show}"
        );
    }

    #[test]
    fn kit_svelte_flash_toast_renders_the_page_flash() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let toast = read(&frontend, "src/components/FlashToast.svelte");
        assert!(
            toast.contains("page.flash.toast"),
            "FlashToast.svelte must read `page.flash.toast`:\n{toast}"
        );
        let code = code_lines(&toast).join("\n");
        assert!(
            !code.contains("$state") && !code.contains("useRemember"),
            "FlashToast.svelte must render the flash from the page, never store it:\n{toast}"
        );
        let types = read(&frontend, "src/types/inertia-props.ts");
        assert!(
            types.contains("    flashDataType: "),
            "the generated types must name the flash data type:\n{types}"
        );
    }

    /// The structs the kit's pages are typed from, as the scaffold's
    /// backend declares them for PAR-078 to PAR-080: the flash data, the
    /// shared `auth`, the dashboard and the notes pages. They go into
    /// `src/controllers/` over the scaffold's own, and `generate-types` runs
    /// on the result. Once the scaffold ships these structs itself, this
    /// list repeats or replaces them and the test fails until it is gone.
    const KIT_BACKEND: [(&str, &str); 4] = [
        (
            "dashboard.rs",
            include_str!("fixtures/kit_svelte/dashboard.rs"),
        ),
        ("notes.rs", include_str!("fixtures/kit_svelte/notes.rs")),
        ("flash.rs", include_str!("fixtures/kit_svelte/flash.rs")),
        ("shared.rs", include_str!("fixtures/kit_svelte/shared.rs")),
    ];

    #[test]
    fn kit_svelte_types_are_what_generate_types_writes_for_the_kit_backend() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let frontend = scaffold(&tmp);
        let project = frontend.parent().expect("project").to_path_buf();
        let shipped = read(&frontend, "src/types/inertia-props.ts");
        for (file, body) in KIT_BACKEND {
            fs::write(project.join("src/controllers").join(file), body)
                .unwrap_or_else(|e| panic!("write src/controllers/{file}: {e}"));
        }
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_suprnova"))
            .arg("generate-types")
            .current_dir(&project)
            .output()
            .expect("run `suprnova generate-types`");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "generate-types failed: {text}");
        assert!(
            text.contains("./frontend/src/types/inertia-props.ts is up to date"),
            "the shipped types differ from what generate-types writes: {text}"
        );
        assert_eq!(read(&frontend, "src/types/inertia-props.ts"), shipped);
        for declaration in [
            "export interface SharedProps {\n  root: string;\n  auth: Auth;\n}",
            "    flashDataType: Flash;",
            "  \"Notes/Index\": NotesIndexProps;",
            "  \"Notes/Show\": NotesShowProps;",
        ] {
            assert!(
                shipped.contains(declaration),
                "the shipped types lack `{declaration}`:\n{shipped}"
            );
        }
    }
}
