//! Tests for the `laravel_auth_gaps` block PAR-127: a rate-limiter key is
//! cleaned the way Laravel's `cleanRateLimiterKey` cleans it, so keys that
//! differ only by a named HTML entity share one bucket; a named limiter
//! renames the keys its limits share, and the throttle middleware counts
//! them as a direct caller does; `hit_for_minute` and `hit_until` set the
//! window; and the middleware writes one header pair, leaves a refused
//! request's buckets where they were, admits no more than the limit in a
//! window that ends around a refusal, keys `throttle:60,1` by the signed-in
//! user and reads `throttle:<guest>|<user>`.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;
use suprnova::http::{HttpResponse, text};
use suprnova::rate_limit::{Limit, LimitResult};
use suprnova::{
    FrameworkError, MiddlewareRegistry, RateLimiter, Request, Router, ThrottleRequestsMiddleware,
    handle_request_with_peer,
};

fn install_test_cache() -> impl Drop {
    let guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    guard
}

/// Signs in the user the `x-test-user` header names, for this request only,
/// the way the session middleware does for a signed-in visitor.
struct SignsIn;

#[suprnova::async_trait]
impl suprnova::Middleware for SignsIn {
    async fn handle(&self, request: Request, next: suprnova::Next) -> suprnova::Response {
        if let Some(user) = request.header("x-test-user") {
            suprnova::Auth::set_user(Arc::new(suprnova::auth::GenericUser::new(
                user.to_owned(),
                None,
                serde_json::Map::new(),
            )));
        }
        next(request).await
    }
}

/// What a test reads off a response: its status and every value of the
/// two rate-limit headers, so a repeated header shows as two values.
struct Answer {
    status: u16,
    limits: Vec<String>,
    remainings: Vec<String>,
}

/// Send `GET path` through `router` from `address`, signed in as `user`
/// when one is given (the router must run [`SignsIn`] first).
async fn send(
    router: &Arc<Router>,
    path: &str,
    user: Option<&str>,
    address: Option<IpAddr>,
) -> Answer {
    let mut headers = Vec::new();
    if let Some(user) = user {
        headers.push(("x-test-user", user));
    }
    let request = crate::common::incoming_get_request(path, &headers).await;
    let response = handle_request_with_peer(
        Arc::clone(router),
        Arc::new(MiddlewareRegistry::new()),
        request,
        address,
    )
    .await;
    let values = |name: &str| {
        response
            .headers()
            .get_all(name)
            .iter()
            .map(|value| value.to_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>()
    };
    Answer {
        status: response.status().as_u16(),
        limits: values("x-ratelimit-limit"),
        remainings: values("x-ratelimit-remaining"),
    }
}

/// A request to hand a named limiter's callback directly.
async fn request() -> Request {
    Request::new(crate::common::incoming_get_request("/", &[]).await)
}

fn address(last: u8) -> Option<IpAddr> {
    Some(IpAddr::from([198, 51, 100, last]))
}

/// The limiter of the falsifier: two limits that share the key `a`.
fn define_api_limiter() {
    RateLimiter::define("api", |_request| {
        vec![Limit::per_minute(2).by("a"), Limit::per_hour(10).by("a")].into()
    });
}

fn keys_of(result: LimitResult) -> Vec<String> {
    match result {
        LimitResult::Single(limit) => vec![limit.key],
        LimitResult::Many(limits) => limits.into_iter().map(|limit| limit.key).collect(),
        LimitResult::Response(_) => panic!("the limiter answered a response, not limits"),
    }
}

// --- Cleaning keys ------------------------------------------------------------

#[tokio::test]
async fn hit_counts_a_key_and_its_entity_free_spelling_in_one_bucket() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    let first = RateLimiter::hit("café", 60).await.expect("first hit");
    let second = RateLimiter::hit("cafe", 60).await.expect("second hit");

    assert_eq!(first, 1, "the first hit opens the bucket");
    assert_eq!(
        second, 2,
        "Laravel cleans \"café\" to \"cafe\" before counting, so the second hit shares its bucket"
    );
}

