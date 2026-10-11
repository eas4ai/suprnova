//! Metrics facade - the user-facing API for counters, histograms, and gauges.
//!
//! Two compile-time shapes:
//!
//! - **`otel` enabled:** instruments are looked up in a global cache keyed by
//!   name, so `Metrics::counter("http.requests.total").inc()` is a constant-
//!   time hashmap lookup plus an atomic increment.
//! - **`otel` disabled (default):** every method is `#[inline(always)]` and
//!   discards its arguments. The compiler erases instrumentation entirely,
//!   so instrumented user code pays zero runtime cost in default builds.
//!
//! The public surface (`Metrics`, `CounterHandle`, `HistogramHandle`,
//! `GaugeHandle`) is identical in both modes.
//!
//! Naming: stable, ASCII, dot-delimited (e.g. `"http.requests.total"`,
//! `"http.request.duration"`). Standard OTel semantic conventions live in
//! `opentelemetry-semantic-conventions::metric::*`.

/// The value of one attribute of a metric: a text, a whole number, a
/// number with a fraction, or yes and no.
///
/// OpenTelemetry attributes have types, and a backend treats them by
/// their type. The status `404` as a text matches no filter on a number
/// and no range, and the attributes the semantic conventions type as a
/// number or as yes and no, `http.response.status_code` and `error` among
/// them, arrive as what they are only when they are sent as that.
///
/// A value is made from what it is written as:
///
/// ```rust
/// use suprnova::{AttrValue, Metrics};
///
/// let requests = Metrics::counter("http.requests.total");
/// // One type for every value: the values are written as they are.
/// requests.inc_with(&[("route", "/posts"), ("method", "GET")]);
/// requests.inc_with(&[("http.response.status_code", 404)]);
/// // Values of several types: each is made an `AttrValue`.
/// requests.inc_with(&[
///     ("route", AttrValue::from("/posts")),
///     ("http.response.status_code", AttrValue::from(404)),
///     ("error", AttrValue::from(true)),
/// ]);
/// ```
///
/// A text is a `&str`, or a reference to what holds one: `&String`,
/// `&&str`, `&Box<str>`, `&Arc<str>`, `&Rc<str>` and `&Cow<str>`. A
/// call with no attribute is `inc()`, `record()` or `set()`: an empty
/// list has no type for its values.
///
/// The type is `#[non_exhaustive]`: OpenTelemetry has lists of values as
/// well, which can be added here.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum AttrValue<'a> {
    /// A text.
    Text(&'a str),
    /// A whole number.
    Int(i64),
    /// A number with a fraction.
    Float(f64),
    /// Yes or no.
    Bool(bool),
}

impl<'a> From<&'a str> for AttrValue<'a> {
    fn from(value: &'a str) -> Self {
        Self::Text(value)
    }
}

impl<'a> From<&'a String> for AttrValue<'a> {
    fn from(value: &'a String) -> Self {
        Self::Text(value)
    }
}

/// What a loop over a list of texts gives: `for route in ROUTES.iter()`.
impl<'a> From<&&'a str> for AttrValue<'a> {
    fn from(value: &&'a str) -> Self {
        Self::Text(value)
    }
}

/// The holders of a text, by reference. A call has its text in one of
/// them as often as in a `String`, and would have to write
/// `value.as_ref()` with the type named without these.
macro_rules! attr_value_from_text_holder {
    ($($holder:ty),+) => {$(
        impl<'a> From<&'a $holder> for AttrValue<'a> {
            fn from(value: &'a $holder) -> Self {
                Self::Text(value)
            }
        }
    )+};
}
attr_value_from_text_holder!(
    Box<str>,
    std::sync::Arc<str>,
    std::rc::Rc<str>,
    std::borrow::Cow<'_, str>
);

impl From<bool> for AttrValue<'_> {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<f64> for AttrValue<'_> {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<f32> for AttrValue<'_> {
    fn from(value: f32) -> Self {
        Self::Float(f64::from(value))
    }
}

