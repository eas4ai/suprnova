//! Vite tags for any page (PAR-159): `InertiaConfig::vite_tags` and
//! `Vite::tags` render the tags of any number of entry points from the
//! build manifest, as Laravel's `Vite::__invoke` does; a missing manifest
//! or entry is an error naming it; the hot file decides development's
//! tags, and production never reads it.
//!
//! Every test sets `development(true)` or `development(false)`: an unset
//! `APP_ENV` is production, so the default would depend on the process.

use std::path::{Path, PathBuf};

use suprnova::testing::TestContainer;
use suprnova::{Frontend, Inertia, InertiaConfig, InertiaResponse, Vite};

use crate::protocol_harness::MockReq;

/// A manifest with two entry points that share a chunk and its CSS.
const TWO_ENTRIES: &str = r#"{
    "src/a.ts": {
        "file": "a-AAA111.js",
        "isEntry": true,
        "css": ["a-AAA222.css"],
        "imports": ["_shared-SSS.js"]
    },
    "src/b.ts": {
        "file": "b-BBB111.js",
        "isEntry": true,
        "css": ["b-BBB222.css"],
        "imports": ["_shared-SSS.js"]
    },
    "src/app.css": {
        "file": "app-CCC111.css",
        "isEntry": true
    },
    "_shared-SSS.js": {
        "file": "shared-SSS.js",
        "css": ["shared-SSS.css"]
    }
}"#;

struct Build {
    _dir: tempfile::TempDir,
    manifest: PathBuf,
    hot: PathBuf,
}

/// A directory holding `manifest` (when given) and, when `hot` is given,
/// a hot file holding it.
fn build(manifest: Option<&str>, hot: Option<&str>) -> Build {
    let dir = tempfile::tempdir().expect("a directory");
    let manifest_path = dir.path().join("manifest.json");
    if let Some(content) = manifest {
        std::fs::write(&manifest_path, content).expect("write the manifest");
    }
    let hot_path = dir.path().join("hot");
    if let Some(content) = hot {
        std::fs::write(&hot_path, content).expect("write the hot file");
    }
    Build {
        _dir: dir,
        manifest: manifest_path,
        hot: hot_path,
    }
}

fn config(build: &Build, development: bool) -> InertiaConfig {
    InertiaConfig::new()
        .frontend(Frontend::Svelte)
        .entry_point("src/a.ts")
        .development(development)
        .manifest_path(&build.manifest)
        .ssr_hot_file(&build.hot)
        .vite_dev_server("http://localhost:5999")
        .ssr_disabled()
}

/// The first visit's document under `config`.
async fn first_visit(config: InertiaConfig) -> String {
    let response = InertiaResponse::new("Home")
        .with_config(config)
        .resolve(&MockReq::new("/home"))
        .await
        .expect("the first visit renders");
    String::from_utf8(response.body().to_vec()).expect("UTF-8")
}

fn position(haystack: &str, needle: &str) -> usize {
    haystack
        .find(needle)
        .unwrap_or_else(|| panic!("{needle} missing from {haystack}"))
}

// ---- vite_tags over the manifest -------------------------------------------

#[test]
fn vite_tags_renders_every_entry_point_with_stylesheets_first() {
    let build = build(Some(TWO_ENTRIES), None);
    let entries: &[&str] = &["src/a.ts", "src/b.ts"];
    let tags = config(&build, false)
        .vite_tags(entries)
        .expect("both entries are in the manifest");

    for file in [
        "/assets/a-AAA111.js",
        "/assets/b-BBB111.js",
        "/assets/a-AAA222.css",
        "/assets/b-BBB222.css",
        "/assets/shared-SSS.css",
        "/assets/shared-SSS.js",
    ] {
        assert!(tags.contains(file), "{file} missing from {tags}");
    }
    let last_stylesheet = tags.rfind("rel=\"stylesheet\"").expect("a stylesheet tag");
    let first_script = position(&tags, "<script");
    assert!(
        last_stylesheet < first_script,
        "a script comes before a stylesheet: {tags}"
    );
}

