//! Checking a tool call's arguments against the tool's input schema before
//! the route reads them (AGENT-003).
//!
//! serde stops at the first field it cannot read, and invalid input must
//! come back naming each field. So before the call reaches the route, the
//! arguments are checked against the schema `schemars` derived for the
//! input type, and the failures are reported at once: a missing required
//! field, a value of the wrong type, and a field the schema does not allow
//! (`additionalProperties: false`), at the top level and in nested objects
//! and arrays. The route's form request still runs serde and the
//! `validator` rules afterwards, as it does for every request.
//!
//! The check is bounded, because the caller chooses the input:
//!
//! - It visits at most [`STEP_BUDGET`] schema nodes in all. A schema that
//!   branches (`anyOf`, `oneOf`) can otherwise cost work exponential in the
//!   input's depth. When the budget runs out, the check reports nothing and
//!   serde and the `validator` rules decide alone, as they would without
//!   it.
//! - It follows nesting and `$ref`s at most [`MAX_DEPTH`] deep.
//! - It reports at most [`MAX_REPORTED`] failures and counts the rest; a
//!   field's path is built only for a reported failure, is cut to
//!   [`MAX_PATH_CHARS`] characters, and has every control character
//!   escaped, so a key the caller chose cannot inflate the answer or forge
//!   a line of it.
//!
//! This covers the subset of JSON Schema draft 2020-12 that `schemars`
//! generates for Rust types: `type` (one or a list), `properties`,
//! `required`, `additionalProperties`, `items`, local `$ref`s into `$defs`,
//! and `anyOf`, `oneOf` and `allOf`. It is written here rather than taken
//! from a validator crate because it needs only that subset, it must name
//! fields in the form the validation results use, it must stay bounded, and
//! it must never fetch a remote `$ref`.

use serde_json::{Map, Value};

/// How deep nested objects, arrays and `$ref`s are followed; a deeper part
/// of the input is not checked, and the route's own validation decides.
const MAX_DEPTH: usize = 32;

/// How many schema nodes one check may visit in all, branches tried and
/// thrown away included.
pub(crate) const STEP_BUDGET: usize = 10_000;

/// How many failures a report names; the rest are counted.
pub(crate) const MAX_REPORTED: usize = 20;

/// The longest field path a report shows, in characters, before it is cut
/// with an ellipsis.
pub(crate) const MAX_PATH_CHARS: usize = 200;

/// One field of the input that breaks the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputFailure {
    /// The field's path: `title`, `address.city`, `tags[2]`, cut and
    /// escaped for the report.
    pub(crate) field: String,
    /// What is wrong with it.
    pub(crate) message: &'static str,
}

/// What a check found.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct InputReport {
    /// The first [`MAX_REPORTED`] failures.
    pub(crate) failures: Vec<InputFailure>,
    /// How many failures there were beyond those.
    pub(crate) more: usize,
}

impl InputReport {
    /// Whether the input broke nothing the check looked at.
    pub(crate) fn is_empty(&self) -> bool {
        self.failures.is_empty() && self.more == 0
    }
}

/// The failures of `arguments` against `schema`. Empty when the check ran
/// out of its step budget: serde and the route's validation decide then.
pub(crate) fn check(schema: &Value, arguments: &Map<String, Value>) -> InputReport {
    let mut checker = Checker {
        root: schema,
        steps: STEP_BUDGET,
        exhausted: false,
    };
    let mut report = Collected::default();
    checker.check_object_root(arguments, &mut report);
    if checker.exhausted {
        return InputReport::default();
    }
    report.into_report()
}

/// Where a value sits in the input, rendered only for a failure.
#[derive(Clone, Copy)]
enum Path<'a> {
    Root,
    Key(&'a Path<'a>, &'a str),
    Index(&'a Path<'a>, usize),
}

impl Path<'_> {
    /// The path as a report shows it: dotted keys and `[index]`, control
    /// characters escaped, cut to [`MAX_PATH_CHARS`] characters.
    fn render(&self) -> String {
        let mut parts = Vec::new();
        let mut at = self;
        loop {
            match at {
                Path::Root => break,
                Path::Key(parent, key) => {
                    parts.push(Part::Key(key));
                    at = parent;
                }
                Path::Index(parent, index) => {
                    parts.push(Part::Index(*index));
                    at = parent;
                }
            }
        }
        if parts.is_empty() {
            return "input".to_string();
        }
        let mut out = String::new();
        let mut chars = 0usize;
        let mut push = |out: &mut String, text: &str| -> bool {
            for c in text.chars() {
                if chars >= MAX_PATH_CHARS {
                    return false;
                }
                out.push(c);
                chars += 1;
            }
            true
        };
        for (position, part) in parts.iter().rev().enumerate() {
            let fits = match part {
                Part::Key(key) => {
                    let dot = position > 0 && push(&mut out, ".");
                    (position == 0 || dot) && push(&mut out, &escape(key))
                }
                Part::Index(index) => push(&mut out, &format!("[{index}]")),
            };
            if !fits {
                out.push_str("...");
                break;
            }
        }
        out
    }
}

