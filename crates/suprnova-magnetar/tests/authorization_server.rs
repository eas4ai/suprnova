//! AGENT-005: the authorization server plugin on SQLite.
//!
//! Every test drives the mounted plugin routes the way a browser and an MCP
//! client would, and checks tokens through the verification API a resource
//! server calls. The live PostgreSQL and MySQL runs of the concurrency
//! property are in `authorization_server_live.rs`.

#![cfg(all(feature = "authorization-server", feature = "seaorm-sqlite"))]

#[path = "fixtures/authorization_server_harness.rs"]
mod harness;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use harness::*;
use magnetar::authorization_server::{
    AccessVerification, ClientMetadataError, ClientMetadataPolicy, GrantHandler,
    InvalidAccessReason, IssueTokens, NewGrant, TokenError, TokenIssuer, TokenRequest,
    TokenResponse,
};
use magnetar::plugin::{Method, WireBody, WireRequest};
use magnetar::sessions::JwtEpochStore;
use sea_orm::EntityTrait;
use serde_json::json;
use sha2::{Digest, Sha256};

const EVIL: &str = "https://evil.example.test/phish";
const LOOPBACK_REDIRECT: &str = "http://127.0.0.1:3000/callback";
const LOOPBACK_REQUEST: &str = "http://127.0.0.1:51000/callback";

fn invalid(reason: InvalidAccessReason) -> AccessVerification {
    AccessVerification::Invalid(reason)
}

fn document(client_id: &str, redirect_uri: &str) -> serde_json::Value {
    json!({
        "client_id": client_id,
        "client_name": "Tools Client",
        "client_uri": "https://tools.client.test/",
        "logo_uri": "javascript:alert(document.cookie)",
        "redirect_uris": [redirect_uri],
        "token_endpoint_auth_method": "none",
    })
}

#[tokio::test]
async fn agent_005_code_flow_with_pkce_issues_tokens_bound_to_resource_and_scopes() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT]).await;
    let (verifier, challenge) = pkce();

    let authorized = h
        .authorize(
            &client_id,
            REDIRECT,
            &challenge,
            "tools:read",
            Some(&bearer),
        )
        .await;
    assert_eq!(status(&authorized), 302);
    let approval = location(&authorized).expect("the approval page");
    assert!(
        approval.starts_with("/oauth/approve?authorization="),
        "{approval}"
    );
    let id = query_param(&approval, "authorization").expect("a pending id");

    let view = h
        .get(&format!("/oauth/authorizations/{id}"), &[], Some(&bearer))
        .await;
    assert_eq!(status(&view), 200, "{:?}", body(&view));
    assert_no_framing(&view);
    let view = body(&view);
    assert_eq!(view["client"]["name"], "Example Tool");
    assert_eq!(view["client"]["host"], "client.example.test");
    assert_eq!(view["scopes"][0]["name"], "tools:read");
    assert_eq!(view["scopes"][0]["description"], "Read your tools");
    assert_eq!(view["resource"], RESOURCE);

    let decided = h
        .post_form(
            &format!("/oauth/authorizations/{id}"),
            &[("decision", "approve")],
            Some(&bearer),
        )
        .await;
    assert_eq!(status(&decided), 303);
    assert_no_framing(&decided);
    let back = location(&decided).expect("a redirect to the client");
    assert!(back.starts_with(REDIRECT), "{back}");
    assert_eq!(query_param(&back, "state").as_deref(), Some("state-123"));
    assert_eq!(query_param(&back, "iss").as_deref(), Some(ISSUER));
    let code = query_param(&back, "code").expect("a code");

    let tokens = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&tokens), 200, "{:?}", body(&tokens));
    assert_eq!(
        header(&tokens, "cache-control").as_deref(),
        Some("no-store")
    );
    let tokens = body(&tokens);
    assert_eq!(tokens["token_type"], "Bearer");
    assert_eq!(tokens["expires_in"], 3600);
    assert_eq!(tokens["scope"], "tools:read");
    let access = tokens["access_token"].as_str().expect("an access token");
    assert!(tokens["refresh_token"].as_str().is_some());

    match h.verify(access).await {
        AccessVerification::Valid(grant) => {
            assert_eq!(grant.user_id.as_deref(), Some(user_id.as_str()));
            assert_eq!(grant.client_id, client_id);
            assert_eq!(grant.scopes, vec!["tools:read".to_owned()]);
            assert_eq!(grant.token_kind, "access");
            assert_eq!(grant.grant_kind, "authorization_code");
            assert_eq!(grant.resource, RESOURCE);
        }
        other => panic!("the access token must verify, got {other:?}"),
    }
}

#[tokio::test]
async fn agent_005_plain_and_missing_pkce_are_refused() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT, LOOPBACK_REDIRECT]).await;
    let (verifier, challenge) = pkce();

    for (method, challenge) in [
        (Some("plain"), verifier.as_str()),
        (None, challenge.as_str()),
        (Some("S256"), ""),
    ] {
        for redirect_uri in [REDIRECT, LOOPBACK_REQUEST] {
            let mut query = vec![
                ("response_type", "code"),
                ("client_id", client_id.as_str()),
                ("redirect_uri", redirect_uri),
                ("resource", RESOURCE),
                ("state", "s"),
            ];
            if !challenge.is_empty() {
                query.push(("code_challenge", challenge));
            }
            if let Some(method) = method {
                query.push(("code_challenge_method", method));
            }
            let response = h.get("/oauth/authorize", &query, Some(&bearer)).await;
            if redirect_uri == REDIRECT {
                // Not a loopback redirect URI: the error page, no redirect.
                assert_eq!(status(&response), 400, "method {method:?}");
                assert!(location(&response).is_none(), "method {method:?}");
                assert_eq!(body(&response)["error"], "invalid_request");
                continue;
            }
            let back = location(&response).expect("an error redirect to the loopback client");
            assert!(
                back.starts_with("http://127.0.0.1:51000/callback"),
                "{back}"
            );
            assert_eq!(
                query_param(&back, "error").as_deref(),
                Some("invalid_request"),
                "method {method:?}"
            );
            assert_eq!(query_param(&back, "state").as_deref(), Some("s"));
            assert_eq!(query_param(&back, "iss").as_deref(), Some(ISSUER));
            assert!(query_param(&back, "code").is_none());
        }
    }

    // A code minted with S256 cannot be exchanged without the verifier or
    // with the challenge sent back as a plain verifier.
    let code = h
        .approve(&client_id, REDIRECT, &challenge, "tools:read", &bearer)
        .await;
    let missing = h
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", REDIRECT),
                ("client_id", &client_id),
                ("resource", RESOURCE),
            ],
            None,
        )
        .await;
    assert_eq!(status(&missing), 400);
    assert_eq!(body(&missing)["error"], "invalid_request");
    let plain = h.exchange(&client_id, REDIRECT, &code, &challenge).await;
    assert_eq!(status(&plain), 400);
    assert_eq!(body(&plain)["error"], "invalid_grant");
    let right = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&right), 200, "{:?}", body(&right));
}

#[tokio::test]
async fn agent_005_redirect_uris_match_exactly_with_any_port_on_loopback() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h
        .register(&[
            REDIRECT,
            "http://127.0.0.1:3000/callback",
            "http://[::1]:3000/callback",
            "http://localhost:3000/callback",
        ])
        .await;
    let (_, challenge) = pkce();

    for refused in [
        "https://client.example.test/callback/extra",
        "https://client.example.test/callback?x=1",
        "https://client.example.test:8443/callback",
        "https://evil.example.test/callback",
        "http://127.0.0.1:54321/other",
        "http://localhost:4000/callback",
    ] {
        let response = h
            .authorize(&client_id, refused, &challenge, "", Some(&bearer))
            .await;
        assert_eq!(status(&response), 400, "{refused}");
        assert!(location(&response).is_none(), "{refused} must not redirect");
        assert_eq!(body(&response)["error"], "invalid_request");
    }
    for accepted in [
        REDIRECT,
        "http://127.0.0.1:54321/callback",
        "http://[::1]:61000/callback",
        "http://localhost:3000/callback",
    ] {
        let response = h
            .authorize(&client_id, accepted, &challenge, "", Some(&bearer))
            .await;
        let approval = location(&response).expect("the approval page");
        assert!(
            approval.starts_with("/oauth/approve"),
            "{accepted}: {approval}"
        );
    }
}

#[tokio::test]
async fn agent_005_unknown_client_is_refused_without_a_redirect() {
    let h = sqlite_harness().await;
    let (_, challenge) = pkce();
    let response = h
        .authorize("mcl_unknown", REDIRECT, &challenge, "", None)
        .await;
    assert_eq!(status(&response), 400);
    assert!(location(&response).is_none());
    assert_eq!(body(&response)["error"], "invalid_client");
    assert_no_framing(&response);
}

