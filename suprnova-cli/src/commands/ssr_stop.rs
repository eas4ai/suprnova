//! `suprnova ssr:stop` - ask the Inertia SSR worker to shut down.
//!
//! The application binary's `ssr:stop` with the configuration from the
//! flags and the `SUPRNOVA_SSR_*` environment (see [`super::ssr_config`]):
//! `GET {url}/shutdown`, which the worker answers by exiting. With
//! `--graceful`, a worker that is not running is success, so a deploy script
//! can stop a worker that may never have started.

use std::time::Duration;

use super::ssr_config::{self, Flags};

pub fn run(url: Option<String>, timeout_ms: u64, graceful: bool) {
    ssr_config::run(
        Flags {
            url,
            timeout: Some(Duration::from_millis(timeout_ms)),
            ..Flags::default()
        },
        |config| async move {
            let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
            suprnova::console::ssr::stop(&config, graceful, &mut out, &mut err).await
        },
    )
}