#[tokio::test]
async fn markup_in_a_key_is_encoded_before_it_is_stripped() {
    let _cache = install_test_cache();

    RateLimiter::hit("a&b", 60).await.expect("hit a&b");
    assert_eq!(
        RateLimiter::attempts("aab").await.expect("attempts"),
        1,
        "`&` encodes to `&amp;`, which strips to `a`"
    );
    assert_eq!(RateLimiter::clean_rate_limiter_key("<b>"), "lbg");
    assert_eq!(RateLimiter::clean_rate_limiter_key("x²"), "x&sup2;");
}

#[tokio::test]
async fn an_apostrophe_stays_encoded_and_does_not_join_its_plain_spelling() {
    let _cache = install_test_cache();

    assert_eq!(RateLimiter::clean_rate_limiter_key("it's"), "it&#039;s");
    RateLimiter::hit("it's", 60).await.expect("hit it's");
    assert_eq!(
        RateLimiter::hit("its", 60).await.expect("hit its"),
        1,
        "`&#039;` is no named entity, so `it's` keeps a bucket of its own"
    );
}

// --- Named limiters -------------------------------------------------------------

#[tokio::test]
async fn a_named_limiter_hands_shared_keys_their_fallback_keys() {
    define_api_limiter();
    let limiter = RateLimiter::limiter("api").expect("the api limiter is defined");

    assert_eq!(
        keys_of(limiter(&request().await)),
        ["a:attempts:2:decay:60", "a:attempts:10:decay:3600"],
        "two limits keyed `a` must not count in one bucket"
    );
}

#[tokio::test]
async fn a_named_limiter_keeps_keys_that_differ_and_a_single_limit() {
    RateLimiter::define("gaps-distinct", |_request| {
        vec![
            Limit::per_minute(2).by("a"),
            Limit::per_hour(10).by("b"),
            Limit::per_day(50).by("a"),
        ]
        .into()
    });
    RateLimiter::define("gaps-single", |_request| {
        Limit::per_minute(2).by("a").into()
    });
    RateLimiter::define("gaps-response", |_request| {
        HttpResponse::text("refused").status(403).into()
    });
    let request = request().await;

    let distinct = RateLimiter::limiter("gaps-distinct").expect("defined");
    assert_eq!(
        keys_of(distinct(&request)),
        ["a:attempts:2:decay:60", "b", "a:attempts:50:decay:86400"],
        "only the limits that share a key are renamed"
    );
    let single = RateLimiter::limiter("gaps-single").expect("defined");
    assert_eq!(keys_of(single(&request)), ["a"]);
    let response = RateLimiter::limiter("gaps-response").expect("defined");
    assert!(matches!(response(&request), LimitResult::Response(_)));
    assert!(RateLimiter::limiter("gaps-never-defined").is_none());
}

#[tokio::test]
async fn one_request_through_a_named_throttle_counts_each_fallback_bucket_once() {
    let _cache = install_test_cache();
    define_api_limiter();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/api", |_request| async { text("ok") })
            .middleware(
                ThrottleRequestsMiddleware::from_alias_args(&["api"]).expect("throttle:api"),
            )
            .into(),
    );

    let answer = send(&router, "/api", None, None).await;

    assert_eq!(answer.status, 200);
    assert_eq!(
        RateLimiter::attempts("api:a:attempts:2:decay:60")
            .await
            .expect("attempts"),
        1
    );
    assert_eq!(
        RateLimiter::attempts("api:a:attempts:10:decay:3600")
            .await
            .expect("attempts"),
        1
    );
    assert_eq!(
        RateLimiter::attempts("api:a").await.expect("attempts"),
        0,
        "no limit counts under the shared key itself"
    );
}

/// A route behind the limiter named `name`.
fn named_router(name: &str) -> Arc<Router> {
    Arc::new(
        Router::new()
            .get("/named", |_request| async { text("ok") })
            .middleware(ThrottleRequestsMiddleware::by_name(name))
            .into(),
    )
}