#[tokio::test]
async fn agent_005_a_user_who_is_not_signed_in_is_sent_to_sign_in() {
    let h = sqlite_harness().await;
    let client_id = h.register(&[REDIRECT]).await;
    let (_, challenge) = pkce();
    let response = h
        .authorize(&client_id, REDIRECT, &challenge, "tools:read", None)
        .await;
    let sign_in = location(&response).expect("a redirect to sign in");
    assert!(sign_in.starts_with("/login?return_to="), "{sign_in}");
    let return_to = query_param(&sign_in, "return_to").expect("a return address");
    assert!(return_to.starts_with("/oauth/authorize?"), "{return_to}");
    assert_eq!(
        query_param(&return_to, "client_id").as_deref(),
        Some(client_id.as_str())
    );
}

#[tokio::test]
async fn agent_005_client_metadata_document_identifies_a_client() {
    let h = sqlite_harness().await;
    h.resolver.answer(DOCUMENT_HOST, &[PUBLIC_ADDRESS]);
    h.documents.serve(
        DOCUMENT_CLIENT,
        Script::json(document(DOCUMENT_CLIENT, DOCUMENT_REDIRECT)),
    );
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let (verifier, challenge) = pkce();

    let response = h
        .authorize(
            DOCUMENT_CLIENT,
            DOCUMENT_REDIRECT,
            &challenge,
            "tools:read",
            Some(&bearer),
        )
        .await;
    let approval = location(&response).expect("the approval page");
    let id = query_param(&approval, "authorization").expect("a pending id");
    let view = body(
        &h.get(&format!("/oauth/authorizations/{id}"), &[], Some(&bearer))
            .await,
    );
    assert_eq!(view["client"]["name"], "Tools Client");
    assert_eq!(view["client"]["host"], DOCUMENT_HOST);
    assert_eq!(view["client"]["client_uri"], "https://tools.client.test/");
    assert!(
        view["client"]["logo_uri"].is_null(),
        "a non-https link never reaches the approval page"
    );

    let requests = h.documents.requests.lock().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, DOCUMENT_CLIENT);
    assert_eq!(
        requests[0].1,
        format!("{PUBLIC_ADDRESS}:443").parse().unwrap(),
        "the fetch connects to the address that was checked"
    );

    let code = h
        .approve(
            DOCUMENT_CLIENT,
            DOCUMENT_REDIRECT,
            &challenge,
            "tools:read",
            &bearer,
        )
        .await;
    let tokens = h
        .exchange(DOCUMENT_CLIENT, DOCUMENT_REDIRECT, &code, &verifier)
        .await;
    assert_eq!(status(&tokens), 200, "{:?}", body(&tokens));
    assert_eq!(
        h.documents.request_count(),
        1,
        "the second authorization reads the cached document"
    );
    assert!(
        h.limiter
            .keys()
            .iter()
            .any(|key| key.starts_with("authorization-server.client-metadata:")),
        "an uncached fetch spends the metadata-fetch budget"
    );
}

#[tokio::test]
async fn agent_005_client_metadata_fetch_refuses_non_https_and_bad_urls() {
    let h = sqlite_harness().await;
    assert_eq!(
        h.fetcher
            .fetch("http://tools.client.test/oauth/client.json")
            .await,
        Err(ClientMetadataError::NotHttps)
    );
    for bad in [
        "https://tools.client.test/",
        "https://tools.client.test/a/../client.json",
        "https://user:pass@tools.client.test/client.json",
        "https://tools.client.test/client.json#fragment",
    ] {
        assert!(
            matches!(
                h.fetcher.fetch(bad).await,
                Err(ClientMetadataError::InvalidUrl(_))
            ),
            "{bad}"
        );
    }
    assert_eq!(h.resolver.lookup_count(), 0);
    assert_eq!(h.documents.request_count(), 0);

    // Through the authorization endpoint the refusal is an error page.
    let (_, challenge) = pkce();
    let response = h
        .authorize(
            "http://tools.client.test/oauth/client.json",
            DOCUMENT_REDIRECT,
            &challenge,
            "",
            None,
        )
        .await;
    assert_eq!(status(&response), 400);
    assert!(location(&response).is_none());
}

#[tokio::test]
async fn agent_005_client_metadata_fetch_refuses_private_addresses() {
    let h = sqlite_harness().await;
    for (index, answer) in [
        vec!["10.0.0.7"],
        vec!["127.0.0.1"],
        vec!["169.254.169.254"],
        vec!["0.0.0.0"],
        vec!["192.168.1.20"],
        vec!["::1"],
        vec!["fe80::1"],
        vec!["fd00::5"],
        vec!["::ffff:127.0.0.1"],
        vec![PUBLIC_ADDRESS, "10.0.0.7"],
    ]
    .into_iter()
    .enumerate()
    {
        let host = format!("private{index}.client.test");
        let client_id = format!("https://{host}/client.json");
        h.resolver.answer(&host, &answer);
        h.documents.serve(
            &client_id,
            Script::json(document(&client_id, DOCUMENT_REDIRECT)),
        );
        assert!(
            matches!(
                h.fetcher.fetch(&client_id).await,
                Err(ClientMetadataError::ForbiddenAddress(_))
            ),
            "{answer:?} must be refused"
        );
    }
    // The error page names no address: it would report where an internal
    // name points.
    let (_, challenge) = pkce();
    let page = h
        .authorize(
            "https://private0.client.test/client.json",
            DOCUMENT_REDIRECT,
            &challenge,
            "",
            None,
        )
        .await;
    assert_eq!(status(&page), 400);
    assert!(location(&page).is_none());
    let description = body(&page)["error_description"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!description.contains("10.0.0.7"), "{description}");

    for literal in ["https://127.0.0.1/client.json", "https://[::1]/client.json"] {
        assert!(matches!(
            h.fetcher.fetch(literal).await,
            Err(ClientMetadataError::ForbiddenAddress(_))
        ));
    }
    assert_eq!(h.documents.request_count(), 0, "nothing connects");
}

#[tokio::test]
async fn agent_005_client_metadata_fetch_follows_no_redirect() {
    let h = sqlite_harness().await;
    h.resolver.answer(DOCUMENT_HOST, &[PUBLIC_ADDRESS]);
    h.documents.serve(
        DOCUMENT_CLIENT,
        Script {
            status: 302,
            ..Script::json(json!({}))
        },
    );
    assert_eq!(
        h.fetcher.fetch(DOCUMENT_CLIENT).await,
        Err(ClientMetadataError::Redirected(302))
    );
    assert_eq!(h.documents.request_count(), 1);
}

#[tokio::test]
async fn agent_005_client_metadata_fetch_bounds_the_body() {
    let h = sqlite_harness().await;
    h.resolver.answer(DOCUMENT_HOST, &[PUBLIC_ADDRESS]);
    let mut oversized = document(DOCUMENT_CLIENT, DOCUMENT_REDIRECT);
    oversized["padding"] = json!("x".repeat(64 * 1024));
    h.documents.serve(DOCUMENT_CLIENT, Script::json(oversized));
    assert_eq!(
        h.fetcher.fetch(DOCUMENT_CLIENT).await,
        Err(ClientMetadataError::TooLarge)
    );

    let endless = "https://endless.client.test/client.json";
    h.resolver.answer("endless.client.test", &[PUBLIC_ADDRESS]);
    h.documents.serve(
        endless,
        Script {
            endless: true,
            ..Script::json(json!({}))
        },
    );
    assert_eq!(
        h.fetcher.fetch(endless).await,
        Err(ClientMetadataError::TooLarge)
    );
}

