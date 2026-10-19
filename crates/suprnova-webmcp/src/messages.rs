//! The English text of a page tool's validation messages, worded as the
//! server words them (AGENT-016).
//!
//! The server renders a `validator` failure from its English catalog under
//! the id `validation-<code>`, with the rule's bounds as arguments and the
//! field's client name humanized as Laravel's displayable attribute
//! (`unitPrice` reads `unit price`), and falls back to
//! `Validation failed for field '<name>'` for a rule the catalog does not word
//! (`validator_error_message` and `render_validation_message` in
//! `framework/src/error.rs`). A page cannot read that catalog, so
//! `catalog.json` carries the entries of `validator`'s built-in rules,
//! generated from it. The framework's `wasm_messages` test regenerates the
//! table and fails when the two differ.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

/// The generated table.
const TABLE: &str = include_str!("catalog.json");

/// The table: each catalog id with its pattern.
#[derive(Deserialize)]
struct Table {
    messages: HashMap<String, Vec<Part>>,
}

/// One piece of a pattern, as Fluent resolves it.
#[derive(Deserialize)]
#[serde(untagged)]
enum Part {
    /// Text as it is.
    Text { text: String },
    /// An argument's text.
    Arg { arg: String },
    /// The variant whose key equals an argument, else the default one.
    Select {
        select: String,
        variants: Vec<Variant>,
        default: String,
    },
}

/// One variant of a selection.
#[derive(Deserialize)]
struct Variant {
    key: String,
    pattern: Vec<Part>,
}

/// The table, read once. `None` only if the checked-in file does not
/// parse, and then every message takes the server's fallback text.
fn table() -> Option<&'static Table> {
    static READ: OnceLock<Option<Table>> = OnceLock::new();
    READ.get_or_init(|| serde_json::from_str(TABLE).ok())
        .as_ref()
}

/// The message for `error` on the field the client sends as `field`.
pub(crate) fn message(field: &str, error: &validator::ValidationError) -> String {
    // The author's own message wins, untranslated, as on the server.
    if let Some(text) = &error.message {
        return text.to_string();
    }
    let fallback = || format!("Validation failed for field '{field}'");
    let id = format!("validation-{}", error.code.replace('_', "-"));
    let Some(pattern) = table().and_then(|table| table.messages.get(&id)) else {
        return fallback();
    };
    let mut args: HashMap<&str, String> = HashMap::new();
    for (name, value) in &error.params {
        // `value` echoes the input and `other` is a sibling field's value,
        // which the server never puts in a message either.
        if name == "value" || name == "other" {
            continue;
        }
        args.insert(name.as_ref(), argument(value));
    }
    args.insert("kind", kind(error).to_string());
    args.insert("field", humanized(field));
    let mut out = String::new();
    // Fluent reports a missing argument as an error, and the server then
    // gives its fallback text.
    if render(pattern, &args, &mut out) {
        out
    } else {
        fallback()
    }
}

/// Which bounds a `length` or `range` failure carries, the server's
/// `$kind`, so one message words each case.
fn kind(error: &validator::ValidationError) -> &'static str {
    let has = |name: &str| error.params.contains_key(name);
    if has("equal") {
        "equal"
    } else if has("min") && has("max") {
        "range"
    } else if has("min") {
        "min"
    } else if has("max") {
        "max"
    } else {
        "other"
    }
}

/// `value` as Fluent prints the argument the server passes for it.
fn argument(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => fluent_number(&number.to_string()),
        other => other.to_string(),
    }
}

/// A number as Fluent prints one read from `text`: the value's own digits,
/// padded to as many fraction digits as `text` has (`40.0` stays `40.0`).
/// Text that is not a number prints as it is.
fn fluent_number(text: &str) -> String {
    let Ok(value) = text.parse::<f64>() else {
        return text.to_string();
    };
    let mut out = value.to_string();
    if let Some(dot) = text.find('.') {
        let wanted = text.len() - dot - 1;
        match out.find('.') {
            Some(at) => {
                let have = out.len() - at - 1;
                out.push_str(&"0".repeat(wanted.saturating_sub(have)));
            }
            None => {
                out.push('.');
                out.push_str(&"0".repeat(wanted));
            }
        }
    }
    out
}

/// `field` as the server shows it in a message: snake-cased, each `_` a
/// space, nested segments included (`items.1.unitPrice` reads
/// `items.1.unit price`).
fn humanized(field: &str) -> String {
    let mut snake = String::with_capacity(field.len() + 4);
    for (position, c) in field.chars().enumerate() {
        if c.is_uppercase() {
            if position > 0 {
                snake.push('_');
            }
            snake.extend(c.to_lowercase());
        } else {
            snake.push(c);
        }
    }
    snake.replace('_', " ")
}

/// Write `pattern` with `args` into `out`; `false` when it needs an
/// argument it was not given.
fn render(pattern: &[Part], args: &HashMap<&str, String>, out: &mut String) -> bool {
    for part in pattern {
        match part {
            Part::Text { text } => out.push_str(text),
            Part::Arg { arg } => match args.get(arg.as_str()) {
                Some(value) => out.push_str(value),
                None => return false,
            },
            Part::Select {
                select,
                variants,
                default,
            } => {
                let Some(value) = args.get(select.as_str()) else {
                    return false;
                };
                let chosen = variants
                    .iter()
                    .find(|variant| &variant.key == value)
                    .or_else(|| variants.iter().find(|variant| &variant.key == default));
                match chosen {
                    Some(variant) if render(&variant.pattern, args, out) => {}
                    _ => return false,
                }
            }
        }
    }
    true
}