#[tokio::test]
async fn two_identical_clauses_count_their_one_bucket_once_each_as_laravel_does() {
    let _cache = install_test_cache();
    RateLimiter::define("gaps-identical", |_request| {
        vec![Limit::per_minute(2).by("a"), Limit::per_minute(2).by("a")].into()
    });
    let router = named_router("gaps-identical");
    let bucket = "gaps-identical:a:attempts:2:decay:60";

    let first = send(&router, "/named", None, None).await;

    assert_eq!(first.status, 200);
    assert_eq!(
        RateLimiter::attempts(bucket).await.expect("attempts"),
        2,
        "both clauses get the one fallback key, and Laravel hits it once for each"
    );
    assert_eq!(first.remainings, ["0"]);
    assert_eq!(
        send(&router, "/named", None, None).await.status,
        429,
        "the bucket a direct caller reads already holds the limit"
    );
    assert_eq!(
        RateLimiter::attempts(bucket).await.expect("attempts"),
        2,
        "the refused request is not counted"
    );
}

#[tokio::test]
async fn keys_that_clean_alike_share_one_bucket_for_the_middleware_and_a_direct_caller() {
    let _cache = install_test_cache();
    RateLimiter::define("gaps-clean-alike", |_request| {
        vec![
            Limit::per_minute(5).by("café"),
            Limit::per_hour(10).by("cafe"),
        ]
        .into()
    });
    let router = named_router("gaps-clean-alike");

    assert_eq!(send(&router, "/named", None, None).await.status, 200);
    for key in ["gaps-clean-alike:café", "gaps-clean-alike:cafe"] {
        assert_eq!(
            RateLimiter::attempts(key).await.expect("attempts"),
            2,
            "{key} reads the one bucket, counted once for each limit"
        );
    }
    assert_eq!(
        RateLimiter::hit("gaps-clean-alike:cafe", 60)
            .await
            .expect("hit"),
        3,
        "a direct caller counts in the bucket the middleware counts in"
    );

    let second = send(&router, "/named", None, None).await;
    assert_eq!(second.status, 200);
    assert_eq!(
        second.remainings,
        ["0"],
        "the minute's limit of 5 holds the direct hit and both counts of each request"
    );
    assert_eq!(second.limits, ["5"]);
    assert_eq!(send(&router, "/named", None, None).await.status, 429);
}

// --- hit_for_minute and hit_until ---------------------------------------------

#[tokio::test]
async fn hit_for_minute_opens_a_sixty_second_window() {
    let _cache = install_test_cache();

    assert_eq!(RateLimiter::hit_for_minute("k").await.expect("hit"), 1);
    let available_in = RateLimiter::available_in("k").await.expect("available_in");
    assert!(
        (59..=60).contains(&available_in),
        "the window is 60 seconds, not {available_in}"
    );
    assert_eq!(
        RateLimiter::hit("k", 60).await.expect("hit"),
        2,
        "it counts in the bucket `hit(key, 60)` counts in"
    );
}

#[tokio::test]
async fn hit_until_opens_a_window_that_ends_at_the_given_time() {
    let _cache = install_test_cache();
    let ten_seconds_ahead = suprnova::chrono::Utc::now() + suprnova::chrono::Duration::seconds(10);

    assert_eq!(
        RateLimiter::hit_until("k", ten_seconds_ahead)
            .await
            .expect("hit"),
        1
    );
    let available_in = RateLimiter::available_in("k").await.expect("available_in");
    assert!(
        (9..=10).contains(&available_in),
        "the window ends ten seconds ahead, not {available_in}"
    );
}

