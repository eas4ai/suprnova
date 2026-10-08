//! The starter kits on Inertia 3.8 (PAR-076 to PAR-080), read from a fresh
//! scaffold: `suprnova new` runs as a user runs it, and each test asserts on
//! the files it wrote. Hermetic: no npm, no network. The kits' type check
//! and builds are `kit_typecheck.rs`'s ignored tests.

/// The React kit: `suprnova new <name> --frontend react`.
mod react {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

    /// The auth pages, by component name. Every one submits through `Form`.
    const AUTH_PAGES: [&str; 5] = [
        "auth/Login",
        "auth/Register",
        "auth/ForgotPassword",
        "auth/ResetPassword",
        "auth/VerifyEmail",
    ];

    /// Every page the kit ships, by component name.
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

    /// A scaffolded React project: the temporary directory that owns it and
    /// its `frontend/` directory.
    struct Kit {
        _tmp: tempfile::TempDir,
        frontend: PathBuf,
    }

    impl Kit {
        /// Run `suprnova new kit-react --frontend react` in a fresh directory.
        fn scaffold() -> Kit {
            let tmp = tempfile::tempdir().expect("tempdir");
            let output = Command::new(BIN)
                .args([
                    "new",
                    "kit-react",
                    "--no-interaction",
                    "--no-git",
                    "--frontend",
                    "react",
                ])
                .current_dir(tmp.path())
                .output()
                .expect("spawn suprnova new");
            assert!(
                output.status.success(),
                "`suprnova new kit-react --frontend react` failed:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let frontend = tmp.path().join("kit-react/frontend");
            Kit {
                _tmp: tmp,
                frontend,
            }
        }

        /// The text of `frontend/<rel>`.
        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.frontend.join(rel))
                .unwrap_or_else(|e| panic!("read frontend/{rel}: {e}"))
        }

        /// The text of the page a handler renders as `component`.
        fn page(&self, component: &str) -> String {
            self.read(&format!("src/pages/{component}.tsx"))
        }

