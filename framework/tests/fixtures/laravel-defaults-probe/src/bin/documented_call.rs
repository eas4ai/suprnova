//! A program that runs migrations without `#[suprnova::main]`, so nothing
//! reads the package's settings for it: it installs `unsigned_ids` with
//! the one documented call, before its first migration.

fn main() -> std::process::ExitCode {
    suprnova::schema::Schema::use_unsigned_ids();
    match suprnova::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(laravel_defaults_probe::run()),
        Err(error) => {
            eprintln!("could not build the Tokio runtime: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