#[tokio::test]
async fn hit_until_a_time_that_has_passed_opens_no_window() {
    let _cache = install_test_cache();
    let a_minute_ago = suprnova::chrono::Utc::now() - suprnova::chrono::Duration::seconds(60);

    assert_eq!(
        RateLimiter::hit_until("gone", a_minute_ago)
            .await
            .expect("hit"),
        1
    );
    assert_eq!(
        RateLimiter::available_in("gone")
            .await
            .expect("available_in"),
        0
    );
    assert_eq!(
        RateLimiter::attempts("gone").await.expect("attempts"),
        0,
        "a counter with no window must not be left to count forever"
    );

    RateLimiter::hit("open", 60).await.expect("open a window");
    assert_eq!(
        RateLimiter::hit_until("open", a_minute_ago)
            .await
            .expect("hit"),
        2,
        "a bucket with an open window counts the hit in that window"
    );
    assert_eq!(RateLimiter::attempts("open").await.expect("attempts"), 2);
    assert!(
        RateLimiter::available_in("open")
            .await
            .expect("available_in")
            >= 59
    );
}

// --- Response headers -----------------------------------------------------------

#[tokio::test]
async fn a_route_behind_two_limits_answers_one_header_pair_the_lowest() {
    let _cache = install_test_cache();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/two", |_request| async { text("ok") })
            .middleware(ThrottleRequestsMiddleware::with_limits(vec![
                Limit::per_minute(5).by("wide"),
                Limit::per_hour(3).by("narrow"),
            ]))
            .into(),
    );

    let answer = send(&router, "/two", None, None).await;

    assert_eq!(answer.status, 200);
    assert_eq!(answer.remainings, ["2"], "one pair, the limit with 2 left");
    assert_eq!(answer.limits, ["3"], "the pair is the narrow limit's");
}

#[tokio::test]
async fn a_named_limiter_with_two_limits_answers_one_header_pair() {
    let _cache = install_test_cache();
    define_api_limiter();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/api", |_request| async { text("ok") })
            .middleware(ThrottleRequestsMiddleware::by_name("api"))
            .into(),
    );

    let answer = send(&router, "/api", None, None).await;

    assert_eq!(answer.remainings, ["1"]);
    assert_eq!(answer.limits, ["2"]);
}

#[tokio::test]
async fn a_handlers_own_remaining_header_stays_when_it_is_equal_or_lower() {
    let _cache = install_test_cache();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/lower", |_request| async {
                text("ok").map(|response| response.header("X-RateLimit-Remaining", "0"))
            })
            .get("/equal", |_request| async {
                text("ok").map(|response| {
                    response
                        .header("X-RateLimit-Limit", "7")
                        .header("X-RateLimit-Remaining", "4")
                })
            })
            .get("/higher", |_request| async {
                text("ok").map(|response| {
                    response
                        .header("X-RateLimit-Limit", "100")
                        .header("X-RateLimit-Remaining", "99")
                })
            })
            .middleware(ThrottleRequestsMiddleware::with(5, 1, ""))
            .into(),
    );

    let lower = send(&router, "/lower", None, None).await;
    assert_eq!(lower.remainings, ["0"], "the handler's lower count stays");
    assert!(lower.limits.is_empty(), "and no pair is added to it");

    let equal = send(&router, "/equal", None, None).await;
    assert_eq!(equal.remainings, ["4"], "an equal count stays too");
    assert_eq!(equal.limits, ["7"]);

    let higher = send(&router, "/higher", None, None).await;
    assert_eq!(
        higher.remainings,
        ["4"],
        "a higher count is replaced, not repeated"
    );
    assert_eq!(higher.limits, ["5"]);
}

// --- Refused requests -----------------------------------------------------------

#[tokio::test]
async fn a_refused_request_leaves_the_bucket_count_where_it_was() {
    let _cache = install_test_cache();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/refused", |_request| async { text("ok") })
            .middleware(
                ThrottleRequestsMiddleware::from_alias_args(&["2", "1"]).expect("throttle:2,1"),
            )
            .into(),
    );
    let bucket = "ip:198.51.100.7:path:/refused";

    assert_eq!(
        send(&router, "/refused", None, address(7)).await.status,
        200
    );
    assert_eq!(
        send(&router, "/refused", None, address(7)).await.status,
        200
    );
    let refused = send(&router, "/refused", None, address(7)).await;
    assert_eq!(refused.status, 429);
    assert_eq!(refused.remainings, ["0"]);
    assert_eq!(RateLimiter::attempts(bucket).await.expect("attempts"), 2);

    assert_eq!(
        send(&router, "/refused", None, address(7)).await.status,
        429
    );
    assert_eq!(
        RateLimiter::attempts(bucket).await.expect("attempts"),
        2,
        "every refused request is given back"
    );
}

