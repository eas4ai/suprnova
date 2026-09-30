//! The registration a request type makes of its fields' input names, so
//! validation errors reported under Rust names can be keyed by the names
//! the client sent. The runtime half is `suprnova::data::input_names`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, GenericArgument, Ident, PathArguments, Type};

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

/// The registration of a struct serde's own derive handles, reading only
/// the renames and skips of its serde attributes. `None` for a generic
/// struct or one without named fields.
pub(crate) fn lenient_registration(input: &DeriveInput) -> syn::Result<Option<TokenStream>> {
    let syn::Data::Struct(data) = &input.data else {
        return Ok(None);
    };
    let syn::Fields::Named(named) = &data.fields else {
        return Ok(None);
    };
    if !input.generics.params.is_empty() {
        return Ok(None);
    }
    let container = crate::serde_attrs::parse_container_lenient(&input.attrs)?;
    let mut fields = Vec::new();
    for field in &named.named {
        let names = crate::serde_attrs::field_names_lenient(field, &container)?;
        if names.skip_deserializing {
            continue;
        }
        fields.push(InputFieldSpec {
            rust: field
                .ident
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            input: names.deserialize,
            ty: &field.ty,
        });
    }
    Ok(Some(registration(&input.ident, &fields)))
}

/// `#[derive(InputNames)]`: the registration alone, for a plain
/// `#[derive(Deserialize, Validate)]` struct a request object nests, so
/// its validation errors are keyed by the names serde reads too.
pub fn derive_input_names(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    match lenient_registration(&input) {
        Ok(Some(registration)) => registration.into(),
        Ok(None) => syn::Error::new_spanned(
            &input.ident,
            "#[derive(InputNames)] needs a struct with named fields and no generic or lifetime parameters",
        )
        .to_compile_error()
        .into(),
        Err(e) => e.to_compile_error().into(),
    }
}
