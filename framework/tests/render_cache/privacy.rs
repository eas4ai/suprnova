//! Task 18: the privacy-leak suite. No principal's bytes reach another
//! visitor, no locale's bytes reach another locale, and a route that
//! declares and observes nothing still caches.
//!
//! # Why this file exists separately from `render_cache_middleware.rs`
//!
//! The RenderCache publication guard was reviewed eight times and broken in
//! six of them. Every attack below was proven against a version of that
//! guard its author believed closed, most of them over real HTTP with the
//! render count establishing a cache hit. Ruling R82 makes each one a
//! permanent test *here*, independent of whatever the middleware suite did
//! to fix it, so a regression that reintroduces any of them fails in a file
//! that was never part of the fix.
//!
//! Independence is the whole point, so this file shares nothing with that
//! suite: its own support module, its own routes, its own policies, its own
//! harness. See `render_cache_privacy_support`'s module doc.
//!
//! # How every test here proves what it claims
//!
//! A leak is a request served without its handler running. So each test
//! dispatches two or more requests across the boundary it is about and
//! asserts three things together: the render count (a hit is a count that
//! did not move), the body (whose page was actually served), and, where the
//! route is meant to cache, that the repeat *is* a hit. A test that
//! asserted only "the bodies differ" would pass against a guard that
//! declines everything, and a test that asserted only the count would pass
//! against one that served the wrong page twice.
//!
//! Every test was proven by reverting the production line that closes its
//! attack and watching it fail with a cross-boundary serve. The line per
//! test is recorded in this iteration's task 18 report.
//!
//! # Test runtime
//!
//! Every test is `#[serial_test::serial]`: `RenderCache`'s installed
//! runtime and the process-wide global middleware registry are both
//! process-global, so two of these running at once would install over each
//! other. Plain `#[tokio::test]` (current-thread), not
//! `flavor = "multi_thread"`: the harness uses `TestContainer::fake()`,
//! which writes a thread-local, and a multi-thread runtime can migrate a
//! future between worker threads between polls, making that registration
//! invisible to whichever thread resumes the test.
use crate::render_cache_privacy_support;

use render_cache_privacy_support::{
    AUTHZ_DRIVEN_ROUTE, Harness, IMPERSONATED_ROUTE, LOCALE_LATE_MIDDLEWARE_ROUTE,
    LOCALE_NESTED_SCOPE_ROUTE, LOCALE_SWITCHES_ROUTE, LOCALE_VARIES_ROUTE, NAMED_GUARD_ONLY_ROUTE,
    NAMED_THEN_DEFAULT_ROUTE, PLAIN_ROUTE, PRINCIPAL_DECLARED_AUTHZ_ROUTE,
    PRINCIPAL_DECLARED_READS_IDENTITY_ROUTE, PRIVATE_ROUTE, READS_AUTH_ID_ROUTE,
    READS_COOKIE_ROUTE, READS_CRATE_ROOT_AUTH_USER_ID_ROUTE, READS_GLOBAL_FLAG_ROUTE,
    READS_OVERRIDE_FLAG_ROUTE, READS_SESSION_MUT_ROUTE, READS_USER_SCOPED_FLAG_ROUTE,
    REQUEST_AUTH_USER_ID_ROUTE, STITCHED_DOCUMENT_KEY, STITCHED_ROUTE,
    TENANT_DECLARED_READS_IDENTITY_ROUTE, TENANT_VARIES_ROUTE, UNDECLARED_LOCALE_ROUTE, attribute,
    boot_with_cache_installed_before_the_auth_middleware, boot_with_render_cache, counting_route,
    dispatch_get, ensure_role_gate, island_tag, route_is_under_a_policy, session_cookie,
    stitched_scope,
};
use suprnova::StatusCode;
use suprnova::render_cache::RenderCache;

/// R103's positive control, in the shape the ruling names: the same visitor
/// twice on `path` must render exactly once, and both responses must carry a
/// validator, before the caller goes on to prove a boundary holds. Returns
/// the render count afterwards, so each test expresses its own expectations
/// relative to it rather than to a hard-coded total.
///
/// # Why every leak test needs one
///
/// Sixteen of this suite's twenty assertions are that something did *not*
/// get cached, and a route that is never cached can never leak. The review
/// measured both halves of that: sixteen tests pass against a guard that
/// declines every publication, and *all* twenty pass when a single route's
/// `.try_render_cache(...)` opt-in is deleted from the support module. A
/// regression that disabled the cache, for the whole boot or for one single
/// route, would have left this suite green, which is the opposite of why it
/// exists.
///
/// So each leak test first proves the safe direction really is stored, in
/// this same boot and this same harness. Where the attacked route can itself
/// cache for a visitor who does not cross the boundary, this runs against
/// that route on a different parameter, which is the strongest form. Where
/// the attacked route can never be stored - the undeclared-dimension
/// declines, and the session and cookie reads - this runs against a sibling
/// route that declares the dimension, and the test additionally asserts
/// through [`route_is_under_a_policy`] that its own route is still attached,
/// because no behavioural signal distinguishes "declining correctly" from
/// "never registered".
///
/// The `ETag` is the load-bearing half of the header check: a declined
/// render returns the handler's own response with no cache validators at all
/// (`render_and_publish` returns early), so a route that publishes and a
/// route that declines are told apart by exactly this.
async fn same_visitor_twice_is_a_hit(
    harness: &Harness,
    path: &str,
    headers: &[(&str, &str)],
) -> u64 {
    let before = counting_route::renders();
    let first = dispatch_get(harness, path, headers).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        before + 1,
        "positive control: the first request to {path} must run the handler"
    );
    assert!(
        first.header("etag").is_some(),
        "positive control: {path} must publish, so its own response carries a \
         validator - a declined render carries none"
    );

    let second = dispatch_get(harness, path, headers).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        before + 1,
        "positive control: the second identical request to {path} must be a cache \
         hit. If it is not, this boot is storing nothing and the boundary \
         assertion below would pass against a cache that had been switched off"
    );
    assert!(
        second.header("etag").is_some(),
        "positive control: the hit replays the stored validator"
    );
    assert_eq!(second.text(), first.text());
    counting_route::renders()
}

// ── R82, attack 1 ──────────────────────────────────────────────────────