        /// Every `.tsx` file under `frontend/src/<dir>`, with its text.
        fn sources(&self, dir: &str) -> Vec<(PathBuf, String)> {
            let mut out = Vec::new();
            collect_tsx(&self.frontend.join("src").join(dir), &mut out);
            out
        }
    }

    fn collect_tsx(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                collect_tsx(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "tsx") {
                let body = fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                out.push((path, body));
            }
        }
    }

    /// Every `href` value of an `<a>` element in `source`, as written.
    fn anchor_hrefs(source: &str) -> Vec<String> {
        let mut hrefs = Vec::new();
        for (at, _) in source.match_indices("<a") {
            let rest = &source[at + 2..];
            // `<a` opens an anchor only when a space, a line end or `>`
            // follows; `<abbr>` and `<article>` are other elements.
            if !rest.starts_with([' ', '\n', '\r', '\t', '>']) {
                continue;
            }
            let tag = rest.split_once('>').map_or(rest, |(tag, _)| tag);
            if let Some((_, value)) = tag.split_once("href=") {
                hrefs.push(value.split_whitespace().next().unwrap_or("").to_owned());
            }
        }
        hrefs
    }

    #[test]
    fn kit_react_anchor_reader_sees_every_form() {
        assert_eq!(
            anchor_hrefs("<a href={`${root}/`}>Home</a>"),
            ["{`${root}/`}"]
        );
        assert_eq!(
            anchor_hrefs("<a\n  className=\"x\"\n  href=\"https://inertiajs.com\"\n>"),
            ["\"https://inertiajs.com\""]
        );
        assert!(anchor_hrefs("<abbr title=\"x\">x</abbr><article></article>").is_empty());
        assert!(anchor_hrefs("<Link href={`${root}/notes`}>Notes</Link>").is_empty());
    }

    /// PAR-076: the manifest declares the adapter, `@inertiajs/core` and
    /// `@inertiajs/vite` at `^3.8.0` and a `check` script, and the Vite
    /// config runs the plugin with the SSR bundle where `ssr:start` reads it.
    #[test]
    fn kit_react_pins_inertia_3_8_and_the_vite_plugin() {
        let kit = Kit::scaffold();
        let manifest: serde_json::Value =
            serde_json::from_str(&kit.read("package.json")).expect("package.json parses");
        for package in ["@inertiajs/react", "@inertiajs/core"] {
            assert_eq!(
                manifest["dependencies"][package], "^3.8.0",
                "{package} must be a dependency at ^3.8.0: {manifest}"
            );
        }
        let vite = manifest["dependencies"]["@inertiajs/vite"]
            .as_str()
            .or_else(|| manifest["devDependencies"]["@inertiajs/vite"].as_str());
        assert_eq!(
            vite,
            Some("^3.8.0"),
            "@inertiajs/vite at ^3.8.0: {manifest}"
        );
        let check = manifest["scripts"]["check"].as_str().unwrap_or("");
        assert!(
            check.contains("tsc"),
            "the kit needs a `check` script that type-checks it: {manifest}"
        );
        assert_eq!(
            manifest["scripts"]["build:ssr"], "vite build --ssr src/ssr.tsx",
            "{manifest}"
        );

        let config = kit.read("vite.config.ts");
        for needle in [
            "import inertia from '@inertiajs/vite'",
            "inertia({ ssr: 'src/ssr.tsx' })",
            "base: './',",
            "outDir: 'bootstrap/ssr'",
            "entryFileNames: 'ssr.js'",
            "outDir: '../public/assets'",
            "input: 'src/main.tsx'",
            "port: Number(process.env.VITE_PORT) || 5765",
        ] {
            assert!(
                config.contains(needle),
                "vite.config.ts lacks {needle}:\n{config}"
            );
        }

        for (path, body) in kit.sources("").into_iter().chain([
            (kit.frontend.join("package.json"), kit.read("package.json")),
            (kit.frontend.join("vite.config.ts"), config.clone()),
            (
                kit.frontend.join("src/lib/app.ts"),
                kit.read("src/lib/app.ts"),
            ),
        ]) {
            assert!(
                !body.contains("3.6.1"),
                "{} still cites Inertia 3.6.1",
                path.display()
            );
        }
    }

    /// PAR-076: both entries resolve pages through the plugin's `pages`
    /// shorthand and pass the title callback, the server head and the
    /// layout; the browser entry passes the document's CSP nonce.
    #[test]
    fn kit_react_entries_resolve_pages_through_the_plugin() {
        let kit = Kit::scaffold();
        let main = kit.read("src/main.tsx");
        let ssr = kit.read("src/ssr.tsx");
        for (file, body) in [("main.tsx", &main), ("ssr.tsx", &ssr)] {
            assert!(
                !body.contains("import.meta.glob"),
                "{file} resolves pages with its own glob:\n{body}"
            );
            for needle in [
                "createInertiaApp({",
                "pages: './pages',",
                "title,",
                "layout,",
                "serverHead: true,",
                "import { layout, title } from './lib/app'",
                "<LangProvider>",
            ] {
                assert!(body.contains(needle), "{file} lacks {needle}:\n{body}");
            }
        }
        assert!(
            main.contains(
                "document.querySelector<HTMLMetaElement>('meta[property=\"csp-nonce\"]')"
            ) && main.contains("nonce,"),
            "main.tsx must pass the nonce of <meta property=\"csp-nonce\">:\n{main}"
        );
        assert!(
            main.contains("initLang(props.initialPage)"),
            "main.tsx loads the catalog before the first render:\n{main}"
        );
        assert!(
            !ssr.contains("createServer(") && !ssr.contains("initLang("),
            "the plugin wraps ssr.tsx's createInertiaApp call in createServer, \
             and the SSR pass loads no catalog:\n{ssr}"
        );
        for comment in ["XSRF-TOKEN", "hydration content mismatch"] {
            assert!(
                main.contains(comment),
                "main.tsx lost the {comment} comment"
            );
        }

        let app = kit.read("src/lib/app.ts");
        for needle in [
            "export const appName = 'Kit React'",
            "title ? `${title} - ${appName}` : appName",
            "name.startsWith('auth/') ? GuestLayout : AppLayout",
        ] {
            assert!(
                app.contains(needle),
                "src/lib/app.ts lacks {needle}:\n{app}"
            );
        }
    }

    /// PAR-077 and PAR-078: the guest and application layouts, and the flash
    /// toast both show, read from the page and never kept in state.
    #[test]
    fn kit_react_ships_both_layouts_and_the_flash_toast() {
        let kit = Kit::scaffold();
        let app = kit.read("src/layouts/AppLayout.tsx");
        for needle in [
            "href={`${root}/dashboard`}",
            "href={`${root}/notes`}",
            "<Link href={`${root}/logout`} method=\"post\" as=\"button\"",
            "user.name",
            "heading",
            "<FlashToast />",
        ] {
            assert!(app.contains(needle), "AppLayout lacks {needle}:\n{app}");
        }
        // A `Link` that posts preserves the page's state by default, and a
        // visit that preserves state keeps the layout props: signing out
        // from the dashboard would carry its heading onto the next page
        // (`packages/react/src/Link.ts` and `App.ts` in Inertia 3.8.0).
        assert!(
            app.contains(
                "<Link href={`${root}/logout`} method=\"post\" as=\"button\" preserveState={false}"
            ),
            "the sign-out Link must not preserve state:\n{app}"
        );
        let guest = kit.read("src/layouts/GuestLayout.tsx");
        for needle in [
            "<Link href={`${root}/login`}",
            "<Link href={`${root}/register`}",
            "<FlashToast />",
        ] {
            assert!(
                guest.contains(needle),
                "GuestLayout lacks {needle}:\n{guest}"
            );
        }

        let toast = kit.read("src/components/FlashToast.tsx");
        for needle in ["usePage().flash", "flash.toast", "toast.message"] {
            assert!(
                toast.contains(needle),
                "FlashToast lacks {needle}:\n{toast}"
            );
        }
        for stored in ["useState", "useRemember", "useEffect"] {
            assert!(
                !toast.contains(stored),
                "FlashToast must render the toast from the page, not keep it ({stored}):\n{toast}"
            );
        }

        // The layouts come from `createInertiaApp`'s `layout` option, so a
        // page never renders one itself; a page that did would remount it on
        // every visit.
        for (path, body) in kit.sources("pages") {
            assert!(
                !body.contains("layouts/"),
                "{} renders a layout itself",
                path.display()
            );
        }
    }

    /// PAR-077: every in-application navigation is a `Link`; an anchor
    /// stays only for an external `https://` URL.
    #[test]
    fn kit_react_never_links_an_application_route_with_an_anchor() {
        let kit = Kit::scaffold();
        let mut offenders = Vec::new();
        let mut scanned = 0usize;
        for dir in ["pages", "layouts", "components"] {
            for (path, body) in kit.sources(dir) {
                scanned += 1;
                for href in anchor_hrefs(&body) {
                    if !(href.starts_with("\"https://") || href.starts_with("'https://")) {
                        offenders.push(format!("{}: href={href}", path.display()));
                    }
                }
            }
        }
        assert_eq!(
            scanned, 13,
            "ten pages, two layouts and the toast component"
        );
        assert!(
            offenders.is_empty(),
            "anchors to application routes:\n{}",
            offenders.join("\n")
        );
    }

    /// PAR-077: every page sets its title with `Head`.
    #[test]
    fn kit_react_titles_every_page_with_head() {
        let kit = Kit::scaffold();
        let pages = kit.sources("pages");
        assert_eq!(
            pages.len(),
            PAGES.len(),
            "the kit ships exactly these pages: {PAGES:?}"
        );
        for component in PAGES {
            let body = kit.page(component);
            assert!(
                body.contains("<Head title=")
                    && body.contains("Head")
                    && body.contains("from '@inertiajs/react'"),
                "{component} sets no title with Head:\n{body}"
            );
        }
    }

    /// PAR-079: every auth page submits through the `Form` component and
    /// reads `errors` and `processing` from it, never `useForm`.
    #[test]
    fn kit_react_auth_pages_submit_through_the_form_component() {
        let kit = Kit::scaffold();
        for (component, action) in AUTH_PAGES.into_iter().zip([
            "/login",
            "/register",
            "/forgot-password",
            "/reset-password",
            "/email/verification-notification",
        ]) {
            let body = kit.page(component);
            for needle in [
                format!("action={{`${{root}}{action}`}}"),
                "method=\"post\"".to_owned(),
                "<Form".to_owned(),
                "({ errors, processing }) =>".to_owned(),
                "disabled={processing}".to_owned(),
            ] {
                assert!(
                    body.contains(&needle),
                    "{component} lacks {needle}:\n{body}"
                );
            }
            for refused in ["useForm", "onSubmit", "preventDefault"] {
                assert!(
                    !body.contains(refused),
                    "{component} submits with {refused}, not the Form component:\n{body}"
                );
            }
        }
        let login = kit.page("auth/Login");
        assert!(
            login.contains("name=\"remember\"")
                && login.contains("remember: Boolean(data.remember)"),
            "Login keeps the remember checkbox and sends it as a boolean:\n{login}"
        );
        let reset = kit.page("auth/ResetPassword");
        assert!(
            reset.contains("<input type=\"hidden\" name=\"token\" value={token} />"),
            "ResetPassword sends the mailed token back in the form body:\n{reset}"
        );
    }

    /// PAR-077 and PAR-079: the dashboard defers `stats`, loads
    /// `recent_notes` when scrolled into view, polls `stats`, changes the
    /// display name through `useHttp` optimistically, and sets the layout's
    /// heading.
    #[test]
    fn kit_react_dashboard_uses_deferred_when_visible_poll_and_http() {
        let kit = Kit::scaffold();
        let body = kit.page("Dashboard");
        for needle in [
            "<Deferred data=\"stats\" fallback=",
            "<WhenVisible data=\"recent_notes\" fallback=",
            "usePoll(10000, { only: ['stats'] })",
            "useHttp<",
            ".optimistic(",
            ".post(`${root}/profile/name`",
            "profile.errors.name",
            "setLayoutProps({ heading: 'Dashboard' })",
            "<Head title=\"Dashboard\" />",
        ] {
            assert!(body.contains(needle), "Dashboard lacks {needle}:\n{body}");
        }
    }

    /// PAR-079: the notes page creates a note through `Form`, remembers its
    /// search, scrolls the cursor-paginated rows, and opens a note through an
    /// instant visit that prefetches on hover.
    #[test]
    fn kit_react_notes_pages_scroll_remember_and_visit_instantly() {
        let kit = Kit::scaffold();
        let index = kit.page("Notes/Index");
        for needle in [
            "<Form",
            "action={`${root}/notes`}",
            "method=\"post\"",
            "name=\"title\"",
            "name=\"body\"",
            "useRemember(",
            "router.get(`${root}/notes`, { search: value }",
            "preserveState: true",
            "<InfiniteScroll",
            "data=\"notes\"",
            "href={`${root}/notes/${note.id}`}",
            "prefetch",
            "component=\"Notes/Show\"",
            "pageProps={(_props, shared) => ({ ...shared, note })}",
        ] {
            assert!(
                index.contains(needle),
                "Notes/Index lacks {needle}:\n{index}"
            );
        }
        let show = kit.page("Notes/Show");
        for needle in [
            "<Head title={note.title} />",
            "<Link href={`${root}/notes`}",
        ] {
            assert!(show.contains(needle), "Notes/Show lacks {needle}:\n{show}");
        }
    }

    /// The kit's `inertia-props.ts` is what `suprnova generate-types` writes
    /// for the handlers the kit contract names: the scaffold's controllers,
    /// with the notes controller and the flash struct as the contract
    /// declares them. The backend lane writes those two; until its
    /// controllers are on this branch, `notes.rs` and `flash.rs` below stand
    /// in for them.
    #[test]
    fn kit_react_types_are_what_generate_types_writes_for_the_kit_contract() {
        let dir = tempfile::tempdir().expect("tempdir");
        let controllers = dir.path().join("src/controllers");
        let props = dir.path().join("src/props");
        fs::create_dir_all(&controllers).expect("create src/controllers");
        fs::create_dir_all(&props).expect("create src/props");
        for (name, body) in [
            ("home.rs", suprnova_cli::templates::home_controller()),
            ("auth.rs", suprnova_cli::templates::auth_controller()),
            (
                "dashboard.rs",
                suprnova_cli::templates::dashboard_controller(),
            ),
            (
                "email_verification.rs",
                suprnova_cli::templates::email_verification_controller(),
            ),
            (
                "password_reset.rs",
                suprnova_cli::templates::password_reset_controller(),
            ),
            ("notes.rs", CONTRACT_NOTES_CONTROLLER),
        ] {
            fs::write(controllers.join(name), body).unwrap_or_else(|e| panic!("write {name}: {e}"));
        }
        fs::write(props.join("flash.rs"), CONTRACT_FLASH).expect("write flash.rs");

        let structs = suprnova_cli::commands::generate_types::scan_inertia_props(dir.path());
        let expected = suprnova_cli::commands::generate_types::generate_typescript(
            &structs,
            suprnova_cli::commands::generate_types::PageTypes::default(),
        );
        let shipped = suprnova_cli::templates::react::inertia_props_types();
        assert_eq!(
            shipped, expected,
            "react's inertia-props.ts.tpl is not what `suprnova generate-types` writes \
             for the kit contract.\nexpected:\n{expected}\nshipped:\n{shipped}"
        );
        for declared in [
            "\"Notes/Show\": NoteShowProps;",
            "flashDataType: Toast;",
            "export interface DashboardProps",
        ] {
            assert!(shipped.contains(declared), "the types lack {declared}");
        }
    }

    /// The notes handlers as the kit contract declares them: `Notes/Index`
    /// through `Inertia::paginate` (no struct, so no `Pages` entry) and
    /// `Notes/Show` with `note: {id, title, body, created_at}`.
    const CONTRACT_NOTES_CONTROLLER: &str = r#"
use serde::Serialize;
use suprnova::{handler, inertia_response, Inertia, InertiaProps, Request, Response};

#[derive(Serialize)]
pub struct NoteView {
    pub id: u64,
    pub title: String,
    pub body: Option<String>,
    pub created_at: String,
}

#[derive(InertiaProps)]
pub struct NoteShowProps {
    pub note: NoteView,
}

#[handler]
pub async fn index(req: Request) -> Response {
    let paginator = notes_for(&req).await?;
    Inertia::paginate("Notes/Index", "notes", paginator).into()
}

#[handler]
pub async fn show(req: Request) -> Response {
    let note = note_for(&req).await?;
    inertia_response!(&req, "Notes/Show", NoteShowProps { note })
}
"#;

    /// The flash struct as the kit contract declares it.
    const CONTRACT_FLASH: &str = r#"
use serde::Serialize;
use suprnova::InertiaProps;

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToastKind {
    Success,
    Info,
    Error,
}

#[derive(InertiaProps, Serialize)]
#[inertia_props(flash)]
pub struct Toast {
    pub kind: ToastKind,
    pub message: String,
}
"#;
}
