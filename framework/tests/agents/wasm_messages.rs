//! AGENT-016: a page tool reports the server's English message for each
//! built-in `validator` rule, so an agent reads one wording for one mistake
//! whichever side checked its input.
//!
//! The WebAssembly crate cannot read the framework's catalog at run time,
//! so it carries the messages of `validator`'s built-in rules as a table
//! generated from the English catalog,
//! `crates/suprnova-webmcp/src/catalog.json`. The first test regenerates
//! the table with the parser the server reads the catalog with, and fails
//! when the checked-in table differs. The second pins the server's own
//! rendering of the texts the crate's tests expect.

use std::path::Path;
use std::sync::Arc;

use fluent_syntax::ast::{
    Entry, Expression, InlineExpression, Pattern, PatternElement, VariantKey,
};
use serde_json::{Map, Value, json};
use suprnova::container::testing::TestContainer;
use suprnova::data::input_names::input_key;
use suprnova::{
    FluentTranslator, Locale, LocalizationConfig, Translator, Validate, ValidationErrors,
    scope_locale,
};

/// The framework's English validation catalog.
const CATALOG: &str = include_str!("../../src/localization/catalogs/en/validation.ftl");

/// The table the WebAssembly crate carries.
const TABLE: &str = include_str!("../../../crates/suprnova-webmcp/src/catalog.json");

/// The table's path from the framework's manifest directory.
const TABLE_PATH: &str = "../crates/suprnova-webmcp/src/catalog.json";

/// The command that rewrites the table from the catalog.
const REGENERATE: &str = "SUPRNOVA_WEBMCP_CATALOG=write cargo nextest run -p suprnova --test agents \
                          -E 'test(/agent_016_page_tool_messages_match_the_english_catalog/)'";

/// The codes `validator` 0.20's derive reports for its built-in rules. The
/// server keys each as `validation-<code>`, `_` folded to `-`, and words
/// it from the catalog when the catalog has that id.
const BUILT_IN_CODES: [&str; 12] = [
    "contains",
    "credit_card",
    "does_not_contain",
    "email",
    "ip",
    "length",
    "must_match",
    "non_control_character",
    "range",
    "regex",
    "required",
    "url",
];

/// The table for the catalog: each built-in rule's message the catalog
/// defines, as text pieces, arguments and selections on `$kind`.
fn generate() -> String {
    let resource = match fluent_syntax::parser::parse(CATALOG) {
        Ok(resource) => resource,
        Err((_, errors)) => panic!("the English catalog does not parse: {errors:?}"),
    };
    let mut messages = Map::new();
    for code in BUILT_IN_CODES {
        let id = format!("validation-{}", code.replace('_', "-"));
        let found = resource.body.iter().find_map(|entry| match entry {
            Entry::Message(message) if message.id.name == id => Some(message),
            _ => None,
        });
        let Some(message) = found else {
            continue;
        };
        let pattern = message
            .value
            .as_ref()
            .unwrap_or_else(|| panic!("`{id}` has no value"));
        messages.insert(id.clone(), parts(&id, pattern));
    }
    let table = json!({
        "about": format!(
            "The English text of each built-in validator rule's message, generated from \
             framework/src/localization/catalogs/en/validation.ftl by the agents test \
             wasm_messages. Do not edit it. After a change to the catalog, regenerate it with: \
             {REGENERATE}"
        ),
        "messages": messages,
    });
    let mut text = serde_json::to_string_pretty(&table).expect("the table serializes");
    text.push('\n');
    text
}

/// `pattern` as the table writes it. Any form the page tool's renderer does
/// not read fails the generation, so a new catalog form is taught to the
/// crate instead of being rendered wrong.
fn parts(id: &str, pattern: &Pattern<&str>) -> Value {
    Value::Array(
        pattern
            .elements
            .iter()
            .map(|element| match element {
                PatternElement::TextElement { value } => json!({ "text": value }),
                PatternElement::Placeable { expression } => match expression {
                    Expression::Inline(InlineExpression::VariableReference { id: variable }) => {
                        json!({ "arg": variable.name })
                    }
                    Expression::Select { selector, variants } => {
                        let InlineExpression::VariableReference { id: variable } = selector else {
                            panic!("`{id}` selects on something other than a variable");
                        };
                        // The server passes `$kind` as a string, so a
                        // variant matches by its name; a numeric selector
                        // would need Fluent's plural rules.
                        assert_eq!(
                            variable.name, "kind",
                            "`{id}` selects on `${}`; the page tool renderer selects on `$kind` only",
                            variable.name
                        );
                        let mut default = None;
                        let variants: Vec<Value> = variants
                            .iter()
                            .map(|variant| {
                                let VariantKey::Identifier { name } = &variant.key else {
                                    panic!("`{id}` has a numeric variant key");
                                };
                                if variant.default {
                                    default = Some(*name);
                                }
                                json!({ "key": name, "pattern": parts(id, &variant.value) })
                            })
                            .collect();
                        json!({
                            "select": variable.name,
                            "variants": variants,
                            "default": default.unwrap_or_else(|| panic!("`{id}` has no default variant")),
                        })
                    }
                    other => panic!("`{id}` uses {other:?}, which the page tool renderer does not read"),
                },
            })
            .collect(),
    )
}

