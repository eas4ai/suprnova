//! Testing utilities for suprnova framework
//!
//! Provides Jest-like testing helpers including:
//! - `expect!` macro for fluent assertions with clear expected/received output
//! - `describe!` and `test!` macros for test organization
//! - `TestDatabase` for isolated database tests
//! - `TestContainer` for dependency injection in tests
//! - `TestClock` to move the time the framework reads
//! - `TestClient` to drive HTTP requests through the application in-process
//!
//! # Example
//!
//! ```rust,ignore
//! use suprnova::{describe, test, expect};
//! use suprnova::testing::TestDatabase;
//!
//! describe!("UserService", {
//!     test!("creates a user", async fn(db: TestDatabase) {
//!         let service = UserService::new();
//!         let user = service.create("test@example.com").await.unwrap();
//!
//!         expect!(user.email).to_equal("test@example.com".to_string());
//!     });
//! });
//! ```

mod client;
mod expect;
mod inertia;
mod response;

#[cfg(any(test, feature = "testing"))]
pub use crate::clock::{TestClock, TestClockGuard, TestClockHandle};
pub use crate::container::testing::{TestContainer, TestContainerGuard};
pub use crate::database::testing::TestDatabase;
pub use crate::page_budget::{BudgetFinding, PageAudit, PageBudget};
pub use client::{ActingUser, TestClient, TestRequest};
pub use expect::{Expect, set_current_test_name};
pub use inertia::{AssertableInertia, ReloadRequest};
pub use response::TestResponse;

#[cfg(any(test, feature = "testing"))]
use crate::crypto::EncryptionKey;

/// Install a deterministic encryption key for tests. Idempotent - the
/// underlying `Crypt` facade is `OnceLock`-backed, so the second call is
/// a no-op and safe to call from every test that touches an encrypted
/// cast. The chosen key is a fixed 32-byte zero key encoded as URL-safe
/// base64 (no padding), giving deterministic ciphertext behaviour
/// across runs.
///
/// **Test-only.** Bypasses the production APP_KEY validation path.
/// Production code must go through `Crypt::init` from
/// `Server::from_config` instead.
#[cfg(any(test, feature = "testing"))]
pub fn install_test_encryption_key() {
    // 43 chars × 6 bits = 258 bits ≈ 32 bytes (the trailing 2 bits are
    // ignored). URL_SAFE_NO_PAD decode of "A" * 43 yields 32 zero bytes.
    const TEST_KEY_B64: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let key = EncryptionKey::from_base64(TEST_KEY_B64)
        .expect("32-byte test key parses from canonical zero-base64");
    // `_test_install_key` (in `crate::crypto`) returns false if a key
    // was already installed - we ignore the return because idempotent
    // installation is the contract.
    let _ = crate::crypto::_test_install_key(key);
}

/// Let every router this test process builds sign agent auth's identity
/// assertions with a key made for the process, when the environment names
/// none (`AGENT_AUTH_KEY_ID` and `AGENT_AUTH_PRIVATE_KEY`).
///
/// A router with agent auth on refuses to build without a signing key, in
/// every environment. A test of an application that serves agents calls
/// this once, before it builds the application's router, rather than
/// carrying a key: the key is generated, so no private key sits in the
/// source for a scanner to flag or anyone to sign with. It is the same key
/// for the rest of the process, so one test's nodes verify each other's
/// assertions. A key the environment names still wins.
///
/// **Test-only.** It exists with the `testing` feature, which an
/// application's release build turns off.
#[cfg(all(any(test, feature = "testing"), feature = "agents"))]
pub fn install_test_agent_auth_key() {
    crate::agents::agent_auth::install_test_key();
}

/// Install a key *ring* (current + previous fallback list) for tests
/// that exercise `APP_KEY` rotation. Same one-shot `OnceLock` contract
/// as [`install_test_encryption_key`] - the first installer in a test
/// binary wins; subsequent calls are no-ops. Returns `true` iff this
/// call actually installed the ring.
///
/// Test binaries that rotate keys at runtime cannot install twice;
/// they must install the *final* ring once, then use
/// [`crate::crypto::testing::encrypt_string_under`] to mint ciphertext
/// under arbitrary keys (simulating data written when the old key was
/// current).
///
/// **Test-only.** Production code must go through
/// `Crypt::init_with_keyring` via `Server::from_config`.
#[cfg(any(test, feature = "testing"))]
pub fn install_test_encryption_keyring(
    current: EncryptionKey,
    previous: Vec<EncryptionKey>,
) -> bool {
    crate::crypto::_test_install_keyring(current, previous)
}