/// The whole numbers that every `i64` holds.
macro_rules! attr_value_from_int {
    ($($int:ty),+) => {$(
        impl From<$int> for AttrValue<'_> {
            fn from(value: $int) -> Self {
                Self::Int(i64::from(value))
            }
        }
    )+};
}
attr_value_from_int!(i64, i32, i16, i8, u32, u16, u8);

/// The whole numbers an `i64` may be too small for. An attribute is a
/// label of a measurement, and a label that is as large as that is a
/// mistake of its own: it is sent as the largest `i64`, and the
/// measurement is not lost for it.
macro_rules! attr_value_from_large_int {
    ($($int:ty),+) => {$(
        impl From<$int> for AttrValue<'_> {
            fn from(value: $int) -> Self {
                Self::Int(i64::try_from(value).unwrap_or(i64::MAX))
            }
        }
    )+};
}
attr_value_from_large_int!(u64, usize);

impl From<isize> for AttrValue<'_> {
    fn from(value: isize) -> Self {
        Self::Int(i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX }))
    }
}

#[cfg(feature = "otel")]
mod real {
    use super::AttrValue;
    use opentelemetry::KeyValue;
    use opentelemetry::global;
    use opentelemetry::metrics::{Counter, Gauge, Histogram};
    use std::sync::Arc;

    /// Build a fresh instrument handle on every call. We intentionally
    /// don't cache at this layer because OTel's SDK already caches
    /// instruments keyed by (name, unit, description) inside the meter,
    /// so repeated builds collapse to the same underlying `Arc<Counter>`
    /// / `Arc<Histogram>` / `Arc<Gauge>` after the first call. Caching
    /// at our layer was a trap: if `Metrics::counter("x")` ran once
    /// before `init_telemetry` installed the real provider, the cached
    /// handle was bound to the no-op meter permanently - silent data
    /// loss. Going through `global::meter("suprnova")` per call always
    /// resolves to whatever provider is currently installed.
    fn to_keyvalues<'a, V>(attrs: &[(&'static str, V)]) -> Vec<KeyValue>
    where
        V: Into<AttrValue<'a>> + Copy,
    {
        attrs
            .iter()
            .map(|(key, value)| KeyValue::new(*key, to_value((*value).into())))
            .collect()
    }

    /// The attribute value with the type it has. A text is the one that
    /// is copied: OpenTelemetry keeps what it is given.
    pub(super) fn to_value(value: AttrValue<'_>) -> opentelemetry::Value {
        match value {
            AttrValue::Text(text) => opentelemetry::Value::from(text.to_owned()),
            AttrValue::Int(int) => opentelemetry::Value::from(int),
            AttrValue::Float(float) => opentelemetry::Value::from(float),
            AttrValue::Bool(flag) => opentelemetry::Value::from(flag),
        }
    }

    /// Entry point for creating metric instruments. Each call resolves
    /// the current provider via `global::meter("suprnova")` and asks
    /// it for an instrument with the given name. The OTel SDK caches
    /// instruments per `(name, unit, description)` internally.
    pub struct Metrics;

    impl Metrics {
        /// Get a monotonically-increasing counter.
        pub fn counter(name: &'static str) -> CounterHandle {
            let counter = global::meter("suprnova").u64_counter(name).build();
            CounterHandle(Arc::new(counter))
        }

        /// Get an `f64` histogram for value distributions.
        pub fn histogram(name: &'static str) -> HistogramHandle {
            let hist = global::meter("suprnova").f64_histogram(name).build();
            HistogramHandle(Arc::new(hist))
        }

        /// Get a synchronous `f64` gauge.
        pub fn gauge(name: &'static str) -> GaugeHandle {
            let gauge = global::meter("suprnova").f64_gauge(name).build();
            GaugeHandle(Arc::new(gauge))
        }
    }

    /// Cheap-to-clone handle backed by a cached `Counter<u64>`.
    #[derive(Clone)]
    pub struct CounterHandle(pub(crate) Arc<Counter<u64>>);

    impl CounterHandle {
        /// Increment by 1.
        pub fn inc(&self) {
            self.0.add(1, &[]);
        }
        /// Increment by `n`.
        pub fn inc_by(&self, n: u64) {
            self.0.add(n, &[]);
        }
        /// Increment by 1 with attributes. A value is a text, a number
        /// or yes and no, see [`AttrValue`](super::AttrValue).
        pub fn inc_with<'a, V>(&self, attrs: &[(&'static str, V)])
        where
            V: Into<AttrValue<'a>> + Copy,
        {
            self.0.add(1, &to_keyvalues(attrs));
        }
    }

    /// Cheap-to-clone handle backed by a cached `Histogram<f64>`.
    #[derive(Clone)]
    pub struct HistogramHandle(pub(crate) Arc<Histogram<f64>>);

    impl HistogramHandle {
        /// Record a value.
        pub fn record(&self, value: f64) {
            self.0.record(value, &[]);
        }
        /// Record a value with attributes. A value of an attribute is a
        /// text, a number or yes and no, see
        /// [`AttrValue`](super::AttrValue).
        pub fn record_with<'a, V>(&self, value: f64, attrs: &[(&'static str, V)])
        where
            V: Into<AttrValue<'a>> + Copy,
        {
            self.0.record(value, &to_keyvalues(attrs));
        }
    }

    /// Cheap-to-clone handle backed by a cached `Gauge<f64>`.
    #[derive(Clone)]
    pub struct GaugeHandle(pub(crate) Arc<Gauge<f64>>);

    impl GaugeHandle {
        /// Set the gauge to `value`. (OTel's gauge API uses `record` -
        /// we expose `set` for clarity to users.)
        pub fn set(&self, value: f64) {
            self.0.record(value, &[]);
        }
        /// Set the gauge to `value` with attributes. A value of an
        /// attribute is a text, a number or yes and no, see
        /// [`AttrValue`](super::AttrValue).
        pub fn set_with<'a, V>(&self, value: f64, attrs: &[(&'static str, V)])
        where
            V: Into<AttrValue<'a>> + Copy,
        {
            self.0.record(value, &to_keyvalues(attrs));
        }
    }
}