/// A logged-in user's page must not be served to a different logged-in
/// user, nor to an anonymous visitor, on a route that declares no per-user
/// keying. The original break: the class was narrowed to `PrivateCached`
/// but the key was never repartitioned, so one key held one person's page
/// and everybody hit it.
#[tokio::test]
#[serial_test::serial]
async fn a_logged_in_users_page_never_reaches_another_visitor_on_a_route_with_no_per_user_keying() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, sibling shape: this route can never be stored
    // (it reads an identity and declares no dimension), so the control is a
    // route that reads the same identity and *does* declare `Principal`.
    let base =
        same_visitor_twice_is_a_hit(&harness, "/privacy/private/1", &[("x-test-login", "alice")])
            .await;
    assert!(
        route_is_under_a_policy(&harness, READS_AUTH_ID_ROUTE),
        "the attacked route must still be attached to a policy: a deleted opt-in \
         makes every assertion below vacuous"
    );

    let alice = dispatch_get(&harness, READS_AUTH_ID_ROUTE, &[("x-test-login", "alice")]).await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(alice.text().contains("for alice"), "got {}", alice.text());

    let bob = dispatch_get(&harness, READS_AUTH_ID_ROUTE, &[("x-test-login", "bob")]).await;
    assert!(
        !bob.text().contains("alice"),
        "a route with no declared Principal variance must never serve alice's rendered \
         body to bob - got {}",
        bob.text()
    );
    assert!(bob.text().contains("for bob"), "got {}", bob.text());

    let anonymous = dispatch_get(&harness, READS_AUTH_ID_ROUTE, &[]).await;
    assert!(
        !anonymous.text().contains("alice") && !anonymous.text().contains("bob"),
        "nor to an anonymous visitor - got {}",
        anonymous.text()
    );

    assert_eq!(
        counting_route::renders(),
        base + 3,
        "none of the three renders was ever safe to store: a bug that merely narrowed \
         the served class without repartitioning the key would have rendered once and \
         served that page to all three"
    );

    let alice_again =
        dispatch_get(&harness, READS_AUTH_ID_ROUTE, &[("x-test-login", "alice")]).await;
    assert!(
        alice_again.text().contains("for alice"),
        "got {}",
        alice_again.text()
    );
    assert_ne!(
        alice_again.text(),
        alice.text(),
        "the body carries its own render number, so a fresh number is independent \
         evidence that the handler really ran again"
    );
    assert_eq!(
        counting_route::renders(),
        base + 4,
        "and the route never publishes at all, so even a repeat of the same identity \
         renders again"
    );
}

// ── R82, attack 2 ──────────────────────────────────────────────────────

/// The framework has more than one way to reach the current user, and the
/// crate-root `suprnova::auth_user_id()` consults request state before the
/// session-backed path - so a bearer-token or remember-me identity is read
/// without `Auth::id()`'s own explicit observation ever running. The read
/// has to be observed at the request-state seam itself, not at one named
/// accessor.
#[tokio::test]
#[serial_test::serial]
async fn an_identity_read_through_the_crate_root_accessor_never_crosses_visitors() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, sibling shape: this route can never be stored
    // (it reads an identity and declares no dimension), so the control is a
    // route that reads the same identity and *does* declare `Principal`.
    let base =
        same_visitor_twice_is_a_hit(&harness, "/privacy/private/1", &[("x-test-login", "alice")])
            .await;
    assert!(
        route_is_under_a_policy(&harness, READS_CRATE_ROOT_AUTH_USER_ID_ROUTE),
        "the attacked route must still be attached to a policy"
    );

    let alice = dispatch_get(
        &harness,
        READS_CRATE_ROOT_AUTH_USER_ID_ROUTE,
        &[("x-test-login", "alice")],
    )
    .await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(alice.text().contains("for alice"), "got {}", alice.text());

    let bob = dispatch_get(
        &harness,
        READS_CRATE_ROOT_AUTH_USER_ID_ROUTE,
        &[("x-test-login", "bob")],
    )
    .await;
    assert!(
        !bob.text().contains("alice"),
        "an identity read through the crate-root accessor is still an identity read - \
         got {}",
        bob.text()
    );
    assert!(bob.text().contains("for bob"), "got {}", bob.text());
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "neither render was ever safe to store"
    );
}

// ── R82, attack 3 ──────────────────────────────────────────────────────

/// A body can depend on an authorization decision without touching any
/// identity accessor at all. That narrows the class without setting any of
/// the flags a material-based guard inspects, so the guard has to drive off
/// the classification reason and require a partitioning dimension for it.
#[tokio::test]
#[serial_test::serial]
async fn a_body_driven_by_an_authorization_decision_alone_never_crosses_roles() {
    ensure_role_gate();
    let harness = boot_with_render_cache().await;
    // R103 positive control, sibling shape: `AuthorizationRead` requires the
    // `Principal` dimension, so the same gate-driven handler on a route that
    // declares it does cache - here through the guard's empty-set arm, since
    // an anonymous visitor's key resolves `Anonymous` and the gate names no
    // identity.
    let base =
        same_visitor_twice_is_a_hit(&harness, "/privacy/principal-declared-authz/1", &[]).await;
    assert!(
        route_is_under_a_policy(&harness, AUTHZ_DRIVEN_ROUTE),
        "the attacked route must still be attached to a policy"
    );
    assert!(
        route_is_under_a_policy(&harness, PRINCIPAL_DECLARED_AUTHZ_ROUTE),
        "and so must its control"
    );

    let admin = dispatch_get(&harness, AUTHZ_DRIVEN_ROUTE, &[("x-test-role", "admin")]).await;
    assert_eq!(admin.status, StatusCode::OK);
    assert!(
        admin.text().contains("allowed=true"),
        "sanity: the gate is registered and actually decides - got {}",
        admin.text()
    );

    let guest = dispatch_get(&harness, AUTHZ_DRIVEN_ROUTE, &[("x-test-role", "guest")]).await;
    assert!(
        guest.text().contains("allowed=false"),
        "a route with no declared variance must never serve an authorization-gated body \
         computed for one role to a request that would have gotten a different decision \
         - got {}",
        guest.text()
    );
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "an authorization read narrows the class with nothing in the key to partition \
         by, which must decline to store, not merely narrow"
    );
}

// ── R82, attack 4 ──────────────────────────────────────────────────────

/// A tenant-keyed route partitions by tenant and by nothing else. A handler
/// that also reads an identity therefore serves one user's page to another
/// user inside the same tenant unless the identity read requires its own
/// dimension.
#[tokio::test]
#[serial_test::serial]
async fn a_tenant_keyed_route_never_crosses_users_inside_one_tenant() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, sibling shape: this route can never be stored
    // (its render reads an identity it does not declare), so the control is a
    // route that declares `Tenant` and reads only the tenant.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/tenant-varies/1",
        &[("x-test-tenant", "acme")],
    )
    .await;
    assert!(
        route_is_under_a_policy(&harness, TENANT_DECLARED_READS_IDENTITY_ROUTE),
        "the attacked route must still be attached to a policy"
    );

    let alice = dispatch_get(
        &harness,
        "/privacy/tenant-declared-reads-identity/1",
        &[("x-test-tenant", "acme"), ("x-test-login", "alice")],
    )
    .await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(
        alice.text().contains("tenant=acme") && alice.text().contains("for alice"),
        "got {}",
        alice.text()
    );

    let bob = dispatch_get(
        &harness,
        "/privacy/tenant-declared-reads-identity/1",
        &[("x-test-tenant", "acme"), ("x-test-login", "bob")],
    )
    .await;
    assert!(
        !bob.text().contains("for alice"),
        "two users of one tenant derive the same key on a Tenant-only route, so alice's \
         page must never be served to bob - got {}",
        bob.text()
    );
    assert!(bob.text().contains("for bob"), "got {}", bob.text());
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "the identity read requires the Principal dimension, which this route does not \
         declare, so neither render was ever safe to store"
    );
    // The route pattern is named through the support module's own constant
    // so a rename cannot silently detach the policy from the dispatches
    // above.
    assert_eq!(
        TENANT_DECLARED_READS_IDENTITY_ROUTE,
        "/privacy/tenant-declared-reads-identity/{id}"
    );
}

