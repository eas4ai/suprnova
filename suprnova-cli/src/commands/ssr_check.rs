//! `suprnova ssr:check` - check the Inertia SSR worker's health.
//!
//! The application binary's `ssr:check` with the configuration from the
//! flags and the `SUPRNOVA_SSR_*` environment (see [`super::ssr_config`]):
//! the SSR gateway's health check, `GET {url}/health` answering 2xx, which
//! every `@inertiajs/*/server` `createServer()` bundle serves. Laravel's
//! `inertia:check-ssr` asks the same question.
//!
//! Use it in CI or a deploy pipeline's smoke test:
//!
//! ```bash
//! suprnova ssr:start &
//! ./wait-until.sh suprnova ssr:check
//! # ...run e2e tests...
//! ```

use std::time::Duration;

use super::ssr_config::{self, Flags};

pub fn run(url: Option<String>, timeout_ms: u64) {
    ssr_config::run(
        Flags {
            url,
            timeout: Some(Duration::from_millis(timeout_ms)),
            ..Flags::default()
        },
        |config| async move {
            let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
            suprnova::console::ssr::check(&config, &mut out, &mut err).await
        },
    )
}
