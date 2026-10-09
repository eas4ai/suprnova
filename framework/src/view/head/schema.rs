//! JSON-LD schemas for the document head: schema.org objects built with
//! chained calls, each written as its own `<script type="application/ld+json">`
//! element.

use serde_json::{Map, Value};

/// A schema.org object, written as one JSON-LD script element. Laravel
/// Head's `Schema` factory.
///
/// ```rust
/// use suprnova::head::{HeadBuilder, Schema};
///
/// let head = HeadBuilder::new()
///     .schema(Schema::of("Product").set("name", "Desk"))
///     .schema(Schema::breadcrumbs().item("Home", "https://example.com/"))
///     .schema(Schema::faq().question("Is it free?", "Yes."));
/// assert_eq!(head.to_array().as_array().map(Vec::len), Some(3));
/// ```
///
/// Every value is written as JSON with `<`, `>` and `&` escaped, so a value
/// that holds `</script>` cannot end the element.
#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    object: Map<String, Value>,
}

impl Schema {
    /// An object of the schema.org type `kind`, such as `Article`,
    /// `Product`, `Organization` or a type of your own.
    pub fn of(kind: impl Into<String>) -> Self {
        let mut object = Map::new();
        object.insert(
            "@context".to_string(),
            Value::String("https://schema.org".to_string()),
        );
        object.insert("@type".to_string(), Value::String(kind.into()));
        Self { object }
    }

    /// A `BreadcrumbList`. Add items with [`Self::item`] or
    /// [`Self::items`]; positions follow the order they are added in.
    pub fn breadcrumbs() -> Self {
        Self::of("BreadcrumbList").set("itemListElement", Value::Array(Vec::new()))
    }

    /// An `FAQPage`. Add entries with [`Self::question`] or
    /// [`Self::questions`].
    pub fn faq() -> Self {
        Self::of("FAQPage").set("mainEntity", Value::Array(Vec::new()))
    }

    /// Set `key` to `value`, replacing an earlier value.
    pub fn set(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.object.insert(key.into(), value.into());
        self
    }

    /// Append one breadcrumb, positioned after the ones before it.
    pub fn item(mut self, name: impl Into<String>, url: impl Into<String>) -> Self {
        let position = match self.object.get("itemListElement") {
            Some(Value::Array(items)) => items.len() + 1,
            _ => 1,
        };
        self.push(
            "itemListElement",
            serde_json::json!({
                "@type": "ListItem",
                "position": position,
                "name": name.into(),
                "item": url.into(),
            }),
        );
        self
    }

    /// Append breadcrumbs in order, each a name and a URL.
    pub fn items<N, U>(self, items: impl IntoIterator<Item = (N, U)>) -> Self
    where
        N: Into<String>,
        U: Into<String>,
    {
        items
            .into_iter()
            .fold(self, |schema, (name, url)| schema.item(name, url))
    }

    /// Append one FAQ entry.
    pub fn question(mut self, question: impl Into<String>, answer: impl Into<String>) -> Self {
        self.push(
            "mainEntity",
            serde_json::json!({
                "@type": "Question",
                "name": question.into(),
                "acceptedAnswer": {
                    "@type": "Answer",
                    "text": answer.into(),
                },
            }),
        );
        self
    }

    /// Append FAQ entries in order, each a question and its answer.
    pub fn questions<Q, A>(self, entries: impl IntoIterator<Item = (Q, A)>) -> Self
    where
        Q: Into<String>,
        A: Into<String>,
    {
        entries
            .into_iter()
            .fold(self, |schema, (question, answer)| {
                schema.question(question, answer)
            })
    }

    /// The object as JSON.
    pub fn to_value(&self) -> Value {
        Value::Object(self.object.clone())
    }

    /// Append `entry` to the list under `key`, which becomes a list when
    /// it held another value.
    fn push(&mut self, key: &str, entry: Value) {
        match self.object.get_mut(key) {
            Some(Value::Array(items)) => items.push(entry),
            _ => {
                self.object
                    .insert(key.to_string(), Value::Array(vec![entry]));
            }
        }
    }
}

impl From<Value> for Schema {
    /// A schema written as the JSON you give, `@context` included if you
    /// want one.
    fn from(value: Value) -> Self {
        match value {
            Value::Object(object) => Self { object },
            other => {
                let mut object = Map::new();
                object.insert("@graph".to_string(), other);
                Self { object }
            }
        }
    }
}