#[cfg(not(feature = "otel"))]
mod stub {
    use super::AttrValue;

    /// Entry point for creating metric instruments. In default builds
    /// (no `otel` feature) all methods are inert no-ops.
    pub struct Metrics;

    impl Metrics {
        /// Return a counter handle for the given metric name. No-op in the stub build.
        #[inline(always)]
        pub fn counter(_name: &'static str) -> CounterHandle {
            CounterHandle
        }
        /// Return a histogram handle for the given metric name. No-op in the stub build.
        #[inline(always)]
        pub fn histogram(_name: &'static str) -> HistogramHandle {
            HistogramHandle
        }
        /// Return a gauge handle for the given metric name. No-op in the stub build.
        #[inline(always)]
        pub fn gauge(_name: &'static str) -> GaugeHandle {
            GaugeHandle
        }
    }

    /// Zero-cost stub. All methods compile to nothing.
    #[derive(Clone)]
    pub struct CounterHandle;

    impl CounterHandle {
        /// Increment the counter by 1.
        #[inline(always)]
        pub fn inc(&self) {}
        /// Increment the counter by `_n`.
        #[inline(always)]
        pub fn inc_by(&self, _n: u64) {}
        /// Increment the counter by 1, attaching the supplied attribute set.
        #[inline(always)]
        pub fn inc_with<'a, V>(&self, _attrs: &[(&'static str, V)])
        where
            V: Into<AttrValue<'a>> + Copy,
        {
        }
    }

    /// Zero-cost stub. All methods compile to nothing.
    #[derive(Clone)]
    pub struct HistogramHandle;

    impl HistogramHandle {
        /// Record an observation of `_value`.
        #[inline(always)]
        pub fn record(&self, _value: f64) {}
        /// Record an observation of `_value`, attaching the supplied attribute set.
        #[inline(always)]
        pub fn record_with<'a, V>(&self, _value: f64, _attrs: &[(&'static str, V)])
        where
            V: Into<AttrValue<'a>> + Copy,
        {
        }
    }

    /// Zero-cost stub. All methods compile to nothing.
    #[derive(Clone)]
    pub struct GaugeHandle;

    impl GaugeHandle {
        /// Set the gauge to `_value`.
        #[inline(always)]
        pub fn set(&self, _value: f64) {}
        /// Set the gauge to `_value`, attaching the supplied attribute set.
        #[inline(always)]
        pub fn set_with<'a, V>(&self, _value: f64, _attrs: &[(&'static str, V)])
        where
            V: Into<AttrValue<'a>> + Copy,
        {
        }
    }
}