// ── R82, attack 5 ──────────────────────────────────────────────────────

/// An identity taken through a guard other than the default one is invisible
/// to `Auth::id()`, which is what the key's `Principal` dimension is always
/// built from. The key resolves `Anonymous` and partitions nothing, while
/// the body is a specific person's.
#[tokio::test]
#[serial_test::serial]
async fn an_identity_taken_through_a_non_default_guard_never_crosses_visitors() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, on the attacked route itself: a visitor signed
    // in on *both* guards under one identity derives a key the render's own
    // observation matches, so this route does cache when nothing crosses.
    // Parameter 1 here, parameter 2 for the attack, so the control's entry
    // can never be what the attack hits.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/named-guard-only/1",
        &[("x-test-login", "carol"), ("x-test-named-login", "carol")],
    )
    .await;

    let carol = dispatch_get(
        &harness,
        "/privacy/named-guard-only/2",
        &[("x-test-named-login", "carol")],
    )
    .await;
    assert_eq!(carol.status, StatusCode::OK);
    assert!(carol.text().contains("for carol"), "got {}", carol.text());

    let dora = dispatch_get(
        &harness,
        "/privacy/named-guard-only/2",
        &[("x-test-named-login", "dora")],
    )
    .await;
    assert!(
        !dora.text().contains("carol"),
        "the route declares Principal, but `Auth::id()` sees nothing for either request, \
         so both keys resolve Anonymous and carol's page must never be served to dora - \
         got {}",
        dora.text()
    );
    assert!(dora.text().contains("for dora"), "got {}", dora.text());
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "a declared dimension that resolves to a constant partitions nothing, so the \
         observed value must be compared against it and both renders declined"
    );
    assert_eq!(NAMED_GUARD_ONLY_ROUTE, "/privacy/named-guard-only/{id}");
}

// ── R82, attack 6 ──────────────────────────────────────────────────────

/// `session_mut` is the idiomatic read-and-touch accessor for session state,
/// and its closure can read whatever it also mutates. It records a session
/// read exactly as `session()` does, or a render depending on session state
/// through it is stored as though it depended on nothing.
#[tokio::test]
#[serial_test::serial]
async fn a_session_mut_read_is_observed_and_declines_storage() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, sibling shape: a session read narrows to
    // `Uncacheable` whatever the route declares, so there is no dimension to
    // declare and the control is simply a route that does cache in this boot.
    let base = same_visitor_twice_is_a_hit(&harness, "/privacy/plain/1", &[]).await;
    assert!(
        route_is_under_a_policy(&harness, READS_SESSION_MUT_ROUTE),
        "the attacked route must still be attached to a policy"
    );

    let first = dispatch_get(&harness, READS_SESSION_MUT_ROUTE, &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(
        first.header("etag").is_none(),
        "a declined render carries no validator, which is the same signal the \
         control above used in the other direction"
    );
    let second = dispatch_get(&harness, READS_SESSION_MUT_ROUTE, &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "a session value read forces Uncacheable, so the second identical request must \
         render again rather than hit"
    );
    assert_ne!(
        first.text(),
        second.text(),
        "the bodies carry their own render numbers, so two distinct bodies is \
         independent evidence that the second request really did run the handler"
    );
}

// ── R82, attack 7 ──────────────────────────────────────────────────────

/// `RenderCache::install` appends to the global middleware registry, so
/// `RenderCacheMiddleware` derives the key before any per-route middleware
/// runs. An impersonation middleware - which the framework explicitly
/// supports - then sets the real identity after key derivation, so the key
/// holds a genuine but wrong private identity.
#[tokio::test]
#[serial_test::serial]
async fn impersonation_after_key_derivation_never_serves_one_target_to_another() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, on the attacked route itself: with no
    // impersonation the key alice derives and the identity the render sees
    // are the same, so this route caches. Parameter 1 for the control,
    // parameter 2 for the attack.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/impersonated/1",
        &[("x-test-login", "alice")],
    )
    .await;

    let first = dispatch_get(
        &harness,
        "/privacy/impersonated/2",
        &[
            ("x-test-login", "alice"),
            ("x-test-impersonate", "victim-one"),
        ],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(
        first.text().contains("for victim-one"),
        "sanity: the impersonation middleware really does run after key derivation and \
         before the handler - got {}",
        first.text()
    );

    let second = dispatch_get(
        &harness,
        "/privacy/impersonated/2",
        &[
            ("x-test-login", "alice"),
            ("x-test-impersonate", "victim-two"),
        ],
    )
    .await;
    assert!(
        !second.text().contains("victim-one"),
        "both requests derive the same key (alice, the impersonator), so victim-one's \
         page must never be served to a request rendering as victim-two - got {}",
        second.text()
    );
    assert!(
        second.text().contains("for victim-two"),
        "got {}",
        second.text()
    );
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "the key says alice and the render said victim-one, which must decline"
    );
    assert_eq!(IMPERSONATED_ROUTE, "/privacy/impersonated/{id}");
}

// ── R82, attack 8 ──────────────────────────────────────────────────────

/// A cookie read produces no classification reason of its own. Cookies
/// carry private material by nature, so a cookie read counts as a session
/// read or it costs the guard nothing at all.
#[tokio::test]
#[serial_test::serial]
async fn a_cookie_read_counts_as_a_session_read_and_declines_storage() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, sibling shape: same reasoning as the
    // `session_mut` test - a session read narrows unconditionally, so the
    // control is a route that does cache in this boot.
    let base = same_visitor_twice_is_a_hit(&harness, "/privacy/plain/1", &[]).await;
    assert!(
        route_is_under_a_policy(&harness, READS_COOKIE_ROUTE),
        "the attacked route must still be attached to a policy"
    );

    let first = dispatch_get(
        &harness,
        READS_COOKIE_ROUTE,
        &[("cookie", "session=first-visitor")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);
    let second = dispatch_get(
        &harness,
        READS_COOKIE_ROUTE,
        &[("cookie", "session=second-visitor")],
    )
    .await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "a cookie read must produce a session read, which forces Uncacheable; a read \
         that produced no classification reason at all would have published the first \
         visitor's page and served it to the second"
    );
    assert_ne!(first.text(), second.text());
}

