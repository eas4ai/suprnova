//! Emits a document model from the parsed attribute and struct: the
//! `DocumentModel` impl that writes and reads the BSON, the serde
//! `Serialize` that honours `hidden` and `visible`, route binding by the
//! key, one relation accessor per embedded field, and the module of event
//! type names `#[suprnova::observer]` reads.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Ident, Visibility};

use super::parse::{DocumentInput, Embed};
use crate::model::parse::to_snake;

pub fn emit(input: &DocumentInput) -> TokenStream {
    let mut item = input.item.clone();
    item.attrs.insert(
        0,
        syn::parse_quote!(#[derive(::core::clone::Clone, ::core::fmt::Debug)]),
    );

    let model = emit_model(input);
    let serialize = emit_serialize(input);
    let route_binding = emit_route_binding(input);
    let embeds = emit_embeds(input);
    let events = emit_events(input);

    quote! {
        #item
        #model
        #serialize
        #route_binding
        #embeds
        #events
    }
}

fn string_list(names: &[String]) -> TokenStream {
    quote! { &[#(#names),*] }
}

fn emit_model(input: &DocumentInput) -> TokenStream {
    let ident = &input.item.ident;
    let struct_name = ident.to_string();
    let key = &input.key;
    let key_name = key.to_string();
    let key_type = &input.key_type;
    let collection = &input.collection;
    let connection = match &input.connection {
        Some(name) => quote! { ::core::option::Option::Some(#name) },
        None => quote! { ::core::option::Option::None },
    };
    let field_names: Vec<String> = input.fields.iter().map(|f| f.ident.to_string()).collect();
    let fields = string_list(&field_names);
    let timestamps = match &input.timestamps {
        Some((created, updated)) => {
            quote! { ::core::option::Option::Some((#created, #updated)) }
        }
        None => quote! { ::core::option::Option::None },
    };
    let soft_deletes = match &input.soft_deletes {
        Some(column) => quote! { ::core::option::Option::Some(#column) },
        None => quote! { ::core::option::Option::None },
    };
    let hidden = string_list(&input.hidden);
    let visible = match &input.visible {
        Some(list) => {
            let list = string_list(list);
            quote! { ::core::option::Option::Some(#list) }
        }
        None => quote! { ::core::option::Option::None },
    };
    let fillable = match (&input.fillable, &input.guarded) {
        (Some(allowed), _) => quote! { ::suprnova::Fillable::fillable(::std::vec![#(#allowed),*]) },
        (None, Some(blocked)) => {
            quote! { ::suprnova::Fillable::guarded(::std::vec![#(#blocked),*]) }
        }
        (None, None) => quote! { ::suprnova::Fillable::guarded(::std::vec![#key_name]) },
    };

    let reads = input.fields.iter().map(|field| {
        let name = &field.ident;
        let ty = &field.ty;
        let field_name = name.to_string();
        let storage = if field.ident == *key {
            "_id".to_owned()
        } else {
            field_name.clone()
        };
        let empty = field.empty_when_missing;
        match &field.cast {
            Some(cast) => quote! {
                #name: ::suprnova::mongodb::__read_cast_field::<#ty, #cast>(
                    &mut document, #storage, #struct_name, #field_name,
                )?
            },
            None => quote! {
                #name: ::suprnova::mongodb::__read_field::<#ty>(
                    &mut document, #storage, #struct_name, #field_name, #empty,
                )?
            },
        }
    });
    let writes = input.fields.iter().map(|field| {
        let name = &field.ident;
        let ty = &field.ty;
        let field_name = name.to_string();
        let storage = if field.ident == *key {
            "_id".to_owned()
        } else {
            field_name.clone()
        };
        let value = match &field.cast {
            Some(cast) => quote! {
                ::suprnova::mongodb::__write_cast_field::<#ty, #cast>(
                    &self.#name, #struct_name, #field_name,
                )?
            },
            None => quote! {
                ::suprnova::mongodb::__write_field(&self.#name, #struct_name, #field_name)?
            },
        };
        quote! { document.insert(#storage, #value); }
    });

    quote! {
        impl ::suprnova::mongodb::DocumentModel for #ident {
            type Key = #key_type;
            const COLLECTION: &'static str = #collection;
            const CONNECTION: ::core::option::Option<&'static str> = #connection;
            const KEY_FIELD: &'static str = #key_name;
            const FIELDS: &'static [&'static str] = #fields;
            const TIMESTAMPS: ::core::option::Option<(&'static str, &'static str)> = #timestamps;
            const SOFT_DELETES: ::core::option::Option<&'static str> = #soft_deletes;
            const HIDDEN: &'static [&'static str] = #hidden;
            const VISIBLE: ::core::option::Option<&'static [&'static str]> = #visible;

            fn key(&self) -> &Self::Key {
                &self.#key
            }

            fn fillable() -> ::suprnova::Fillable {
                #fillable
            }

            fn from_document(
                mut document: ::suprnova::bson::Document,
            ) -> ::core::result::Result<Self, ::suprnova::FrameworkError> {
                ::core::result::Result::Ok(Self { #(#reads),* })
            }

            fn to_document(
                &self,
            ) -> ::core::result::Result<::suprnova::bson::Document, ::suprnova::FrameworkError> {
                let mut document = ::suprnova::bson::Document::new();
                #(#writes)*
                ::core::result::Result::Ok(document)
            }
        }
    }
}

fn emit_serialize(input: &DocumentInput) -> TokenStream {
    let ident = &input.item.ident;
    quote! {
        impl ::suprnova::serde::Serialize for #ident {
            fn serialize<__S>(
                &self,
                serializer: __S,
            ) -> ::core::result::Result<__S::Ok, __S::Error>
            where
                __S: ::suprnova::serde::Serializer,
            {
                ::suprnova::mongodb::__serialize_document(self, serializer)
            }
        }
    }
}

fn emit_route_binding(input: &DocumentInput) -> TokenStream {
    let ident = &input.item.ident;
    let key = &input.key;
    let key_name = key.to_string();
    let key_type = &input.key_type;
    quote! {
        #[::suprnova::__async_trait::async_trait]
        impl ::suprnova::RouteBinding for #ident {
            fn route_key_name() -> &'static str {
                #key_name
            }

            fn route_key(&self) -> ::std::string::String {
                <#key_type as ::suprnova::mongodb::DocumentKey>::to_route_key(&self.#key)
            }

            async fn resolve_route_binding(
                value: &str,
                field: ::core::option::Option<&str>,
            ) -> ::core::result::Result<::core::option::Option<Self>, ::suprnova::FrameworkError> {
                ::suprnova::mongodb::__resolve_document_route_binding::<Self>(value, field, false)
                    .await
            }

            async fn resolve_soft_deletable_route_binding(
                value: &str,
                field: ::core::option::Option<&str>,
            ) -> ::core::result::Result<::core::option::Option<Self>, ::suprnova::FrameworkError> {
                ::suprnova::mongodb::__resolve_document_route_binding::<Self>(value, field, true)
                    .await
            }
        }
    }
}

fn emit_embeds(input: &DocumentInput) -> TokenStream {
    let ident = &input.item.ident;
    let accessors: Vec<TokenStream> = input
        .fields
        .iter()
        .filter_map(|field| {
            let name = &field.ident;
            let field_name = name.to_string();
            match &field.embed {
                Some(Embed::One(inner)) => {
                    let doc = format!(
                        " The `{field_name}` document embedded in this one, as a relation \
                         that saves or deletes it."
                    );
                    Some(quote! {
                        #[doc = #doc]
                        pub fn #name(&mut self) -> ::suprnova::mongodb::EmbedsOne<'_, Self, #inner> {
                            ::suprnova::mongodb::EmbedsOne::new(self, #field_name)
                        }
                    })
                }
                Some(Embed::Many(inner)) => {
                    let doc = format!(
                        " The `{field_name}` documents embedded in this one, as a relation \
                         that saves and destroys them."
                    );
                    Some(quote! {
                        #[doc = #doc]
                        pub fn #name(&mut self) -> ::suprnova::mongodb::EmbedsMany<'_, Self, #inner> {
                            ::suprnova::mongodb::EmbedsMany::new(self, #field_name)
                        }
                    })
                }
                None => None,
            }
        })
        .collect();
    if accessors.is_empty() {
        return TokenStream::new();
    }
    quote! {
        impl #ident {
            #(#accessors)*
        }
    }
}

/// The event names, in the order the observer reads them.
const EVENTS: &[(&str, &str)] = &[
    ("Retrieving", "before a query reads documents of the model."),
    ("Retrieved", "for each document a query read."),
    (
        "Saving",
        "before a document is inserted or updated. Cancellable.",
    ),
    ("Creating", "before a document is inserted. Cancellable."),
    ("Created", "after a document was inserted."),
    ("Updating", "before a document is updated. Cancellable."),
    ("Updated", "after a document was updated."),
    ("Saved", "after a document was inserted or updated."),
    (
        "Deleting",
        "before a document is deleted or trashed. Cancellable.",
    ),
    ("Deleted", "after a document was deleted or trashed."),
    ("Trashed", "after a document was soft deleted."),
    (
        "Restoring",
        "before a trashed document is restored. Cancellable.",
    ),
    ("Restored", "after a trashed document was restored."),
    ("ForceDeleting", "before `force_delete` removes a document."),
    ("ForceDeleted", "after `force_delete` removed a document."),
];

fn emit_events(input: &DocumentInput) -> TokenStream {
    let ident = &input.item.ident;
    let module = Ident::new(&to_snake(&ident.to_string()), Span::call_site());
    let (module_vis, events_vis, alias_vis): (Visibility, TokenStream, TokenStream) =
        match &input.item.vis {
            Visibility::Public(_) => (input.item.vis.clone(), quote!(pub), quote!(pub)),
            Visibility::Restricted(restricted) if restricted.path.is_ident("crate") => (
                input.item.vis.clone(),
                quote!(pub(crate)),
                quote!(pub(crate)),
            ),
            other => (
                other.clone(),
                quote!(pub(super)),
                quote!(pub(in super::super)),
            ),
        };
    let module_doc = format!(
        " The lifecycle event types of `{ident}`, by the names \
         `#[suprnova::observer({ident})]` reads."
    );
    let aliases = EVENTS.iter().map(|(name, when)| {
        let event = Ident::new(name, Span::call_site());
        let doc = format!(" Fires {when}");
        quote! {
            #[doc = #doc]
            #alias_vis type #event = ::suprnova::mongodb::events::#event<super::super::#ident>;
        }
    });
    quote! {
        #[doc = #module_doc]
        #module_vis mod #module {
            #[doc = #module_doc]
            #events_vis mod events {
                #(#aliases)*
            }
        }
    }
}
