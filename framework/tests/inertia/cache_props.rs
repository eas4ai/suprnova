//! Compile-time and boot-time checks fail before a shared shell can be used.
use suprnova::{Inertia, InertiaProps};

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct CacheProps {
    #[inertia(public)]
    title: String,
    #[inertia(per_request)]
    viewer: suprnova::Prop,
}

#[test]
fn cache_012_undeclared_field_fails_to_build() {
    trybuild::TestCases::new().compile_fail("tests/inertia/compile_fail/cache/cache_012_*.rs");
}

#[test]
fn cache_012_boot_refuses_unlisted_shared_props_with_route_and_name() {
    let _guard = suprnova::testing::TestContainer::fake();
    Inertia::share("auth", serde_json::json!({"name": "private"})).unwrap();
    let error =
        Inertia::cache_page::<CacheProps>("/private-page", "Cache", &[], &["errors", "flash"])
            .expect_err("auth needs an explicit boundary");
    let message = error.to_string();
    assert!(message.contains("/private-page"), "{message}");
    assert!(message.contains("auth"), "{message}");
}

#[test]
fn cache_012_typed_macro_conversion_keeps_lazy_props_and_wire_names() {
    let _guard = suprnova::testing::TestContainer::fake();
    Inertia::cache_page::<CacheProps>("/cache", "Cache", &[], &["errors", "flash"]).unwrap();
    let props = CacheProps {
        title: "public".to_owned(),
        viewer: suprnova::Prop::lazy(|| async { serde_json::json!("private") }),
    };
    let response = suprnova::inertia::InertiaPageProps::apply_props(
        &props,
        suprnova::InertiaResponse::new("Cache"),
    );
    assert!(response.is_ok());
    use suprnova::inertia::InertiaPageProps as _;
    let borrowed = &props;
    assert!(
        borrowed
            .apply_props(suprnova::InertiaResponse::new("Cache"))
            .is_ok()
    );
    assert_eq!(
        <CacheProps as suprnova::inertia::CacheableInertiaProps>::PUBLIC,
        &["title"]
    );
    assert_eq!(
        <CacheProps as suprnova::inertia::CacheableInertiaProps>::PER_REQUEST,
        &["viewer"]
    );
}

#[test]
fn cache_012_plain_typed_props_keep_serialization_and_object_errors() {
    use suprnova::inertia::InertiaPageProps;
    struct Fails;
    impl suprnova::serde::Serialize for Fails {
        fn serialize<S: suprnova::serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(suprnova::serde::ser::Error::custom("deliberate failure"))
        }
    }
    let _guard = suprnova::testing::TestContainer::fake();
    let error = match "not an object".apply_props(suprnova::InertiaResponse::new("Cache")) {
        Err(error) => error,
        Ok(_) => panic!("non-object props must fail"),
    };
    assert!(error.to_string().contains("JSON object"));
    let error = match Fails.apply_props(suprnova::InertiaResponse::new("Cache")) {
        Err(error) => error,
        Ok(_) => panic!("serialization must fail"),
    };
    assert!(
        error
            .to_string()
            .contains("typed props failed to serialize")
    );
    assert!(error.to_string().contains("deliberate failure"));
}

#[test]
fn cache_012_boot_refuses_eager_shared_hooks_before_they_can_cross_a_guard() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    struct EagerShares;
    impl suprnova::InertiaMiddlewareHooks for EagerShares {
        fn share(
            &self,
            _: &dyn suprnova::InertiaRequestExt,
        ) -> suprnova::indexmap::IndexMap<String, suprnova::Prop> {
            CALLS.fetch_add(1, Ordering::SeqCst);
            [("viewer".to_owned(), suprnova::Prop::eager("private".into()))].into()
        }
    }
    let _guard = suprnova::testing::TestContainer::fake();
    Inertia::install(
        &suprnova::InertiaConfig::new()
            .development(true)
            .hooks(EagerShares),
    )
    .unwrap();
    let error = Inertia::cache_page::<CacheProps>(
        "/guarded-cache",
        "Cache",
        &[],
        &["errors", "flash", "viewer"],
    )
    .expect_err("opaque hooks must not run ahead of route guards");
    assert!(error.to_string().contains("/guarded-cache"));
    assert!(error.to_string().contains("middleware hook"));
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
}

#[test]
fn cache_012_shared_props_accept_an_explicit_public_declaration() {
    let _guard = suprnova::testing::TestContainer::fake();
    assert!(
        Inertia::cache_page::<CacheProps>("/public-shared", "Cache", &["errors", "flash"], &[],)
            .is_ok()
    );
}

#[test]
fn cache_013_unknown_field_and_non_props_fail_to_build() {
    trybuild::TestCases::new().compile_fail("tests/inertia/compile_fail/cache/cache_013_*.rs");
}
