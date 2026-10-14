//! An SQL model's relations to documents (PAR-185): `HasManyDocuments<D>`
//! and `HasOneDocument<D>` in `relations = { ... }`.
//!
//! Each adds a relation method, the `<name>_loaded` and `<name>_count`
//! accessors, and arms in the model's eager-load dispatchers that read the
//! documents from their collection: `with` loads them with one query,
//! a dotted path continues through the document's own relations, and
//! `with_count` counts them with one aggregation. The key check, spanned at
//! the declaration, makes a relation whose key types cannot match a
//! compile error naming both models.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};

use super::parse::{DocumentRelationDecl, DocumentRelationKind, ModelInput};

/// The arms one relation adds to each dispatcher.
pub struct Arms {
    pub eager: TokenStream,
    pub recurse: TokenStream,
    pub recurse_batched: TokenStream,
    pub count: TokenStream,
    pub aggregate: TokenStream,
}

/// The model field the documents' foreign key holds.
fn local_field(input: &ModelInput, relation: &DocumentRelationDecl) -> String {
    relation
        .local_key
        .clone()
        .unwrap_or_else(|| input.primary_key.clone())
}

/// The stored form of each parent's local key, `None` where it is null.
fn parent_keys(input: &ModelInput, relation: &DocumentRelationDecl) -> TokenStream {
    let struct_name = input.item.ident.to_string();
    let local = local_field(input, relation);
    let local_ident = format_ident!("{local}");
    quote! {
        parents
            .iter()
            .map(|parent| {
                ::suprnova::mongodb::__present(::suprnova::mongodb::__write_field(
                    &parent.#local_ident,
                    #struct_name,
                    #local,
                ))
            })
            .collect::<::core::result::Result<::std::vec::Vec<_>, ::suprnova::FrameworkError>>()?
    }
}

