//! A binary built with `#[suprnova::main]`. The macro reads
//! `unsigned_ids = true` from this package's
//! `[package.metadata.suprnova.schema]` and installs it before the body
//! runs, so the library's migrations create unsigned keys on MySQL.

#[suprnova::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    laravel_defaults_probe::run().await
}
