//! Fresh processes prove provider construction is not supplied by another test.
use std::process::Command;
use suprnova::SearchRegistry;

const CHILD_MODE: &str = "SUPRNOVA_SEARCH_BOOT_CHILD";

#[test]
fn provider_boot_child() {
    if std::env::var_os(CHILD_MODE).is_none() {
        return;
    }
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
        suprnova::Application::new()
            .run_with_args(["app", "schedule:run"])
            .await
            .expect("worker validation must find the environment-selected provider");
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
