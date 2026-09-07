//! Hot Complete hits: an entry prepared once at publication, one response
//! builder shared with the cold path, hot slots beside the stored bytes, and
//! the ledger's batched reread.

use std::sync::Arc;

use bytes::Bytes;
use http::header::{AGE, CACHE_CONTROL, CONTENT_TYPE, ETAG, VARY, WARNING};
use http::{HeaderName, Method, Response, StatusCode};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::RenderCacheErrorKind;
use suprnova_live::render_cache::coherence::{FreshnessState, age_seconds, warning_header};
use suprnova_live::render_cache::entry::{CompleteEntry, EntryHeader, SafeHeaders};
use suprnova_live::render_cache::generation::{
    DependencyIdentity, GenerationLedger, GenerationSet, MemoryGenerationLedger,
};
use suprnova_live::render_cache::hot::{HotEntry, HotRequest, ResponseParts, respond, serve_hot};
use suprnova_live::render_cache::http::{
    ConditionalOutcome, cache_control_value, conditional_matches, evaluate_conditional, vary_value,
};
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::policy::{
    FreshnessPolicy, RepresentationClass, SharedCachePolicy,
};
use suprnova_live::render_cache::store::{
    MemoryRenderStore, MemoryStoreLimits, PublicationFence, PublishOutcome, RenderStore,
};
use suprnova_live::render_cache::variance::{
    DimensionValue, VarianceDescriptor, VarianceDimension,
};

const BODY: &[u8] = b"<!doctype html><html><body>hot</body></html>";
const OTHER_BODY: &[u8] = b"<!doctype html><html><body>other</body></html>";
const PUBLISHED_AT_MS: u64 = 1_000;
const CONTENT_TYPE_VALUE: &str = "text/html; charset=utf-8";

fn keys_from(root: u8) -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-test").expect("key id"),
        RootKey::new(vec![root; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fn keys() -> SnapshotKeyRing {
    keys_from(11)
}

fn fence(token: u64) -> PublicationFence {
    PublicationFence {
        epoch: 1,
        generation_digest: [0_u8; 32],
        token,
    }
}

fn freshness() -> FreshnessPolicy {
    FreshnessPolicy::new(60_000, 30_000, 30_000).expect("freshness policy")
}

fn shared() -> SharedCachePolicy {
    SharedCachePolicy::SMaxAge { seconds: 30 }
}

fn locale_variance() -> VarianceDescriptor {
    let mut descriptor = VarianceDescriptor::new();
    descriptor
        .declare(
            VarianceDimension::Locale,
            DimensionValue::Public("de-DE".to_owned()),
        )
        .expect("locale dimension");
    descriptor
}

fn entry_with(
    keys: &SnapshotKeyRing,
    variance: VarianceDescriptor,
    seed_deadline_ms: Option<u64>,
    headers: SafeHeaders,
    body: Bytes,
) -> CompleteEntry {
    CompleteEntry::new(
        EntryHeader {
            key: RenderKey::for_test(keys, "/hot"),
            class: RepresentationClass::PublicShared,
            variance,
            published_at_ms: PUBLISHED_AT_MS,
            fresh_ms: 60_000,
            stale_servable_ms: 30_000,
            stale_on_error_ms: 30_000,
            observed: GenerationSet::default(),
            epoch: 1,
            seed_deadline_ms,
            status: 200,
            headers,
            content_encoding: None,
        },
        body,
    )
}

fn safe_headers() -> SafeHeaders {
    SafeHeaders::from_pairs([
        ("content-type", CONTENT_TYPE_VALUE),
        ("x-content-type-options", "nosniff"),
    ])
    .expect("safe headers")
}

fn plain_entry(keys: &SnapshotKeyRing, body: &'static [u8]) -> CompleteEntry {
    entry_with(
        keys,
        VarianceDescriptor::new(),
        None,
        safe_headers(),
        Bytes::from_static(body),
    )
}

fn hot_fixture(keys: &SnapshotKeyRing, body: &'static [u8]) -> Arc<HotEntry> {
    Arc::new(
        HotEntry::prepare(
            plain_entry(keys, body),
            shared(),
            &freshness(),
            PUBLISHED_AT_MS,
            fence(1),
        )
        .expect("prepare"),
    )
}

fn parts_of<'a>(
    entry: &'a CompleteEntry,
    policy: &'a FreshnessPolicy,
    cache_control_override: Option<&'static str>,
) -> ResponseParts<'a> {
    let header = entry.header();
    ResponseParts {
        status: header.status,
        class: header.class,
        shared: shared(),
        freshness: policy,
        headers: &header.headers,
        variance: &header.variance,
        validator: entry.validator(),
        body: entry.body(),
        published_at_ms: PUBLISHED_AT_MS,
        seed_deadline_ms: header.seed_deadline_ms,
        cache_control_override,
    }
}