#[test]
fn agent_016_page_tool_messages_match_the_english_catalog() {
    let generated = generate();
    if std::env::var("SUPRNOVA_WEBMCP_CATALOG").as_deref() == Ok("write") {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(TABLE_PATH);
        std::fs::write(&path, &generated).expect("the table is written");
        return;
    }
    assert!(
        generated == TABLE,
        "crates/suprnova-webmcp/src/catalog.json differs from the English catalog; regenerate it \
         with:\n{REGENERATE}\n\ngenerated:\n{generated}"
    );
}

/// The server's twins of the crate tests' input types: the same fields,
/// renames and rules, through the server's `#[tool_input]`.
mod twins {
    use suprnova::{Validate, tool_input};

    /// Signing up, with one rule of each kind the catalog words.
    #[allow(dead_code, reason = "only the validation messages are under test")]
    #[tool_input]
    #[serde(rename_all = "camelCase")]
    pub struct SignUp {
        /// The name to show.
        #[validate(length(min = 3, max = 20))]
        pub display_name: String,
        /// Age in years.
        #[validate(range(min = 18, max = 120))]
        pub age_years: u32,
        /// Where to write.
        #[validate(email)]
        pub contact_email: String,
        /// What friends call you.
        #[validate(length(min = 1, message = "Say what friends call you."))]
        pub nickname: String,
    }

    /// A line of an order.
    #[allow(dead_code, reason = "only the validation messages are under test")]
    #[tool_input]
    #[serde(rename_all = "camelCase")]
    pub struct OrderLine {
        /// The price of one, in cents.
        #[validate(range(min = 1))]
        pub unit_price: u32,
    }

    /// An order with a renamed nested list.
    #[allow(dead_code, reason = "only the validation messages are under test")]
    #[tool_input]
    #[serde(rename_all = "camelCase")]
    pub struct PlaceOrder {
        /// Who orders.
        #[validate(length(min = 1))]
        pub customer_name: String,
        /// What is ordered.
        #[serde(rename = "items")]
        #[validate(nested)]
        pub line_items: Vec<OrderLine>,
    }
}

/// The server's 422 `errors` for `T` read from `input`, keyed and worded as
/// a form request words them, in English from the embedded catalog.
async fn server_errors<T>(input: Value) -> Value
where
    T: serde::de::DeserializeOwned + Validate,
{
    let value: T = serde_json::from_value(input).expect("the input reads");
    let errors = value.validate().expect_err("the input breaks a rule");
    let keyed = ValidationErrors::from_validator_keyed(errors, input_key::<T>);
    TestContainer::scope(async move {
        let lang = tempfile::tempdir().expect("a temporary directory");
        let en = lang.path().join("en");
        std::fs::create_dir_all(&en).expect("the locale directory");
        std::fs::write(en.join("app.ftl"), "app-name = Fixture\n").expect("a catalog file");
        let config = LocalizationConfig {
            default_locale: Locale::parse("en").expect("en"),
            fallback_locale: Locale::parse("en").expect("en"),
            use_isolating: false,
            detection: vec![],
            session_key: "locale".into(),
            cookie_name: "locale".into(),
            parents: Default::default(),
        };
        let translator = FluentTranslator::from_dir(lang.path(), &config).expect("a translator");
        TestContainer::bind::<dyn Translator>(Arc::new(translator));
        scope_locale(Locale::parse("en").expect("en"), async move {
            keyed.to_json()["errors"].clone()
        })
        .await
    })
    .await
}

#[tokio::test]
async fn agent_016_the_server_words_the_crate_tests_messages() {
    // `crates/suprnova-webmcp/src/tests.rs` expects these texts from a page
    // tool; the server renders the same for the same type and input.
    let sign_up = server_errors::<twins::SignUp>(json!({
        "displayName": "Al",
        "ageYears": 12,
        "contactEmail": "not-an-address",
        "nickname": ""
    }))
    .await;
    assert_eq!(
        sign_up,
        json!({
            "displayName": ["The display name field must be between 3 and 20 characters."],
            "ageYears": ["The age years field must be between 18 and 120."],
            "contactEmail": ["The contact email field must be a valid email address."],
            "nickname": ["Say what friends call you."]
        }),
        "{sign_up:#}"
    );
    let order = server_errors::<twins::PlaceOrder>(json!({
        "customerName": "",
        "items": [{ "unitPrice": 5 }, { "unitPrice": 0 }]
    }))
    .await;
    assert_eq!(
        order,
        json!({
            "customerName": ["The customer name field must be at least 1 characters."],
            "items.1.unitPrice": ["The items.1.unit price field must be at least 1."]
        }),
        "{order:#}"
    );
}
