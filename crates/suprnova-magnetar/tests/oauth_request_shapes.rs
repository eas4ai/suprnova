//! Declarative OAuth request-shape rendering tests
//! (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s request-shape
//! surface and `10-providers.md`'s Apple/TikTok dossiers).
//!
//! Every case below calls the *same* `render_authorization_request`/
//! `render_token_request` functions with a different `*RequestShape` value.
//! None of these functions, nor this test file, ever matches on a provider
//! name -- the whole point of the declarative surface is that a
//! provider-specific request shape falls out of data alone.

#![cfg(feature = "oauth")]

use magnetar::oauth::{
    AuthorizationRequestParams, AuthorizationRequestShape, OAuthProtocolError, PkcePosture,
    TokenRequestParams, TokenRequestShape, TokenResponseBody, parse_token_response_body,
    render_authorization_request, render_token_request,
};
use secrecy::SecretString;

fn find<'a>(wire: &'a [(String, String)], key: &str) -> Option<&'a str> {
    wire.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn has_key(wire: &[(String, String)], key: &str) -> bool {
    wire.iter().any(|(k, _)| k == key)
}

// --- Default (RFC 6749 spec-compliant) shape -------------------------------

#[test]
fn default_authorization_shape_uses_client_id_and_space_delimited_scopes() {
    let shape = AuthorizationRequestShape::default();
    let params = AuthorizationRequestParams {
        client_id: "abc123".to_owned(),
        redirect_uri: Some("https://app.example/callback".to_owned()),
        scopes: vec!["openid".to_owned(), "profile".to_owned()],
        state: Some("csrf-state".to_owned()),
        code_challenge: Some("challenge-value".to_owned()),
        nonce: None,
    };

    let wire = render_authorization_request(&shape, &params).expect("PKCE challenge supplied");

    assert_eq!(find(&wire, "client_id"), Some("abc123"));
    assert!(
        !has_key(&wire, "client_key"),
        "default shape must not use client_key"
    );
    assert_eq!(find(&wire, "scope"), Some("openid profile"));
    assert_eq!(find(&wire, "response_type"), Some("code"));
    assert_eq!(find(&wire, "code_challenge"), Some("challenge-value"));
    assert_eq!(find(&wire, "code_challenge_method"), Some("S256"));
    assert!(
        !has_key(&wire, "response_mode"),
        "default shape sets no response_mode"
    );
}

#[test]
fn default_shape_omits_scope_when_no_scopes_requested_and_not_always_sent() {
    let shape = AuthorizationRequestShape::default();
    let params = AuthorizationRequestParams {
        client_id: "abc123".to_owned(),
        redirect_uri: None,
        scopes: Vec::new(),
        state: None,
        code_challenge: Some("challenge-value".to_owned()),
        nonce: None,
    };

    let wire = render_authorization_request(&shape, &params).expect("PKCE challenge supplied");

    assert!(!has_key(&wire, "scope"));
}

#[test]
fn required_pkce_without_a_code_challenge_is_rejected() {
    let shape = AuthorizationRequestShape::default();
    let params = AuthorizationRequestParams {
        client_id: "abc123".to_owned(),
        redirect_uri: None,
        scopes: Vec::new(),
        state: None,
        code_challenge: None,
        nonce: None,
    };

    let outcome = render_authorization_request(&shape, &params);

    match outcome {
        Err(OAuthProtocolError::InvalidRequestShape { field, .. }) => {
            assert_eq!(field, "code_challenge");
        }
        other => panic!("expected InvalidRequestShape, got {other:?}"),
    }
}

// --- Apple: PKCE disabled + response_mode=form_post -------------------------

fn apple_authorization_shape() -> AuthorizationRequestShape {
    // docs/specs/suprnova-magnetar/10-providers.md's Apple dossier: PKCE
    // disabled (Apple rejects code_challenge on this flow) and
    // response_mode=form_post; client_id_param stays the RFC default.
    AuthorizationRequestShape {
        pkce: PkcePosture::Disabled,
        response_mode: Some("form_post".to_owned()),
        ..AuthorizationRequestShape::default()
    }
}

#[test]
fn apple_shape_disables_pkce_and_sets_form_post_response_mode() {
    let shape = apple_authorization_shape();
    let params = AuthorizationRequestParams {
        client_id: "com.example.app".to_owned(),
        redirect_uri: Some("https://app.example/callback/apple".to_owned()),
        scopes: vec!["name".to_owned(), "email".to_owned()],
        state: Some("csrf-state".to_owned()),
        code_challenge: None,
        nonce: None,
    };

    let wire = render_authorization_request(&shape, &params)
        .expect("PKCE disabled, no challenge required");

    assert_eq!(find(&wire, "response_mode"), Some("form_post"));
    assert!(!has_key(&wire, "code_challenge"));
    assert!(!has_key(&wire, "code_challenge_method"));
    assert_eq!(find(&wire, "client_id"), Some("com.example.app"));
}

