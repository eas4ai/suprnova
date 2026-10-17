//! Task 5's bypass probe: one route whose every cost is counted.
//!
//! Split out of `mod.rs` by Task 5b's review, which found that file at
//! 2,694 lines. The probe handler, its inline template, and the four
//! counters that make each of its costs observable are one subject, so
//! they live in one file; `mod.rs` re-exports [`probe_route`], so no
//! test's imports change. A pure move: nothing here behaves differently
//! from the same code in `mod.rs`.

use super::*;

/// Task 5: the bypass probe. One handler behind `/probe/{id}` (authority
/// coherence) and `/probe-leased/{id}` (lease coherence), doing one of each
/// thing a hit is supposed to remove: a handler call, an ORM query, a
/// template render, and a serialization. Each is counted separately in
/// [`probe_route`], so "a hit ran nothing" is four observations rather than
/// one inference from a render count.
///
/// It never touches [`counting_route`]: the two counter sets stay
/// independent so a probe dispatch cannot perturb any other test in this
/// binary, and so a bypass test reads only counters it owns.
pub(super) async fn probe_handler(request: Request) -> Response {
    probe_route::on_handler_call();
    let id: i64 = request
        .param("id")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    probe_route::on_query();
    let title = Post::query()
        .first()
        .await?
        .map_or_else(|| "none".to_owned(), |post| post.title);
    let payload = probe_route::serialize(&serde_json::json!({
        "id": id,
        "title": title.clone(),
    }));
    Ok(HttpResponse::html(probe_route::render(ProbeTemplate {
        id,
        title,
        payload,
    })))
}

/// The probe's template. Declared inline rather than as a file under
/// `tests/templates/`: the point is only that a real Askama render happens
/// on a miss and does not happen on a hit, and an inline source keeps the
/// whole probe - handler, template, counters - readable in one place.
#[derive(askama::Template)]
#[template(
    source = "<p>probe {{ id }} over {{ title }} carrying {{ payload }}</p>",
    ext = "html"
)]
struct ProbeTemplate {
    id: i64,
    title: String,
    payload: String,
}

/// Task 5: what the bypass probe route did, counted one cost at a time.
///
/// Process-global atomics with the same lifetime rules as
/// [`counting_route`]'s own counters: zeroed by [`reset`], which every boot
/// calls, and readable by a test at any point. A bypass test resets them
/// after the miss it needs as a baseline and then reads them again after
/// the hit, so what it measures is one request, never a whole test.
pub mod probe_route {
    use super::*;

    static HANDLER_CALLS: AtomicU64 = AtomicU64::new(0);
    static QUERIES: AtomicU64 = AtomicU64::new(0);
    static TEMPLATE_RENDERS: AtomicU64 = AtomicU64::new(0);
    static SERIALIZATIONS: AtomicU64 = AtomicU64::new(0);

    /// How many times the probe handler has been entered.
    pub fn handler_calls() -> u64 {
        HANDLER_CALLS.load(Ordering::SeqCst)
    }

    /// How many ORM queries the probe handler has issued.
    pub fn queries() -> u64 {
        QUERIES.load(Ordering::SeqCst)
    }

    /// How many Askama renders the probe handler has run.
    pub fn template_renders() -> u64 {
        TEMPLATE_RENDERS.load(Ordering::SeqCst)
    }

    /// How many payload serializations the probe handler has run.
    pub fn serializations() -> u64 {
        SERIALIZATIONS.load(Ordering::SeqCst)
    }

    /// Zeroes all four counters.
    pub fn reset() {
        HANDLER_CALLS.store(0, Ordering::SeqCst);
        QUERIES.store(0, Ordering::SeqCst);
        TEMPLATE_RENDERS.store(0, Ordering::SeqCst);
        SERIALIZATIONS.store(0, Ordering::SeqCst);
    }

    pub(crate) fn on_handler_call() {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn on_query() {
        QUERIES.fetch_add(1, Ordering::SeqCst);
    }

    /// Renders `template`, counting the render. The wrapper is what makes
    /// the render observable: `Template::render` itself reports nothing.
    pub(crate) fn render<T: askama::Template>(template: T) -> String {
        TEMPLATE_RENDERS.fetch_add(1, Ordering::SeqCst);
        template.render().expect("the probe template renders")
    }

    /// Serializes `value`, counting the serialization.
    pub(crate) fn serialize(value: &serde_json::Value) -> String {
        SERIALIZATIONS.fetch_add(1, Ordering::SeqCst);
        serde_json::to_string(value).expect("a probe payload serializes")
    }
}