// ── R82, attack 9 ──────────────────────────────────────────────────────

/// `Lang::set_locale` is documented as supported mid-request, and the key
/// was already fixed at the pre-switch locale by the time the handler runs.
/// Both requests here derive the *same* key and render *different*
/// languages, so a guard that misses the switch serves one language's page
/// to the other.
#[tokio::test]
#[serial_test::serial]
async fn a_mid_render_locale_switch_never_publishes_under_the_pre_switch_key() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, on the attacked route itself: with no switch
    // requested the render's locale and the key's locale are the same, so
    // this route caches. Parameter 1 for the control, parameter 2 for the
    // attack.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/locale-switches/1",
        &[("x-test-locale", "en")],
    )
    .await;

    let french = dispatch_get(
        &harness,
        "/privacy/locale-switches/2",
        &[("x-test-locale", "en"), ("x-test-switch-to", "fr")],
    )
    .await;
    assert_eq!(french.status, StatusCode::OK);
    assert!(
        french.text().contains("before=en") && french.text().contains("after=fr"),
        "sanity: the key was derived at en and the body was rendered at fr - got {}",
        french.text()
    );

    let german = dispatch_get(
        &harness,
        "/privacy/locale-switches/2",
        &[("x-test-locale", "en"), ("x-test-switch-to", "de")],
    )
    .await;
    assert!(
        !german.text().contains("after=fr"),
        "both requests key on en, so the French body must never be served to the request \
         that switched to German - got {}",
        german.text()
    );
    assert!(german.text().contains("after=de"), "got {}", german.text());
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "the locale the render actually used differs from the one the key holds, which \
         must decline"
    );
    assert_eq!(LOCALE_SWITCHES_ROUTE, "/privacy/locale-switches/{id}");
}

// ── R82, attack 10 ─────────────────────────────────────────────────────

/// `scope_locale` is the framework's own API for a mid-render locale
/// switch, and the nested scope it opens pops the instant its future
/// resolves - before the handler returns, and long before any post-render
/// re-read of the same task-local could look. Re-derivation cannot see
/// this; only recording the value at the point of each read can.
#[tokio::test]
#[serial_test::serial]
async fn a_render_inside_a_nested_scope_locale_never_publishes_under_the_outer_locales_key() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, on the attacked route itself: with no switch
    // requested the render's locale and the key's locale are the same, so
    // this route caches. Parameter 1 for the control, parameter 2 for the
    // attack.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/locale-nested-scope/1",
        &[("x-test-locale", "en")],
    )
    .await;

    let french = dispatch_get(
        &harness,
        "/privacy/locale-nested-scope/2",
        &[("x-test-locale", "en"), ("x-test-nested-locale", "fr")],
    )
    .await;
    assert_eq!(french.status, StatusCode::OK);
    assert!(
        french.text().contains("locale=fr"),
        "sanity: the body really was rendered inside the nested scope - got {}",
        french.text()
    );

    let german = dispatch_get(
        &harness,
        "/privacy/locale-nested-scope/2",
        &[("x-test-locale", "en"), ("x-test-nested-locale", "de")],
    )
    .await;
    assert!(
        !german.text().contains("locale=fr"),
        "both requests key on the outer en scope, so the French body must never be \
         served to the German one - got {}",
        german.text()
    );
    assert!(german.text().contains("locale=de"), "got {}", german.text());
    assert_eq!(counting_route::renders(), base + 2);
    assert_eq!(
        LOCALE_NESTED_SCOPE_ROUTE,
        "/privacy/locale-nested-scope/{id}"
    );
}

// ── R82, attack 11 ─────────────────────────────────────────────────────

/// A per-route locale middleware always composes closer to the handler than
/// `RenderCacheMiddleware`, which is the only position such a middleware can
/// occupy. It sets the locale after the key is derived and its own scope
/// pops before its `next(request)` returns.
#[tokio::test]
#[serial_test::serial]
async fn a_locale_middleware_installed_after_the_cache_never_publishes_under_the_earlier_key() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, on the attacked route itself: with no switch
    // requested the render's locale and the key's locale are the same, so
    // this route caches. Parameter 1 for the control, parameter 2 for the
    // attack.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/locale-late-middleware/1",
        &[("x-test-locale", "en")],
    )
    .await;

    let french = dispatch_get(
        &harness,
        "/privacy/locale-late-middleware/2",
        &[("x-test-locale", "en"), ("x-test-late-locale", "fr")],
    )
    .await;
    assert_eq!(french.status, StatusCode::OK);
    assert!(
        french.text().contains("locale=fr"),
        "sanity: the late middleware really does supply the locale the handler sees - \
         got {}",
        french.text()
    );

    let german = dispatch_get(
        &harness,
        "/privacy/locale-late-middleware/2",
        &[("x-test-locale", "en"), ("x-test-late-locale", "de")],
    )
    .await;
    assert!(
        !german.text().contains("locale=fr"),
        "both requests key on en, so the French body must never be served to the German \
         one - got {}",
        german.text()
    );
    assert!(german.text().contains("locale=de"), "got {}", german.text());
    assert_eq!(counting_route::renders(), base + 2);
    assert_eq!(
        LOCALE_LATE_MIDDLEWARE_ROUTE,
        "/privacy/locale-late-middleware/{id}"
    );
}

// ── R82, attack 12 ─────────────────────────────────────────────────────

/// A handler that builds its body from one guard's identity and then
/// touches a second accessor for an unrelated check. A record that keeps one
/// slot per dimension keeps only the second value, which is exactly the one
/// the key was built from, so the comparison passes while the body came from
/// the first.
#[tokio::test]
#[serial_test::serial]
async fn a_second_identity_touch_never_overwrites_the_first_the_body_was_built_from() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, on the attacked route itself: when both
    // accessors resolve to one identity there is only one observed value and
    // it is the key's, so this route caches. That is also the sharpest form
    // of the control here, because it shows the guard comparing values
    // rather than declining whenever two accessors are touched. Parameter 1
    // for the control, parameter 2 for the attack.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/named-then-default/1",
        &[("x-test-login", "alice"), ("x-test-named-login", "alice")],
    )
    .await;

    let carol = dispatch_get(
        &harness,
        "/privacy/named-then-default/2",
        &[("x-test-login", "zed"), ("x-test-named-login", "carol")],
    )
    .await;
    assert_eq!(carol.status, StatusCode::OK);
    assert!(
        carol.text().contains("for carol"),
        "sanity: the body comes from the named guard, not from `Auth::id()` - got {}",
        carol.text()
    );

    let dora = dispatch_get(
        &harness,
        "/privacy/named-then-default/2",
        &[("x-test-login", "zed"), ("x-test-named-login", "dora")],
    )
    .await;
    assert!(
        !dora.text().contains("carol"),
        "both requests are signed in as zed on the default guard, so both keys are \
         zed's; carol's page must never be served to the request whose body came from \
         dora - got {}",
        dora.text()
    );
    assert!(dora.text().contains("for dora"), "got {}", dora.text());
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "a render that saw two different values for one dimension cannot be represented \
         by any one key, so every observed value must be compared and both renders \
         declined"
    );
    assert_eq!(NAMED_THEN_DEFAULT_ROUTE, "/privacy/named-then-default/{id}");
}