fn header_pairs(response: &Response<Bytes>) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().expect("header value is text").to_owned(),
            )
        })
        .collect();
    pairs.sort();
    pairs
}

fn header_text(response: &Response<Bytes>, name: HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .map(|value| value.to_str().expect("header value is text").to_owned())
}

#[test]
fn a_hot_get_returns_the_same_arc_that_was_published() {
    let keys = keys();
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 4,
        max_bytes: 4_096,
    });
    let key = RenderKey::for_test(&keys, "/a");
    let hot = hot_fixture(&keys, BODY);
    assert_eq!(
        store.publish_hot(
            &key,
            Bytes::from_static(b"stored"),
            Arc::clone(&hot),
            fence(1),
            1_000
        ),
        PublishOutcome::Published
    );
    let served = store.hot_get(&key).expect("hot entry");
    assert!(
        Arc::ptr_eq(&served, &hot),
        "the published Arc is handed back, never rebuilt"
    );
    assert_eq!(served.published_at_ms(), PUBLISHED_AT_MS);
    assert_eq!(served.fence(), fence(1));
    assert_eq!(served.entry().body().as_ref(), BODY);
    assert!(
        store
            .hot_get(&RenderKey::for_test(&keys, "/absent"))
            .is_none(),
        "a key that was never published has no hot entry"
    );
}

#[tokio::test]
async fn a_trait_publish_has_no_hot_entry_and_a_hot_publish_replaces_it_under_a_newer_fence() {
    let keys = keys();
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 4,
        max_bytes: 4_096,
    });
    let key = RenderKey::for_test(&keys, "/a");
    assert_eq!(
        store
            .publish(
                &key,
                Bytes::from_static(b"plain"),
                fence(1),
                1_000,
                u64::MAX
            )
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    assert!(
        store.hot_get(&key).is_none(),
        "a trait publication carries no hot entry"
    );
    assert!(store.get(&key).await.expect("get").is_some());

    let first = hot_fixture(&keys, BODY);
    assert_eq!(
        store.publish_hot(
            &key,
            Bytes::from_static(b"hot-1"),
            Arc::clone(&first),
            fence(2),
            2_000
        ),
        PublishOutcome::Published
    );
    assert!(
        Arc::ptr_eq(&store.hot_get(&key).expect("hot entry"), &first),
        "the newer fence installs the hot entry"
    );

    let second = hot_fixture(&keys, OTHER_BODY);
    assert_eq!(
        store.publish_hot(
            &key,
            Bytes::from_static(b"hot-2"),
            Arc::clone(&second),
            fence(2),
            3_000
        ),
        PublishOutcome::Fenced,
        "an equal fence never supersedes"
    );
    assert!(
        Arc::ptr_eq(&store.hot_get(&key).expect("hot entry"), &first),
        "the fenced publication leaves the hot entry untouched"
    );
    assert_eq!(
        store.get(&key).await.expect("get").expect("entry").bytes,
        Bytes::from_static(b"hot-1"),
        "and leaves the bytes untouched"
    );
}

#[tokio::test]
async fn hot_and_plain_entries_share_one_byte_budget_and_one_lru_order() {
    let keys = keys();
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 2,
        max_bytes: 4_096,
    });
    let first = RenderKey::for_test(&keys, "/a");
    let second = RenderKey::for_test(&keys, "/b");
    let third = RenderKey::for_test(&keys, "/c");
    let first_bytes = Bytes::from(vec![1_u8; 10]);
    let second_bytes = Bytes::from(vec![2_u8; 20]);
    let third_bytes = Bytes::from(vec![3_u8; 30]);

    assert_eq!(
        store.publish_hot(
            &first,
            first_bytes,
            hot_fixture(&keys, BODY),
            fence(1),
            1_000
        ),
        PublishOutcome::Published
    );
    assert_eq!(
        store
            .publish(&second, second_bytes.clone(), fence(1), 2_000, u64::MAX)
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    assert_eq!(
        store.publish_hot(
            &third,
            third_bytes.clone(),
            hot_fixture(&keys, OTHER_BODY),
            fence(1),
            3_000
        ),
        PublishOutcome::Published
    );

    assert!(
        store.get(&first).await.expect("get").is_none(),
        "the least recently used entry is evicted"
    );
    assert!(
        store.hot_get(&first).is_none(),
        "its hot entry is evicted with it"
    );
    assert!(store.get(&second).await.expect("get").is_some());
    assert!(
        store.hot_get(&second).is_none(),
        "the trait publication still has no hot entry"
    );
    assert!(store.get(&third).await.expect("get").is_some());
    assert!(store.hot_get(&third).is_some());

    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 2);
    assert_eq!(
        inspection.bytes,
        second_bytes.len() + third_bytes.len(),
        "only the stored bytes are counted, once each"
    );
}

