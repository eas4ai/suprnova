//! Cookie ciphertext is bound to its logical cookie name (v2 AAD).
//!
//! These are the mass-logout guards and the name-binding property itself: if
//! one fails, stop and report rather than weakening the test.
//!
//! A cookie value under the v1 label, which has no cookie name in it, opens
//! in no cookie. The tests of that need a key ring they know, and `Crypt`
//! holds one ring per process, installed by whichever test of this binary
//! gets there first. So each of them runs as the only test of a child
//! process of this binary and installs its own ring there; the parent test
//! runs the child and asserts that it passed.

#![cfg(feature = "testing")]

use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};

use suprnova::crypto::testing::{encrypt_string_for_under, encrypt_string_under};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{
    AadVersion, Auth, Cookie, CookiePrefix, Crypt, CryptPurpose, EncryptionKey, KeyOrigin,
    Middleware,
};

use crate::cookie_prefix_roundtrip::{MemoryStore, post_request};

static INSTALLED: LazyLock<()> = LazyLock::new(|| {
    let _ = suprnova::crypto::_test_install_key(EncryptionKey::generate());
});

fn init_crypt() {
    LazyLock::force(&INSTALLED);
}

/// The logical name of the session cookie of [`SessionConfig::default`].
const SESSION_COOKIE: &str = "suprnova_session";

/// Turns the `*_child` tests below from no-ops into their real body.
/// Only [`run_child`] sets it, in the environment of the process it starts.
const KNOWN_RING_CHILD: &str = "SUPRNOVA_COOKIE_KNOWN_RING_CHILD";

#[test]
fn ciphertext_from_one_cookie_fails_as_another() {
    init_crypt();
    let cookie = Cookie::encrypted("cookie_a", "payload").expect("encrypt");
    let wire = cookie.value().to_string();

    assert!(
        Cookie::read_encrypted_for("cookie_b", &wire).is_err(),
        "cookie_a's ciphertext must not decrypt as cookie_b"
    );
    assert_eq!(
        Cookie::read_encrypted_for("cookie_a", &wire).expect("own name decrypts"),
        "payload"
    );
}

#[test]
fn prefix_flip_does_not_invalidate_existing_cookies() {
    // Mass-logout guard. The AAD binds the logical name, so the wire-name
    // change a prefix flip causes must not change the AAD.
    init_crypt();
    let written_before_flip = Cookie::encrypted("suprnova_session", "sid.123").expect("encrypt");
    let wire = written_before_flip.value().to_string();
    let logical = CookiePrefix::strip("__Host-suprnova_session");

    assert_eq!(
        Cookie::read_encrypted_for(logical, &wire).expect("prefix flip is safe"),
        "sid.123"
    );
}

#[test]
fn a_v1_cookie_value_opens_under_no_key_of_the_ring() {
    run_child("cookie_name_bound_aad::cookie_reads_under_a_known_ring_child");
}

#[test]
fn a_v1_session_or_remember_cookie_is_a_request_without_it() {
    run_child("cookie_name_bound_aad::session_cookies_under_a_known_ring_child");
}

