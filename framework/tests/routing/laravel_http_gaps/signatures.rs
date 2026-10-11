//! PAR-116: the `ValidateSignature` route middleware, its alias, the
//! parameters it ignores, and `url::has_valid_signature_ignoring`.
//!
//! Every route is served on a real socket by the suite's `server` helper,
//! so a request crosses the middleware chain as it does in production.
//! Route names start with `par116.` and paths with `/par116/`, so no other
//! test of the binary collides with them.

use std::net::SocketAddr;

use suprnova::middleware::register_middleware_alias_with_args;
use suprnova::routing::{SignatureVerdict, ValidateSignature, verify_signature_ignoring};
use suprnova::{Crypt, EncryptionKey, HttpResponse, Request, Router, url};

fn ensure_crypt() {
    if !Crypt::is_initialized() {
        Crypt::init(EncryptionKey::generate());
    }
}

fn now() -> i64 {
    suprnova::clock::now().timestamp()
}

/// The handler every guarded route runs: it answers `ran` and the `user`
/// parameter it read, so a test sees both that it ran and on what.
async fn guarded(request: Request) -> suprnova::Response {
    Ok(HttpResponse::text(format!(
        "ran:{}",
        request.query_param("user").as_deref().unwrap_or("none")
    )))
}

/// One route per way of building the middleware.
fn router() -> Router {
    register_middleware_alias_with_args("signed", ValidateSignature::from_alias_args);
    Router::new()
        .get("/par116/plain/{id}", guarded)
        .middleware(ValidateSignature::new())
        .name("par116.plain")
        .get("/par116/ignoring", guarded)
        .middleware(ValidateSignature::new().ignore(["utm_source"]))
        .name("par116.ignoring")
        .get("/par116/relative", guarded)
        .middleware(ValidateSignature::relative(["utm_source"]))
        .name("par116.relative")
        .get("/par116/alias-relative", guarded)
        .middleware_named("signed:relative,utm_source")
        .name("par116.alias.relative")
        .get("/par116/alias-plain", guarded)
        .middleware_named("signed")
        .name("par116.alias.plain")
}

async fn serve() -> SocketAddr {
    ensure_crypt();
    crate::laravel_delta::server(router()).await
}

async fn get(address: SocketAddr, target: &str) -> (u16, String) {
    let (status, _, body) = crate::http_wire::request(address, "GET", target, &[]).await;
    (status, body)
}

/// The refusal Laravel's `InvalidSignatureException` gives: 403 and the
/// message `Invalid signature.` in the framework's usual JSON error body.
fn assert_refused(reply: (u16, String), what: &str) {
    let (status, body) = reply;
    assert_eq!(status, 403, "{what}: {body}");
    let json: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|error| panic!("{what}: {error}: {body}"));
    assert_eq!(json["message"], "Invalid signature.", "{what}: {body}");
    assert!(!body.contains("ran:"), "{what}: the handler ran: {body}");
}

fn signature_of(signed: &str) -> &str {
    signed
        .rsplit_once("signature=")
        .expect("a signed URL carries a signature")
        .1
}

#[tokio::test]
async fn a_valid_signed_url_runs_the_handler() {
    let address = serve().await;
    let signed = url::signed_url("/par116/plain/7?user=alice", None).expect("sign");
    assert_eq!(get(address, &signed).await, (200, "ran:alice".to_string()));

    let temporary =
        url::temporary_signed_route("par116.plain", &[("id", "7")], now() + 3600).expect("sign");
    assert_eq!(
        get(address, &temporary).await,
        (200, "ran:none".to_string())
    );
}

#[tokio::test]
async fn the_signed_text_is_key_sorted_so_reordered_keys_still_pass() {
    let address = serve().await;
    let signed = url::signed_url("/par116/plain/7?b=2&a=1&user=alice", None).expect("sign");
    let signature = signature_of(&signed);
    let reordered = format!("/par116/plain/7?user=alice&b=2&a=1&signature={signature}");
    assert_eq!(
        get(address, &reordered).await,
        (200, "ran:alice".to_string())
    );
}