#[tokio::test]
async fn agent_005_client_metadata_fetch_bounds_the_time() {
    let policy = ClientMetadataPolicy {
        timeout: Duration::from_millis(200),
        ..ClientMetadataPolicy::default()
    };
    let h = build(
        magnetar::default_schema::database().await,
        config(),
        policy,
        Vec::new(),
    )
    .await;
    h.resolver.answer(DOCUMENT_HOST, &[PUBLIC_ADDRESS]);
    h.documents.serve(
        DOCUMENT_CLIENT,
        Script {
            delay: Duration::from_secs(30),
            ..Script::json(document(DOCUMENT_CLIENT, DOCUMENT_REDIRECT))
        },
    );
    let started = Instant::now();
    assert_eq!(
        h.fetcher.fetch(DOCUMENT_CLIENT).await,
        Err(ClientMetadataError::TimedOut)
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn agent_005_client_metadata_document_must_name_its_own_url() {
    let h = sqlite_harness().await;
    h.resolver.answer(DOCUMENT_HOST, &[PUBLIC_ADDRESS]);
    h.documents.serve(
        DOCUMENT_CLIENT,
        Script::json(document(
            "https://tools.client.test/oauth/other.json",
            DOCUMENT_REDIRECT,
        )),
    );
    assert_eq!(
        h.fetcher.fetch(DOCUMENT_CLIENT).await,
        Err(ClientMetadataError::ClientIdMismatch)
    );
}

#[tokio::test]
async fn agent_005_dynamic_registration_accepts_only_public_clients() {
    let h = sqlite_harness().await;
    let registered = h
        .post_json(
            "/oauth/register",
            json!({
                "client_name": "Tool",
                "redirect_uris": [REDIRECT, "http://127.0.0.1:8080/cb"],
            }),
        )
        .await;
    assert_eq!(status(&registered), 201, "{:?}", body(&registered));
    let registered = body(&registered);
    assert!(
        registered["client_id"]
            .as_str()
            .unwrap()
            .starts_with("mcl_")
    );
    assert_eq!(registered["token_endpoint_auth_method"], "none");
    assert_eq!(registered["client_name"], "Tool");
    assert!(
        h.limiter
            .keys()
            .iter()
            .any(|key| key.starts_with("authorization-server.register:"))
    );

    for (metadata, error) in [
        (
            json!({"redirect_uris": [REDIRECT], "token_endpoint_auth_method": "client_secret_basic"}),
            "invalid_client_metadata",
        ),
        (
            json!({"redirect_uris": [REDIRECT], "client_secret": "s3cret"}),
            "invalid_client_metadata",
        ),
        (
            json!({"redirect_uris": [REDIRECT], "grant_types": ["client_credentials"]}),
            "invalid_client_metadata",
        ),
        (
            json!({"redirect_uris": [REDIRECT], "scope": "admin"}),
            "invalid_client_metadata",
        ),
        (json!({"redirect_uris": []}), "invalid_redirect_uri"),
        (
            json!({"redirect_uris": ["http://client.example.test/cb"]}),
            "invalid_redirect_uri",
        ),
        (
            json!({"redirect_uris": ["javascript:alert(1)"]}),
            "invalid_redirect_uri",
        ),
        (
            json!({"redirect_uris": ["https://client.example.test/cb#x"]}),
            "invalid_redirect_uri",
        ),
    ] {
        let response = h.post_json("/oauth/register", metadata.clone()).await;
        assert_eq!(status(&response), 400, "{metadata}");
        assert_eq!(body(&response)["error"], error, "{metadata}");
    }

    let oversized = h
        .post_json(
            "/oauth/register",
            json!({"redirect_uris": [REDIRECT], "client_name": "x".repeat(20 * 1024)}),
        )
        .await;
    assert_eq!(status(&oversized), 413);

    h.limiter.set_mode(LimiterMode::Reject);
    let limited = h
        .post_json("/oauth/register", json!({"redirect_uris": [REDIRECT]}))
        .await;
    assert_eq!(status(&limited), 429);
    assert!(header(&limited, "retry-after").is_some());
}

#[tokio::test]
async fn agent_005_refresh_rotates_on_every_use() {
    let h = sqlite_harness().await;
    let issued = h.issue("tools:read tools:write").await;

    let first = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(status(&first), 200, "{:?}", body(&first));
    let first = body(&first);
    let rotated = first["refresh_token"].as_str().unwrap().to_owned();
    assert_ne!(rotated, issued.refresh_token);
    assert!(matches!(
        h.verify(first["access_token"].as_str().unwrap()).await,
        AccessVerification::Valid(_)
    ));

    // A narrower scope on refresh narrows the access token only.
    let second = h
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &rotated),
                ("client_id", &issued.client_id),
                ("scope", "tools:read"),
            ],
            None,
        )
        .await;
    assert_eq!(status(&second), 200, "{:?}", body(&second));
    let second = body(&second);
    assert_eq!(second["scope"], "tools:read");
    let third = h
        .refresh(&issued.client_id, second["refresh_token"].as_str().unwrap())
        .await;
    assert_eq!(body(&third)["scope"], "tools:read tools:write");

    let wider = h
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                (
                    "refresh_token",
                    body(&third)["refresh_token"].as_str().unwrap(),
                ),
                ("client_id", &issued.client_id),
                ("scope", "tools:admin"),
            ],
            None,
        )
        .await;
    assert_eq!(body(&wider)["error"], "invalid_scope");
}

#[tokio::test]
async fn agent_005_refresh_token_reuse_revokes_the_grant() {
    let h = sqlite_harness().await;
    let issued = h.issue("tools:read").await;
    let rotated = h.refresh(&issued.client_id, &issued.refresh_token).await;
    let rotated = body(&rotated);
    let new_access = rotated["access_token"].as_str().unwrap().to_owned();
    let new_refresh = rotated["refresh_token"].as_str().unwrap().to_owned();

    let reused = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(status(&reused), 400);
    assert_eq!(body(&reused)["error"], "invalid_grant");

    assert_eq!(
        h.verify(&issued.access_token).await,
        invalid(InvalidAccessReason::Revoked)
    );
    assert_eq!(
        h.verify(&new_access).await,
        invalid(InvalidAccessReason::Revoked)
    );
    let after = h.refresh(&issued.client_id, &new_refresh).await;
    assert_eq!(body(&after)["error"], "invalid_grant");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_005_concurrent_refresh_token_reuse_revokes_the_grant() {
    concurrent_refresh_reuse(Arc::new(sqlite_harness().await)).await;
}

#[tokio::test]
async fn agent_005_refresh_rotation_is_guarded_in_the_database() {
    guarded_rotation(&sqlite_harness().await).await;
}

#[tokio::test]
async fn agent_005_an_authorization_code_works_once() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT]).await;
    let (verifier, challenge) = pkce();
    let code = h
        .approve(&client_id, REDIRECT, &challenge, "tools:read", &bearer)
        .await;
    let first = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&first), 200);
    let access = body(&first)["access_token"].as_str().unwrap().to_owned();

    let second = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&second), 400);
    assert_eq!(body(&second)["error"], "invalid_grant");
    assert_eq!(
        h.verify(&access).await,
        invalid(InvalidAccessReason::Revoked),
        "a replayed code revokes what its first exchange issued"
    );
}

#[tokio::test]
async fn agent_005_the_code_is_bound_to_its_client_and_redirect_uri() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h
        .register(&[REDIRECT, "https://client.example.test/other"])
        .await;
    let other_client = h.register(&[REDIRECT]).await;
    let (verifier, challenge) = pkce();
    let code = h
        .approve(&client_id, REDIRECT, &challenge, "tools:read", &bearer)
        .await;
    let wrong_client = h.exchange(&other_client, REDIRECT, &code, &verifier).await;
    assert_eq!(body(&wrong_client)["error"], "invalid_grant");
    let wrong_redirect = h
        .exchange(
            &client_id,
            "https://client.example.test/other",
            &code,
            &verifier,
        )
        .await;
    assert_eq!(body(&wrong_redirect)["error"], "invalid_grant");
    let right = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&right), 200);
}

#[tokio::test]
async fn agent_005_tokens_are_bound_to_their_resource() {
    let h = sqlite_harness().await;
    let issued = h.issue("tools:read").await;
    assert_eq!(
        h.server
            .verify_access_token(&issued.access_token, "https://other.example.test/mcp")
            .await
            .unwrap(),
        invalid(InvalidAccessReason::WrongResource)
    );
    // Scheme and host compare without case, the way RFC 3986 defines them.
    assert!(matches!(
        h.server
            .verify_access_token(&issued.access_token, "HTTPS://APP.EXAMPLE.TEST/mcp")
            .await
            .unwrap(),
        AccessVerification::Valid(_)
    ));

    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let (verifier, challenge) = pkce();
    let unknown = h
        .get(
            "/oauth/authorize",
            &[
                ("response_type", "code"),
                ("client_id", &issued.client_id),
                ("redirect_uri", REDIRECT),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
                ("resource", "https://other.example.test/mcp"),
            ],
            Some(&bearer),
        )
        .await;
    assert_eq!(status(&unknown), 400);
    assert!(location(&unknown).is_none());
    assert_eq!(body(&unknown)["error"], "invalid_target");

    let code = h
        .approve(
            &issued.client_id,
            REDIRECT,
            &challenge,
            "tools:read",
            &bearer,
        )
        .await;
    let other_resource = h
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", REDIRECT),
                ("client_id", &issued.client_id),
                ("code_verifier", &verifier),
                ("resource", "https://other.example.test/mcp"),
            ],
            None,
        )
        .await;
    assert_eq!(body(&other_resource)["error"], "invalid_target");
}

#[tokio::test]
async fn agent_005_an_epoch_change_ends_the_users_grants() {
    let h = sqlite_harness().await;
    let issued = h.issue("tools:read").await;
    assert_eq!(
        h.server.list_grants(&issued.user_id).await.unwrap().len(),
        1
    );
    let (_, challenge) = pkce();
    let pending = h
        .authorize(
            &issued.client_id,
            REDIRECT,
            &challenge,
            "",
            Some(&issued.bearer),
        )
        .await;
    let id = query_param(&location(&pending).unwrap(), "authorization").unwrap();

    JwtEpochStore::bump_auth_epoch(h.storage.as_ref(), &issued.user_id)
        .await
        .expect("a password change moves the epoch");

    assert_eq!(
        h.verify(&issued.access_token).await,
        invalid(InvalidAccessReason::EpochChanged)
    );
    let refreshed = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(body(&refreshed)["error"], "invalid_grant");
    assert!(
        h.server
            .list_grants(&issued.user_id)
            .await
            .unwrap()
            .is_empty()
    );

    // The session is from the old epoch: a decision it posts is refused,
    // and a new authorization sends it to sign in.
    let decided = h
        .post_form(
            &format!("/oauth/authorizations/{id}"),
            &[("decision", "approve")],
            Some(&issued.bearer),
        )
        .await;
    assert_eq!(status(&decided), 401);
    assert!(location(&decided).is_none());
    let response = h
        .authorize(
            &issued.client_id,
            REDIRECT,
            &challenge,
            "",
            Some(&issued.bearer),
        )
        .await;
    assert!(location(&response).unwrap().starts_with("/login"));
}