enum Part<'a> {
    Key(&'a str),
    Index(usize),
}

/// `key` with every control character written as an escape, so a key
/// cannot end the line of the report it appears in.
fn escape(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for c in key.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

/// The failures found so far: the first ones rendered, the rest counted.
#[derive(Default)]
struct Collected {
    failures: Vec<InputFailure>,
    more: usize,
}

impl Collected {
    fn push(&mut self, path: &Path<'_>, message: &'static str) {
        if self.failures.len() < MAX_REPORTED {
            self.failures.push(InputFailure {
                field: path.render(),
                message,
            });
        } else {
            self.more = self.more.saturating_add(1);
        }
    }

    fn total(&self) -> usize {
        self.failures.len().saturating_add(self.more)
    }

    fn into_report(self) -> InputReport {
        InputReport {
            failures: self.failures,
            more: self.more,
        }
    }
}

/// One check's state: the schema's root for `$ref`s and the steps left.
struct Checker<'s> {
    root: &'s Value,
    steps: usize,
    exhausted: bool,
}

impl<'s> Checker<'s> {
    fn check_object_root(&mut self, arguments: &Map<String, Value>, report: &mut Collected) {
        let root = self.root;
        let value = Value::Object(arguments.clone());
        self.check_value(root, &value, &Path::Root, 0, report);
    }

    /// Spend one step; `false`, and the check marked exhausted, when none
    /// is left.
    fn step(&mut self) -> bool {
        if self.steps == 0 {
            self.exhausted = true;
            return false;
        }
        self.steps -= 1;
        true
    }

