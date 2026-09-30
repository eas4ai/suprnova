//! The names a request object's fields go by in the input the client sends.
//!
//! Validation reports an error under the field's Rust name: the `validator`
//! derive knows nothing of serde's renames. When a `#[derive(Data)]` or
//! `#[derive(FormRequest)]` struct renames its fields (`rename_all =
//! "camelCase"`), the client sent `unitPrice` and would get its error back
//! under `unit_price`. Each derive registers, per type, the Rust name and the
//! input key of every field, and the type an error key continues into when
//! the field holds a nested object. [`input_key`] walks a dotted error key
//! through that registry, so `items.1.unit_price` becomes `items.1.unitPrice`
//! and the error bag, Precognition's `Validate-Only` filter and the page all
//! use the names the input used.

use std::collections::HashMap;
use std::sync::OnceLock;

/// One type's fields, as its derive registers them. Types are keyed by
/// `std::any::type_name`, which needs no `'static` bound, so the
/// `FormRequest` trait's own extractor can look up any implementing type,
/// a hand-written impl included. Registration and lookup compute the name
/// in the same build.
#[doc(hidden)]
pub struct InputNames {
    /// The registering type's `std::any::type_name`.
    pub type_name: fn() -> &'static str,
    /// Its fields.
    pub fields: &'static [InputField],
}

/// One field of an [`InputNames`] entry.
#[doc(hidden)]
pub struct InputField {
    /// The name validation reports: the Rust field name.
    pub rust: &'static str,
    /// The key the input carries the field under.
    pub input: &'static str,
    /// The type whose fields a longer error key continues with, when the
    /// field holds an object.
    pub nested: Option<fn() -> &'static str>,
    /// The segments between the field and that type's fields: one for each
    /// list index or map key (`items.1.name` has one).
    pub dynamic_segments: u8,
}

inventory::collect!(InputNames);

fn registry() -> &'static HashMap<&'static str, &'static [InputField]> {
    static REGISTRY: OnceLock<HashMap<&'static str, &'static [InputField]>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        inventory::iter::<InputNames>
            .into_iter()
            .map(|entry| ((entry.type_name)(), entry.fields))
            .collect()
    })
}

/// `key`, a validation error key in `T`'s Rust field names, in the names the
/// input uses. A segment the registry does not know, and every segment
/// after it, is kept as it is: a key a hook already wrote in input names, a
/// struct-level `__all__`, or a field of a type that registered nothing.
#[doc(hidden)]
pub fn input_key<T: ?Sized>(key: &str) -> String {
    input_path(std::any::type_name::<T>(), key)
}

fn input_path(root: &'static str, key: &str) -> String {
    let registry = registry();
    let mut current = Some(root);
    let mut dynamic = 0_u8;
    key.split('.')
        .map(|segment| {
            if dynamic > 0 {
                dynamic -= 1;
                return segment.to_owned();
            }
            let field = current
                .and_then(|type_name| registry.get(type_name))
                .and_then(|fields| fields.iter().find(|field| field.rust == segment));
            match field {
                Some(field) => {
                    current = field.nested.map(|type_name| type_name());
                    dynamic = field.dynamic_segments;
                    field.input.to_owned()
                }
                None => {
                    current = None;
                    segment.to_owned()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Order;
    struct Line;

    fn line_name() -> &'static str {
        std::any::type_name::<Line>()
    }

    fn order_name() -> &'static str {
        std::any::type_name::<Order>()
    }

    inventory::submit! {
        InputNames {
            type_name: order_name,
            fields: &[
                InputField { rust: "customer_name", input: "customerName", nested: None, dynamic_segments: 0 },
                InputField { rust: "line_items", input: "lineItems", nested: Some(line_name as fn() -> &'static str), dynamic_segments: 1 },
                InputField { rust: "billing_line", input: "billingLine", nested: Some(line_name as fn() -> &'static str), dynamic_segments: 0 },
            ],
        }
    }

    inventory::submit! {
        InputNames {
            type_name: line_name,
            fields: &[InputField { rust: "unit_price", input: "unitPrice", nested: None, dynamic_segments: 0 }],
        }
    }

    #[test]
    fn a_key_takes_the_input_names_at_every_level() {
        assert_eq!(input_key::<Order>("customer_name"), "customerName");
        assert_eq!(input_key::<Order>("line_items"), "lineItems");
        assert_eq!(
            input_key::<Order>("line_items.3.unit_price"),
            "lineItems.3.unitPrice"
        );
        assert_eq!(
            input_key::<Order>("billing_line.unit_price"),
            "billingLine.unitPrice"
        );
    }

    #[test]
    fn an_index_that_matches_a_field_name_stays_an_index() {
        // The segment after `line_items` is its index, whatever it spells.
        assert_eq!(
            input_key::<Order>("line_items.unit_price.unit_price"),
            "lineItems.unit_price.unitPrice"
        );
    }

    #[test]
    fn unknown_segments_and_types_pass_through() {
        assert_eq!(input_key::<Order>("__all__"), "__all__");
        assert_eq!(input_key::<Order>("customerName"), "customerName");
        assert_eq!(input_key::<Order>("other.unit_price"), "other.unit_price");
        assert_eq!(input_key::<String>("customer_name"), "customer_name");
    }
}
