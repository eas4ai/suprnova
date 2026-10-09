//! The dogfood app's users directory driven through `TestClient`: the
//! app's own routes behind the middleware stack its bootstrap registers,
//! with no port and no hand-written HTTP harness.
//!
//! `Inertia::install` runs inside the bootstrap, so the component
//! assertions also check `frontend/src/pages/Users/Index.svelte` exists.

use std::sync::Arc;

use app::models::users::User;
use suprnova::testing::TestClient;
use suprnova::{EncryptionKey, MiddlewareRegistry, Model};

/// How many users the fixture database holds: more than one page of 20.
const SEEDED_USERS: i64 = 25;

static CRYPT_INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
static STACK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
static DB: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// The session cookie is encrypted, and so is a cursor.
fn ensure_crypt_initialised() {
    CRYPT_INIT.get_or_init(|| {
        suprnova::Crypt::init(EncryptionKey::generate());
    });
}

/// One file-backed database for the binary, seeded once; see
/// `paginated_users_e2e.rs` for why a file and not a shared in-memory
/// database. Its own file name, so the two binaries never share one.
async fn ensure_seeded_db() {
    DB.get_or_init(|| async {
        let db_path =
            std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("inertia-test-client.sqlite");
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

/// The app's router behind the app's whole middleware stack.
async fn app_client() -> TestClient {
    ensure_crypt_initialised();
    ensure_seeded_db().await;
    suprnova::App::bind_if_absent::<dyn suprnova::CacheStore>(Arc::new(
        suprnova::InMemoryCache::new(),
    ));
    // The stack goes on the process-global registry; once per process, so
    // tests sharing one never stack it twice.
    STACK.get_or_init(|| {
        app::bootstrap::register_http_stack();
        app::bootstrap::register_inertia_shared_data();
    });
    TestClient::new(app::routes::register(), MiddlewareRegistry::from_global())
}

/// One test on purpose: nextest runs every test in its own process, and
/// two processes would rebuild the one fixture file under each other.
#[tokio::test]
async fn intt_users_directory_through_the_test_client() {
    let client = app_client().await;

    // An Inertia visit, asserted with scopes over the rows.
    client
        .get("/users")
        .inertia()
        .send()
        .await
        .assert_ok()
        .assert_inertia_with(|page| {
            page.component("Users/Index")
                .url("/users")
                .where_("appName", "Suprnova")
                .has_count_with("users", 20, |user| {
                    user.where_("id", 1)
                        .where_("name", "user-001")
                        .missing("email")
                        .etc();
                })
                .scope("users", |users| {
                    users.each(|user| {
                        user.where_type("id", "integer").missing("email").etc();
                    });
                });
        });

    // A first visit is the HTML document; its page reloads through the
    // same client with nothing attached.
    let page = client
        .get("/users")
        .send()
        .await
        .assert_ok()
        .assert_inertia();
    page.component("Users/Index").count_between("users", 1, 20);
    page.reload_only(["users"])
        .await
        .where_("users.19.id", 20)
        .where_type("users.0.name", "string");
}