#[tokio::test]
async fn agent_005_revocation_answers_200_and_revokes() {
    let h = sqlite_harness().await;
    let issued = h.issue("tools:read").await;

    let revoked = h
        .post_form(
            "/oauth/revoke",
            &[
                ("token", &issued.access_token),
                ("client_id", &issued.client_id),
            ],
            None,
        )
        .await;
    assert_eq!(status(&revoked), 200);
    assert_eq!(
        h.verify(&issued.access_token).await,
        invalid(InvalidAccessReason::Revoked)
    );

    // Revoking an access token leaves the grant; the refresh token works.
    let refreshed = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(status(&refreshed), 200);
    let refreshed = body(&refreshed);
    let access = refreshed["access_token"].as_str().unwrap().to_owned();
    let refresh = refreshed["refresh_token"].as_str().unwrap().to_owned();

    // Revoking the refresh token ends the grant.
    let revoked = h
        .post_form("/oauth/revoke", &[("token", &refresh)], None)
        .await;
    assert_eq!(status(&revoked), 200);
    assert_eq!(
        h.verify(&access).await,
        invalid(InvalidAccessReason::Revoked)
    );
    assert_eq!(
        body(&h.refresh(&issued.client_id, &refresh).await)["error"],
        "invalid_grant"
    );

    for token in ["mat_unknown", refresh.as_str()] {
        let again = h
            .post_form("/oauth/revoke", &[("token", token)], None)
            .await;
        assert_eq!(status(&again), 200, "{token}");
    }
    let missing = h.post_form("/oauth/revoke", &[], None).await;
    assert_eq!(status(&missing), 400);
    assert_eq!(body(&missing)["error"], "invalid_request");
}

#[tokio::test]
async fn agent_005_tokens_and_codes_are_stored_only_as_hashes() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT]).await;
    let (verifier, challenge) = pkce();
    let code = h
        .approve(&client_id, REDIRECT, &challenge, "tools:read", &bearer)
        .await;
    let tokens = body(&h.exchange(&client_id, REDIRECT, &code, &verifier).await);
    let access = tokens["access_token"].as_str().unwrap();
    let refresh = tokens["refresh_token"].as_str().unwrap();

    let rows = magnetar::authorization_server::sql::tokens::Entity::find()
        .all(&h.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let stored = format!("{rows:?}");
    let steps = format!(
        "{:?}",
        magnetar::authorization_server::sql::authorizations::Entity::find()
            .all(&h.db)
            .await
            .unwrap()
    );
    for secret in [access, refresh, code.as_str()] {
        assert!(!stored.contains(secret), "a token is stored in plain text");
        assert!(!steps.contains(secret), "a code is stored in plain text");
    }
    let digest = |token: &str| format!("{:x}", Sha256::digest(token.as_bytes()));
    assert!(rows.iter().any(|row| row.token_hash == digest(access)));
    assert!(rows.iter().any(|row| row.token_hash == digest(refresh)));
}

#[tokio::test]
async fn agent_005_the_metadata_document_describes_the_server() {
    let h = build(
        magnetar::default_schema::database().await,
        config(),
        ClientMetadataPolicy::default(),
        vec![Arc::new(TestAgentGrant)],
    )
    .await;
    let document = h.server.metadata_document();
    assert_eq!(document["issuer"], ISSUER);
    assert_eq!(
        document["authorization_endpoint"],
        format!("{ISSUER}/oauth/authorize")
    );
    assert_eq!(document["token_endpoint"], format!("{ISSUER}/oauth/token"));
    assert_eq!(
        document["registration_endpoint"],
        format!("{ISSUER}/oauth/register")
    );
    assert_eq!(
        document["revocation_endpoint"],
        format!("{ISSUER}/oauth/revoke")
    );
    assert_eq!(document["response_types_supported"], json!(["code"]));
    assert_eq!(
        document["grant_types_supported"],
        json!([
            "authorization_code",
            "refresh_token",
            "urn:example:test-grant"
        ])
    );
    assert_eq!(
        document["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(
        document["token_endpoint_auth_methods_supported"],
        json!(["none"])
    );
    assert_eq!(
        document["scopes_supported"],
        json!(["tools:read", "tools:write"])
    );
    assert_eq!(document["client_id_metadata_document_supported"], true);
    assert_eq!(
        document["authorization_response_iss_parameter_supported"],
        true
    );
}

#[tokio::test]
async fn agent_005_approval_routes_need_a_signed_in_user_and_refuse_framing() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT]).await;
    let (_, challenge) = pkce();
    let response = h
        .authorize(
            &client_id,
            REDIRECT,
            &challenge,
            "tools:read",
            Some(&bearer),
        )
        .await;
    let id = query_param(&location(&response).unwrap(), "authorization").unwrap();
    let path = format!("/oauth/authorizations/{id}");

    let view = h.get(&path, &[], None).await;
    assert_eq!(status(&view), 401);
    assert_no_framing(&view);
    let decided = h.post_form(&path, &[("decision", "approve")], None).await;
    assert_eq!(status(&decided), 401);
    assert_no_framing(&decided);
    assert!(location(&decided).is_none());

    let stranger = h.create_user().await;
    let stranger_bearer = h.sign_in(&stranger).await;
    assert_eq!(
        status(&h.get(&path, &[], Some(&stranger_bearer)).await),
        404
    );
    let hijack = h
        .post_form(&path, &[("decision", "approve")], Some(&stranger_bearer))
        .await;
    assert_eq!(status(&hijack), 404);
    assert!(location(&hijack).is_none());

    // The owner can still decide: the stranger's attempts spent nothing.
    let owner = h
        .post_form(&path, &[("decision", "approve")], Some(&bearer))
        .await;
    assert_eq!(status(&owner), 303);
}

#[tokio::test]
async fn agent_005_a_denial_redirects_with_access_denied() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT]).await;
    let (_, challenge) = pkce();
    let response = h
        .authorize(
            &client_id,
            REDIRECT,
            &challenge,
            "tools:read",
            Some(&bearer),
        )
        .await;
    let id = query_param(&location(&response).unwrap(), "authorization").unwrap();
    let denied = h
        .post_form(
            &format!("/oauth/authorizations/{id}"),
            &[("decision", "deny")],
            Some(&bearer),
        )
        .await;
    let back = location(&denied).expect("a redirect to the client");
    assert_eq!(
        query_param(&back, "error").as_deref(),
        Some("access_denied")
    );
    assert_eq!(query_param(&back, "state").as_deref(), Some("state-123"));
    assert_eq!(query_param(&back, "iss").as_deref(), Some(ISSUER));
    assert!(query_param(&back, "code").is_none());
}

struct TestAgentGrant;

#[async_trait]
impl GrantHandler for TestAgentGrant {
    fn grant_type(&self) -> &str {
        "urn:example:test-grant"
    }

    async fn exchange(
        &self,
        request: &TokenRequest,
        issuer: &TokenIssuer<'_>,
    ) -> Result<TokenResponse, TokenError> {
        if request.param("assertion") != Some("trusted") {
            return Err(TokenError::invalid_grant("the assertion is not trusted"));
        }
        issuer
            .issue(
                NewGrant {
                    kind: "agent".to_owned(),
                    user_id: None,
                    client_id: "agent-registration-1".to_owned(),
                    client_name: "Test agent".to_owned(),
                    client_host: "agent.example.test".to_owned(),
                    scopes: vec!["tools:read".to_owned()],
                    resource: RESOURCE.to_owned(),
                    auth_epoch: None,
                },
                IssueTokens {
                    access_kind: "agent_access".to_owned(),
                    refresh: false,
                    access_lifetime: Some(Duration::from_secs(600)),
                },
            )
            .await
    }
}