#[tokio::test]
async fn a_changed_query_is_refused() {
    let address = serve().await;
    let signed = url::signed_url("/par116/plain/7?user=alice", None).expect("sign");
    assert_refused(
        get(address, &signed.replace("user=alice", "user=eve")).await,
        "a changed value",
    );
    assert_refused(
        get(address, &signed.replace("/plain/7?", "/plain/7?user=eve&")).await,
        "a prepended duplicate",
    );
    assert_refused(
        get(address, &signed.replace("/plain/7", "/plain/8")).await,
        "a changed path",
    );
}

#[tokio::test]
async fn an_expired_temporary_signed_route_is_refused_as_invalid() {
    let address = serve().await;
    let expired =
        url::temporary_signed_route("par116.plain", &[("id", "7")], now() - 60).expect("sign");
    assert_refused(get(address, &expired).await, "an expired URL");

    // The middleware answers expired and forged alike; the verdict still
    // tells them apart for a handler that wants to offer a fresh link.
    assert_eq!(
        url::signature_verdict(&Request::for_test("GET", &expired)).expect("verdict"),
        SignatureVerdict::Expired
    );
    let forged = format!("/par116/plain/7?signature={}", "0".repeat(64));
    assert_eq!(
        url::signature_verdict(&Request::for_test("GET", &forged)).expect("verdict"),
        SignatureVerdict::Invalid
    );
}

#[tokio::test]
async fn a_request_without_a_signature_is_refused() {
    let address = serve().await;
    assert_refused(get(address, "/par116/plain/7").await, "no query");
    assert_refused(
        get(address, "/par116/plain/7?user=alice").await,
        "a query without a signature",
    );
    assert_refused(
        get(address, "/par116/plain/7?signature=not-hex").await,
        "a malformed signature",
    );
}

#[tokio::test]
async fn a_repeated_signature_is_refused() {
    let address = serve().await;
    let signed = url::signed_url("/par116/ignoring?user=alice", None).expect("sign");
    let signature = signature_of(&signed);
    for target in [
        format!("{signed}&signature={signature}"),
        format!("{signed}&signature=deadbeef"),
        format!("/par116/ignoring?user=alice&signature=deadbeef&signature={signature}"),
    ] {
        assert_refused(get(address, &target).await, &target);
        assert!(
            !url::has_valid_signature_ignoring(&Request::for_test("GET", &target), &["utm_source"])
                .expect("verify"),
            "{target}"
        );
    }
    // A repeated expires is refused too, ignored or not.
    let temporary = url::signed_url("/par116/ignoring", Some(now() + 3600)).expect("sign");
    let doubled = temporary.replace("expires=", "expires=1&expires=");
    assert_refused(get(address, &doubled).await, "a repeated expires");
}

#[tokio::test]
async fn ignore_lets_an_appended_parameter_through() {
    let address = serve().await;
    let signed = url::signed_url("/par116/ignoring?user=alice", None).expect("sign");
    assert_eq!(
        get(address, &format!("{signed}&utm_source=mail")).await,
        (200, "ran:alice".to_string())
    );
    // Repeated, and before the signature: every pair of the key is left out.
    let signature = signature_of(&signed);
    let repeated =
        format!("/par116/ignoring?utm_source=a&user=alice&utm_source=b&signature={signature}");
    assert_eq!(
        get(address, &repeated).await,
        (200, "ran:alice".to_string())
    );

    // Ignoring one parameter covers every other one still.
    let changed = format!(
        "{}&utm_source=mail",
        signed.replace("user=alice", "user=eve")
    );
    assert_refused(get(address, &changed).await, "a changed signed value");
    assert_refused(
        get(address, "/par116/ignoring?utm_source=mail").await,
        "an ignored parameter alone",
    );
}

#[tokio::test]
async fn relative_and_the_alias_ignore_the_parameters_they_name() {
    let address = serve().await;
    for path in ["/par116/relative", "/par116/alias-relative"] {
        let signed = url::signed_url(&format!("{path}?user=alice"), None).expect("sign");
        assert_eq!(
            get(address, &signed).await,
            (200, "ran:alice".to_string()),
            "{path}"
        );
        assert_eq!(
            get(address, &format!("{signed}&utm_source=mail")).await,
            (200, "ran:alice".to_string()),
            "{path}"
        );
        assert_refused(
            get(address, &format!("{signed}&ref=x")).await,
            &format!("{path}: a parameter it does not ignore"),
        );
    }

    // The bare alias ignores nothing.
    let signed = url::signed_url("/par116/alias-plain?user=alice", None).expect("sign");
    assert_eq!(get(address, &signed).await, (200, "ran:alice".to_string()));
    assert_refused(
        get(address, &format!("{signed}&utm_source=mail")).await,
        "the bare alias",
    );
}

