//! Model serialization shares one policy across serde, arrays and JSON.
//! Unfiltered attributes remain available to persistence without recursive serialization.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use super::parse::ModelInput;

/// Emit direct column access so collections read identity without visibility filters.
pub fn emit_field_value(idents: &[Ident]) -> TokenStream {
    let arms = idents.iter().map(|ident| {
        let name = ident.to_string();
        quote! { #name => ::suprnova::serde_json::to_value(&self.#ident).ok(), }
    });
    quote! {
        fn field_value(&self, name: &str) -> ::core::option::Option<::suprnova::serde_json::Value> {
            match name { #(#arms)* _ => ::core::option::Option::None }
        }
    }
}

// Read serde options so the view excludes skipped fields and preserves
// the argument types of custom serializers.
fn has_serde_option(field: &syn::Field, names: &[&str]) -> bool {
    field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
        .any(|attr| {
            attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|items| {
                items
                    .iter()
                    .any(|item| names.iter().any(|name| item.path().is_ident(*name)))
            })
        })
}

/// Emit a borrowed serde view so model output honors field attributes without recursion.
/// The view retains serde renames and custom serializers but excludes runtime state.
pub fn emit_serialize(input: &ModelInput) -> TokenStream {
    let ident = &input.item.ident;
    let attrs = input
        .item
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"));
    let fields: Vec<_> = input
        .item
        .fields
        .iter()
        .filter(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|name| name != "__eager" && name != "__pivot")
                && !has_serde_option(field, &["skip", "skip_serializing"])
        })
        .map(|field| (field, has_serde_option(field, &["serialize_with", "with"])))
        .collect();
    let lifetime = fields
        .iter()
        .any(|(_, custom)| !custom)
        .then(|| quote! { <'a> });
    let declarations = fields.iter().map(|(field, custom)| {
        let ident = &field.ident;
        let ty = &field.ty;
        let attrs = field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("serde"));
        let ty = if *custom {
            quote! { #ty }
        } else {
            quote! { &'a #ty }
        };
        quote! { #(#attrs)* #ident: #ty }
    });
    let values = fields.iter().map(|(field, custom)| {
        let ident = &field.ident;
        if *custom {
            // Generic callbacks expect the original field type, not a reference type.
            quote! { #ident: ::core::clone::Clone::clone(&self.#ident) }
        } else {
            quote! { #ident: &self.#ident }
        }
    });
    quote! {
        impl #ident {
            fn __suprnova_attributes(&self) -> ::core::result::Result<
                ::suprnova::serde_json::Value, ::suprnova::serde_json::Error,
            > {
                #[derive(::suprnova::serde::Serialize)]
                #(#attrs)*
                struct __SuprnovaAttributes #lifetime { #(#declarations,)* }
                ::suprnova::serde_json::to_value(__SuprnovaAttributes { #(#values,)* })
            }
        }

        impl ::suprnova::serde::Serialize for #ident {
            fn serialize<S>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error>
            where S: ::suprnova::serde::Serializer {
                let value = <Self as ::suprnova::Model>::__serialization_value(self)
                    .map_err(<S::Error as ::suprnova::serde::ser::Error>::custom)?;
                ::suprnova::serde::Serialize::serialize(&value, serializer)
            }
        }
    }
}

/// The deserializer serde's derive calls for `field`: its `deserialize_with`
/// path, or `<module>::deserialize` for `with = "module"`. A factory attribute
/// is written in the field's serialized form, so it must be read back the way
/// serde reads the field, not through the field type's own `Deserialize`.
fn field_deserializer(field: &syn::Field) -> Option<syn::LitStr> {
    let mut found = None;
    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
    {
        let Ok(items) = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        ) else {
            continue;
        };
        for item in items {
            let syn::Meta::NameValue(pair) = item else {
                continue;
            };
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(path),
                ..
            }) = &pair.value
            else {
                continue;
            };
            if pair.path.is_ident("deserialize_with") {
                found = Some(path.clone());
            } else if pair.path.is_ident("with") {
                found = Some(syn::LitStr::new(
                    &format!("{}::deserialize", path.value()),
                    path.span(),
                ));
            }
        }
    }
    found
}

/// Emit `Persistable::with_definition_attributes`, which sets each field a
/// factory attribute set names on a model the definition already built.
///
/// Every column is a candidate, a field serde skips on output included, so
/// such a field can take an attribute and keeps its built value when the set
/// names other fields. Rebuilding the whole model through serde would lose it:
/// the model's output leaves it out. A field is named as serde names it on
/// output (its `rename`, else the container's `rename_all`, else the field
/// name), the keys the model's own JSON carries.
pub fn emit_definition_attributes(input: &ModelInput) -> syn::Result<TokenStream> {
    let struct_ident = &input.item.ident;
    let container = crate::serde_attrs::parse_container_lenient(&input.item.attrs)?;
    let mut arms: Vec<(String, TokenStream)> = Vec::new();
    for field in &input.item.fields {
        let Some(ident) = field.ident.as_ref() else {
            continue;
        };
        if ident == "__eager" || ident == "__pivot" {
            continue;
        }
        let name = crate::serde_attrs::field_names_lenient(field, &container)?.serialize;
        let ty = &field.ty;
        let decode = match field_deserializer(field) {
            Some(path) => quote! {{
                #[derive(::suprnova::serde::Deserialize)]
                #[serde(crate = "::suprnova::serde")]
                struct __SuprnovaFieldValue(#[serde(deserialize_with = #path)] #ty);
                ::suprnova::serde_json::from_value::<__SuprnovaFieldValue>(__suprnova_value)
                    .map(|__SuprnovaFieldValue(__suprnova_inner)| __suprnova_inner)
            }},
            None => quote! { ::suprnova::serde_json::from_value::<#ty>(__suprnova_value) },
        };
        let arm = quote! {
            #name => {
                self.#ident = #decode.map_err(|__suprnova_error| {
                    ::suprnova::FrameworkError::bad_request(::std::format!(
                        "factory attribute `{}`: {}",
                        #name,
                        __suprnova_error,
                    ))
                })?;
            }
        };
        // Two fields serialized under one name would emit an unreachable arm.
        // The later field takes the name, as its value is the one the model's
        // JSON keeps under that key.
        match arms.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, existing)) => *existing = arm,
            None => arms.push((name, arm)),
        }
    }
    let arms = arms.into_iter().map(|(_, arm)| arm);
    Ok(quote! {
        fn with_definition_attributes(
            mut self,
            attributes: ::suprnova::eloquent::Attrs,
        ) -> ::core::result::Result<Self, ::suprnova::FrameworkError> {
            for (__suprnova_name, __suprnova_value) in attributes.0 {
                match __suprnova_name.as_str() {
                    #(#arms)*
                    _ => {
                        return ::core::result::Result::Err(
                            ::suprnova::FrameworkError::bad_request(::std::format!(
                                "factory attribute `{}` is not a serialized field of {}",
                                __suprnova_name,
                                ::core::stringify!(#struct_ident),
                            )),
                        );
                    }
                }
            }
            ::core::result::Result::Ok(self)
        }
    })
}

