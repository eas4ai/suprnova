//! The names a page tool input's fields go by in the input the agent sends.
//!
//! `validator` reports an error under the field's Rust name; it knows
//! nothing of serde's renames. The server's form request rekeys its errors
//! by the names the client sent (`framework/src/data/input_names.rs`), so a
//! page tool does the same: `#[tool_input]` generates each input type's
//! table, and [`input_key`] walks an error key through it, so
//! `line_items.1.unit_price` becomes `items.1.unitPrice` as it would on the
//! server.

/// The field table of a page tool input, which `#[tool_input]` generates.
///
/// A nested input type must carry the attribute too, so its fields are
/// named as the client sends them at every level.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is nested in a page tool input but has no `#[suprnova_webmcp::tool_input]`",
    label = "the type of a `#[validate(nested)]` field",
    note = "add `#[suprnova_webmcp::tool_input]` to `{Self}`, so its validation errors use the \
            names the client sends"
)]
pub trait InputNames {
    /// The field whose Rust name is `rust`, or `None` for a key the table
    /// does not hold.
    fn input_field(rust: &str) -> Option<InputField>;
}

/// One field of an input type.
#[derive(Debug, Clone, Copy)]
pub struct InputField {
    /// The key the client sends the field under.
    pub input: &'static str,
    /// The table a longer error key continues with, when the field holds a
    /// nested input.
    pub nested: Option<fn(&str) -> Option<InputField>>,
    /// The segments between the field and the nested type's fields: one
    /// for each list index or map key (`items.1.name` has one).
    pub dynamic_segments: u8,
}

/// `key`, a validation error key in `T`'s Rust field names, in the names
/// the client sends. A segment the table does not hold, and every segment
/// after it, stays as it is, as on the server.
pub(crate) fn input_key<T: InputNames>(key: &str) -> String {
    let mut table: Option<fn(&str) -> Option<InputField>> = Some(T::input_field);
    let mut dynamic = 0_u8;
    key.split('.')
        .map(|segment| {
            if dynamic > 0 {
                dynamic -= 1;
                return segment.to_owned();
            }
            match table.and_then(|lookup| lookup(segment)) {
                Some(field) => {
                    table = field.nested;
                    dynamic = field.dynamic_segments;
                    field.input.to_owned()
                }
                None => {
                    table = None;
                    segment.to_owned()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}
