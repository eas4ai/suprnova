//! The registration a request type makes of its fields' input names, so
//! validation errors reported under Rust names can be keyed by the names
//! the client sent. The runtime half is `suprnova::data::input_names`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{GenericArgument, Ident, PathArguments, Type};

/// One field: its Rust name (what validation reports), the key the input
/// carries it under, and its type.
pub(crate) struct InputFieldSpec<'a> {
    pub(crate) rust: String,
    pub(crate) input: String,
    pub(crate) ty: &'a Type,
}

/// The type a field's nested validation errors continue with, and how many
/// key segments (list indexes, map keys) come before that type's fields.
/// `Option`, `Box`, `Field`, `Rc` and `Arc` add none; a list or a set adds
/// one index, a map one key.
fn peel(ty: &Type) -> (&Type, u8) {
    match ty {
        Type::Paren(inner) => peel(&inner.elem),
        Type::Group(inner) => peel(&inner.elem),
        Type::Array(array) => {
            let (inner, dynamic) = peel(&array.elem);
            (inner, dynamic.saturating_add(1))
        }
        Type::Slice(slice) => {
            let (inner, dynamic) = peel(&slice.elem);
            (inner, dynamic.saturating_add(1))
        }
        Type::Path(path) if path.qself.is_none() => {
            let Some(last) = path.path.segments.last() else {
                return (ty, 0);
            };
            let args: Vec<&Type> = match &last.arguments {
                PathArguments::AngleBracketed(bracketed) => bracketed
                    .args
                    .iter()
                    .filter_map(|arg| match arg {
                        GenericArgument::Type(ty) => Some(ty),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            match (last.ident.to_string().as_str(), args.as_slice()) {
                ("Option" | "Box" | "Field" | "Rc" | "Arc", [inner]) => peel(inner),
                (
                    "Vec" | "VecDeque" | "LinkedList" | "HashSet" | "BTreeSet" | "IndexSet",
                    [inner],
                ) => {
                    let (inner, dynamic) = peel(inner);
                    (inner, dynamic.saturating_add(1))
                }
                ("HashMap" | "BTreeMap" | "IndexMap", [_, value, ..]) => {
                    let (inner, dynamic) = peel(value);
                    (inner, dynamic.saturating_add(1))
                }
                _ => (ty, 0),
            }
        }
        _ => (ty, 0),
    }
}

/// The `inventory` registration of `struct_name`'s input names, keyed by
/// type name. Emit it only for a type with no generic parameters: the name
/// is taken at a fixed type.
pub(crate) fn registration(struct_name: &Ident, fields: &[InputFieldSpec<'_>]) -> TokenStream {
    let mut nested_fns = Vec::new();
    let mut entries = Vec::new();
    for (position, field) in fields.iter().enumerate() {
        let (inner, dynamic) = peel(field.ty);
        let nested_fn = format_ident!("__suprnova_input_nested_{}", position);
        let rust = &field.rust;
        let input = &field.input;
        nested_fns.push(quote! {
            fn #nested_fn() -> &'static str {
                ::core::any::type_name::<#inner>()
            }
        });
        entries.push(quote! {
            ::suprnova::data::input_names::InputField {
                rust: #rust,
                input: #input,
                nested: ::core::option::Option::Some(#nested_fn as fn() -> &'static str),
                dynamic_segments: #dynamic,
            }
        });
    }
    quote! {
        const _: () = {
            fn __suprnova_input_type() -> &'static str {
                ::core::any::type_name::<#struct_name>()
            }
            #(#nested_fns)*
            ::suprnova::inventory::submit! {
                ::suprnova::data::input_names::InputNames {
                    type_name: __suprnova_input_type,
                    fields: &[#(#entries),*],
                }
            }
        };
    }
}