    fn check_value(
        &mut self,
        schema: &'s Value,
        value: &Value,
        path: &Path<'_>,
        depth: usize,
        report: &mut Collected,
    ) {
        if self.exhausted || depth > MAX_DEPTH || !self.step() {
            return;
        }
        let Some(schema) = resolve(self.root, schema) else {
            return;
        };
        let Some(object) = schema.as_object() else {
            // `true` and `{}` allow anything; `false` allows nothing, which
            // `schemars` does not generate for an input.
            return;
        };
        for key in ["anyOf", "oneOf"] {
            if let Some(Value::Array(branches)) = object.get(key) {
                let mut fits = false;
                for branch in branches {
                    let mut trial = Collected::default();
                    self.check_value(branch, value, path, depth + 1, &mut trial);
                    if self.exhausted {
                        return;
                    }
                    if trial.total() == 0 {
                        fits = true;
                        break;
                    }
                }
                if !fits {
                    report.push(path, "does not match any of the allowed shapes");
                    return;
                }
            }
        }
        if let Some(Value::Array(parts)) = object.get("allOf") {
            for part in parts {
                self.check_value(part, value, path, depth + 1, report);
            }
        }
        if let Some(expected) = object.get("type")
            && !type_matches(expected, value)
        {
            report.push(path, type_message(expected));
            return;
        }
        match value {
            Value::Object(fields) => {
                if let Some(Value::Array(required)) = object.get("required") {
                    for name in required.iter().filter_map(Value::as_str) {
                        if !fields.contains_key(name) {
                            report.push(&Path::Key(path, name), "is required");
                        }
                    }
                }
                let properties = object.get("properties").and_then(Value::as_object);
                for (name, field) in fields {
                    if self.exhausted {
                        return;
                    }
                    let at = Path::Key(path, name);
                    match properties.and_then(|properties| properties.get(name)) {
                        Some(field_schema) => {
                            self.check_value(field_schema, field, &at, depth + 1, report);
                        }
                        None => match object.get("additionalProperties") {
                            Some(Value::Bool(false)) => report.push(&at, "is not allowed"),
                            Some(extra) if extra.is_object() => {
                                self.check_value(extra, field, &at, depth + 1, report);
                            }
                            _ => {}
                        },
                    }
                }
            }
            Value::Array(items) => {
                if let Some(item_schema) = object.get("items").filter(|items| items.is_object()) {
                    for (index, item) in items.iter().enumerate() {
                        if self.exhausted {
                            return;
                        }
                        self.check_value(
                            item_schema,
                            item,
                            &Path::Index(path, index),
                            depth + 1,
                            report,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// `schema`, or the definition its local `$ref` names. `None` for a `$ref`
/// that is not a local one into `$defs` or `definitions`: nothing is ever
/// fetched.
fn resolve<'a>(root: &'a Value, schema: &'a Value) -> Option<&'a Value> {
    let Some(reference) = schema.get("$ref").and_then(Value::as_str) else {
        return Some(schema);
    };
    let name = reference
        .strip_prefix("#/$defs/")
        .or_else(|| reference.strip_prefix("#/definitions/"))?;
    root.get("$defs")
        .or_else(|| root.get("definitions"))
        .and_then(|definitions| definitions.get(name))
}

/// Whether `value` is of a type `expected` names: one type, or a list.
fn type_matches(expected: &Value, value: &Value) -> bool {
    match expected {
        Value::String(name) => is_type(name, value),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .any(|name| is_type(name, value)),
        _ => true,
    }
}

/// Whether `value` is of the JSON Schema type `name`.
fn is_type(name: &str, value: &Value) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        _ => true,
    }
}

/// The message for a value that is not of the type `expected` names. A
/// fixed text for each case `schemars` generates, so a report allocates
/// nothing for it.
fn type_message(expected: &Value) -> &'static str {
    let names: Vec<&str> = match expected {
        Value::String(name) => vec![name.as_str()],
        Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let nullable = names.contains(&"null");
    let main = names.iter().find(|name| **name != "null").copied();
    match (main, nullable) {
        (Some("object"), false) => "must be an object",
        (Some("object"), true) => "must be an object or null",
        (Some("array"), false) => "must be an array",
        (Some("array"), true) => "must be an array or null",
        (Some("string"), false) => "must be a string",
        (Some("string"), true) => "must be a string or null",
        (Some("integer"), false) => "must be an integer",
        (Some("integer"), true) => "must be an integer or null",
        (Some("number"), false) => "must be a number",
        (Some("number"), true) => "must be a number or null",
        (Some("boolean"), false) => "must be a boolean",
        (Some("boolean"), true) => "must be a boolean or null",
        (None, true) => "must be null",
        _ => "has the wrong type",
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_PATH_CHARS, MAX_REPORTED, check};
    use serde_json::{Value, json};

    fn args(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer", "minimum": 0 },
                "nickname": { "type": ["string", "null"] },
                "address": { "$ref": "#/$defs/Address" },
                "tags": { "type": "array", "items": { "type": "string" } },
            },
            "required": ["name", "age", "address"],
            "additionalProperties": false,
            "$defs": {
                "Address": {
                    "type": "object",
                    "properties": { "city": { "type": "string" } },
                    "required": ["city"],
                },
            },
        })
    }

    #[test]
    fn agent_003_every_failing_field_is_named() {
        let report = check(
            &schema(),
            &args(json!({ "age": 1.5, "address": {}, "tags": ["a", 2], "extra": true })),
        );
        let fields: Vec<&str> = report
            .failures
            .iter()
            .map(|failure| failure.field.as_str())
            .collect();
        assert_eq!(
            fields,
            vec!["name", "age", "address.city", "tags[1]", "extra"],
            "{report:?}"
        );
        assert_eq!(report.more, 0);
    }

    #[test]
    fn agent_003_valid_input_passes_and_null_fits_an_optional_field() {
        let report = check(
            &schema(),
            &args(json!({
                "name": "Ada",
                "age": 36,
                "nickname": null,
                "address": { "city": "London" },
            })),
        );
        assert!(report.is_empty(), "{report:?}");
    }

    #[test]
    fn agent_003_a_remote_ref_is_never_followed() {
        let schema = json!({
            "type": "object",
            "properties": { "x": { "$ref": "https://example.test/schema.json" } },
        });
        assert!(check(&schema, &args(json!({ "x": 1 }))).is_empty());
    }

    /// A recursive schema whose four branches all accept the same field.
    fn branching() -> Value {
        let variant = |op: &str| {
            json!({
                "type": "object",
                "properties": { "op": { "type": "string", "const": op }, "arg": { "$ref": "#/$defs/E" } },
                "required": ["op", "arg"],
            })
        };
        json!({
            "type": "object",
            "properties": { "expr": { "$ref": "#/$defs/E" } },
            "required": ["expr"],
            "$defs": { "E": { "oneOf": [variant("a"), variant("b"), variant("c"), variant("d")] } },
        })
    }

    #[test]
    fn agent_003_a_branching_schema_stops_at_the_step_budget() {
        let mut expr = json!({ "op": "a" });
        for _ in 0..20 {
            expr = json!({ "op": "a", "arg": expr });
        }
        let started = std::time::Instant::now();
        let report = check(&branching(), &args(json!({ "expr": expr })));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "took {:?}",
            started.elapsed()
        );
        assert!(
            report.is_empty(),
            "out of budget, serde decides: {report:?}"
        );
    }

    #[test]
    fn agent_003_reports_are_capped_cut_and_escaped() {
        let schema = json!({
            "type": "object",
            "properties": {
                "groups": {
                    "type": "object",
                    "additionalProperties": { "type": "array", "items": { "type": "integer" } },
                },
            },
        });
        let key = format!("{}\n- forged{}", "k".repeat(300), "k".repeat(300));
        let values = vec![json!("x"); 100];
        let report = check(&schema, &args(json!({ "groups": { key: values } })));
        assert_eq!(report.failures.len(), MAX_REPORTED);
        assert_eq!(report.more, 100 - MAX_REPORTED);
        for failure in &report.failures {
            assert!(!failure.field.contains('\n'), "{}", failure.field);
            assert!(
                failure.field.chars().count() <= MAX_PATH_CHARS + 3,
                "{}",
                failure.field
            );
            assert!(failure.field.ends_with("..."), "{}", failure.field);
            assert!(failure.field.starts_with("groups."), "{}", failure.field);
        }
    }
}
