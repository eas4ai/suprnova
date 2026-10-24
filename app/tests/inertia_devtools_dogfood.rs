//! The dogfood app's users directory recorded by Inertia DevTools: the
//! app's own routes, Inertia configuration and shared data, with DevTools
//! switched on for this binary only and storing into a temporary
//! directory.
//!
//! The entry has to name the app's own code: the render call in
//! `src/controllers/user.rs`, the `Inertia::share` in `src/bootstrap.rs`,
//! the page file under `frontend/src/pages`.

use std::path::Path;
use std::sync::Arc;

use app::models::users::User;
use serde_json::Value;
use suprnova::testing::TestClient;
use suprnova::{DevToolsConfig, Inertia, MiddlewareRegistry, Model, Request, Response, Router};

/// How many users the fixture database holds.
const SEEDED_USERS: i64 = 3;

static SHARED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
static DB: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// One file-backed database for the binary, seeded once, under its own
/// name; see `paginated_users_e2e.rs` for why a file.
async fn ensure_seeded_db() {
    DB.get_or_init(|| async {
        let db_path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("inertia-devtools-dogfood.sqlite");
        for suffix in ["", "-wal", "-shm"] {
            let mut path = db_path.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(std::path::PathBuf::from(path));
        }
        let config = suprnova::database::DatabaseConfig::builder()
            .url(format!("sqlite://{}?mode=rwc", db_path.display()))
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = suprnova::database::DbConnection::connect(&config)
            .await
            .unwrap_or_else(|e| panic!("connect fixture sqlite at {}: {e}", db_path.display()));
        <app::migrations::Migrator as sea_orm_migration::MigratorTrait>::up(conn.inner(), None)
            .await
            .unwrap_or_else(|e| panic!("migrate fixture sqlite at {}: {e}", db_path.display()));
        suprnova::App::singleton(conn);
        for i in 1..=SEEDED_USERS {
            User::create(suprnova::attrs! {
                name: format!("user-{i:03}"),
                email: format!("user-{i:03}@example.com"),
                password: "pw",
            })
            .await
            .expect("seed user");
        }
    })
    .await;
}

/// A page rendered with `inertia_response!`, whose component the macro
/// checks against the app's `frontend/src/pages` when this file compiles.
async fn macro_page(req: Request) -> Response {
    suprnova::inertia_response!(&req, "Users/Index", { "users": [] })
}

/// The app's routes, and the macro page, behind the app's Inertia
/// configuration with DevTools recording into `dir`.
async fn recording_client(dir: &Path) -> TestClient {
    suprnova::testing::install_test_encryption_key();
    ensure_seeded_db().await;
    suprnova::App::bind_if_absent::<dyn suprnova::CacheStore>(Arc::new(
        suprnova::InMemoryCache::new(),
    ));
    SHARED.get_or_init(app::bootstrap::register_inertia_shared_data);
    let config = app::bootstrap::inertia_config()
        .devtools(DevToolsConfig::new().enabled(true).storage_path(dir));
    let router: Router = app::routes::register()
        .get("/macro-page", macro_page)
        .into();
    TestClient::new(
        router,
        MiddlewareRegistry::new().append(Inertia::middleware(&config)),
    )
}

/// The 1-based line of the first line of the app source file `file` that
/// contains `needle`.
fn line_of(file: &str, needle: &str) -> u64 {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(file))
        .unwrap_or_else(|e| panic!("read {file}: {e}"));
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle} is not in {file}")) as u64
        + 1
}

/// The stored entry a response names.
fn entry(dir: &Path, response: &suprnova::testing::TestResponse) -> Value {
    let id = response
        .header("x-inertia-devtools-id")
        .expect("a recorded response carries its entry id");
    let bytes = std::fs::read(dir.join(format!("{id}.json"))).expect("the entry file");
    serde_json::from_slice(&bytes).expect("an entry that is JSON")
}