#[test]
fn serve_hot_matches_respond_for_every_request_shape() {
    let keys = keys();
    let policy = freshness();
    let stale = warning_header(FreshnessState::StaleServable);
    assert!(
        stale.is_some(),
        "the stale case must actually carry a warning"
    );
    for seed_deadline_ms in [None, Some(PUBLISHED_AT_MS + 45_000)] {
        let entry = entry_with(
            &keys,
            locale_variance(),
            seed_deadline_ms,
            safe_headers(),
            Bytes::from_static(BODY),
        );
        let etag = entry.validator().etag();
        let hot = HotEntry::prepare(entry.clone(), shared(), &policy, PUBLISHED_AT_MS, fence(1))
            .expect("prepare");
        for method in [Method::GET, Method::HEAD] {
            for if_none_match in [None, Some(etag.as_str()), Some("\"sha256-other\"")] {
                for (now_ms, warning) in [
                    (PUBLISHED_AT_MS + 5_000, None),
                    (PUBLISHED_AT_MS + 70_000, stale),
                ] {
                    let request = HotRequest {
                        method: &method,
                        if_none_match,
                        now_ms,
                    };
                    let hot_response = serve_hot(&hot, request, warning);
                    let cold_response = respond(parts_of(&entry, &policy, None), request, warning)
                        .expect("respond");
                    let shape = format!(
                        "{method} inm={if_none_match:?} now={now_ms} seed={seed_deadline_ms:?}"
                    );
                    assert_eq!(
                        hot_response.status(),
                        cold_response.status(),
                        "status differs for {shape}"
                    );
                    assert_eq!(
                        header_pairs(&hot_response),
                        header_pairs(&cold_response),
                        "headers differ for {shape}"
                    );
                    assert_eq!(
                        hot_response.body(),
                        cold_response.body(),
                        "body differs for {shape}"
                    );
                }
            }
        }
    }
}

#[test]
fn serve_hot_shares_the_stored_body_and_forms_the_documented_headers() {
    let keys = keys();
    let policy = freshness();
    let body = Bytes::from(vec![b'x'; 65_536]);
    let entry = entry_with(&keys, locale_variance(), None, safe_headers(), body.clone());
    let hot = HotEntry::prepare(entry.clone(), shared(), &policy, PUBLISHED_AT_MS, fence(1))
        .expect("prepare");
    let now_ms = PUBLISHED_AT_MS + 7_500;
    let method = Method::GET;
    let response = serve_hot(
        &hot,
        HotRequest {
            method: &method,
            if_none_match: None,
            now_ms,
        },
        None,
    );

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body().len(), 65_536);
    assert_eq!(
        response.body().as_ptr(),
        hot.entry().body().as_ptr(),
        "the stored body is shared, never copied"
    );
    assert_eq!(
        response.body().as_ptr(),
        body.as_ptr(),
        "and it is the very allocation that was published"
    );
    assert_eq!(
        header_text(&response, ETAG).as_deref(),
        Some(entry.validator().etag().as_str())
    );
    assert_eq!(
        header_text(&response, CACHE_CONTROL).as_deref(),
        Some(
            cache_control_value(RepresentationClass::PublicShared, shared(), &policy, None)
                .as_str()
        )
    );
    assert_eq!(
        header_text(&response, AGE).as_deref(),
        Some(age_seconds(PUBLISHED_AT_MS, now_ms).to_string().as_str())
    );
    assert_eq!(
        header_text(&response, VARY).as_deref(),
        Some(
            vary_value(&locale_variance())
                .expect("a declared locale varies")
                .as_str()
        )
    );
    assert_eq!(
        header_text(&response, CONTENT_TYPE).as_deref(),
        Some(CONTENT_TYPE_VALUE)
    );
    assert_eq!(
        header_text(&response, HeaderName::from_static("x-content-type-options")).as_deref(),
        Some("nosniff"),
        "every other stored safe header is replayed"
    );
    assert!(
        header_text(&response, WARNING).is_none(),
        "a fresh hit carries no warning"
    );

    let unvarying = HotEntry::prepare(
        plain_entry(&keys, BODY),
        shared(),
        &policy,
        PUBLISHED_AT_MS,
        fence(1),
    )
    .expect("prepare");
    let unvarying_response = serve_hot(
        &unvarying,
        HotRequest {
            method: &method,
            if_none_match: None,
            now_ms,
        },
        None,
    );
    assert!(
        vary_value(&VarianceDescriptor::new()).is_none(),
        "nothing varies without a declared dimension"
    );
    assert!(
        header_text(&unvarying_response, VARY).is_none(),
        "so the response carries no Vary at all"
    );
}