#[tokio::test]
async fn a_refused_request_gives_back_the_limits_that_admitted_it() {
    let _cache = install_test_cache();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/pair", |_request| async { text("ok") })
            .middleware(ThrottleRequestsMiddleware::with_limits(vec![
                Limit::per_minute(5).by("wide"),
                Limit::per_minute(1).by("narrow"),
            ]))
            .into(),
    );

    assert_eq!(send(&router, "/pair", None, None).await.status, 200);
    assert_eq!(send(&router, "/pair", None, None).await.status, 429);

    assert_eq!(
        RateLimiter::attempts("wide").await.expect("attempts"),
        1,
        "the limit that admitted the refused request is given back as well"
    );
    assert_eq!(RateLimiter::attempts("narrow").await.expect("attempts"), 1);
}

#[tokio::test]
async fn a_concurrent_burst_is_admitted_on_the_post_increment_count() {
    let _cache = install_test_cache();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/burst", |_request| async { text("ok") })
            .middleware(ThrottleRequestsMiddleware::with(3, 1, ""))
            .into(),
    );

    let answers =
        futures::future::join_all((0..12).map(|_| send(&router, "/burst", None, address(9)))).await;

    let admitted = answers.iter().filter(|answer| answer.status == 200).count();
    assert_eq!(admitted, 3, "the burst admits exactly the limit");
    assert_eq!(
        RateLimiter::attempts("ip:198.51.100.9:path:/burst")
            .await
            .expect("attempts"),
        3,
        "the count ends at the requests admitted"
    );
}

// --- Windows that end around a refusal ------------------------------------------

/// A store whose windows end when the test says so, and which holds every
/// decrement back until the test runs it, as a give-back that runs after
/// its window ended would. It can also let another request take a place in
/// a bucket just before the next count there, so a request that passed the
/// check finds the bucket full when it counts.
#[derive(Default)]
struct WindowsEndOnCue {
    inner: InMemoryCache,
    held_decrements: Mutex<Vec<(String, i64)>>,
    racing: Mutex<Option<String>>,
}

impl WindowsEndOnCue {
    /// End the window of the bucket stored under `key`: its count and its
    /// timer go, as they do when their time to live runs out.
    async fn end_window(&self, key: &str) {
        self.inner.forget(key).await.expect("forget the count");
        self.inner
            .forget(&format!("{key}:timer"))
            .await
            .expect("forget the timer");
    }

    /// Run the decrements held back so far, after the window they were
    /// meant for has ended.
    async fn run_held_decrements(&self) {
        let held = std::mem::take(&mut *self.held_decrements.lock().expect("held decrements"));
        for (key, amount) in held {
            self.inner
                .decrement(&key, amount)
                .await
                .expect("run a held decrement");
        }
    }

    /// Let another request take one place in `key`'s bucket just before the
    /// next count there.
    fn race_next_count(&self, key: &str) {
        *self.racing.lock().expect("racing key") = Some(key.to_owned());
    }
}