#[tokio::test]
async fn agent_005_the_token_endpoint_dispatches_extension_grants() {
    let h = build(
        magnetar::default_schema::database().await,
        config(),
        ClientMetadataPolicy::default(),
        vec![Arc::new(TestAgentGrant)],
    )
    .await;
    let issued = h
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "urn:example:test-grant"),
                ("assertion", "trusted"),
            ],
            None,
        )
        .await;
    assert_eq!(status(&issued), 200, "{:?}", body(&issued));
    let issued = body(&issued);
    assert!(issued.get("refresh_token").is_none());
    assert_eq!(issued["expires_in"], 600);
    match h.verify(issued["access_token"].as_str().unwrap()).await {
        AccessVerification::Valid(grant) => {
            assert_eq!(grant.token_kind, "agent_access");
            assert_eq!(grant.grant_kind, "agent");
            assert_eq!(grant.user_id, None);
            assert_eq!(grant.client_id, "agent-registration-1");
        }
        other => panic!("the extension's token must verify, got {other:?}"),
    }
    let refused = h
        .post_form(
            "/oauth/token",
            &[
                ("grant_type", "urn:example:test-grant"),
                ("assertion", "forged"),
            ],
            None,
        )
        .await;
    assert_eq!(body(&refused)["error"], "invalid_grant");

    let unknown = h
        .post_form("/oauth/token", &[("grant_type", "urn:example:nope")], None)
        .await;
    assert_eq!(status(&unknown), 400);
    assert_eq!(body(&unknown)["error"], "unsupported_grant_type");

    // A built-in grant type cannot be replaced.
    let server = magnetar::authorization_server::AuthorizationServer::new(
        config(),
        Arc::new(magnetar::authorization_server::SqlAuthorizationStore::new(
            h.db.clone(),
        )),
        h.storage.clone(),
        h.limiter.clone(),
        h.fetcher.clone(),
    )
    .unwrap();
    struct Hijack;
    #[async_trait]
    impl GrantHandler for Hijack {
        fn grant_type(&self) -> &str {
            "refresh_token"
        }
        async fn exchange(
            &self,
            _request: &TokenRequest,
            _issuer: &TokenIssuer<'_>,
        ) -> Result<TokenResponse, TokenError> {
            Err(TokenError::server_error())
        }
    }
    assert!(server.with_grant_handler(Arc::new(Hijack)).is_err());
}

#[tokio::test]
async fn agent_005_users_list_and_revoke_their_grants() {
    let h = sqlite_harness().await;
    let issued = h.issue("tools:read").await;
    assert!(matches!(
        h.verify(&issued.access_token).await,
        AccessVerification::Valid(_)
    ));

    let grants = h.server.list_grants(&issued.user_id).await.unwrap();
    assert_eq!(grants.len(), 1);
    let grant = &grants[0];
    assert_eq!(grant.client_id, issued.client_id);
    assert_eq!(grant.client_name, "Example Tool");
    assert_eq!(grant.client_host, "client.example.test");
    assert_eq!(grant.scopes[0].name, "tools:read");
    assert_eq!(grant.scopes[0].description, "Read your tools");
    assert!(
        grant.last_used_at.is_some(),
        "a verification records the last use"
    );

    let stranger = h.create_user().await;
    assert!(
        !h.server
            .revoke_grant(&stranger, &grant.grant_id)
            .await
            .unwrap()
    );
    assert!(matches!(
        h.verify(&issued.access_token).await,
        AccessVerification::Valid(_)
    ));

    assert!(
        h.server
            .revoke_grant(&issued.user_id, &grant.grant_id)
            .await
            .unwrap()
    );
    assert_eq!(
        h.verify(&issued.access_token).await,
        invalid(InvalidAccessReason::Revoked)
    );
    assert!(
        h.server
            .list_grants(&issued.user_id)
            .await
            .unwrap()
            .is_empty()
    );

    let second = h.issue("tools:read").await;
    assert_eq!(
        h.server.revoke_all_grants(&second.user_id).await.unwrap(),
        1
    );
    assert_eq!(
        h.verify(&second.access_token).await,
        invalid(InvalidAccessReason::Revoked)
    );
}

#[tokio::test]
async fn agent_005_every_endpoint_bounds_its_request() {
    let h = sqlite_harness().await;
    let large = "x".repeat(20 * 1024);
    for path in ["/oauth/token", "/oauth/revoke", "/oauth/authorizations/abc"] {
        let response = h.post_form(path, &[("token", &large)], None).await;
        assert_eq!(status(&response), 413, "{path}");
    }
    let authorize = h.get("/oauth/authorize", &[("state", &large)], None).await;
    assert_eq!(status(&authorize), 413);

    let mut bytes = WireRequest::new(Method::Post, "/oauth/token");
    bytes.body = WireBody::Bytes {
        content_type: Some("application/x-www-form-urlencoded".to_owned()),
        bytes: format!("grant_type=refresh_token&refresh_token={large}").into_bytes(),
    };
    assert_eq!(status(&h.send(bytes, None).await), 413);

    // A repeated parameter is refused (RFC 6749 section 3.2).
    let mut repeated = WireRequest::new(Method::Post, "/oauth/token");
    repeated.body = WireBody::Bytes {
        content_type: Some("application/x-www-form-urlencoded".to_owned()),
        bytes: b"grant_type=refresh_token&grant_type=authorization_code".to_vec(),
    };
    let response = h.send(repeated, None).await;
    assert_eq!(status(&response), 400);
    assert_eq!(body(&response)["error"], "invalid_request");
}

#[tokio::test]
async fn agent_005_expired_tokens_and_lapsed_steps_are_pruned() {
    let mut short = config();
    short.access_token_lifetime = Duration::from_secs(1);
    let h = build(
        magnetar::default_schema::database().await,
        short,
        ClientMetadataPolicy::default(),
        Vec::new(),
    )
    .await;
    let issued = h.issue("tools:read").await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        h.verify(&issued.access_token).await,
        invalid(InvalidAccessReason::Expired)
    );
    assert!(h.server.prune_expired().await.unwrap() >= 1);
    assert_eq!(
        h.verify(&issued.access_token).await,
        invalid(InvalidAccessReason::Unknown)
    );
    // The refresh token has not expired, so it survives and still rotates.
    let refreshed = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(status(&refreshed), 200, "{:?}", body(&refreshed));
}

#[tokio::test]
async fn agent_005_an_error_before_sign_in_never_redirects_to_the_client() {
    let h = sqlite_harness().await;
    let registered = h.register(&[EVIL]).await;
    let document_client = "https://evil.example.test/client.json";
    h.resolver.answer("evil.example.test", &[PUBLIC_ADDRESS]);
    h.documents.serve(
        document_client,
        Script::json(json!({
            "client_id": document_client,
            "client_name": "Anything",
            "redirect_uris": [EVIL],
        })),
    );
    // The document client twice: the second lookup is served from cache.
    for client_id in [registered.as_str(), document_client, document_client] {
        let response = h
            .get(
                "/oauth/authorize",
                &[
                    ("client_id", client_id),
                    ("redirect_uri", EVIL),
                    ("response_type", "token"),
                ],
                None,
            )
            .await;
        let target = location(&response).expect("a redirect to sign in");
        assert!(target.starts_with("/login?return_to="), "{target}");
    }
}

#[tokio::test]
async fn agent_005_a_signed_in_users_error_redirects_only_to_a_loopback_client() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[EVIL, LOOPBACK_REDIRECT]).await;
    let (_, challenge) = pkce();
    for (name, value, error) in [
        ("response_type", "token", "unsupported_response_type"),
        (
            "resource",
            "https://other.example.test/mcp",
            "invalid_target",
        ),
        ("scope", "tools:admin", "invalid_scope"),
    ] {
        for redirect_uri in [EVIL, LOOPBACK_REQUEST] {
            let mut query = vec![
                ("response_type", "code"),
                ("client_id", client_id.as_str()),
                ("redirect_uri", redirect_uri),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("resource", RESOURCE),
                ("state", "s"),
            ];
            query.retain(|(key, _)| *key != name);
            query.push((name, value));
            let response = h.get("/oauth/authorize", &query, Some(&bearer)).await;
            if redirect_uri == EVIL {
                assert_eq!(status(&response), 400, "{name}");
                assert!(location(&response).is_none(), "{name} must not redirect");
                assert_eq!(body(&response)["error"], error);
                assert_no_framing(&response);
            } else {
                let back = location(&response).expect("an error redirect to loopback");
                assert!(
                    back.starts_with("http://127.0.0.1:51000/callback"),
                    "{back}"
                );
                assert_eq!(query_param(&back, "error").as_deref(), Some(error));
                assert_eq!(query_param(&back, "state").as_deref(), Some("s"));
                assert_eq!(query_param(&back, "iss").as_deref(), Some(ISSUER));
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_005_a_concurrently_replayed_code_revokes_its_tokens() {
    concurrent_code_replay(Arc::new(sqlite_harness().await)).await;
}

#[tokio::test]
async fn agent_005_a_code_replayed_while_redeeming_revokes_its_tokens() {
    use magnetar::authorization_server::{AuthorizationStore, SqlAuthorizationStore};

    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT]).await;
    let (verifier, challenge) = pkce();
    let code = h
        .approve(&client_id, REDIRECT, &challenge, "tools:read", &bearer)
        .await;
    let first = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&first), 200);
    let access = body(&first)["access_token"].as_str().unwrap().to_owned();

    // Put the code back in the state a replay sees while the first exchange
    // is still writing its grant.
    let store = SqlAuthorizationStore::new(h.db.clone());
    let digest = format!("{:x}", Sha256::digest(code.as_bytes()));
    let now = chrono::Utc::now();
    let record = store
        .find_authorization(&digest, "code", now)
        .await
        .unwrap()
        .expect("the redeemed code is remembered");
    let grant_id = record
        .state
        .split_once(':')
        .map(|(_, grant)| grant.to_owned())
        .expect("the state names the grant");
    assert!(
        store
            .transition_authorization(
                &digest,
                "code",
                &record.state,
                &format!("redeeming:{grant_id}"),
                Some(now)
            )
            .await
            .unwrap()
    );

    let replay = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(body(&replay)["error"], "invalid_grant");
    assert_eq!(
        h.verify(&access).await,
        invalid(InvalidAccessReason::Revoked)
    );
}