#[cfg(feature = "otel")]
pub use real::{CounterHandle, GaugeHandle, HistogramHandle, Metrics};
#[cfg(not(feature = "otel"))]
pub use stub::{CounterHandle, GaugeHandle, HistogramHandle, Metrics};

#[cfg(test)]
mod tests {
    use super::*;

    // These tests must compile and pass in BOTH feature configurations.
    // In the stub case they verify the API exists and accepts the right
    // shapes; in the real case they verify cache identity and
    // no-panic behavior before `init_telemetry` has installed providers.

    #[test]
    fn a_value_is_made_from_what_it_is_written_as() {
        assert_eq!(AttrValue::from("/posts"), AttrValue::Text("/posts"));
        let owned = String::from("GET");
        assert_eq!(AttrValue::from(&owned), AttrValue::Text("GET"));
        assert_eq!(AttrValue::from(404), AttrValue::Int(404));
        assert_eq!(AttrValue::from(404_u16), AttrValue::Int(404));
        assert_eq!(AttrValue::from(-1_i8), AttrValue::Int(-1));
        assert_eq!(AttrValue::from(3_usize), AttrValue::Int(3));
        assert_eq!(AttrValue::from(0.5), AttrValue::Float(0.5));
        assert_eq!(AttrValue::from(0.5_f32), AttrValue::Float(0.5));
        assert_eq!(AttrValue::from(true), AttrValue::Bool(true));
    }

    /// A reference to what holds a text is a value, so a call does not
    /// have to take the `&str` out of it.
    #[test]
    fn a_reference_to_what_holds_a_text_is_a_text() {
        use std::borrow::Cow;
        use std::rc::Rc;
        use std::sync::Arc;

        let boxed: Box<str> = "boxed".into();
        let shared: Arc<str> = "shared".into();
        let counted: Rc<str> = "counted".into();
        let borrowed: Cow<'_, str> = Cow::Borrowed("borrowed");
        let owned: Cow<'_, str> = Cow::Owned("owned".to_owned());
        assert_eq!(AttrValue::from(&boxed), AttrValue::Text("boxed"));
        assert_eq!(AttrValue::from(&shared), AttrValue::Text("shared"));
        assert_eq!(AttrValue::from(&counted), AttrValue::Text("counted"));
        assert_eq!(AttrValue::from(&borrowed), AttrValue::Text("borrowed"));
        assert_eq!(AttrValue::from(&owned), AttrValue::Text("owned"));

        const ROUTES: [&str; 2] = ["/posts", "/users"];
        let c = Metrics::counter("test.typed.by_reference");
        for route in ROUTES.iter() {
            assert_eq!(AttrValue::from(route), AttrValue::Text(route));
            c.inc_with(&[("route", route)]);
        }
        c.inc_with(&[("route", &shared)]);
        c.inc_with(&[("route", &borrowed)]);
    }