/// The closures that store, take and put back the loaded documents of
/// `relation` in a parent's eager cache.
fn cache_closures(relation: &DocumentRelationDecl) -> (TokenStream, TokenStream, TokenStream) {
    let name = relation.name.to_string();
    let target = &relation.target;
    match relation.kind {
        DocumentRelationKind::HasMany => (
            quote! { |parent, rows| parent.__eager.set_many::<#target>(#name, rows) },
            quote! { |parent| parent.__eager.take_many::<#target>(#name) },
            quote! { |parent, rows| parent.__eager.set_many::<#target>(#name, rows) },
        ),
        DocumentRelationKind::HasOne => (
            quote! { |parent, rows| parent.__eager.set_one::<#target>(#name, rows.into_iter().next()) },
            quote! { |parent| parent.__eager.take_one::<#target>(#name).map(|row| ::std::vec![row]) },
            quote! { |parent, mut rows| parent.__eager.set_one::<#target>(#name, rows.pop()) },
        ),
    }
}

/// The relation method and the accessors of every relation to documents.
pub fn emit_methods(input: &ModelInput) -> TokenStream {
    let struct_ident = &input.item.ident;
    let struct_name = struct_ident.to_string();
    let methods = input.document_relations.iter().map(|relation| {
        let name = &relation.name;
        let name_str = name.to_string();
        let target = &relation.target;
        let fk = relation.foreign_key.as_str();
        let fk_ident = format_ident!("{fk}");
        let local = local_field(input, relation);
        let local_ident = format_ident!("{local}");
        let span = name.span();
        let check = quote_spanned! {span=>
            ::suprnova::mongodb::__relation_keys::<Self, #target, _, _, _, _>(
                |parent: &Self| &parent.#local_ident,
                |document: &#target| &document.#fk_ident,
            );
        };
        let (ty, loaded_ty, loaded_body, what) = match relation.kind {
            DocumentRelationKind::HasMany => (
                quote!(HasManyDocuments),
                quote!(::core::option::Option<&[#target]>),
                quote!(self.__eager.loaded_many::<#target>(#name_str)),
                "documents",
            ),
            DocumentRelationKind::HasOne => (
                quote!(HasOneDocument),
                quote!(::core::option::Option<&#target>),
                quote!(self.__eager.get_one::<#target>(#name_str)),
                "document",
            ),
        };
        let loaded = format_ident!("{name}_loaded");
        let count = format_ident!("{name}_count");
        let method_doc = format!(
            " The `{}` {what} whose `{fk}` holds this row's `{local}`, read from their \
             collection.",
            quote!(#target).to_string().replace(' ', "")
        );
        let loaded_doc = format!(
            " The `{name_str}` that `with([\"{name_str}\"])` loaded, or `None` when no query \
             loaded them."
        );
        let count_doc = format!(
            " The number of `{name_str}` that `with_count([\"{name_str}\"])` counted, or `None` \
             when no query counted them."
        );
        quote! {
            #[doc = #method_doc]
            pub fn #name(&self) -> ::suprnova::mongodb::#ty<#target> {
                #check
                ::suprnova::mongodb::#ty::<#target>::__new(
                    #fk,
                    ::suprnova::mongodb::__write_field(&self.#local_ident, #struct_name, #local),
                )
                .__lazy_load(::suprnova::eloquent::lazy_loading::LazyLoadGuard::for_relation(
                    &self.__eager,
                    #struct_name,
                    #name_str,
                ))
            }

            #[doc = #loaded_doc]
            pub fn #loaded(&self) -> #loaded_ty {
                #loaded_body
            }

            #[doc = #count_doc]
            pub fn #count(&self) -> ::core::option::Option<u64> {
                self.__eager.get_count(#name_str)
            }
        }
    });
    if input.document_relations.is_empty() {
        return TokenStream::new();
    }
    quote! {
        impl #struct_ident {
            #(#methods)*
        }
    }
}

/// The dispatcher arms of one relation to documents.
pub fn arms(input: &ModelInput, relation: &DocumentRelationDecl) -> Arms {
    let name = relation.name.to_string();
    let target = &relation.target;
    let fk = relation.foreign_key.as_str();
    let keys = parent_keys(input, relation);
    let (store, take, put_back) = cache_closures(relation);
    let eager = quote! {
        #name => {
            if predicate.take().is_some() {
                return ::core::result::Result::Err(::suprnova::FrameworkError::internal(
                    ::std::format!(
                        "`{}` reads the relation `{}` from MongoDB, which takes no \
                         `with_where` constraint; filter its documents with `{}().query()`",
                        ::std::any::type_name::<Self>(),
                        #name,
                        #name,
                    ),
                ));
            }
            let keys = #keys;
            ::suprnova::mongodb::__load_documents::<Self, #target>(&mut *parents, keys, #fk, #store)
                .await?;
            return ::core::result::Result::Ok(());
        }
    };
    let recurse = quote! {
        #name => {
            let mut parents: [&mut Self; 1] = [self];
            return ::suprnova::mongodb::__load_nested_documents::<Self, #target>(
                &mut parents, #take, #put_back, rest, missing_only,
            )
            .await;
        }
    };
    let recurse_batched = quote! {
        #name => {
            return ::suprnova::mongodb::__load_nested_documents::<Self, #target>(
                parents, #take, #put_back, rest, missing_only,
            )
            .await;
        }
    };
    let count = quote! {
        #name => {
            let keys = #keys;
            let counts = ::suprnova::mongodb::__count_documents::<#target>(#fk, &keys).await?;
            for (parent, count) in parents.iter_mut().zip(counts) {
                parent.__eager.set_count(#name, count);
            }
            return ::core::result::Result::Ok(());
        }
    };
    let aggregate = quote! {
        #name => {
            return ::core::result::Result::Err(::suprnova::FrameworkError::internal(
                ::std::format!(
                    "`{}` cannot aggregate `{}` over `{}`: its documents live in MongoDB, \
                     which an SQL aggregate does not reach; aggregate them with \
                     `{}().query().sum(\"{}\")` and the like",
                    ::std::any::type_name::<Self>(),
                    column,
                    #name,
                    #name,
                    column,
                ),
            ));
        }
    };
    Arms {
        eager,
        recurse,
        recurse_batched,
        count,
        aggregate,
    }
}