struct RefreshingAgentGrant {
    lifetime: Option<Duration>,
}

#[async_trait]
impl GrantHandler for RefreshingAgentGrant {
    fn grant_type(&self) -> &str {
        "urn:example:refreshing-agent"
    }

    async fn exchange(
        &self,
        _request: &TokenRequest,
        issuer: &TokenIssuer<'_>,
    ) -> Result<TokenResponse, TokenError> {
        issuer
            .issue(
                NewGrant {
                    kind: "agent".to_owned(),
                    user_id: None,
                    client_id: "agent-registration-1".to_owned(),
                    client_name: "Agent".to_owned(),
                    client_host: "agent.example.test".to_owned(),
                    scopes: vec!["tools:read".to_owned()],
                    resource: RESOURCE.to_owned(),
                    auth_epoch: None,
                },
                IssueTokens {
                    access_kind: "agent_access".to_owned(),
                    refresh: true,
                    access_lifetime: self.lifetime,
                },
            )
            .await
    }
}

async fn refreshing_agent_harness(lifetime: impl Into<Option<Duration>>) -> Harness {
    build(
        magnetar::default_schema::database().await,
        config(),
        ClientMetadataPolicy::default(),
        vec![Arc::new(RefreshingAgentGrant {
            lifetime: lifetime.into(),
        })],
    )
    .await
}

#[tokio::test]
async fn agent_005_an_extension_grant_without_a_lifetime_follows_the_configuration() {
    let db = magnetar::default_schema::database().await;
    let handler = || -> Vec<Arc<dyn GrantHandler>> {
        vec![Arc::new(RefreshingAgentGrant { lifetime: None })]
    };
    let before = build(
        db.clone(),
        config(),
        ClientMetadataPolicy::default(),
        handler(),
    )
    .await;
    let issued = body(
        &before
            .post_form(
                "/oauth/token",
                &[("grant_type", "urn:example:refreshing-agent")],
                None,
            )
            .await,
    );
    assert_eq!(issued["expires_in"], 3600);
    let mut shorter = config();
    shorter.access_token_lifetime = Duration::from_secs(300);
    let after = build(db, shorter, ClientMetadataPolicy::default(), handler()).await;
    let refreshed = after
        .refresh(
            "agent-registration-1",
            issued["refresh_token"].as_str().unwrap(),
        )
        .await;
    assert_eq!(status(&refreshed), 200, "{:?}", body(&refreshed));
    assert_eq!(body(&refreshed)["expires_in"], 300);
}

#[tokio::test]
async fn agent_005_refresh_keeps_an_extension_grants_token_kind_and_lifetime() {
    let h = refreshing_agent_harness(Duration::from_secs(600)).await;
    let issued = body(
        &h.post_form(
            "/oauth/token",
            &[("grant_type", "urn:example:refreshing-agent")],
            None,
        )
        .await,
    );
    let refresh = issued["refresh_token"].as_str().expect("a refresh token");
    let refreshed = h.refresh("agent-registration-1", refresh).await;
    assert_eq!(status(&refreshed), 200, "{:?}", body(&refreshed));
    let refreshed = body(&refreshed);
    assert_eq!(refreshed["expires_in"], 600);
    match h.verify(refreshed["access_token"].as_str().unwrap()).await {
        AccessVerification::Valid(grant) => {
            assert_eq!(grant.grant_kind, "agent");
            assert_eq!(grant.token_kind, "agent_access");
        }
        other => panic!("the refreshed token must verify, got {other:?}"),
    }
}

#[tokio::test]
async fn agent_005_an_unrepresentable_lifetime_is_refused_not_a_panic() {
    let h = sqlite_harness().await;
    let mut huge = config();
    huge.refresh_token_lifetime = Duration::from_secs(10_000_000_000_000);
    assert!(
        magnetar::authorization_server::AuthorizationServer::new(
            huge,
            Arc::new(magnetar::authorization_server::SqlAuthorizationStore::new(
                h.db.clone()
            )),
            h.storage.clone(),
            h.limiter.clone(),
            h.fetcher.clone(),
        )
        .is_err(),
        "the configuration check refuses a lifetime no timestamp can hold"
    );

    for lifetime in [Duration::from_secs(10_000_000_000_000), Duration::ZERO] {
        let agent = refreshing_agent_harness(lifetime).await;
        let response = agent
            .post_form(
                "/oauth/token",
                &[("grant_type", "urn:example:refreshing-agent")],
                None,
            )
            .await;
        assert_eq!(status(&response), 400, "{lifetime:?}");
        assert_eq!(body(&response)["error"], "invalid_request");
    }
}

#[tokio::test]
async fn agent_005_a_client_without_the_refresh_grant_gets_no_refresh_token() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let registered = h
        .post_json(
            "/oauth/register",
            json!({"redirect_uris": [REDIRECT], "grant_types": ["authorization_code"]}),
        )
        .await;
    let client_id = body(&registered)["client_id"].as_str().unwrap().to_owned();
    let (verifier, challenge) = pkce();
    let code = h
        .approve(&client_id, REDIRECT, &challenge, "tools:read", &bearer)
        .await;
    let tokens = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&tokens), 200, "{:?}", body(&tokens));
    assert!(body(&tokens).get("refresh_token").is_none());
}

#[tokio::test]
async fn agent_005_pending_authorizations_are_capped_per_user() {
    let h = sqlite_harness().await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT]).await;
    let (_, challenge) = pkce();
    let mut ids = Vec::new();
    for _ in 0..25 {
        let response = h
            .authorize(
                &client_id,
                REDIRECT,
                &challenge,
                "tools:read",
                Some(&bearer),
            )
            .await;
        ids.push(query_param(&location(&response).unwrap(), "authorization").unwrap());
    }
    for (index, id) in ids.iter().enumerate() {
        let view = h
            .get(&format!("/oauth/authorizations/{id}"), &[], Some(&bearer))
            .await;
        let expected = if index < 5 { 404 } else { 200 };
        assert_eq!(status(&view), expected, "pending authorization {index}");
    }
}

#[tokio::test]
async fn agent_005_code_redemption_is_guarded_in_the_database() {
    guarded_redemption(&sqlite_harness().await).await;
}

#[tokio::test]
async fn agent_005_a_decision_is_guarded_in_the_database() {
    guarded_decision(&sqlite_harness().await).await;
}

#[tokio::test]
async fn agent_005_listing_drops_grants_with_no_live_token() {
    let mut short = config();
    short.access_token_lifetime = Duration::from_secs(1);
    let h = build(
        magnetar::default_schema::database().await,
        short,
        ClientMetadataPolicy::default(),
        Vec::new(),
    )
    .await;
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let registered = h
        .post_json(
            "/oauth/register",
            json!({"redirect_uris": [REDIRECT], "grant_types": ["authorization_code"]}),
        )
        .await;
    let client_id = body(&registered)["client_id"].as_str().unwrap().to_owned();
    let (verifier, challenge) = pkce();
    let code = h
        .approve(&client_id, REDIRECT, &challenge, "tools:read", &bearer)
        .await;
    assert_eq!(
        status(&h.exchange(&client_id, REDIRECT, &code, &verifier).await),
        200
    );
    assert_eq!(h.server.list_grants(&user_id).await.unwrap().len(), 1);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        h.server.list_grants(&user_id).await.unwrap().is_empty(),
        "a grant whose only token expired is not listed"
    );
}

#[tokio::test]
async fn agent_005_pruning_deletes_grants_past_their_retention() {
    use magnetar::authorization_server::sql::{grants, tokens};
    use sea_orm::{ColumnTrait, QueryFilter};

    let h = sqlite_harness().await;
    let revoked = h.issue("tools:read").await;
    let idle = h.issue("tools:read").await;
    let live = h.issue("tools:read").await;
    let grant_of = |user: String| {
        let server = h.server.clone();
        async move { server.list_grants(&user).await.unwrap()[0].grant_id.clone() }
    };
    let revoked_grant = grant_of(revoked.user_id.clone()).await;
    let idle_grant = grant_of(idle.user_id.clone()).await;
    let live_grant = grant_of(live.user_id.clone()).await;
    let long_ago = chrono::Utc::now() - chrono::Duration::days(31);

    // One grant revoked 31 days ago; one 31 days old whose tokens are gone.
    assert!(
        h.server
            .revoke_grant(&revoked.user_id, &revoked_grant)
            .await
            .unwrap()
    );
    grants::Entity::update_many()
        .col_expr(
            grants::Column::RevokedAt,
            sea_orm::sea_query::Expr::value(long_ago),
        )
        .filter(grants::Column::Id.eq(revoked_grant.clone()))
        .exec(&h.db)
        .await
        .unwrap();
    tokens::Entity::delete_many()
        .filter(tokens::Column::GrantId.eq(idle_grant.clone()))
        .exec(&h.db)
        .await
        .unwrap();
    grants::Entity::update_many()
        .col_expr(
            grants::Column::CreatedAt,
            sea_orm::sea_query::Expr::value(long_ago),
        )
        .col_expr(
            grants::Column::LastUsedAt,
            sea_orm::sea_query::Expr::value(long_ago),
        )
        .filter(grants::Column::Id.eq(idle_grant.clone()))
        .exec(&h.db)
        .await
        .unwrap();

    assert!(h.server.prune_expired().await.unwrap() >= 2);
    let remaining: Vec<String> = grants::Entity::find()
        .all(&h.db)
        .await
        .unwrap()
        .into_iter()
        .map(|grant| grant.id)
        .collect();
    assert!(
        !remaining.contains(&revoked_grant),
        "a long-revoked grant is deleted"
    );
    assert!(
        !remaining.contains(&idle_grant),
        "a long-idle grant without tokens is deleted"
    );
    assert!(remaining.contains(&live_grant), "a live grant stays");
    let orphans = tokens::Entity::find()
        .filter(tokens::Column::GrantId.eq(revoked_grant))
        .all(&h.db)
        .await
        .unwrap();
    assert!(orphans.is_empty(), "a deleted grant's tokens go with it");
    assert!(matches!(
        h.verify(&live.access_token).await,
        AccessVerification::Valid(_)
    ));
}