    #[test]
    fn a_number_that_no_i64_holds_is_the_largest_one() {
        assert_eq!(AttrValue::from(u64::MAX), AttrValue::Int(i64::MAX));
        assert_eq!(AttrValue::from(usize::MAX), AttrValue::Int(i64::MAX));
        assert_eq!(
            AttrValue::from(u64::try_from(i64::MAX).unwrap()),
            AttrValue::Int(i64::MAX)
        );
    }

    #[test]
    fn the_handles_take_values_of_every_type() {
        let c = Metrics::counter("test.typed.requests");
        c.inc_with(&[("route", "/posts"), ("method", "GET")]);
        c.inc_with(&[("http.response.status_code", 404)]);
        c.inc_with(&[("error", true)]);
        c.inc_with(&[
            ("route", AttrValue::from("/posts")),
            ("http.response.status_code", AttrValue::from(404_u16)),
            ("error", AttrValue::from(false)),
            ("ratio", AttrValue::from(0.25)),
        ]);

        let route = String::from("/posts/{id}");
        let h = Metrics::histogram("test.typed.duration");
        h.record_with(1.5, &[("route", route.as_str())]);
        h.record_with(1.5, &[("route", &route)]);

        let g = Metrics::gauge("test.typed.depth");
        g.set_with(7.0, &[("shard", 3_usize)]);
    }

    /// A value has to arrive with its type. As a text, `404` matches no
    /// filter on a number, which is what this change is for.
    #[cfg(feature = "otel")]
    #[test]
    fn a_value_is_sent_with_the_type_it_has() {
        use opentelemetry::Value;

        assert_eq!(
            real::to_value(AttrValue::Int(404)),
            Value::I64(404),
            "a number is no text"
        );
        assert_eq!(real::to_value(AttrValue::Bool(true)), Value::Bool(true));
        assert_eq!(real::to_value(AttrValue::Float(0.25)), Value::F64(0.25));
        assert_eq!(
            real::to_value(AttrValue::Text("/posts")),
            Value::from("/posts".to_owned())
        );
    }

    #[test]
    fn counter_noop_before_init() {
        let c = Metrics::counter("test.requests.total");
        c.inc();
        c.inc_by(42);
        c.inc_with(&[("env", "test"), ("route", "/")]);
    }

    #[test]
    fn histogram_noop_before_init() {
        let h = Metrics::histogram("test.request.duration");
        h.record(1.25);
        h.record_with(2.5, &[("route", "/api")]);
    }

    #[test]
    fn gauge_noop_before_init() {
        let g = Metrics::gauge("test.queue.depth");
        g.set(0.0);
        g.set_with(7.0, &[("queue", "default")]);
    }

    // Each Metrics::counter()/histogram()/gauge() call returns a fresh
    // handle (we don't cache at this layer - see the module-level note
    // for rationale). The OTel SDK is responsible for instrument
    // identity under the (name, unit, description) key. These tests
    // verify the API is idempotent in observable behavior: repeated
    // calls produce usable handles that record without panicking.
    #[cfg(feature = "otel")]
    #[test]
    fn counter_repeated_calls_produce_usable_handles() {
        let a = Metrics::counter("test.repeated.counter");
        let b = Metrics::counter("test.repeated.counter");
        a.inc();
        b.inc_by(5);
        a.inc_with(&[("env", "test")]);
    }

    #[cfg(feature = "otel")]
    #[test]
    fn histogram_repeated_calls_produce_usable_handles() {
        let a = Metrics::histogram("test.repeated.histogram");
        let b = Metrics::histogram("test.repeated.histogram");
        a.record(1.0);
        b.record_with(2.0, &[("env", "test")]);
    }

    #[cfg(feature = "otel")]
    #[test]
    fn gauge_repeated_calls_produce_usable_handles() {
        let a = Metrics::gauge("test.repeated.gauge");
        let b = Metrics::gauge("test.repeated.gauge");
        a.set(1.0);
        b.set_with(2.0, &[("env", "test")]);
    }
}