#[test]
fn vite_tags_renders_a_shared_chunk_and_its_css_once() {
    let build = build(Some(TWO_ENTRIES), None);
    let tags = config(&build, false)
        .vite_tags(["src/a.ts", "src/b.ts", "src/a.ts"])
        .expect("the entries are in the manifest");
    assert_eq!(tags.matches("/assets/shared-SSS.css").count(), 1, "{tags}");
    assert_eq!(tags.matches("/assets/shared-SSS.js").count(), 1, "{tags}");
    assert_eq!(tags.matches("/assets/a-AAA111.js").count(), 1, "{tags}");
    assert!(
        tags.contains("<link rel=\"modulepreload\" href=\"/assets/shared-SSS.js\">"),
        "{tags}"
    );
}

#[test]
fn vite_tags_renders_a_css_entry_as_a_stylesheet() {
    let build = build(Some(TWO_ENTRIES), None);
    let tags = config(&build, false)
        .vite_tags(["src/app.css"])
        .expect("the entry is in the manifest");
    assert_eq!(
        tags,
        "<link rel=\"stylesheet\" href=\"/assets/app-CCC111.css\">\n"
    );
}

#[test]
fn vite_tags_escapes_the_urls_it_writes() {
    let build = build(Some(TWO_ENTRIES), None);
    let tags = config(&build, false)
        .assets_base_url("https://cdn.test/a\"b")
        .vite_tags(["src/app.css"])
        .expect("the entry is in the manifest");
    assert!(
        tags.contains("href=\"https://cdn.test/a&quot;b/app-CCC111.css\""),
        "{tags}"
    );
}

#[test]
fn vite_tags_refuses_a_manifest_that_does_not_exist() {
    let build = build(None, None);
    let error = config(&build, false)
        .vite_tags(["src/a.ts"])
        .expect_err("there is no manifest");
    let message = error.to_string();
    assert!(
        message.contains(&build.manifest.display().to_string()),
        "the error names the manifest path: {message}"
    );
}

#[test]
fn vite_tags_refuses_an_entry_the_manifest_lacks() {
    let build = build(Some(TWO_ENTRIES), None);
    let error = config(&build, false)
        .vite_tags(["src/a.ts", "src/missing.ts"])
        .expect_err("the manifest lacks the entry");
    assert!(
        error.to_string().contains("src/missing.ts"),
        "the error names the entry: {error}"
    );
}

#[tokio::test]
async fn vite_tags_match_the_tags_the_inertia_shell_writes() {
    let build = build(Some(TWO_ENTRIES), None);
    let config = config(&build, false).entry_points(["src/app.css"]);
    let tags = config
        .vite_tags(["src/a.ts", "src/app.css"])
        .expect("the entries are in the manifest");
    let document = first_visit(config).await;
    assert!(
        document.contains(&tags),
        "the shell writes the same tags:\n{tags}\nin\n{document}"
    );
}

// ---- The hot file -----------------------------------------------------------

#[test]
fn hot_file_answers_the_path_ssr_hot_file_sets() {
    assert_eq!(
        InertiaConfig::new()
            .ssr_hot_file("storage/vite.hot")
            .hot_file(),
        Path::new("storage/vite.hot")
    );
    assert_eq!(InertiaConfig::new().hot_file(), Path::new("public/hot"));
}

#[tokio::test]
async fn development_without_a_hot_file_uses_an_existing_manifest() {
    let build = build(Some(TWO_ENTRIES), None);
    let document = first_visit(config(&build, true)).await;
    assert!(document.contains("/assets/a-AAA111.js"), "{document}");
    assert!(!document.contains("localhost:5999"), "{document}");
    assert!(!document.contains("@vite/client"), "{document}");

    let tags = config(&build, true)
        .vite_tags(["src/a.ts"])
        .expect("the manifest holds the entry");
    assert!(tags.contains("/assets/a-AAA111.js"), "{tags}");
    assert!(!tags.contains("@vite/client"), "{tags}");
}