#[tokio::test]
async fn agent_005_a_return_address_that_browsers_read_as_another_host_is_dropped() {
    use magnetar::authorization_server::{AuthorizeOutcome, AuthorizeRequest};

    let h = sqlite_harness().await;
    let client_id = h.register(&[REDIRECT]).await;
    let (_, challenge) = pkce();
    let query: std::collections::BTreeMap<String, String> = [
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", REDIRECT),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", RESOURCE),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect();
    for hostile in [
        "//evil.test/x",
        "/\\evil.test/x",
        "https://evil.test/x",
        "evil",
    ] {
        let outcome = h
            .server
            .authorize(AuthorizeRequest {
                query: &query,
                session: None,
                client_address: None,
                return_to: hostile,
            })
            .await
            .unwrap();
        let AuthorizeOutcome::Redirect(sign_in) = outcome else {
            panic!("a redirect to sign in");
        };
        assert_eq!(
            query_param(&sign_in, "return_to").as_deref(),
            Some("/oauth/authorize"),
            "{hostile:?} must not be the return address"
        );
    }
}

#[tokio::test]
async fn agent_005_lifetimes_and_the_pending_cap_are_bounded_in_the_configuration() {
    let h = sqlite_harness().await;
    let attempt = |config: magnetar::authorization_server::AuthorizationServerConfig| {
        magnetar::authorization_server::AuthorizationServer::new(
            config,
            Arc::new(magnetar::authorization_server::SqlAuthorizationStore::new(
                h.db.clone(),
            )),
            h.storage.clone(),
            h.limiter.clone(),
            h.fetcher.clone(),
        )
        .is_ok()
    };
    let mut sub_second_access = config();
    sub_second_access.access_token_lifetime = Duration::from_millis(500);
    assert!(!attempt(sub_second_access), "a sub-second access lifetime");
    let mut fractional_refresh = config();
    fractional_refresh.refresh_token_lifetime = Duration::from_millis(1500);
    assert!(
        !attempt(fractional_refresh),
        "a fractional refresh lifetime"
    );
    let mut too_many = config();
    too_many.max_pending_per_user = 10_001;
    assert!(!attempt(too_many), "a pending cap above 10,000");
    let mut largest = config();
    largest.max_pending_per_user = 10_000;
    assert!(attempt(largest), "a pending cap of 10,000");

    let agent = refreshing_agent_harness(Duration::from_millis(500)).await;
    let response = agent
        .post_form(
            "/oauth/token",
            &[("grant_type", "urn:example:refreshing-agent")],
            None,
        )
        .await;
    assert_eq!(status(&response), 400);
    assert_eq!(body(&response)["error"], "invalid_request");
}

#[tokio::test]
async fn agent_005_a_user_grant_follows_the_configured_access_lifetime() {
    let db = magnetar::default_schema::database().await;
    let before = build(
        db.clone(),
        config(),
        ClientMetadataPolicy::default(),
        Vec::new(),
    )
    .await;
    let issued = before.issue("tools:read").await;

    // The operator shortens the lifetime; an existing grant's next token
    // follows it.
    let mut shorter = config();
    shorter.access_token_lifetime = Duration::from_secs(300);
    let after = build(db, shorter, ClientMetadataPolicy::default(), Vec::new()).await;
    let refreshed = after
        .refresh(&issued.client_id, &issued.refresh_token)
        .await;
    assert_eq!(status(&refreshed), 200, "{:?}", body(&refreshed));
    assert_eq!(body(&refreshed)["expires_in"], 300);
}

#[tokio::test]
async fn agent_005_pruning_keeps_grants_inside_their_retention() {
    pruning_respects_the_retention_boundaries(&sqlite_harness().await).await;
}

#[tokio::test]
async fn agent_005_trimming_is_scoped_and_keeps_the_new_step() {
    trimming_is_scoped_and_keeps_the_new_step(&sqlite_harness().await).await;
}

#[tokio::test]
async fn agent_005_the_final_code_step_ignores_expiry() {
    the_final_code_step_ignores_expiry(&sqlite_harness().await).await;
}

/// Faults a [`FaultyStore`] injects, each counted down as it fires.
#[derive(Default)]
struct Faults {
    /// Rotations that fail.
    rotations: AtomicUsize,
    /// Grant revocations that fail.
    revocations: AtomicUsize,
    /// Final code steps (`redeeming:G` to `redeemed:G`) that fail.
    redeemed_steps: AtomicUsize,
    /// Replay marks (`redeeming:G` to `replayed:G`) that fail.
    replayed_steps: AtomicUsize,
    /// Mark the code replayed just before the next final code step.
    replay_before_final: std::sync::atomic::AtomicBool,
}

fn fire(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
            left.checked_sub(1)
        })
        .is_ok()
}

fn injected() -> magnetar::Error {
    magnetar::Error::Internal {
        message: "injected storage fault".to_owned(),
    }
}

/// The SQL store with faults injected at chosen steps.
struct FaultyStore {
    inner: magnetar::authorization_server::SqlAuthorizationStore,
    faults: Arc<Faults>,
}