// ── R82, attack 13 ─────────────────────────────────────────────────────

/// The feature middleware resolves the identity once, before the render,
/// and stashes it where `is_enabled!` reads it ambiently during the render.
/// Nothing the render touches is an instrumented accessor, so the read has
/// to be observed inside the evaluator itself - on the miss, where the
/// database evaluator runs, and on the hit, where the cached evaluator
/// never reaches it and must replay what the miss consulted.
#[tokio::test]
#[serial_test::serial]
async fn a_feature_flags_identity_is_observed_on_the_evaluator_miss_and_replayed_on_the_hit() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, sibling shape: a *globally* scoped flag's answer
    // does not depend on the reader, so reading one records nothing and the
    // page still caches, even for a signed-in visitor whose id the feature
    // middleware has already put in the ambient context. Without this
    // control, a change that made every flag read uncacheable would leave
    // this test green while disabling the cache for every page in an
    // application that checks any flag.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/reads-global-flag/1",
        &[("x-test-login", "alice")],
    )
    .await;
    assert!(
        route_is_under_a_policy(&harness, READS_USER_SCOPED_FLAG_ROUTE),
        "the attacked route must still be attached to a policy"
    );
    assert!(
        route_is_under_a_policy(&harness, READS_GLOBAL_FLAG_ROUTE),
        "and so must its control"
    );

    let alice = dispatch_get(
        &harness,
        "/privacy/reads-user-scoped-flag/1",
        &[("x-test-login", "alice")],
    )
    .await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(
        alice.text().contains("enabled=true"),
        "sanity: the user-scoped rule really does belong to alice - got {}",
        alice.text()
    );
    assert_eq!(
        counting_route::renders(),
        base + 1,
        "sanity: the first request is the flag evaluator's own miss"
    );

    // The same identity again. The flag evaluator's cache now holds
    // alice's answer, so this read never reaches the database evaluator -
    // it is served from the cached evaluator's own entry, which has to
    // replay the identity axes the miss consulted or nothing narrows.
    let alice_again = dispatch_get(
        &harness,
        "/privacy/reads-user-scoped-flag/1",
        &[("x-test-login", "alice")],
    )
    .await;
    assert_eq!(alice_again.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "the flag-cache hit is exactly as identity-dependent as the miss, so this render \
         must decline too rather than publish alice's page under a key with no \
         Principal dimension"
    );

    let bob = dispatch_get(
        &harness,
        "/privacy/reads-user-scoped-flag/1",
        &[("x-test-login", "bob")],
    )
    .await;
    assert!(
        bob.text().contains("enabled=false"),
        "bob has no rule and falls through to the default, so his answer differs from \
         alice's and he must never be served her page - got {}",
        bob.text()
    );
    assert_eq!(
        counting_route::renders(),
        base + 3,
        "three renders: the miss, the flag-cache hit, and bob"
    );
    assert_eq!(
        READS_USER_SCOPED_FLAG_ROUTE,
        "/privacy/reads-user-scoped-flag/{id}"
    );
}

// ── R82, attack 14 ─────────────────────────────────────────────────────

/// A route may declare `Principal` and still partition nothing, if the
/// cache is installed before the auth middleware: the declared dimension
/// resolves `Anonymous` for every request while the render observes a real
/// principal. "Declared" is not the same as "partitions".
#[tokio::test]
#[serial_test::serial]
async fn a_principal_declaring_route_whose_key_resolves_anonymous_never_crosses_visitors() {
    let harness = boot_with_cache_installed_before_the_auth_middleware().await;
    // R103 positive control, sibling shape: under this boot no
    // `Principal`-declaring route can cache at all, because the key always
    // resolves `Anonymous` while the render sees a real identity. The tenant
    // middleware is still registered before `install` here, so a
    // `Tenant`-declaring route does cache and proves this boot stores
    // representations.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/tenant-varies/1",
        &[("x-test-tenant", "acme")],
    )
    .await;
    assert!(
        route_is_under_a_policy(&harness, PRINCIPAL_DECLARED_READS_IDENTITY_ROUTE),
        "the attacked route must still be attached to a policy"
    );

    let alice = dispatch_get(
        &harness,
        "/privacy/principal-declared-reads-identity/1",
        &[("x-test-login", "alice")],
    )
    .await;
    assert_eq!(alice.status, StatusCode::OK);
    assert!(alice.text().contains("for alice"), "got {}", alice.text());

    let bob = dispatch_get(
        &harness,
        "/privacy/principal-declared-reads-identity/1",
        &[("x-test-login", "bob")],
    )
    .await;
    assert!(
        !bob.text().contains("alice"),
        "the route declares Principal, but the cache runs before the auth middleware, so \
         every key resolves Anonymous and alice's page must never be served to bob - \
         got {}",
        bob.text()
    );
    assert!(bob.text().contains("for bob"), "got {}", bob.text());
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "checking only that the dimension is declared is not enough: the observed value \
         has to be compared against the value the key actually holds"
    );
    assert_eq!(
        PRINCIPAL_DECLARED_READS_IDENTITY_ROUTE,
        "/privacy/principal-declared-reads-identity/{id}"
    );
}

// ── R82, attack 15 ─────────────────────────────────────────────────────