#[test]
fn apple_shape_never_emits_code_challenge_even_if_one_is_supplied() {
    // Defensive: a Disabled posture must drop a caller-supplied challenge
    // rather than send it, since Apple rejects `code_challenge` outright.
    let shape = apple_authorization_shape();
    let params = AuthorizationRequestParams {
        client_id: "com.example.app".to_owned(),
        redirect_uri: None,
        scopes: Vec::new(),
        state: None,
        code_challenge: Some("should-be-dropped".to_owned()),
        nonce: None,
    };

    let wire = render_authorization_request(&shape, &params).expect("PKCE disabled never errors");

    assert!(!has_key(&wire, "code_challenge"));
    assert!(!has_key(&wire, "code_challenge_method"));
}

// --- TikTok: client_key param, comma scopes, always-emitted scope ----------

fn tiktok_authorization_shape() -> AuthorizationRequestShape {
    // docs/specs/suprnova-magnetar/10-providers.md's TikTok dossier:
    // `client_key` (not `client_id`), comma-delimited scopes, scope always
    // emitted, PKCE required.
    AuthorizationRequestShape {
        client_id_param: "client_key".to_owned(),
        scope_delimiter: ",".to_owned(),
        always_send_scope: true,
        pkce: PkcePosture::Required,
        response_mode: None,
        requires_nonce: false,
    }
}

fn tiktok_token_shape() -> TokenRequestShape {
    TokenRequestShape {
        client_id_param: "client_key".to_owned(),
        scope_delimiter: ",".to_owned(),
        always_send_scope: true,
        accept_http_success_error_body: true,
    }
}

#[test]
fn tiktok_authorization_shape_uses_client_key_and_comma_scopes() {
    let shape = tiktok_authorization_shape();
    let params = AuthorizationRequestParams {
        client_id: "tt_client_key".to_owned(),
        redirect_uri: Some("https://app.example/callback/tiktok".to_owned()),
        scopes: vec!["user.info.basic".to_owned(), "video.list".to_owned()],
        state: Some("csrf-state".to_owned()),
        code_challenge: Some("challenge-value".to_owned()),
        nonce: None,
    };

    let wire = render_authorization_request(&shape, &params).expect("PKCE challenge supplied");

    assert!(
        !has_key(&wire, "client_id"),
        "TikTok must not use client_id"
    );
    assert_eq!(find(&wire, "client_key"), Some("tt_client_key"));
    assert_eq!(find(&wire, "scope"), Some("user.info.basic,video.list"));
}

#[test]
fn tiktok_authorization_shape_always_emits_scope_even_when_empty() {
    let shape = tiktok_authorization_shape();
    let params = AuthorizationRequestParams {
        client_id: "tt_client_key".to_owned(),
        redirect_uri: None,
        scopes: Vec::new(),
        state: None,
        code_challenge: Some("challenge-value".to_owned()),
        nonce: None,
    };

    let wire = render_authorization_request(&shape, &params).expect("PKCE challenge supplied");

    assert_eq!(find(&wire, "scope"), Some(""));
}

#[test]
fn tiktok_token_shape_uses_client_key_and_always_emits_scope() {
    let shape = tiktok_token_shape();
    let params = TokenRequestParams {
        client_id: "tt_client_key".to_owned(),
        code: SecretString::from("auth-code"),
        redirect_uri: Some("https://app.example/callback/tiktok".to_owned()),
        code_verifier: Some(SecretString::from("verifier-value")),
        scopes: Vec::new(),
    };

    let wire = render_token_request(&shape, &params);

    assert!(!has_key(&wire, "client_id"));
    assert_eq!(find(&wire, "client_key"), Some("tt_client_key"));
    assert_eq!(find(&wire, "scope"), Some(""));
    assert_eq!(find(&wire, "grant_type"), Some("authorization_code"));
    assert_eq!(find(&wire, "code_verifier"), Some("verifier-value"));
}

// --- HTTP-200 OAuth error body detection ------------------------------------

#[test]
fn tiktok_shape_flags_that_http_200_bodies_may_still_be_errors() {
    let shape = tiktok_token_shape();
    assert!(
        shape.accept_http_success_error_body,
        "TikTok can encode an OAuth error in an HTTP 200 response body"
    );

    // A raw body a TikTok token endpoint could deliver with a 200 status:
    // detection must not depend on the (here, absent) status code at all.
    let http_200_body =
        r#"{"error": "invalid_grant", "error_description": "authorization code expired"}"#;

    match parse_token_response_body(http_200_body).expect("body parses") {
        TokenResponseBody::Error(error) => {
            assert_eq!(
                error.error_description.as_deref(),
                Some("authorization code expired")
            );
        }
        TokenResponseBody::Success(_) => {
            panic!("HTTP-200 body carrying an `error` field must classify as an error response")
        }
    }
}

#[test]
fn default_shape_never_expects_http_200_error_bodies() {
    let shape = TokenRequestShape::default();
    assert!(!shape.accept_http_success_error_body);
}
