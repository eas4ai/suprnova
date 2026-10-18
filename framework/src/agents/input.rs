//! Checking a tool call's arguments against the tool's input schema before
//! the route reads them (AGENT-003).
//!
//! serde stops at the first field it cannot read, and invalid input must
//! come back naming each field. So before the call reaches the route, the
//! arguments are checked against the schema `schemars` derived for the
//! input type, and every failure is reported at once: a missing required
//! field, a value of the wrong type, and a field the schema does not allow
//! (`additionalProperties: false`), at the top level and in nested objects
//! and arrays. The route's form request still runs serde and the
//! `validator` rules afterwards, as it does for every request.
//!
//! This covers the subset of JSON Schema draft 2020-12 that `schemars`
//! generates for Rust types: `type` (one or a list), `properties`,
//! `required`, `additionalProperties`, `items`, local `$ref`s into `$defs`,
//! and `anyOf`, `oneOf` and `allOf`. It is written here rather than taken
//! from a validator crate because it needs only that subset, it must name
//! fields in the form the validation results use, and it must never fetch a
//! remote `$ref`.

use serde_json::{Map, Value};

/// How deep nested objects, arrays and `$ref`s are followed; a deeper
/// schema is not checked further, and the route's own validation decides.
const MAX_DEPTH: usize = 32;

/// One field of the input that breaks the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputFailure {
    /// The field's path: `title`, `address.city`, `tags[2]`.
    pub(crate) field: String,
    /// What is wrong with it.
    pub(crate) message: String,
}

/// Every field of `arguments` that breaks `schema`.
pub(crate) fn check(schema: &Value, arguments: &Map<String, Value>) -> Vec<InputFailure> {
    let mut failures = Vec::new();
    check_value(
        schema,
        schema,
        &Value::Object(arguments.clone()),
        "",
        0,
        &mut failures,
    );
    failures
}

fn check_value(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
    depth: usize,
    failures: &mut Vec<InputFailure>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(schema) = resolve(root, schema) else {
        return;
    };
    let Some(object) = schema.as_object() else {
        // `true` and `{}` allow anything; `false` allows nothing, which
        // `schemars` does not generate for an input.
        return;
    };
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = object.get(key) {
            let fits = branches.iter().any(|branch| {
                let mut trial = Vec::new();
                check_value(root, branch, value, path, depth + 1, &mut trial);
                trial.is_empty()
            });
            if !fits {
                failures.push(failure(path, "does not match any of the allowed shapes"));
                return;
            }
        }
    }
    if let Some(Value::Array(parts)) = object.get("allOf") {
        for part in parts {
            check_value(root, part, value, path, depth + 1, failures);
        }
    }
    if let Some(expected) = object.get("type")
        && !type_matches(expected, value)
    {
        failures.push(failure(
            path,
            &format!("must be {}", describe_type(expected)),
        ));
        return;
    }
    match value {
        Value::Object(fields) => {
            if let Some(Value::Array(required)) = object.get("required") {
                for name in required.iter().filter_map(Value::as_str) {
                    if !fields.contains_key(name) {
                        failures.push(failure(&join(path, name), "is required"));
                    }
                }
            }
            let properties = object.get("properties").and_then(Value::as_object);
            for (name, field) in fields {
                match properties.and_then(|properties| properties.get(name)) {
                    Some(field_schema) => check_value(
                        root,
                        field_schema,
                        field,
                        &join(path, name),
                        depth + 1,
                        failures,
                    ),
                    None => match object.get("additionalProperties") {
                        Some(Value::Bool(false)) => {
                            failures.push(failure(&join(path, name), "is not allowed"));
                        }
                        Some(extra) if extra.is_object() => {
                            check_value(root, extra, field, &join(path, name), depth + 1, failures);
                        }
                        _ => {}
                    },
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = object.get("items").filter(|items| items.is_object()) {
                for (index, item) in items.iter().enumerate() {
                    check_value(
                        root,
                        item_schema,
                        item,
                        &format!("{path}[{index}]"),
                        depth + 1,
                        failures,
                    );
                }
            }
        }
        _ => {}
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

/// `expected` in words: `a string`, `an integer or null`.
fn describe_type(expected: &Value) -> String {
    let names: Vec<&str> = match expected {
        Value::String(name) => vec![name.as_str()],
        Value::Array(names) => names.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    names
        .iter()
        .map(|name| match *name {
            "object" | "array" | "integer" => format!("an {name}"),
            "null" => "null".to_string(),
            other => format!("a {other}"),
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

/// The path of `name` under `path`.
fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}.{name}")
    }
}

fn failure(path: &str, message: &str) -> InputFailure {
    InputFailure {
        field: if path.is_empty() {
            "input".to_string()
        } else {
            path.to_string()
        },
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::check;
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
        let failures = check(
            &schema(),
            &args(json!({ "age": 1.5, "address": {}, "tags": ["a", 2], "extra": true })),
        );
        let fields: Vec<&str> = failures
            .iter()
            .map(|failure| failure.field.as_str())
            .collect();
        assert_eq!(
            fields,
            vec!["name", "age", "address.city", "tags[1]", "extra"],
            "{failures:?}"
        );
    }

    #[test]
    fn agent_003_valid_input_passes_and_null_fits_an_optional_field() {
        let failures = check(
            &schema(),
            &args(json!({
                "name": "Ada",
                "age": 36,
                "nickname": null,
                "address": { "city": "London" },
            })),
        );
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn agent_003_a_remote_ref_is_never_followed() {
        let schema = json!({
            "type": "object",
            "properties": { "x": { "$ref": "https://example.test/schema.json" } },
        });
        assert!(check(&schema, &args(json!({ "x": 1 }))).is_empty());
    }
}