/// A declared dimension that resolves to a constant partitions nothing.
/// This asserts the positive direction for `Principal`: two distinct
/// principals derive two distinct keys, each caches in its own partition,
/// and neither ever sees the other's entry. A test that dispatched the same
/// login twice could not distinguish a real partition from a guard that
/// always passes.
#[tokio::test]
#[serial_test::serial]
async fn two_distinct_principals_derive_two_distinct_keys_and_two_distinct_entries() {
    let harness = boot_with_render_cache().await;

    // Every claim below is measured on the request path. The only public
    // key-derivation entry point, `RenderCache::key_for_route_for_test`, goes
    // through `key_input_for_test`, which re-implements the `Principal` arm
    // rather than calling `variance_descriptor` - so a key-text assertion on
    // its own would measure the helper, and did: the review forced
    // production's `Principal` arm to a constant and the key texts still came
    // back distinct while the route had stopped partitioning.
    //
    // The helper is used here only as a *probe against production storage*:
    // each key is handed to `RenderCache::inspect`, which is a raw lookup in
    // the store production actually wrote. An entry comes back only if the
    // helper's key and production's key agree, so a divergence between them
    // fails here rather than hiding.
    let alice_key =
        RenderCache::key_for_route_for_test(PRIVATE_ROUTE, &[("id", "1")], Some("alice"));
    let bob_key = RenderCache::key_for_route_for_test(PRIVATE_ROUTE, &[("id", "1")], Some("bob"));

    let alice = dispatch_get(&harness, "/privacy/private/1", &[("x-test-login", "alice")]).await;
    assert_eq!(alice.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 1);
    assert!(
        alice.header("etag").is_some(),
        "alice's render published, so it carries a validator"
    );
    assert!(
        RenderCache::inspect(&alice_key)
            .await
            .expect("inspect")
            .is_some(),
        "production stored alice's render under the key alice's own request derives"
    );
    assert!(
        RenderCache::inspect(&bob_key)
            .await
            .expect("inspect")
            .is_none(),
        "and stored nothing under bob's, which is what makes the two principals two \
         partitions rather than one"
    );

    // The key is a digest, never a transcript. Asserted on the key that
    // production has just proven it stores under, so this covers the real
    // encoding and not the helper's.
    for forbidden in ["alice", "bob", "privacy", "private"] {
        assert!(
            !alice_key.contains(forbidden),
            "the key text must carry no private or route material: found {forbidden:?} \
             in {alice_key}"
        );
    }

    let alice_again =
        dispatch_get(&harness, "/privacy/private/1", &[("x-test-login", "alice")]).await;
    assert_eq!(
        counting_route::renders(),
        1,
        "alice's own repeat is a hit: the route caches, in her own partition"
    );
    assert_eq!(alice_again.text(), alice.text());
    assert!(
        alice_again.header("etag").is_some(),
        "the hit replays the stored validator"
    );

    let bob = dispatch_get(&harness, "/privacy/private/1", &[("x-test-login", "bob")]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "bob derives a different key, so he misses rather than hitting alice's entry"
    );
    assert_ne!(
        bob.text(),
        alice.text(),
        "and gets his own page, not hers - got {}",
        bob.text()
    );
    assert!(
        RenderCache::inspect(&bob_key)
            .await
            .expect("inspect")
            .is_some(),
        "bob's own render is now stored, under his own key"
    );

    let alice_third =
        dispatch_get(&harness, "/privacy/private/1", &[("x-test-login", "alice")]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "alice's entry survived bob's miss"
    );
    assert_eq!(alice_third.text(), alice.text());
    assert_eq!(PRIVATE_ROUTE, "/privacy/private/{id}");
}

// ── R82, attack 16 (the sixth review's residual) ───────────────────────

/// A reader who carries no identity on a scoped flag's axis reaches the
/// flag's fall-through answer, which is an answer a reader *with* an id
/// would not get. Recording nothing for such a reader publishes that page
/// under a shared key that the override's owner then hits, bypassing their
/// own override.
#[tokio::test]
#[serial_test::serial]
async fn a_reader_with_no_identity_on_a_scoped_flags_axis_never_publishes_a_shared_entry() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, sibling shape: an identity-less reader of a flag
    // with no identity rule at all still caches, so the decline below is
    // about the flag's scope and not about flag reads in general.
    let base = same_visitor_twice_is_a_hit(&harness, "/privacy/reads-global-flag/1", &[]).await;
    assert!(
        route_is_under_a_policy(&harness, READS_OVERRIDE_FLAG_ROUTE),
        "the attacked route must still be attached to a policy"
    );

    let anonymous = dispatch_get(&harness, "/privacy/reads-override-flag/1", &[]).await;
    assert_eq!(anonymous.status, StatusCode::OK);
    assert!(
        anonymous.text().contains("enabled=false"),
        "sanity: an anonymous reader falls through to the flag's global rule - got {}",
        anonymous.text()
    );
    assert_eq!(counting_route::renders(), base + 1);

    let bob = dispatch_get(
        &harness,
        "/privacy/reads-override-flag/1",
        &[("x-test-login", "bob")],
    )
    .await;
    assert!(
        bob.text().contains("enabled=true"),
        "bob has a per-user override and must never be served the identity-less reader's \
         flag decision from cache - got {}",
        bob.text()
    );
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "the identity-less reader records a bare read on the flag's scoped axis, which \
         the route does not declare, so its page is never published"
    );
    assert_eq!(
        READS_OVERRIDE_FLAG_ROUTE,
        "/privacy/reads-override-flag/{id}"
    );
}

// ── The negative direction (R79, restated by R82 as not optional) ──────