#[suprnova::async_trait]
impl CacheStore for WindowsEndOnCue {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        self.inner.get_raw(key).await
    }
    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        self.inner.put_raw(key, value, ttl).await
    }
    async fn add_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<bool, FrameworkError> {
        self.inner.add_raw(key, value, ttl).await
    }
    async fn has(&self, key: &str) -> Result<bool, FrameworkError> {
        self.inner.has(key).await
    }
    async fn forget(&self, key: &str) -> Result<bool, FrameworkError> {
        self.inner.forget(key).await
    }
    async fn flush(&self) -> Result<(), FrameworkError> {
        self.inner.flush().await
    }
    async fn increment(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        let raced = {
            let mut racing = self.racing.lock().expect("racing key");
            racing.take_if(|racing| racing.as_str() == key).is_some()
        };
        if raced {
            self.inner.increment(key, 1).await?;
        }
        self.inner.increment(key, amount).await
    }
    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        self.held_decrements
            .lock()
            .expect("held decrements")
            .push((key.to_owned(), amount));
        // Nothing has changed yet, so the answer is the count as it stands.
        let current = self.inner.get_raw(key).await?;
        Ok(current.and_then(|count| count.parse().ok()).unwrap_or(0))
    }
    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        self.inner.tagged_put_raw(tags, key, value, ttl).await
    }
    async fn flush_tags(&self, tags: &[&str]) -> Result<(), FrameworkError> {
        self.inner.flush_tags(tags).await
    }
    async fn acquire_lock(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        self.inner.acquire_lock(key, ttl).await
    }
    async fn release_lock(&self, key: &str, token: &str) -> Result<bool, FrameworkError> {
        self.inner.release_lock(key, token).await
    }
    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, FrameworkError> {
        self.inner.refresh_lock(key, token, ttl).await
    }
    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, FrameworkError> {
        self.inner.touch(key, ttl).await
    }
}

/// Bind a [`WindowsEndOnCue`] store for this test and return it with a route
/// behind one limit of two requests a minute, counted under `key`.
fn windows_on_cue(key: &str) -> (impl Drop, Arc<WindowsEndOnCue>, Arc<Router>) {
    let guard = TestContainer::fake();
    let store = Arc::new(WindowsEndOnCue::default());
    TestContainer::bind::<dyn CacheStore>(store.clone());
    let router = Arc::new(
        Router::new()
            .get("/cue", |_request| async { text("ok") })
            .middleware(ThrottleRequestsMiddleware::with_limits(vec![
                Limit::per_minute(2).by(key),
            ]))
            .into(),
    );
    (guard, store, router)
}

/// The statuses of `count` requests sent one after another.
async fn statuses(router: &Arc<Router>, count: usize) -> Vec<u16> {
    let mut statuses = Vec::with_capacity(count);
    for _ in 0..count {
        statuses.push(send(router, "/cue", None, None).await.status);
    }
    statuses
}

#[tokio::test]
async fn a_refusal_whose_window_ends_first_leaves_the_next_window_admitting_the_limit() {
    let (_guard, store, router) = windows_on_cue("cue:one");

    assert_eq!(statuses(&router, 3).await, [200, 200, 429]);
    store.end_window("cue:one").await;
    store.run_held_decrements().await;

    assert_eq!(
        statuses(&router, 3).await,
        [200, 200, 429],
        "the next window admits exactly the limit of 2"
    );
    assert_eq!(RateLimiter::attempts("cue:one").await.expect("attempts"), 2);
}

#[tokio::test]
async fn several_refusals_whose_window_ends_first_leave_the_next_window_admitting_the_limit() {
    let (_guard, store, router) = windows_on_cue("cue:several");

    assert_eq!(statuses(&router, 5).await, [200, 200, 429, 429, 429]);
    store.end_window("cue:several").await;
    store.run_held_decrements().await;

    assert_eq!(
        statuses(&router, 3).await,
        [200, 200, 429],
        "no refusal of the last window buys a place in this one"
    );
    assert_eq!(
        RateLimiter::attempts("cue:several")
            .await
            .expect("attempts"),
        2
    );
}

#[tokio::test]
async fn a_request_that_loses_the_last_place_leaves_the_next_window_admitting_the_limit() {
    let (_guard, store, router) = windows_on_cue("cue:race");

    assert_eq!(statuses(&router, 1).await, [200]);
    store.race_next_count("cue:race");
    assert_eq!(
        statuses(&router, 2).await,
        [429, 429],
        "another request takes the last place before this one counts, and the \
         bucket stays full for the rest of the window"
    );
    store.end_window("cue:race").await;
    store.run_held_decrements().await;

    assert_eq!(
        statuses(&router, 3).await,
        [200, 200, 429],
        "the next window admits exactly the limit of 2"
    );
}

// --- Who `throttle:60,1` counts ---------------------------------------------------