#[test]
fn cookie_reads_under_a_known_ring_child() {
    if std::env::var(KNOWN_RING_CHILD).is_err() {
        return;
    }
    let ring = KnownRing::install();

    // A name-bound value opens under either key of the ring, and says which.
    for (key, key_origin, plaintext) in [
        (&ring.current, KeyOrigin::Current, "bound-current"),
        (&ring.previous, KeyOrigin::Previous(0), "bound-previous"),
    ] {
        let wire = encrypt_string_for_under(key, CryptPurpose::Cookie, SESSION_COOKIE, plaintext)
            .expect("encrypt name-bound");
        assert_eq!(
            Cookie::read_encrypted_for(SESSION_COOKIE, &wire).expect("a name-bound value opens"),
            plaintext
        );
        let (_, origin) =
            Crypt::decrypt_string_for_with_origin(CryptPurpose::Cookie, SESSION_COOKIE, &wire)
                .expect("a name-bound value opens with its origin");
        assert_eq!(origin.key, key_origin);
        assert_eq!(origin.aad, AadVersion::Current);
    }

    // The same keys with the v1 label: nothing opens, and the error quotes
    // neither the value nor its plaintext.
    for key in [&ring.current, &ring.previous] {
        let wire =
            encrypt_string_under(key, CryptPurpose::Cookie, "v1-plaintext").expect("encrypt v1");
        let err = Cookie::read_encrypted_for(SESSION_COOKIE, &wire)
            .expect_err("a v1 cookie value must not open");
        let message = format!("{err}");
        assert!(message.contains("AEAD decrypt failed"), "{message}");
        assert!(!message.contains(&wire), "{message}");
        assert!(!message.contains("v1-plaintext"), "{message}");
    }

    // A stored value keeps its fallback: the v1 label of a cast column
    // under the previous key opens, and says so on both axes.
    let stored = encrypt_string_under(&ring.previous, CryptPurpose::Cast, "stored")
        .expect("encrypt v1 cast value");
    let (plain, origin) =
        Crypt::decrypt_string_for_with_origin(CryptPurpose::Cast, "users.secret", &stored)
            .expect("the fallback opens a v1 stored value");
    assert_eq!(plain, "stored");
    assert_eq!(origin.key, KeyOrigin::Previous(0));
    assert_eq!(origin.aad, AadVersion::Legacy);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_cookies_under_a_known_ring_child() {
    if std::env::var(KNOWN_RING_CHILD).is_err() {
        return;
    }
    let ring = KnownRing::install();
    let session_id = "k".repeat(40);
    let store = Arc::new(MemoryStore::default());
    let mut stored = SessionData::new(session_id.clone(), "csrf-token".to_string());
    stored.user_id = Some("user-7".to_string());
    stored.put("marker", true);
    store.write(&stored).await.expect("seed the stored session");
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let middleware = SessionMiddleware::with_store(config, store);

    // Control: the name-bound session cookie loads the stored session and
    // its user, so the ring of this process is the one the keys make.
    let bound = encrypt_string_for_under(
        &ring.current,
        CryptPurpose::Cookie,
        SESSION_COOKIE,
        &session_id,
    )
    .expect("encrypt name-bound session cookie");
    let seen = send(&middleware, SESSION_COOKIE, &bound).await;
    assert_eq!(seen.status, 200);
    assert_eq!(seen.marker, Some(true));
    assert_eq!(seen.user_id.as_deref(), Some("user-7"));

    // The v1 label under either key: a request without a session.
    for key in [&ring.current, &ring.previous] {
        let v1 = encrypt_string_under(key, CryptPurpose::Cookie, &session_id).expect("encrypt v1");
        let seen = send(&middleware, SESSION_COOKIE, &v1).await;
        assert_eq!(
            seen.status, 200,
            "a v1 session cookie must not fail the request"
        );
        assert_eq!(
            seen.marker, None,
            "a v1 session cookie must not load the session"
        );
        assert_eq!(
            seen.user_id, None,
            "a v1 session cookie must not sign anyone in"
        );
    }

    // A v1 remember-me cookie: no sign-in, and the response clears it.
    let remember = suprnova::auth::remember::COOKIE_NAME;
    let v1 = encrypt_string_under(&ring.current, CryptPurpose::Cookie, "selector.verifier")
        .expect("encrypt v1 remember cookie");
    let seen = send(&middleware, remember, &v1).await;
    assert_eq!(
        seen.status, 200,
        "a v1 remember cookie must not fail the request"
    );
    assert_eq!(
        seen.user_id, None,
        "a v1 remember cookie must not sign anyone in"
    );
    let cleared = format!("{remember}=");
    assert!(
        seen.set_cookies
            .iter()
            .any(|cookie| cookie.starts_with(&cleared) && cookie.contains("Max-Age=0")),
        "the response must clear the v1 remember cookie: {:?}",
        seen.set_cookies
    );
}

/// The ring a child installs: `current` is `APP_KEY`, `previous` the one
/// entry of `APP_KEY_PREVIOUS`.
struct KnownRing {
    current: EncryptionKey,
    previous: EncryptionKey,
}

impl KnownRing {
    /// Generate both keys and install them as the ring of this process.
    /// Only a child calls it, as the first thing it does, so every read
    /// in the child uses this ring.
    fn install() -> Self {
        let ring = Self {
            current: EncryptionKey::generate(),
            previous: EncryptionKey::generate(),
        };
        let installed = suprnova::testing::install_test_encryption_keyring(
            ring.current.clone(),
            vec![ring.previous.clone()],
        );
        assert!(
            installed,
            "a child installs its ring before anything else does"
        );
        ring
    }
}

/// Run the test `name` of this binary as the only test of a child
/// process, with [`KNOWN_RING_CHILD`] set, and fail unless it ran and
/// passed.
fn run_child(name: &str) {
    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--exact", name, "--nocapture"])
        .env(KNOWN_RING_CHILD, "1")
        .output()
        .expect("spawn the known-ring child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("running 1 test"),
        "the child filter matched no test (the module path changed?); stdout:\n{stdout}"
    );
    assert!(
        output.status.success(),
        "the child failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// What the handler behind the session middleware saw, and what the
/// middleware answered.
struct Seen {
    status: u16,
    marker: Option<bool>,
    user_id: Option<String>,
    set_cookies: Vec<String>,
}

/// The `marker` value of the session and the user id, as the handler saw
/// them.
type Observation = (Option<bool>, Option<String>);

/// Send one request that carries the cookie `name=value` through
/// `middleware`.
async fn send(middleware: &SessionMiddleware, name: &str, value: &str) -> Seen {
    let observed: Arc<Mutex<Observation>> = Arc::new(Mutex::new((None, None)));
    let observed_in_handler = observed.clone();
    let next: suprnova::middleware::Next = Arc::new(move |_request| {
        let observed = observed_in_handler.clone();
        Box::pin(async move {
            let marker =
                suprnova::session::session().and_then(|session| session.get::<bool>("marker"));
            *observed.lock().unwrap() = (marker, Auth::id());
            Ok(suprnova::HttpResponse::text("handled"))
        })
    });
    let request = post_request(Some((name, value))).await;
    let response = match middleware.handle(request, next).await {
        Ok(response) | Err(response) => response.into_hyper(),
    };
    let set_cookies = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|header| header.to_str().ok())
        .map(str::to_owned)
        .collect();
    let (marker, user_id) = observed.lock().unwrap().clone();
    Seen {
        status: response.status().as_u16(),
        marker,
        user_id,
        set_cookies,
    }
}
