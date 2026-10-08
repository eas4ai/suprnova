//! `suprnova ssr:start` - run the Inertia SSR worker in the foreground.
//!
//! The application binary's `ssr:start` with the configuration from the
//! flags and the `SUPRNOVA_SSR_*` environment (see [`super::ssr_config`]):
//! it refuses without a bundle or, under
//! `SUPRNOVA_SSR_ENSURE_RUNTIME_EXISTS`, without the runtime, stops a worker
//! still running at `SUPRNOVA_SSR_URL`, and runs `runtime bundle`, forwarding
//! `SIGINT` and `SIGTERM` to it and exiting with its status. Run it under
//! systemd, pm2 or supervisord in production.

use super::ssr_config::{self, Flags};

pub fn run(runtime: Option<String>, bundle: Option<String>) {
    ssr_config::run(
        Flags {
            bundle,
            ..Flags::default()
        },
        |config| async move {
            let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
            suprnova::console::ssr::start(&config, runtime, &mut out, &mut err).await
        },
    )
}