#[async_trait]
impl magnetar::authorization_server::AuthorizationStore for FaultyStore {
    async fn insert_client(
        &self,
        client: magnetar::authorization_server::RegisteredClient,
    ) -> magnetar::Result<()> {
        self.inner.insert_client(client).await
    }
    async fn find_client(
        &self,
        client_id: &str,
    ) -> magnetar::Result<Option<magnetar::authorization_server::RegisteredClient>> {
        self.inner.find_client(client_id).await
    }
    async fn create_grant(
        &self,
        grant: magnetar::authorization_server::GrantRecord,
        tokens: Vec<magnetar::authorization_server::NewTokenRecord>,
    ) -> magnetar::Result<()> {
        self.inner.create_grant(grant, tokens).await
    }
    async fn find_token(
        &self,
        token_hash: &str,
    ) -> magnetar::Result<Option<magnetar::authorization_server::TokenRecord>> {
        self.inner.find_token(token_hash).await
    }
    async fn find_grant(
        &self,
        grant_id: &str,
    ) -> magnetar::Result<Option<magnetar::authorization_server::GrantRecord>> {
        self.inner.find_grant(grant_id).await
    }
    async fn rotate_refresh_token(
        &self,
        refresh_token_id: &str,
        grant_id: &str,
        at: chrono::DateTime<chrono::Utc>,
        replacements: Vec<magnetar::authorization_server::NewTokenRecord>,
    ) -> magnetar::Result<bool> {
        if fire(&self.faults.rotations) {
            return Err(injected());
        }
        self.inner
            .rotate_refresh_token(refresh_token_id, grant_id, at, replacements)
            .await
    }
    async fn revoke_token(
        &self,
        token_id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> magnetar::Result<bool> {
        self.inner.revoke_token(token_id, at).await
    }
    async fn revoke_grant(
        &self,
        grant_id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> magnetar::Result<bool> {
        if fire(&self.faults.revocations) {
            return Err(injected());
        }
        self.inner.revoke_grant(grant_id, at).await
    }
    async fn revoke_user_grants(
        &self,
        user_id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> magnetar::Result<u64> {
        self.inner.revoke_user_grants(user_id, at).await
    }
    async fn touch_grant(
        &self,
        grant_id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> magnetar::Result<()> {
        self.inner.touch_grant(grant_id, at).await
    }
    async fn list_user_grants(
        &self,
        user_id: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> magnetar::Result<Vec<magnetar::authorization_server::GrantRecord>> {
        self.inner.list_user_grants(user_id, now).await
    }
    async fn create_authorization(
        &self,
        record: magnetar::authorization_server::AuthorizationRecord,
    ) -> magnetar::Result<()> {
        self.inner.create_authorization(record).await
    }
    async fn find_authorization(
        &self,
        id: &str,
        kind: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> magnetar::Result<Option<magnetar::authorization_server::AuthorizationRecord>> {
        self.inner.find_authorization(id, kind, now).await
    }
    async fn transition_authorization(
        &self,
        id: &str,
        kind: &str,
        expected: &str,
        next: &str,
        live_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> magnetar::Result<bool> {
        if next.starts_with("redeemed:") {
            if fire(&self.faults.redeemed_steps) {
                return Err(injected());
            }
            if self
                .faults
                .replay_before_final
                .swap(false, Ordering::SeqCst)
            {
                let replayed = expected.replacen("redeeming:", "replayed:", 1);
                self.inner
                    .transition_authorization(id, kind, expected, &replayed, None)
                    .await?;
            }
        }
        if next.starts_with("replayed:") && fire(&self.faults.replayed_steps) {
            return Err(injected());
        }
        self.inner
            .transition_authorization(id, kind, expected, next, live_at)
            .await
    }
    async fn consume_authorization(
        &self,
        id: &str,
        kind: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> magnetar::Result<bool> {
        self.inner.consume_authorization(id, kind, now).await
    }
    async fn trim_authorizations(
        &self,
        user_id: &str,
        kind: &str,
        keep: u64,
        newest: &str,
    ) -> magnetar::Result<u64> {
        self.inner
            .trim_authorizations(user_id, kind, keep, newest)
            .await
    }
    async fn prune_expired(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        grant_cutoff: chrono::DateTime<chrono::Utc>,
    ) -> magnetar::Result<u64> {
        self.inner.prune_expired(now, grant_cutoff).await
    }
}

async fn faulty_harness() -> (Harness, Arc<Faults>) {
    let db = magnetar::default_schema::database().await;
    let faults = Arc::new(Faults::default());
    let store = Arc::new(FaultyStore {
        inner: magnetar::authorization_server::SqlAuthorizationStore::new(db.clone()),
        faults: faults.clone(),
    });
    let h = build_with_store(
        db,
        config(),
        ClientMetadataPolicy::default(),
        Vec::new(),
        store,
    )
    .await;
    (h, faults)
}

/// Every grant of the user, with whether it and each of its tokens is
/// revoked, read straight from the tables.
async fn grants_of(h: &Harness, user_id: &str) -> Vec<(String, bool, Vec<bool>)> {
    use magnetar::authorization_server::sql::{grants, tokens};
    use sea_orm::{ColumnTrait, QueryFilter};

    let mut found = Vec::new();
    for grant in grants::Entity::find()
        .filter(grants::Column::UserId.eq(user_id.to_owned()))
        .all(&h.db)
        .await
        .unwrap()
    {
        let token_states = tokens::Entity::find()
            .filter(tokens::Column::GrantId.eq(grant.id.clone()))
            .all(&h.db)
            .await
            .unwrap()
            .into_iter()
            .map(|token| token.revoked_at.is_some())
            .collect();
        found.push((grant.id, grant.revoked_at.is_some(), token_states));
    }
    found
}

async fn approved_code(h: &Harness) -> (String, String, String, String) {
    let user_id = h.create_user().await;
    let bearer = h.sign_in(&user_id).await;
    let client_id = h.register(&[REDIRECT]).await;
    let (verifier, challenge) = pkce();
    let code = h
        .approve(&client_id, REDIRECT, &challenge, "tools:read", &bearer)
        .await;
    (user_id, client_id, code, verifier)
}

#[tokio::test]
async fn agent_005_a_failed_rotation_leaves_the_grant_working() {
    let (h, faults) = faulty_harness().await;
    let issued = h.issue("tools:read").await;
    faults.rotations.store(1, Ordering::SeqCst);
    let failed = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(status(&failed), 500);
    assert_eq!(body(&failed)["error"], "server_error");
    assert!(
        matches!(
            h.verify(&issued.access_token).await,
            AccessVerification::Valid(_)
        ),
        "a transient rotation fault revokes nothing"
    );
    let retried = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(
        status(&retried),
        200,
        "the same refresh token rotates on retry"
    );
}

#[tokio::test]
async fn agent_005_a_reuse_whose_first_revocation_fails_is_still_revoked() {
    let (h, faults) = faulty_harness().await;
    let issued = h.issue("tools:read").await;
    let rotated = body(&h.refresh(&issued.client_id, &issued.refresh_token).await);
    faults.revocations.store(1, Ordering::SeqCst);
    let reused = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(body(&reused)["error"], "invalid_grant");
    assert_eq!(
        h.verify(rotated["access_token"].as_str().unwrap()).await,
        invalid(InvalidAccessReason::Revoked),
        "the retried revocation took"
    );
}

#[tokio::test]
async fn agent_005_a_reuse_whose_revocations_both_fail_is_revoked_on_the_next_presentation() {
    let (h, faults) = faulty_harness().await;
    let issued = h.issue("tools:read").await;
    let rotated = body(&h.refresh(&issued.client_id, &issued.refresh_token).await);
    let access = rotated["access_token"].as_str().unwrap().to_owned();
    faults.revocations.store(2, Ordering::SeqCst);
    let reused = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(status(&reused), 500);
    assert_eq!(body(&reused)["error"], "server_error");
    let again = h.refresh(&issued.client_id, &issued.refresh_token).await;
    assert_eq!(body(&again)["error"], "invalid_grant");
    assert_eq!(
        h.verify(&access).await,
        invalid(InvalidAccessReason::Revoked)
    );
}

#[tokio::test]
async fn agent_005_an_unrecorded_redemption_revokes_its_grant() {
    let (h, faults) = faulty_harness().await;
    let (user_id, client_id, code, verifier) = approved_code(&h).await;
    faults.redeemed_steps.store(1, Ordering::SeqCst);
    let exchanged = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&exchanged), 500);
    assert_eq!(body(&exchanged)["error"], "server_error");
    assert!(body(&exchanged).get("access_token").is_none());
    let grants = grants_of(&h, &user_id).await;
    assert_eq!(grants.len(), 1);
    let (_, grant_revoked, tokens) = &grants[0];
    assert!(
        *grant_revoked,
        "the grant written before the failed step is revoked"
    );
    assert!(tokens.iter().all(|revoked| *revoked));
}

#[tokio::test]
async fn agent_005_a_replay_whose_mark_fails_still_revokes_the_grant() {
    use magnetar::authorization_server::{AuthorizationStore, SqlAuthorizationStore};

    let (h, faults) = faulty_harness().await;
    let (_, client_id, code, verifier) = approved_code(&h).await;
    let first = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&first), 200);
    let access = body(&first)["access_token"].as_str().unwrap().to_owned();

    // Back to the state a replay sees while the grant is being written,
    // then fail both of the replay's marking attempts.
    let store = SqlAuthorizationStore::new(h.db.clone());
    let digest = format!("{:x}", Sha256::digest(code.as_bytes()));
    let now = chrono::Utc::now();
    let record = store
        .find_authorization(&digest, "code", now)
        .await
        .unwrap()
        .unwrap();
    let grant_id = record.state.split_once(':').unwrap().1.to_owned();
    assert!(
        store
            .transition_authorization(
                &digest,
                "code",
                &record.state,
                &format!("redeeming:{grant_id}"),
                None
            )
            .await
            .unwrap()
    );
    faults.replayed_steps.store(2, Ordering::SeqCst);
    let replay = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(body(&replay)["error"], "invalid_grant");
    assert_eq!(
        h.verify(&access).await,
        invalid(InvalidAccessReason::Revoked)
    );
}

#[tokio::test]
async fn agent_005_an_exchange_that_finds_its_code_replayed_revokes_its_own_grant() {
    let (h, faults) = faulty_harness().await;
    let (user_id, client_id, code, verifier) = approved_code(&h).await;
    faults.replay_before_final.store(true, Ordering::SeqCst);
    let exchanged = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(status(&exchanged), 400);
    assert_eq!(body(&exchanged)["error"], "invalid_grant");
    assert!(body(&exchanged).get("access_token").is_none());
    let grants = grants_of(&h, &user_id).await;
    assert_eq!(grants.len(), 1);
    let (_, grant_revoked, tokens) = &grants[0];
    assert!(
        *grant_revoked,
        "the exchange revoked the grant it had written"
    );
    assert!(!tokens.is_empty() && tokens.iter().all(|revoked| *revoked));
}

#[tokio::test]
async fn agent_005_a_replay_retries_its_mark_for_a_grant_not_yet_written() {
    use magnetar::authorization_server::{AuthorizationStore, SqlAuthorizationStore};

    let (h, faults) = faulty_harness().await;
    let (_, client_id, code, verifier) = approved_code(&h).await;
    // A first exchange has taken the code but not written its grant yet.
    let store = SqlAuthorizationStore::new(h.db.clone());
    let digest = format!("{:x}", Sha256::digest(code.as_bytes()));
    assert!(
        store
            .transition_authorization(&digest, "code", "issued", "redeeming:unwritten", None)
            .await
            .unwrap()
    );
    faults.replayed_steps.store(1, Ordering::SeqCst);
    let replay = h.exchange(&client_id, REDIRECT, &code, &verifier).await;
    assert_eq!(body(&replay)["error"], "invalid_grant");
    let record = store
        .find_authorization(&digest, "code", chrono::Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.state, "replayed:unwritten",
        "the second marking attempt reached the code, so the first exchange will revoke"
    );
}