fn inline_router(arguments: &[&str]) -> Arc<Router> {
    Arc::new(
        Router::new()
            .get("/inline", |_request| async { text("ok") })
            .middleware(SignsIn)
            .middleware(
                ThrottleRequestsMiddleware::from_alias_args(arguments)
                    .expect("the alias registers"),
            )
            .into(),
    )
}

#[tokio::test]
async fn a_signed_in_user_has_one_inline_bucket_across_addresses() {
    let _cache = install_test_cache();
    let router = inline_router(&["1", "1"]);

    assert_eq!(
        send(&router, "/inline", Some("ada"), address(1))
            .await
            .status,
        200
    );
    assert_eq!(
        send(&router, "/inline", Some("ada"), address(2))
            .await
            .status,
        429,
        "a new address buys a signed-in user no new budget"
    );
    assert_eq!(
        send(&router, "/inline", Some("grace"), address(1))
            .await
            .status,
        200,
        "another user on the same address has a budget of her own"
    );
    assert_eq!(
        RateLimiter::attempts("user:ada:path:/inline")
            .await
            .expect("attempts"),
        1
    );
}

#[tokio::test]
async fn guests_on_two_addresses_have_inline_buckets_of_their_own() {
    let _cache = install_test_cache();
    let router = inline_router(&["1", "1"]);

    assert_eq!(send(&router, "/inline", None, address(1)).await.status, 200);
    assert_eq!(send(&router, "/inline", None, address(1)).await.status, 429);
    assert_eq!(send(&router, "/inline", None, address(2)).await.status, 200);
    assert_eq!(
        RateLimiter::attempts("ip:198.51.100.2:path:/inline")
            .await
            .expect("attempts"),
        1
    );
}

#[tokio::test]
async fn guest_and_user_limits_apply_by_who_is_signed_in() {
    let _cache = install_test_cache();
    let router = inline_router(&["1|3", "1"]);

    for request in 1..=3 {
        assert_eq!(
            send(&router, "/inline", Some("ada"), address(3))
                .await
                .status,
            200,
            "a signed-in user's request {request} of 3 is admitted"
        );
    }
    let fourth = send(&router, "/inline", Some("ada"), address(3)).await;
    assert_eq!(fourth.status, 429);
    assert_eq!(fourth.limits, ["3"], "the user's limit is 3");

    let first = send(&router, "/inline", None, address(4)).await;
    assert_eq!(first.status, 200);
    assert_eq!(first.limits, ["1"], "the guest's limit is 1");
    assert_eq!(
        send(&router, "/inline", None, address(4)).await.status,
        429,
        "a guest's second request is refused"
    );
}

#[test]
fn guest_and_user_limits_are_read_at_registration_and_zero_is_refused() {
    assert!(ThrottleRequestsMiddleware::from_alias_args(&["1|3"]).is_ok());
    assert!(ThrottleRequestsMiddleware::from_alias_args(&["1|3", "1", "uploads"]).is_ok());

    let refused = |arguments: &[&str]| {
        ThrottleRequestsMiddleware::from_alias_args(arguments)
            .err()
            .unwrap_or_else(|| panic!("throttle:{} must be refused", arguments.join(",")))
            .to_string()
    };
    assert!(refused(&["0|3", "1"]).contains("would refuse every request"));
    assert!(refused(&["1|0", "1"]).contains("would refuse every request"));
    assert!(
        refused(&["1|x", "1"]).contains("names a limiter"),
        "a first argument that is not two numbers still names a limiter"
    );
}

#[tokio::test]
async fn the_default_limit_keeps_its_key() {
    let _cache = install_test_cache();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/default", |_request| async { text("ok") })
            .middleware(SignsIn)
            .middleware(ThrottleRequestsMiddleware::default())
            .into(),
    );

    send(&router, "/default", Some("ada"), address(5)).await;
    send(&router, "/default", None, address(6)).await;

    assert_eq!(
        RateLimiter::attempts("user:ada").await.expect("attempts"),
        1
    );
    assert_eq!(
        RateLimiter::attempts("ip:198.51.100.6")
            .await
            .expect("attempts"),
        1
    );
}