#[test]
fn the_alias_arguments_read_relative_first_and_refuse_an_empty_name() {
    for arguments in [
        &[][..],
        &["relative"][..],
        &["relative", "utm_source"][..],
        &["utm_source", "ref"][..],
    ] {
        assert!(
            ValidateSignature::from_alias_args(arguments).is_ok(),
            "{arguments:?}"
        );
    }
    let error = ValidateSignature::from_alias_args(&["relative", ""]).expect_err("an empty name");
    assert!(error.to_string().contains("empty"), "{error}");

    // A route naming the typo fails to register.
    register_middleware_alias_with_args("signed", ValidateSignature::from_alias_args);
    let refused = Router::new()
        .get("/par116/typo", guarded)
        .try_middleware_named("signed:relative,");
    assert!(refused.is_err());
}

#[tokio::test]
async fn except_ignores_a_parameter_for_every_instance() {
    // A name no other test uses: the list is process-wide and only grows.
    ValidateSignature::except(["par116_campaign"]);
    let address = serve().await;
    let signed = url::signed_url("/par116/plain/7?user=alice", None).expect("sign");
    let tagged = format!("{signed}&par116_campaign=autumn");
    assert_eq!(get(address, &tagged).await, (200, "ran:alice".to_string()));
    assert_eq!(
        get(
            address,
            &format!(
                "/par116/ignoring?user=alice&signature={}&par116_campaign=x",
                signature_of(&signed)
            )
        )
        .await
        .0,
        403,
        "the signature of another path stays refused"
    );

    // It is the middleware's list: the URL helper verifies the parameter.
    let request = Request::for_test("GET", &tagged);
    assert!(!url::has_valid_signature(&request).expect("verify"));
    assert!(url::has_valid_signature_ignoring(&request, &["par116_campaign"]).expect("verify"));
}

#[test]
fn has_valid_signature_ignoring_leaves_the_named_parameters_out() {
    ensure_crypt();
    let signed = url::signed_url("/par116/helper?user=alice", None).expect("sign");
    let appended = Request::for_test("GET", &format!("{signed}&utm_source=mail"));
    assert!(url::has_valid_signature_ignoring(&appended, &["utm_source"]).expect("verify"));
    assert!(
        !url::has_valid_signature(&appended).expect("verify"),
        "without the ignore list the appended parameter is signed text"
    );

    let changed = Request::for_test(
        "GET",
        &format!(
            "{}&utm_source=mail",
            signed.replace("user=alice", "user=eve")
        ),
    );
    assert!(!url::has_valid_signature_ignoring(&changed, &["utm_source"]).expect("verify"));
}

#[test]
fn signature_and_expires_cannot_be_ignored() {
    ensure_crypt();
    // Ignoring `expires` neither drops it from the signed text nor skips
    // the expiry check.
    let live = url::signed_url("/par116/helper", Some(now() + 3600)).expect("sign");
    assert_eq!(
        verify_signature_ignoring(&live, now(), &["expires"]).expect("verify"),
        SignatureVerdict::Valid
    );
    let lapsed = url::signed_url("/par116/helper", Some(now() - 60)).expect("sign");
    assert_eq!(
        verify_signature_ignoring(&lapsed, now(), &["expires"]).expect("verify"),
        SignatureVerdict::Expired
    );
    // Ignoring `signature` does not make a missing one pass.
    assert_eq!(
        verify_signature_ignoring("/par116/helper", now(), &["signature"]).expect("verify"),
        SignatureVerdict::Invalid
    );
    let signed = url::signed_url("/par116/helper?user=alice", None).expect("sign");
    assert_eq!(
        verify_signature_ignoring(&signed, now(), &["signature"]).expect("verify"),
        SignatureVerdict::Valid
    );
}