/// Emit declared policies and unfiltered runtime fields for shared model serialization.
pub fn emit_to_array_override(
    hidden: &[String],
    visible: Option<&[String]>,
    appends: &[String],
) -> TokenStream {
    let visible = visible.unwrap_or(&[]);
    quote! {
        const HIDDEN: &'static [&'static str] = &[#(#hidden),*];
        const VISIBLE: &'static [&'static str] = &[#(#visible),*];
        const APPENDS: &'static [&'static str] = &[#(#appends),*];

        fn __attributes_to_value(&self) -> ::core::result::Result<
            ::suprnova::serde_json::Value, ::suprnova::FrameworkError,
        > {
            self.__suprnova_attributes().map_err(|error| {
                ::suprnova::FrameworkError::internal(error.to_string())
            })
        }
    }
}

/// Register accessor dispatch so runtime appends can select a declared method.
/// Conversion errors reach serde instead of silently emitting null.
pub fn emit_append_accessor_dispatch(appends: &[String]) -> TokenStream {
    let arms = appends.iter().map(|name| {
        let method: syn::Ident = syn::parse_str(name).expect("validated accessor name");
        quote! {
            #name => ::suprnova::serde_json::to_value(self.#method())
                .map(::core::option::Option::Some)
                .map_err(|error| ::suprnova::FrameworkError::validation(#name, error.to_string())),
        }
    });
    quote! {
        fn __has_append_accessor(name: &str) -> bool {
            [#(#appends),*].contains(&name)
        }
        fn __try_append_accessor(&self, name: &str) -> ::core::result::Result<
            ::core::option::Option<::suprnova::serde_json::Value>, ::suprnova::FrameworkError,
        > {
            match name { #(#arms)* _ => ::core::result::Result::Ok(::core::option::Option::None) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse::ModelInput;
    use super::emit_definition_attributes;
    use quote::quote;

    fn emitted(item: proc_macro2::TokenStream) -> String {
        let input = ModelInput::parse(quote! {}, item).expect("a model");
        emit_definition_attributes(&input)
            .expect("the method")
            .to_string()
    }

    #[test]
    fn each_column_answers_to_its_serialized_name() {
        let emitted = emitted(quote! {
            #[serde(rename_all = "camelCase")]
            pub struct Row {
                pub id: i64,
                pub display_name: String,
                #[serde(skip)]
                pub note: String,
                #[serde(skip_serializing, rename = "secretToken")]
                pub token: String,
                #[serde(deserialize_with = "read_count")]
                pub visits: i64,
                #[serde(with = "stamp")]
                pub seen: i64,
                pub __eager: Cache,
                pub __pivot: Option<Pivot>,
            }
        });
        for (name, field) in [
            ("id", "id"),
            ("displayName", "display_name"),
            ("note", "note"),
            ("secretToken", "token"),
            ("visits", "visits"),
            ("seen", "seen"),
        ] {
            assert!(
                emitted.contains(&format!("\"{name}\" => {{ self . {field} =")),
                "`{name}` sets `{field}`: {emitted}"
            );
        }
        assert!(!emitted.contains("__eager") && !emitted.contains("__pivot"));
        assert!(emitted.contains("deserialize_with = \"read_count\""));
        assert!(emitted.contains("deserialize_with = \"stamp::deserialize\""));
    }

    #[test]
    fn a_shared_serialized_name_goes_to_the_later_field() {
        let emitted = emitted(quote! {
            pub struct Row {
                pub id: i64,
                pub label: String,
                #[serde(rename = "label")]
                pub caption: String,
            }
        });
        assert_eq!(emitted.matches("\"label\" =>").count(), 1, "{emitted}");
        assert!(emitted.contains("self . caption ="), "{emitted}");
        assert!(!emitted.contains("self . label ="), "{emitted}");
    }
}
