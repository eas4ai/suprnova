//! A program that runs the same migrations with neither `#[suprnova::main]`
//! nor the documented call: the columns a program gets without the setting,
//! which the other two binaries are compared with.

fn main() -> std::process::ExitCode {
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