/// A guard strict enough to pass every test above by declining everything
/// has disabled the feature. A route that declares nothing and observes
/// nothing must still cache, and only this assertion catches it.
#[tokio::test]
#[serial_test::serial]
async fn a_route_that_declares_nothing_and_observes_nothing_still_caches() {
    let harness = boot_with_render_cache().await;

    let first = dispatch_get(&harness, "/privacy/plain/1", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(counting_route::renders(), 1);
    assert!(
        first.header("cache-control").is_some() && first.header("etag").is_some(),
        "sanity: the route is genuinely under a policy and genuinely published - a \
         declined render carries neither header"
    );
    assert!(
        route_is_under_a_policy(&harness, PLAIN_ROUTE),
        "and the policy is still attached"
    );

    let second = dispatch_get(&harness, "/privacy/plain/1", &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        counting_route::renders(),
        1,
        "the second request must be a cache hit: a guard that declines everything passes \
         every leak test in this file and ships a feature that stores nothing"
    );
    assert_eq!(second.text(), first.text());
    assert!(
        second.header("etag").is_some(),
        "the hit carries a validator"
    );

    let other = dispatch_get(&harness, "/privacy/plain/2", &[]).await;
    assert_eq!(
        counting_route::renders(),
        2,
        "and a different route parameter is a different key, so it renders"
    );
    assert_ne!(other.text(), first.text());
    assert_eq!(PLAIN_ROUTE, "/privacy/plain/{id}");
}

// ── R79: every dimension the key can partition by must actually do so ──

/// The `Tenant` counterpart of the principal-partition assertion: two
/// tenants derive two entries, and each tenant's repeat is a hit.
#[tokio::test]
#[serial_test::serial]
async fn a_tenant_declaring_route_partitions_by_tenant() {
    let harness = boot_with_render_cache().await;

    let acme = dispatch_get(
        &harness,
        "/privacy/tenant-varies/1",
        &[("x-test-tenant", "acme")],
    )
    .await;
    assert_eq!(acme.status, StatusCode::OK);
    assert!(acme.text().contains("tenant=acme"), "got {}", acme.text());
    assert_eq!(counting_route::renders(), 1);

    let globex = dispatch_get(
        &harness,
        "/privacy/tenant-varies/1",
        &[("x-test-tenant", "globex")],
    )
    .await;
    assert!(
        globex.text().contains("tenant=globex"),
        "globex must never be served acme's page - got {}",
        globex.text()
    );
    assert_eq!(
        counting_route::renders(),
        2,
        "a second tenant is a second key, so it misses"
    );

    let acme_again = dispatch_get(
        &harness,
        "/privacy/tenant-varies/1",
        &[("x-test-tenant", "acme")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        2,
        "and acme's own entry is still there, so the declared dimension genuinely \
         partitions rather than merely declining"
    );
    assert_eq!(acme_again.text(), acme.text());
    assert!(
        acme_again.header("etag").is_some(),
        "the hit replays the stored validator"
    );
    assert_eq!(TENANT_VARIES_ROUTE, "/privacy/tenant-varies/{id}");
}

/// The `Locale` counterpart. Also the positive control for the locale
/// tests above: without it, a guard that declined every locale-declaring
/// route would look exactly as green.
#[tokio::test]
#[serial_test::serial]
async fn a_locale_declaring_route_partitions_by_locale() {
    let harness = boot_with_render_cache().await;

    let german = dispatch_get(
        &harness,
        "/privacy/locale-varies/1",
        &[("x-test-locale", "de")],
    )
    .await;
    assert_eq!(german.status, StatusCode::OK);
    assert!(german.text().contains("locale=de"), "got {}", german.text());
    assert_eq!(counting_route::renders(), 1);

    let french = dispatch_get(
        &harness,
        "/privacy/locale-varies/1",
        &[("x-test-locale", "fr")],
    )
    .await;
    assert!(
        french.text().contains("locale=fr"),
        "the French visitor must never be served the German page - got {}",
        french.text()
    );
    assert_eq!(counting_route::renders(), 2);

    let german_again = dispatch_get(
        &harness,
        "/privacy/locale-varies/1",
        &[("x-test-locale", "de")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        2,
        "the German entry is still there: declared Locale variance stores one entry per \
         locale rather than declining"
    );
    assert_eq!(german_again.text(), german.text());
    assert!(
        german_again.header("etag").is_some(),
        "the hit replays the stored validator"
    );
    assert_eq!(LOCALE_VARIES_ROUTE, "/privacy/locale-varies/{id}");
}

/// The other direction of the same rule: a render that reads the locale on
/// a route with no declared `Locale` dimension would cache one language for
/// everyone, so it declines outright.
#[tokio::test]
#[serial_test::serial]
async fn an_undeclared_locale_read_declines_storage() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, sibling shape, exactly as the ruling prescribes
    // for a never-stored route: the same locale read on a route that *does*
    // declare `Locale` caches.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/locale-varies/1",
        &[("x-test-locale", "de")],
    )
    .await;
    assert!(
        route_is_under_a_policy(&harness, UNDECLARED_LOCALE_ROUTE),
        "the attacked route must still be attached to a policy"
    );

    let first = dispatch_get(
        &harness,
        UNDECLARED_LOCALE_ROUTE,
        &[("x-test-locale", "de")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(first.text().contains("locale=de"), "got {}", first.text());

    let second = dispatch_get(
        &harness,
        UNDECLARED_LOCALE_ROUTE,
        &[("x-test-locale", "de")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        base + 2,
        "an observed locale with no declared Locale dimension must decline, even for two \
         identical requests: the route would otherwise cache one language for everyone"
    );
    assert_ne!(first.text(), second.text());
}

// ── R79 claim 4, carried here rather than borrowed ─────────────────────

/// `Request::auth_user_id()` is `pub`, returns a resolved identity, and has
/// no collector instrumentation at all: on its face it is the round-2 break
/// again. The reason it is not is a *measurement*, not an argument - the
/// field has one writer, `Request::with_auth_user_id`, called from one site
/// inside the WebSocket-upgrade terminator, so an ordinary HTTP request
/// always sees `None`.
///
/// That measurement is the precondition the whole "cannot influence a body"
/// classification rests on, and until this round it lived only in
/// `render_cache_middleware.rs`. This suite exists to be independent of that
/// file, so a restructuring of it - or of its harness, which a parallel
/// branch is editing - could delete the precondition with nothing here
/// noticing. It is carried here too, and stated as both halves: the
/// uninstrumented accessor sees nothing, and the instrumented one sees the
/// signed-in user whose identity the key is then built from.
#[tokio::test]
#[serial_test::serial]
async fn the_uninstrumented_request_accessor_carries_no_identity_and_no_body_crosses() {
    let harness = boot_with_render_cache().await;
    // R103 positive control, on the attacked route itself: alice's key and
    // alice's observed identity agree, so this route caches for her.
    let base = same_visitor_twice_is_a_hit(
        &harness,
        "/privacy/reads-request-auth-user-id/1",
        &[("x-test-login", "alice")],
    )
    .await;

    let alice = dispatch_get(
        &harness,
        "/privacy/reads-request-auth-user-id/1",
        &[("x-test-login", "alice")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        base,
        "still alice's own entry, so the body below is the one her render produced"
    );
    assert!(
        alice.text().contains("stamped=none"),
        "the measurement: `Request::auth_user_id()` is stamped only on the \
         WebSocket-upgrade path, so an ordinary HTTP render sees nothing there. If \
         this ever fails, an uninstrumented accessor is returning a real identity \
         and every 'cannot influence a body' conclusion about it is void - got {}",
        alice.text()
    );
    assert!(
        alice.text().contains("observed=alice"),
        "and the other half: the instrumented accessor does see her, so the render \
         genuinely had an identity available to leak - got {}",
        alice.text()
    );

    let bob = dispatch_get(
        &harness,
        "/privacy/reads-request-auth-user-id/1",
        &[("x-test-login", "bob")],
    )
    .await;
    assert_eq!(
        counting_route::renders(),
        base + 1,
        "bob derives his own key and renders rather than hitting alice's entry"
    );
    assert!(
        !bob.text().contains("observed=alice"),
        "and no body crosses - got {}",
        bob.text()
    );
    assert!(
        bob.text().contains("stamped=none") && bob.text().contains("observed=bob"),
        "got {}",
        bob.text()
    );
    assert_eq!(
        REQUEST_AUTH_USER_ID_ROUTE,
        "/privacy/reads-request-auth-user-id/{id}"
    );
}

// ── The stitched row: a shared shell around a per-principal island ──────

/// The stitched class is the one class that publishes a *shared*
/// representation of a document part of which is per-principal, so it is
/// the one place in this suite where "the handler did not run" is not by
/// itself a leak - the shell is meant to be reused, and the island inside it
/// is meant to be mounted again for whoever asked.
///
/// What must therefore hold instead: two principals share the shell byte for
/// byte and share nothing else. Neither receives the other's island, its
/// signed snapshot, or its identifier, and the scope each island is bound to
/// is the scope of the visitor who received it.
///
/// The positive control is on this same route: the second principal's
/// request must actually be a hit, or a boot that stored nothing would pass
/// every boundary assertion below.
#[tokio::test]
#[serial_test::serial]
async fn two_principals_share_one_stitched_shell_and_never_see_each_others_island() {
    let harness = boot_with_render_cache().await;
    assert!(
        route_is_under_a_policy(&harness, STITCHED_ROUTE),
        "the attacked route must still be attached to a policy: a deleted opt-in \
         makes every assertion below vacuous"
    );

    let before = counting_route::renders();
    let alice = dispatch_get(&harness, STITCHED_ROUTE, &[("x-test-login", "alice")]).await;
    assert_eq!(alice.status, StatusCode::OK, "{}", alice.text());
    assert_eq!(
        counting_route::renders(),
        before + 1,
        "positive control: the first request rendered"
    );
    assert_eq!(
        RenderCache::inspect_route_for_test(STITCHED_ROUTE)
            .await
            .expect("the shell was published")
            .slots,
        1,
        "positive control: what was stored is a shell with one island cut out of it"
    );

    let bob = dispatch_get(&harness, STITCHED_ROUTE, &[("x-test-login", "bob")]).await;
    assert_eq!(bob.status, StatusCode::OK, "{}", bob.text());
    assert_eq!(
        counting_route::renders(),
        before + 1,
        "positive control: the second principal was served from the stored shell. \
         If it was not, this boot is storing nothing and every assertion below \
         would pass against a cache that had been switched off"
    );

    let alice_island = island_tag(&alice.text(), STITCHED_DOCUMENT_KEY).to_owned();
    let bob_island = island_tag(&bob.text(), STITCHED_DOCUMENT_KEY).to_owned();
    assert_ne!(
        alice_island, bob_island,
        "each principal has its own island"
    );
    assert_ne!(
        stitched_scope(&alice.text()),
        stitched_scope(&bob.text()),
        "and each island is bound to the scope of the visitor who received it"
    );
    assert!(
        !bob.text().contains(&alice_island),
        "bob never receives alice's island"
    );
    assert!(
        !bob.text()
            .contains(attribute(&alice_island, "data-suprnova-live-snapshot")),
        "nor the signed snapshot it carried"
    );
    assert!(
        !alice.text().contains(&bob_island) && !bob.text().contains("alice"),
        "and nothing crosses in the other direction either"
    );

    // The shell around the island is the same bytes for both, which is the
    // whole reason this class stores anything at all.
    let shell = |text: &str| text.replace(island_tag(text, STITCHED_DOCUMENT_KEY), "");
    assert_eq!(shell(&alice.text()), shell(&bob.text()));
}

/// The stored shell is shared, but reaching it is not: the route's own guard
/// runs again on every hit, before the entry is served, so a visitor with no
/// identity is turned away rather than handed a document assembled around
/// somebody's island - and so is a visitor presenting only the session
/// cookie of a sign-in that has since ended.
#[tokio::test]
#[serial_test::serial]
async fn an_anonymous_or_signed_out_visitor_never_receives_an_assembled_stitched_document() {
    let harness = boot_with_render_cache().await;
    assert!(route_is_under_a_policy(&harness, STITCHED_ROUTE));
    let alice = dispatch_get(&harness, STITCHED_ROUTE, &[("x-test-login", "alice")]).await;
    assert_eq!(alice.status, StatusCode::OK, "{}", alice.text());
    let alice_island = island_tag(&alice.text(), STITCHED_DOCUMENT_KEY).to_owned();
    let alice_cookie = session_cookie(&alice);
    let published = counting_route::renders();

    let anonymous = dispatch_get(&harness, STITCHED_ROUTE, &[]).await;
    assert_eq!(
        anonymous.status,
        StatusCode::UNAUTHORIZED,
        "the guard ran before the stored entry was served"
    );
    assert!(!anonymous.text().contains(&alice_island));
    assert!(!anonymous.text().contains("data-suprnova-live-snapshot"));

    // The same session, with the sign-in gone: the cookie alone is not an
    // identity, and the entry is not reachable without one.
    let signed_out = dispatch_get(&harness, STITCHED_ROUTE, &[("cookie", &alice_cookie)]).await;
    assert_eq!(signed_out.status, StatusCode::UNAUTHORIZED);
    assert!(
        !signed_out.text().contains(&alice_island),
        "a cleared sign-in never gets the island the session used to carry"
    );
    assert!(!signed_out.text().contains("data-suprnova-live-snapshot"));

    assert_eq!(
        counting_route::renders(),
        published,
        "neither refused request reached the handler either"
    );
    assert!(
        RenderCache::inspect_route_for_test(STITCHED_ROUTE)
            .await
            .is_some(),
        "and the shell they were refused is still exactly where it was"
    );
}

/// A permission-version bump does not have to invalidate a stitched shell,
/// and this proves why: the shell holds no principal material at all, and
/// the island that does is mounted again, under authority derived for the
/// request in front of it, on every single hit. There is no cached island
/// for a revoked permission to survive in.
///
/// This is what makes the stitched class different from `PrivateCached`,
/// where a bump *is* what makes a pre-bump entry a miss: there the stored
/// bytes are one principal's, and here they are nobody's.
#[tokio::test]
#[serial_test::serial]
async fn a_permission_version_bump_leaves_the_shared_shell_and_re_mounts_every_island() {
    let harness = boot_with_render_cache().await;
    assert!(route_is_under_a_policy(&harness, STITCHED_ROUTE));
    let alice = dispatch_get(&harness, STITCHED_ROUTE, &[("x-test-login", "alice")]).await;
    assert_eq!(alice.status, StatusCode::OK, "{}", alice.text());
    let stored = RenderCache::inspect_route_for_test(STITCHED_ROUTE)
        .await
        .expect("the shell was published");
    let shell_before = RenderCache::shell_for_test(STITCHED_ROUTE)
        .await
        .expect("the stored shell");
    assert!(
        !String::from_utf8_lossy(&shell_before).contains("data-suprnova-live-snapshot"),
        "the shared shell carries no principal's signed snapshot to begin with"
    );
    let published = counting_route::renders();

    RenderCache::bump_permission_version()
        .await
        .expect("bump the permission version");

    let stored_after = RenderCache::inspect_route_for_test(STITCHED_ROUTE)
        .await
        .expect("the shell survived the bump");
    assert_eq!(stored_after.kind, stored.kind);
    assert_eq!(stored_after.slots, stored.slots);
    assert_eq!(stored_after.published_at_ms, stored.published_at_ms);
    assert_eq!(
        RenderCache::shell_for_test(STITCHED_ROUTE)
            .await
            .expect("the stored shell"),
        shell_before,
        "the shell entry is unchanged: it never held anything a permission \
         change could make wrong"
    );

    let bob = dispatch_get(&harness, STITCHED_ROUTE, &[("x-test-login", "bob")]).await;
    assert_eq!(bob.status, StatusCode::OK, "{}", bob.text());
    assert_eq!(
        counting_route::renders(),
        published,
        "the hit after the bump is still assembled from the shell"
    );
    assert_ne!(
        stitched_scope(&bob.text()),
        stitched_scope(&alice.text()),
        "and the island in it was mounted for bob, here and now"
    );
}