#[test]
fn a_cache_control_override_replaces_the_computed_value() {
    let keys = keys();
    let policy = freshness();
    let method = Method::GET;
    let request = HotRequest {
        method: &method,
        if_none_match: None,
        now_ms: PUBLISHED_AT_MS + 1_000,
    };
    for seed_deadline_ms in [None, Some(PUBLISHED_AT_MS + 45_000)] {
        let entry = entry_with(
            &keys,
            VarianceDescriptor::new(),
            seed_deadline_ms,
            safe_headers(),
            Bytes::from_static(BODY),
        );
        let fixed = respond(
            parts_of(&entry, &policy, Some("private, no-store")),
            request,
            None,
        )
        .expect("respond");
        assert_eq!(
            header_text(&fixed, CACHE_CONTROL).as_deref(),
            Some("private, no-store"),
            "the fixed value wins, seed deadline or not: {seed_deadline_ms:?}"
        );
        let computed = respond(parts_of(&entry, &policy, None), request, None).expect("respond");
        assert_eq!(
            header_text(&computed, CACHE_CONTROL).as_deref(),
            Some(
                cache_control_value(
                    RepresentationClass::PublicShared,
                    shared(),
                    &policy,
                    seed_deadline_ms.map(|deadline| deadline - request.now_ms),
                )
                .as_str()
            ),
            "and without it the computed value is what is served"
        );
    }
}

#[test]
fn prepare_fails_closed_on_an_unrepresentable_header() {
    let keys = keys();
    let policy = freshness();
    // The entry's own rule bars CR, LF, and NUL from a stored value, which
    // still leaves values no HTTP header may carry, such as this one.
    let headers = SafeHeaders::from_pairs([("content-language", "en\u{1}")])
        .expect("the entry's own rule allows this value");
    let entry = entry_with(
        &keys,
        VarianceDescriptor::new(),
        None,
        headers,
        Bytes::from_static(BODY),
    );
    let error = HotEntry::prepare(entry.clone(), shared(), &policy, PUBLISHED_AT_MS, fence(1))
        .expect_err("an unrepresentable header value fails closed");
    assert_eq!(error.kind(), RenderCacheErrorKind::EntryInvalid);

    let request_method = Method::GET;
    let cold = respond(
        parts_of(&entry, &policy, None),
        HotRequest {
            method: &request_method,
            if_none_match: None,
            now_ms: PUBLISHED_AT_MS,
        },
        None,
    )
    .expect_err("and the same value fails closed on the cold path");
    assert_eq!(cold.kind(), RenderCacheErrorKind::EntryInvalid);
}

#[test]
fn conditional_matches_is_the_rule_evaluate_conditional_applies() {
    let keys = keys();
    let entry = plain_entry(&keys, BODY);
    let validator = *entry.validator();
    let etag = validator.etag();
    let listed = format!("\"sha256-other\", {etag}");

    assert_eq!(
        conditional_matches(Some("*"), &etag),
        ConditionalOutcome::NotModified,
        "a star matches every representation"
    );
    assert_eq!(
        conditional_matches(Some(&listed), &etag),
        ConditionalOutcome::NotModified,
        "a listed candidate matches after trimming"
    );
    assert_eq!(
        conditional_matches(Some("\"sha256-other\""), &etag),
        ConditionalOutcome::Full,
        "an unrelated tag does not match"
    );
    assert_eq!(
        conditional_matches(None, &etag),
        ConditionalOutcome::Full,
        "no header means the full representation"
    );

    for header in [
        None,
        Some("*"),
        Some("\"sha256-other\""),
        Some(listed.as_str()),
        Some(etag.as_str()),
    ] {
        assert_eq!(
            evaluate_conditional(header, &validator),
            conditional_matches(header, &etag),
            "the validator form applies exactly this rule for {header:?}"
        );
    }
}

#[tokio::test]
async fn current_with_epoch_defaults_to_the_two_reads() {
    let ledger = MemoryGenerationLedger::new();
    let identity = DependencyIdentity::table("orders");
    let untouched = DependencyIdentity::table("invoices");
    ledger
        .advance(std::slice::from_ref(&identity))
        .await
        .expect("advance");
    ledger.advance_epoch();

    let (generations, epoch) = ledger
        .current_with_epoch(&[identity.digest(), untouched.digest()])
        .await
        .expect("current with epoch");

    assert_eq!(
        generations.get(&identity),
        Some(1),
        "the advanced identity reads back at generation 1"
    );
    assert_eq!(
        generations.get(&untouched),
        Some(0),
        "an unobserved identity reads back at 0"
    );
    assert_eq!(generations.len(), 2);
    assert_eq!(epoch, 2, "the epoch advanced once from its initial 1");
    assert_eq!(
        (
            ledger
                .current(&[identity.digest(), untouched.digest()])
                .await
                .expect("current"),
            ledger.epoch().await.expect("epoch")
        ),
        (generations, epoch),
        "the default body is exactly the two reads"
    );
}
