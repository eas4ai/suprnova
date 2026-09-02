//! Regression: `#[suprnova::main]` must install Crypt before application
//! bootstrap code runs.

use suprnova::Crypt;

#[suprnova::main(flavor = "current_thread")]
async fn crypt_is_ready_at_runtime_entry() -> bool {
    Crypt::is_initialized()
}

#[test]
fn suprnova_main_initializes_crypt_before_the_async_body() {
    // SAFETY: this is the only test in this binary, so no other thread can
    // race these process-environment writes before the runtime is built.
    unsafe {
        std::env::set_var("APP_ENV", "testing");
        std::env::remove_var("APP_KEY");
        std::env::remove_var("APP_KEY_PREVIOUS");
        std::env::remove_var("APP_PREVIOUS_KEYS");
    }

    assert!(
        crypt_is_ready_at_runtime_entry(),
        "Crypt must be initialized before application bootstrap can initialize Magnetar"
    );
}
