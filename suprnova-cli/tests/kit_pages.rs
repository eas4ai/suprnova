//! The starter kits on Inertia 3.8 (PAR-076 to PAR-080), read from a real
//! scaffold.
//!
//! Each test scaffolds a project with the `suprnova` binary into a temporary
//! directory and asserts on the files it wrote. Nothing here installs a
//! package or builds a bundle; `kit_typecheck.rs` does that, ignored.

mod vue {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Scaffold a Vue project and return the temporary directory that owns
    /// it with the project's `frontend/src` path.
    fn scaffold() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let out = Command::new(env!("CARGO_BIN_EXE_suprnova"))
            .args([
                "new",
                "kit_vue",
                "--no-interaction",
                "--no-git",
                "--frontend",
                "vue",
            ])
            .current_dir(tmp.path())
            .output()
            .expect("spawn suprnova new");
        assert!(
            out.status.success(),
            "`suprnova new --frontend vue` failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let src = tmp.path().join("kit_vue/frontend/src");
        (tmp, src)
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// Every `.vue` file under `dir`, sorted, so a failure names the same
    /// file on every run.
    fn vue_files(dir: &Path) -> Vec<PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "vue") {
                    out.push(path);
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, &mut out);
        out.sort();
        out
    }

    /// The pages the kit ships, by component name.
    const PAGES: [&str; 10] = [
        "Home",
        "Dashboard",
        "Notes/Index",
        "Notes/Show",
        "Error",
        "auth/Login",
        "auth/Register",
        "auth/ForgotPassword",
        "auth/ResetPassword",
        "auth/VerifyEmail",
    ];

    /// The auth pages, each with the route its form posts to.
    const AUTH_FORMS: [(&str, &str); 5] = [
        ("auth/Login", "login"),
        ("auth/Register", "register"),
        ("auth/ForgotPassword", "forgot-password"),
        ("auth/ResetPassword", "reset-password"),
        ("auth/VerifyEmail", "email/verification-notification"),
    ];

    #[test]
    fn kit_vue_scaffold_writes_every_page_layout_and_component() {
        let (_tmp, src) = scaffold();
        for page in PAGES {
            let path = src.join(format!("pages/{page}.vue"));
            assert!(path.is_file(), "missing {}", path.display());
        }
        for file in [
            "layouts/AppLayout.vue",
            "layouts/GuestLayout.vue",
            "components/FlashToast.vue",
        ] {
            assert!(src.join(file).is_file(), "missing src/{file}");
        }
    }

    #[test]
    fn kit_vue_manifest_pins_inertia_3_8_with_the_vite_plugin_and_a_check_script() {
        let (_tmp, src) = scaffold();
        let manifest: serde_json::Value =
            serde_json::from_str(&read(&src.join("../package.json"))).expect("package.json");
        let pin = |name: &str| {
            manifest["dependencies"][name]
                .as_str()
                .or_else(|| manifest["devDependencies"][name].as_str())
                .map(str::to_owned)
        };
        for package in ["@inertiajs/vue3", "@inertiajs/core", "@inertiajs/vite"] {
            assert_eq!(
                pin(package).as_deref(),
                Some("^3.8.0"),
                "{package} must be declared at ^3.8.0: {manifest}"
            );
        }
        assert_eq!(
            manifest["scripts"]["check"].as_str(),
            Some("vue-tsc --noEmit"),
            "the kit needs a `check` script that type-checks it: {manifest}"
        );
        assert_eq!(
            manifest["scripts"]["build:ssr"].as_str(),
            Some("vite build --ssr src/ssr.ts"),
            "{manifest}"
        );
    }

    #[test]
    fn kit_vue_vite_config_adds_the_inertia_plugin_and_keeps_the_ssr_output() {
        let (_tmp, src) = scaffold();
        let config = read(&src.join("../vite.config.ts"));
        for needle in [
            "import inertia from '@inertiajs/vite'",
            "plugins: [inertia(), tailwindcss(), vue()],",
            "base: './',",
            "outDir: 'bootstrap/ssr'",
            "entryFileNames: 'ssr.js'",
            "outDir: '../public/assets'",
            "input: 'src/main.ts'",
        ] {
            assert!(
                config.contains(needle),
                "vite.config.ts lacks `{needle}`:\n{config}"
            );
        }
    }

    #[test]
    fn kit_vue_entries_resolve_pages_through_the_plugin() {
        let (_tmp, src) = scaffold();
        for entry in ["main.ts", "ssr.ts"] {
            let body = read(&src.join(entry));
            assert!(
                body.contains("pages: './pages'"),
                "{entry} must name its pages with the plugin's shorthand:\n{body}"
            );
            assert!(
                !body.contains("import.meta.glob"),
                "{entry} still globs its pages by hand:\n{body}"
            );
            assert!(
                !body.contains("resolve:"),
                "{entry} still passes its own resolver:\n{body}"
            );
        }
    }

    #[test]
    fn kit_vue_entries_pass_title_server_head_nonce_and_layout() {
        let (_tmp, src) = scaffold();
        let main = read(&src.join("main.ts"));
        for needle in [
            "title: (title) => (title ? `${title} - ${appName}` : appName)",
            "serverHead: true",
            "nonce,",
            "meta[property=\"csp-nonce\"]",
            "layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout)",
            "import AppLayout from './layouts/AppLayout.vue'",
            "import GuestLayout from './layouts/GuestLayout.vue'",
        ] {
            assert!(main.contains(needle), "main.ts lacks `{needle}`:\n{main}");
        }
        let ssr = read(&src.join("ssr.ts"));
        for needle in [
            "title: (title) => (title ? `${title} - ${appName}` : appName)",
            "serverHead: true",
            "layout: (name) => (name.startsWith('auth/') ? GuestLayout : AppLayout)",
        ] {
            assert!(ssr.contains(needle), "ssr.ts lacks `{needle}`:\n{ssr}");
        }
        assert!(
            main.contains("const appName = 'Kit Vue'") && ssr.contains("const appName = 'Kit Vue'"),
            "both entries name the application the way the server's default title does"
        );
    }

    /// The Vite plugin wraps a top-level `createInertiaApp(..)` statement in
    /// the SSR entry with `createServer` and `renderToString`
    /// (`packages/vite/src/ssrTransform.ts`, `frameworks/vue.ts`), so the
    /// entry imports neither itself: a second import of either name is a
    /// duplicate declaration in the wrapped module.
    #[test]
    fn kit_vue_ssr_entry_is_the_statement_the_plugin_wraps() {
        let (_tmp, src) = scaffold();
        let ssr = read(&src.join("ssr.ts"));
        assert!(
            ssr.lines()
                .any(|line| line.starts_with("createInertiaApp({")),
            "ssr.ts must call createInertiaApp at the top level:\n{ssr}"
        );
        for refused in ["createServer", "renderToString", "@inertiajs/vue3/server"] {
            assert!(
                ssr.lines()
                    .filter(|line| line.starts_with("import"))
                    .all(|line| !line.contains(refused)),
                "ssr.ts imports `{refused}`, which the plugin adds:\n{ssr}"
            );
        }
    }

    #[test]
    fn kit_vue_layouts_carry_navigation_the_user_the_toast_and_the_heading() {
        let (_tmp, src) = scaffold();
        let app = read(&src.join("layouts/AppLayout.vue"));
        for needle in [
            "<FlashToast />",
            "<slot />",
            "heading",
            "user.name",
            ":href=\"`${root}/dashboard`\"",
            ":href=\"`${root}/notes`\"",
            ":href=\"`${root}/logout`\" method=\"post\" as=\"button\"",
        ] {
            assert!(
                app.contains(needle),
                "AppLayout.vue lacks `{needle}`:\n{app}"
            );
        }
        let guest = read(&src.join("layouts/GuestLayout.vue"));
        for needle in [
            "<FlashToast />",
            "<slot />",
            ":href=\"`${root}/login`\"",
            ":href=\"`${root}/register`\"",
        ] {
            assert!(
                guest.contains(needle),
                "GuestLayout.vue lacks `{needle}`:\n{guest}"
            );
        }
    }

    /// Every `<a ...>` opening tag in `body` that is not a static link to
    /// an absolute `https://` URL.
    fn application_anchors(body: &str) -> Vec<&str> {
        body.match_indices("<a")
            .map(|(at, _)| {
                let tag = &body[at..];
                &tag[..tag.find('>').unwrap_or(tag.len())]
            })
            .filter(|tag| tag[2..].starts_with([' ', '\n']))
            .filter(|tag| !tag.contains("href=\"https://") || tag.contains(":href"))
            .collect()
    }

    #[test]
    fn kit_vue_the_anchor_reader_sees_application_anchors() {
        for body in [
            "<a :href=\"`${root}/`\">Home</a>",
            "<a href=\"/login\">Sign in</a>",
            "<a\n  :href=\"`${root}/notes`\"\n>Notes</a>",
            "<a class=\"x\">bare</a>",
        ] {
            assert_eq!(application_anchors(body).len(), 1, "{body}");
        }
        for body in [
            "<a href=\"https://inertiajs.com\">Inertia</a>",
            "<Link :href=\"`${root}/`\">Home</Link>",
            "<article class=\"x\"></article>",
            "<aside></aside>",
        ] {
            assert!(application_anchors(body).is_empty(), "{body}");
        }
    }

    /// An `<a href>` reloads the whole document; every application route is
    /// reached through `Link`. Only an absolute `https://` URL, a page off
    /// the application, stays an anchor.
    #[test]
    fn kit_vue_no_anchor_reaches_an_application_route() {
        let (_tmp, src) = scaffold();
        let mut offenders = Vec::new();
        let mut files = 0;
        for dir in ["pages", "layouts", "components"] {
            for path in vue_files(&src.join(dir)) {
                files += 1;
                let body = read(&path);
                for tag in application_anchors(&body) {
                    offenders.push(format!("{}: {}", path.display(), tag.trim()));
                }
            }
        }
        assert_eq!(
            files,
            PAGES.len() + 3,
            "the pages, the two layouts and the toast"
        );
        assert!(
            offenders.is_empty(),
            "anchors to application routes:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn kit_vue_every_page_sets_its_title_with_head() {
        let (_tmp, src) = scaffold();
        let pages = vue_files(&src.join("pages"));
        assert_eq!(pages.len(), PAGES.len(), "{pages:?}");
        for path in pages {
            let body = read(&path);
            // The import list that names `Head`: from `import {` to the
            // adapter it imports from, on one line or several.
            let imports_head = body
                .split("import {")
                .skip(1)
                .filter_map(|rest| rest.split_once("from '@inertiajs/vue3'"))
                .filter(|(names, _)| !names.contains("from"))
                .any(|(names, _)| {
                    names
                        .split([',', ' ', '\n', '}'])
                        .any(|name| name == "Head")
                });
            assert!(
                body.contains("<Head ") && imports_head,
                "{} sets no title with Head from @inertiajs/vue3:\n{body}",
                path.display()
            );
        }
    }

    #[test]
    fn kit_vue_auth_pages_submit_through_the_form_component() {
        let (_tmp, src) = scaffold();
        for (page, route) in AUTH_FORMS {
            let body = read(&src.join(format!("pages/{page}.vue")));
            let action = format!(":action=\"`${{root}}/{route}`\"");
            for needle in ["<Form", action.as_str(), "method=\"post\"", "processing"] {
                assert!(body.contains(needle), "{page} lacks `{needle}`:\n{body}");
            }
            assert!(
                !body.contains("useForm"),
                "{page} still submits through useForm:\n{body}"
            );
            assert!(
                !body.contains("@submit"),
                "{page} still has a native submit handler:\n{body}"
            );
            if page != "auth/VerifyEmail" {
                assert!(
                    body.contains("v-slot=\"{ errors, processing }\""),
                    "{page} must read `errors` and `processing` from the form's slot:\n{body}"
                );
            }
        }
        let login = read(&src.join("pages/auth/Login.vue"));
        assert!(
            login.contains("name=\"remember\"") && login.contains("type=\"checkbox\""),
            "Login keeps the remember checkbox:\n{login}"
        );
    }

    #[test]
    fn kit_vue_dashboard_defers_polls_and_saves_the_name_optimistically() {
        let (_tmp, src) = scaffold();
        let body = read(&src.join("pages/Dashboard.vue"));
        for needle in [
            "<Deferred data=\"stats\">",
            "<WhenVisible data=\"recent_notes\"",
            "<template #fallback>",
            "usePoll(10000, { only: ['stats'] })",
            "useHttp",
            ".optimistic(",
            "`${root}/profile/name`",
            "v-if=\"profile.errors.name\"",
            "setLayoutProps({ heading: 'Dashboard' })",
        ] {
            assert!(
                body.contains(needle),
                "Dashboard.vue lacks `{needle}`:\n{body}"
            );
        }
    }

    #[test]
    fn kit_vue_notes_index_scrolls_remembers_its_search_and_creates_notes() {
        let (_tmp, src) = scaffold();
        let body = read(&src.join("pages/Notes/Index.vue"));
        for needle in [
            "<InfiniteScroll data=\"notes\"",
            "useRemember",
            "preserveState: true",
            "reset: ['notes']",
            "<Form",
            ":action=\"`${root}/notes`\"",
            "method=\"post\"",
            "name=\"title\"",
            "name=\"body\"",
            ":href=\"`${root}/notes/${note.id}`\"",
            "prefetch",
            "component=\"Notes/Show\"",
            ":page-props=\"{ note }\"",
        ] {
            assert!(
                body.contains(needle),
                "Notes/Index.vue lacks `{needle}`:\n{body}"
            );
        }
        let show = read(&src.join("pages/Notes/Show.vue"));
        assert!(
            show.contains("note.title") && show.contains("`${root}/notes`"),
            "Notes/Show.vue renders the note and links back to the list:\n{show}"
        );
    }

    /// The toast is a page field the server sends once; the component
    /// renders it from the page and keeps no copy that could show it again.
    #[test]
    fn kit_vue_flash_toast_renders_the_page_flash() {
        let (_tmp, src) = scaffold();
        let body = read(&src.join("components/FlashToast.vue"));
        for needle in ["usePage()", "page.flash", "toast", "import type { Toast }"] {
            assert!(
                body.contains(needle),
                "FlashToast.vue lacks `{needle}`:\n{body}"
            );
        }
        for refused in ["localStorage", "sessionStorage", "useRemember", "watch("] {
            assert!(
                !body.contains(refused),
                "FlashToast.vue keeps the toast with `{refused}`:\n{body}"
            );
        }
    }

    #[test]
    fn kit_vue_types_declare_the_flash_data_type() {
        let (_tmp, src) = scaffold();
        let types = read(&src.join("types/inertia-props.ts"));
        for needle in [
            "export interface Toast {",
            "export interface Pages {",
            "export interface SharedProps {",
            "export type Errors = Record<string, string>;",
            "export type PageProps<C extends keyof Pages>",
            "    flashDataType: Toast;\n",
        ] {
            assert!(
                types.contains(needle),
                "inertia-props.ts lacks `{needle}`:\n{types}"
            );
        }
    }
}
