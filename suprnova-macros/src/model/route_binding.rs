//! Route binding for `#[model]` (BIND-001, BIND-004, BIND-005, BIND-006).
//!
//! Emits, for every model:
//!
//! - `ModelRouteBinding`: the model's columns with how a path segment
//!   parses as each one, its route key (`#[model(route_key = "...")]`, else
//!   its primary key), and one child lookup per relation, through the
//!   relation's own query, by the relation's name.
//! - `RouteBinding`, unless `#[model(custom_route_binding)]`: every lookup
//!   goes through the framework's default lookups, which read
//!   `ModelRouteBinding`.
//! - `RouteValue`, so `route()` takes the model as a parameter value.
//! - A `SoftDeleteTable` entry, which the bare `x::Model` form reads to
//!   leave soft-deleted rows out.

use proc_macro2::TokenStream;
use quote::quote;

use super::parse::{ModelInput, RelationKindAttr};

pub fn emit(input: &ModelInput) -> TokenStream {
    let struct_ident = &input.item.ident;
    let table = &input.table;
    let route_key = input
        .route_key
        .as_ref()
        .map(|(column, _)| column.clone())
        .unwrap_or_else(|| input.primary_key.clone());

    let columns = column_entries(input);
    let children = child_arms(input);
    // Every column's parser comes through the probe trait, imported only
    // when there is a column to call it for.
    let column_parser = if columns.is_empty() {
        TokenStream::new()
    } else {
        quote! { use ::suprnova::database::__ColumnParser as _; }
    };

    let binding_impl = if input.custom_route_binding {
        TokenStream::new()
    } else {
        quote! {
            #[::suprnova::__async_trait::async_trait]
            impl ::suprnova::RouteBinding for #struct_ident {
                fn route_key_name() -> &'static str {
                    #route_key
                }

                fn route_key(&self) -> ::std::string::String {
                    ::suprnova::database::model_route_field(self, #route_key).unwrap_or_default()
                }

                fn route_field(&self, field: &str) -> ::core::option::Option<::std::string::String> {
                    ::suprnova::database::model_route_field(self, field)
                }

                async fn resolve_route_binding(
                    value: &str,
                    field: ::core::option::Option<&str>,
                ) -> ::core::result::Result<::core::option::Option<Self>, ::suprnova::FrameworkError> {
                    ::suprnova::database::resolve_model_route_binding::<Self>(value, field, false).await
                }

                async fn resolve_soft_deletable_route_binding(
                    value: &str,
                    field: ::core::option::Option<&str>,
                ) -> ::core::result::Result<::core::option::Option<Self>, ::suprnova::FrameworkError> {
                    ::suprnova::database::resolve_model_route_binding::<Self>(value, field, true).await
                }

                async fn resolve_child_route_binding(
                    &self,
                    child: &str,
                    value: &str,
                    field: ::core::option::Option<&str>,
                ) -> ::core::result::Result<
                    ::core::option::Option<::suprnova::BoundChild>,
                    ::suprnova::FrameworkError,
                > {
                    ::suprnova::database::resolve_model_child_route_binding(self, child, value, field, false)
                        .await
                }

                async fn resolve_soft_deletable_child_route_binding(
                    &self,
                    child: &str,
                    value: &str,
                    field: ::core::option::Option<&str>,
                ) -> ::core::result::Result<
                    ::core::option::Option<::suprnova::BoundChild>,
                    ::suprnova::FrameworkError,
                > {
                    ::suprnova::database::resolve_model_child_route_binding(self, child, value, field, true)
                        .await
                }

                fn route_binding_info() -> ::suprnova::RouteBindingInfo {
                    ::suprnova::database::model_route_binding_info::<Self>()
                }
            }
        }
    };

    quote! {
        impl ::suprnova::database::ModelRouteBinding for #struct_ident {
            fn route_columns() -> ::std::vec::Vec<::suprnova::RouteColumn> {
                #column_parser
                ::std::vec![#(#columns),*]
            }

            fn model_route_key_name() -> &'static str {
                #route_key
            }

            fn __route_child<'a>(
                &'a self,
                relation: &'a str,
                value: &'a str,
                field: ::core::option::Option<&'a str>,
                trashed: bool,
            ) -> ::suprnova::database::RouteLookup<'a, ::core::option::Option<::suprnova::BoundChild>> {
                let _ = (value, field, trashed);
                match relation {
                    #(#children)*
                    _ => ::std::boxed::Box::pin(async move {
                        ::core::result::Result::Err(::suprnova::FrameworkError::internal(
                            ::std::format!(
                                "`{}` declares no relation `{}` to find a scoped child through",
                                ::core::stringify!(#struct_ident),
                                relation,
                            ),
                        ))
                    }),
                }
            }
        }

        #binding_impl

        impl ::suprnova::RouteValue for #struct_ident {
            fn route_value(&self, field: ::core::option::Option<&str>) -> ::core::option::Option<::std::string::String> {
                ::suprnova::bound_route_value(self, field)
            }
        }

        ::suprnova::inventory::submit! {
            ::suprnova::database::SoftDeleteTable {
                table: #table,
                column: <#struct_ident as ::suprnova::eloquent::EloquentModel>::SOFT_DELETES_COLUMN,
            }
        }
    }
}

/// One `RouteColumn` per column of the model, with its parser. A
/// `unique_id` key also checks the identifier's format.
fn column_entries(input: &ModelInput) -> Vec<TokenStream> {
    let syn::Fields::Named(named) = &input.item.fields else {
        return Vec::new();
    };
    let format = input.unique_id.as_deref().map(|kind| match kind {
        "uuid_v4" => quote! { ::suprnova::eloquent::unique_id::UniqueIdKind::UuidV4 },
        "ulid" => quote! { ::suprnova::eloquent::unique_id::UniqueIdKind::Ulid },
        _ => quote! { ::suprnova::eloquent::unique_id::UniqueIdKind::UuidV7 },
    });
    named
        .named
        .iter()
        .filter_map(|field| {
            let ident = field.ident.as_ref()?;
            let name = ident.to_string();
            if name == "__eager" || name == "__pivot" {
                return None;
            }
            let ty = &field.ty;
            let column = quote! {
                ::suprnova::RouteColumn::new(
                    #name,
                    (&&&&::suprnova::database::__ColumnProbe::<#ty>::new()).__column_parser(),
                )
            };
            Some(match (&format, name == input.primary_key) {
                (Some(kind), true) => quote! { #column.with_format(#kind) },
                _ => column,
            })
        })
        .collect()
}

/// One `__route_child` arm per relation with a single child type: the
/// child, looked up through the relation's own query.
fn child_arms(input: &ModelInput) -> Vec<TokenStream> {
    let Some(relations) = &input.relations else {
        return Vec::new();
    };
    relations
        .iter()
        .filter(|rel| rel.kind != RelationKindAttr::MorphTo)
        .map(|rel| {
            let name = &rel.name;
            let name_str = name.to_string();
            let target = match rel.kind {
                RelationKindAttr::HasManyThrough | RelationKindAttr::HasOneThrough => {
                    rel.through.as_ref().unwrap_or(&rel.target)
                }
                _ => &rel.target,
            };
            quote! {
                #name_str => ::std::boxed::Box::pin(
                    ::suprnova::database::__route_child_lookup::<#target, _>(
                        self.#name(),
                        value,
                        field,
                        trashed,
                    ),
                ),
            }
        })
        .collect()
}
