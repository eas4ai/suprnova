//! Opening a `redis` client, ready for a `rediss://` URL.
//!
//! A TLS connection builds its configuration from rustls's process-wide
//! crypto provider. rustls picks one by itself only when exactly one is
//! compiled in, and this crate's tree carries both `ring` and `aws-lc-rs`,
//! so without an installed provider the first `rediss://` connection would
//! panic. Every Redis client the framework opens goes through [`open`],
//! which installs `ring`'s provider first unless the application installed
//! one of its own.

use std::sync::Once;

/// Install `ring`'s rustls provider, once, when none is installed yet.
pub(crate) fn ensure_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            // Another thread may install one in between; either is fine.
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
    });
}

/// A client for `url`, with the crypto provider a `rediss://` URL needs.
pub(crate) fn open(url: &str) -> redis::RedisResult<redis::Client> {
    ensure_crypto_provider();
    redis::Client::open(url)
}