/// One test on purpose: nextest runs every test in its own process, and
/// two processes would rebuild the one fixture file under each other.
#[tokio::test]
async fn indt_the_users_directory_is_recorded_with_the_apps_own_sources() {
    let dir = tempfile::tempdir().unwrap();
    let client = recording_client(dir.path()).await;

    let visit = client
        .get("/users")
        .inertia()
        .header("X-Inertia-Devtools-Tab", "dogfood-tab")
        .send()
        .await;
    visit.assert_ok();
    assert_eq!(
        visit.header("x-inertia-devtools-parent-out"),
        visit.header("x-inertia-devtools-id")
    );
    let recorded = entry(dir.path(), &visit);
    let meta = &recorded["__meta"];
    assert_eq!(meta["component"], "Users/Index");
    assert_eq!(meta["requestType"], "navigate");
    assert_eq!(meta["tabUuid"], "dogfood-tab");
    assert_eq!(meta["status"], 200);
    // `GET /users` and `POST /users` share the pattern; each is recorded
    // with the name of its own method's route.
    assert_eq!(
        recorded["route"]["name"], "users.index",
        "{}",
        recorded["route"]
    );
    assert_eq!(recorded["route"]["uri"], "/users", "{}", recorded["route"]);
    assert_eq!(recorded["route"]["method"], "GET");
    let action = recorded["route"]["action"].as_str().unwrap();
    assert!(action.ends_with("controllers::user::index"), "{action}");

    let render = &recorded["renderSource"];
    assert!(
        render["file"]
            .as_str()
            .unwrap()
            .ends_with("src/controllers/user.rs"),
        "{render}"
    );
    assert_eq!(
        render["line"],
        line_of(
            "src/controllers/user.rs",
            "Inertia::paginate(\"Users/Index\""
        )
    );
    let page_file = recorded["componentPath"].as_str().unwrap();
    assert!(page_file.ends_with("Users/Index.svelte"), "{page_file}");

    let props = &recorded["props"];
    assert_eq!(props["users"]["inertiaType"], "scroll");
    assert_eq!(props["appName"]["shared"], true);
    let shared_at = &props["appName"]["shareSource"];
    assert!(
        shared_at["file"]
            .as_str()
            .unwrap()
            .ends_with("src/bootstrap.rs"),
        "{shared_at}"
    );
    assert_eq!(
        shared_at["line"],
        line_of("src/bootstrap.rs", "Inertia::share(\"appName\"")
    );
    assert_eq!(recorded["propValues"]["appName"], "Suprnova");

    let store = client.post("/users").inertia().send().await;
    store.assert_status(302);
    let stored = entry(dir.path(), &store);
    assert_eq!(
        stored["route"]["name"], "users.store",
        "{}",
        stored["route"]
    );
    assert_eq!(stored["route"]["uri"], "/users", "{}", stored["route"]);

    let first = client.get("/users").send().await;
    first.assert_ok();
    let id = first.header("x-inertia-devtools-id").unwrap();
    assert!(
        first.body_text().contains(&format!(
            "<script data-inertia-devtools-id type=\"application/json\">\"{id}\"</script></body>"
        )),
        "the first visit's document carries the id tag"
    );
    assert_eq!(
        entry(dir.path(), &first)["__meta"]["requestType"],
        "initial"
    );

    let rendered = client.get("/macro-page").inertia().send().await;
    rendered.assert_ok();
    let recorded = entry(dir.path(), &rendered);
    let render = &recorded["renderSource"];
    assert!(
        render["file"]
            .as_str()
            .unwrap()
            .ends_with("tests/inertia_devtools_dogfood.rs"),
        "{render}"
    );
    let macro_line = line_of(
        "tests/inertia_devtools_dogfood.rs",
        "inertia_response!(&req, \"Users/Index\"",
    );
    assert_eq!(render["line"], macro_line, "the macro's call site");
    assert_eq!(
        recorded["props"]["users"]["renderSource"]["line"],
        macro_line
    );
}