#[tokio::test]
async fn development_with_a_hot_file_points_at_the_url_it_holds() {
    let build = build(Some(TWO_ENTRIES), Some("http://[::1]:5174\n"));
    let document = first_visit(config(&build, true)).await;
    assert!(
        document
            .contains("<script type=\"module\" src=\"http://[::1]:5174/@vite/client\"></script>"),
        "{document}"
    );
    assert!(
        document.contains("<script type=\"module\" src=\"http://[::1]:5174/src/a.ts\"></script>"),
        "{document}"
    );
    assert!(!document.contains("/assets/"), "{document}");
    assert!(!document.contains("localhost:5999"), "{document}");

    let tags = config(&build, true)
        .vite_tags(["src/a.ts", "src/app.css"])
        .expect("hot tags need no manifest");
    assert_eq!(
        tags,
        "<link rel=\"stylesheet\" href=\"http://[::1]:5174/src/app.css\">\n\
         <script type=\"module\" src=\"http://[::1]:5174/@vite/client\"></script>\n\
         <script type=\"module\" src=\"http://[::1]:5174/src/a.ts\"></script>\n"
    );
}

#[tokio::test]
async fn development_with_neither_keeps_the_configured_dev_server() {
    let build = build(None, None);
    let document = first_visit(config(&build, true)).await;
    assert!(
        document.contains(
            "<script type=\"module\" src=\"http://localhost:5999/@vite/client\"></script>"
        ),
        "{document}"
    );
    assert!(
        document
            .contains("<script type=\"module\" src=\"http://localhost:5999/src/a.ts\"></script>"),
        "{document}"
    );
    let tags = config(&build, true)
        .vite_tags(["src/a.ts"])
        .expect("the dev server needs no manifest");
    assert!(tags.contains("http://localhost:5999/src/a.ts"), "{tags}");
}

#[tokio::test]
async fn production_never_points_at_the_hot_file() {
    let build = build(Some(TWO_ENTRIES), Some("http://[::1]:5174"));
    let document = first_visit(config(&build, false)).await;
    assert!(!document.contains("[::1]:5174"), "{document}");
    assert!(!document.contains("@vite/client"), "{document}");
    assert!(document.contains("/assets/a-AAA111.js"), "{document}");

    let tags = config(&build, false)
        .vite_tags(["src/a.ts"])
        .expect("the manifest holds the entry");
    assert!(!tags.contains("[::1]:5174"), "{tags}");
}

#[tokio::test]
async fn production_without_a_manifest_keeps_the_shell_fallback() {
    let build = build(None, Some("http://[::1]:5174"));
    let document = first_visit(config(&build, false)).await;
    assert!(document.contains("/assets/main.js"), "{document}");
    assert!(document.contains("/assets/main.css"), "{document}");
    assert!(!document.contains("[::1]:5174"), "{document}");
}

// ---- The Vite facade --------------------------------------------------------

#[test]
fn the_facade_reads_the_installed_configuration() {
    let _container = TestContainer::fake();
    let build = build(Some(TWO_ENTRIES), Some(" http://[::1]:5174/ \n"));
    Inertia::install(&config(&build, true).entry_points(["src/b.ts"]))
        .expect("development installs without a manifest check");

    assert_eq!(Vite::hot_file(), build.hot);
    assert!(Vite::is_running_hot());
    assert_eq!(
        Vite::dev_server_url().as_deref(),
        Some("http://[::1]:5174/")
    );
    let html = Vite::to_html().expect("hot tags");
    assert!(html.contains("http://[::1]:5174/src/a.ts"), "{html}");
    assert!(html.contains("http://[::1]:5174/src/b.ts"), "{html}");
    let tags = Vite::tags(["src/app.css"]).expect("hot tags");
    assert!(tags.contains("http://[::1]:5174/src/app.css"), "{tags}");

    std::fs::remove_file(&build.hot).expect("remove the hot file");
    assert!(!Vite::is_running_hot());
    assert_eq!(Vite::dev_server_url(), None);
    let html = Vite::to_html().expect("the manifest holds both entries");
    assert!(html.contains("/assets/a-AAA111.js"), "{html}");
    assert!(html.contains("/assets/b-BBB111.js"), "{html}");
}

#[test]
fn the_facade_in_production_never_reads_the_hot_file() {
    let _container = TestContainer::fake();
    let build = build(Some(TWO_ENTRIES), Some("http://[::1]:5174"));
    Inertia::install(&config(&build, false)).expect("the manifest exists");

    assert!(!Vite::is_running_hot());
    assert_eq!(Vite::dev_server_url(), None);
    let html = Vite::to_html().expect("the manifest holds the entry");
    assert!(html.contains("/assets/a-AAA111.js"), "{html}");
    assert!(!html.contains("[::1]"), "{html}");
}
