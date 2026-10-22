//! Fresh processes prove provider construction is not supplied by another test.
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use suprnova::{Application, FrameworkError, SearchRegistry, Searchable};

const CHILD_MODE: &str = "SUPRNOVA_SEARCH_BOOT_CHILD";
static INITIALIZED: AtomicBool = AtomicBool::new(false);

#[suprnova::command(name = "search-boot:noop", description = "Checks search boot")]
async fn provider_boot_noop(_args: Vec<String>) -> Result<(), FrameworkError> {
    Ok(())
}

async fn initialize_generation() {
    use sea_orm_migration::MigratorTrait;
    suprnova::DB::init_with(
        suprnova::DatabaseConfig::builder()
            .url("sqlite::memory:")
            .build(),
    )
    .await
    .unwrap();
    let connection = suprnova::DB::connection().unwrap();
    super::database_support::Migrator::up(connection.inner(), None)
        .await
        .unwrap();
    let index = super::derive::SearchNote::descriptor().name;
    let engine = SearchRegistry::engine_for(index).unwrap();
    assert!(engine.current_generation(index).await.unwrap().is_none());
    suprnova::search::generations::create_index(index)
        .await
        .unwrap();
    let generation = engine.current_generation(index).await.unwrap().unwrap();
    let settings = engine
        .read_settings(super::derive::SearchNote::descriptor(), &generation)
        .await
        .unwrap();
    assert_eq!(settings.index_time.model.as_deref(), Some("boot-model"));
    INITIALIZED.store(true, Ordering::SeqCst);
}

#[test]
fn provider_boot_child() {
    let Ok(mode) = std::env::var(CHILD_MODE) else {
        return;
    };
    suprnova::boot::load_env().expect("load configuration before building the runtime");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        assert!(SearchRegistry::embedder_named("boot", "http").is_err());
        // The shared integration binary also inventories the media fixture. Give
        // that fixture its own provider without pre-registering the HTTP provider.
        #[cfg(feature = "search-media")]
        {
            use suprnova::search::Embedder;
            let mut profile = suprnova::FakeEmbedder::new(8).unwrap().profile().clone();
            profile.inputs.image = true;
            profile.inputs.audio = true;
            profile.inputs.video = true;
            SearchRegistry::register_embedder(
                "media-test",
                std::sync::Arc::new(suprnova::FakeEmbedder::with_profile(profile).unwrap()),
            )
            .unwrap();
        }
        if mode == "worker-validation" {
            Application::new()
                .run_with_args(["app", "schedule:run"])
                .await
                .expect("worker validation must find the environment-selected provider");
        } else if mode == "worker-bootstrap" {
            Application::new()
                .bootstrap(initialize_generation)
                .run_with_args(["app", "schedule:run"])
                .await
                .expect("the application hook must be able to provision its first generation");
            assert!(INITIALIZED.load(Ordering::SeqCst));
        } else if mode == "console-bootstrap" {
            suprnova::console::dispatch_argv_with_init(
                vec!["console".into(), "search-boot:noop".into()],
                initialize_generation,
            )
            .await
            .expect("the console hook must be able to provision its first generation");
            assert!(INITIALIZED.load(Ordering::SeqCst));
        } else if mode == "server-bootstrap" {
            let result = Application::new()
                .bootstrap(initialize_generation)
                .try_routes_async(|| async {
                    Err(FrameworkError::param("search boot reached routes"))
                })
                .run_with_args(["app", "serve", "--no-migrate"])
                .await;
            let error = result.unwrap_err().to_string();
            assert!(error.contains("search boot reached routes"), "{error}");
            assert!(INITIALIZED.load(Ordering::SeqCst));
        } else {
            panic!("unknown child mode: {mode}");
        }
        assert_eq!(
            SearchRegistry::embedder_named("boot", "http")
                .unwrap()
                .profile()
                .id,
            "boot-model"
        );
    });
}

fn run_child(mode: &str) {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "boot::provider_boot_child", "--nocapture"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env(CHILD_MODE, mode)
        .env("APP_ENV", "testing")
        .env("RUST_MIN_STACK", "33554432")
        .env("APP_KEY", suprnova::EncryptionKey::generate().to_base64())
        .env("SEARCH_DRIVER", "collection")
        .env("SEARCH_EMBEDDER", "http")
        .env("SEARCH_EMBED_BASE_URL", "http://127.0.0.1:9/v1")
        .env("SEARCH_EMBED_MODEL", "boot-model")
        .env("SEARCH_EMBED_DIMENSIONS", "8")
        .env("QUEUE_DRIVER", "memory")
        .env("CACHE_DRIVER", "memory")
        .env("MAIL_DRIVER", "log")
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{mode}: {}\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("running 1 test"),
        "child filter matched no test: {stdout}"
    );
}

#[test]
fn worker_boot_constructs_environment_provider_before_validation() {
    run_child("worker-validation");
}

#[test]
fn worker_boot_provisions_first_generation_in_application_hook() {
    run_child("worker-bootstrap");
}

#[test]
fn server_boot_provisions_first_generation_in_application_hook() {
    run_child("server-bootstrap");
}

#[test]
fn console_boot_provisions_first_generation_in_application_hook() {
    run_child("console-bootstrap");
}
