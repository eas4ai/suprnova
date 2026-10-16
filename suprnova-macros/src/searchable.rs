//! Typed search declarations keep engine input tied to the model's Rust fields.

use proc_macro2::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::{Data, DeriveInput, Fields, GenericArgument, Ident, LitStr, Path, PathArguments, Type};

#[derive(Default)]
struct Options {
    index: Option<LitStr>,
    soft_delete: bool,
    touches: Vec<LitStr>,
    should_index: Option<Path>,
    with: Option<Path>,
    scope: Option<Path>,
    scope_fields: Vec<LitStr>,
    settings: Option<Path>,
    geo: Vec<Geo>,
}

struct Geo {
    name: LitStr,
    lat: LitStr,
    lng: LitStr,
}

#[derive(Default)]
struct Roles {
    text: bool,
    weight: Option<f32>,
    filterable: bool,
    sortable: bool,
    facet: bool,
    embed: bool,
    title: bool,
}

impl Roles {
    fn declared(&self) -> bool {
        self.text || self.filterable || self.sortable || self.facet || self.embed || self.title
    }
}

fn method(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Path> {
    let value: LitStr = meta.value()?.parse()?;
    value.parse()
}

fn model_method(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Path> {
    let path = method(meta)?;
    if path.leading_colon.is_some()
        || path.segments.len() != 1
        || path
            .segments
            .iter()
            .any(|segment| !matches!(segment.arguments, PathArguments::None))
    {
        return Err(meta.error("searchable hook must name one model method"));
    }
    Ok(path)
}

fn options(input: &DeriveInput) -> syn::Result<Options> {
    let mut options = Options::default();
    for attr in input
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("searchable"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("index") {
                options.index = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("soft_delete") {
                options.soft_delete = true;
            } else if meta.path.is_ident("should_index") {
                options.should_index = Some(model_method(&meta)?);
            } else if meta.path.is_ident("with") {
                options.with = Some(model_method(&meta)?);
            } else if meta.path.is_ident("scope") {
                options.scope = Some(method(&meta)?);
            } else if meta.path.is_ident("settings") {
                options.settings = Some(method(&meta)?);
            } else if meta.path.is_ident("scope_fields") {
                let content;
                syn::parenthesized!(content in meta.input);
                let values =
                    content.parse_terminated(|input| input.parse::<LitStr>(), syn::Token![,])?;
                options.scope_fields.extend(values);
            } else if meta.path.is_ident("touches") {
                let content;
                syn::parenthesized!(content in meta.input);
                let values =
                    content.parse_terminated(|input| input.parse::<LitStr>(), syn::Token![,])?;
                options.touches.extend(values);
            } else if meta.path.is_ident("geo") {
                let mut name = None;
                let mut lat = None;
                let mut lng = None;
                meta.parse_nested_meta(|part| {
                    if part.path.is_ident("name") {
                        name = Some(part.value()?.parse()?);
                    } else if part.path.is_ident("lat") {
                        lat = Some(part.value()?.parse()?);
                    } else if part.path.is_ident("lng") {
                        lng = Some(part.value()?.parse()?);
                    } else {
                        return Err(part.error("geo accepts name, lat and lng"));
                    }
                    Ok(())
                })?;
                options.geo.push(Geo {
                    name: name.ok_or_else(|| meta.error("geo requires name"))?,
                    lat: lat.ok_or_else(|| meta.error("geo requires lat"))?,
                    lng: lng.ok_or_else(|| meta.error("geo requires lng"))?,
                });
            } else {
                return Err(meta.error("unknown searchable struct option"));
            }
            Ok(())
        })?;
    }
    if options.scope.is_some() == options.scope_fields.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "searchable scope and scope_fields(...) are declared together: scope_fields names every field the scope reads",
        ));
    }
    if options
        .index
        .as_ref()
        .is_some_and(|name| name.value().is_empty())
    {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "searchable index must not be empty",
        ));
    }
    Ok(options)
}

fn weight(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<f32> {
    let value: syn::Expr = meta.value()?.parse()?;
    let parsed = match &value {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Float(number) => number.base10_parse::<f32>()?,
            syn::Lit::Int(number) => number.base10_parse::<f32>()?,
            _ => return Err(meta.error("searchable weight must be a positive number")),
        },
        _ => return Err(meta.error("searchable weight must be a positive number")),
    };
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(meta.error("searchable weight must be finite and positive"));
    }
    Ok(parsed)
}

fn roles(field: &syn::Field) -> syn::Result<Roles> {
    let mut roles = Roles::default();
    for attr in field
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("searchable"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("text") {
                roles.text = true;
                if meta.input.peek(syn::token::Paren) {
                    meta.parse_nested_meta(|part| {
                        if !part.path.is_ident("weight") {
                            return Err(part.error("text accepts only weight"));
                        }
                        roles.weight = Some(weight(&part)?);
                        Ok(())
                    })?;
                }
            } else if meta.path.is_ident("weight") {
                roles.weight = Some(weight(&meta)?);
            } else if meta.path.is_ident("filterable") {
                roles.filterable = true;
            } else if meta.path.is_ident("sortable") {
                roles.sortable = true;
            } else if meta.path.is_ident("facet") {
                roles.facet = true;
                roles.filterable = true;
            } else if meta.path.is_ident("embed") {
                roles.embed = true;
            } else if meta.path.is_ident("title") {
                roles.title = true;
            } else {
                return Err(meta.error("unknown searchable field option"));
            }
            Ok(())
        })?;
    }
    if roles.weight.is_some() && !roles.text {
        return Err(syn::Error::new_spanned(
            field,
            "searchable weight requires text",
        ));
    }
    Ok(roles)
}

fn inner_type<'a>(ty: &'a Type, wrapper: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else { return None };
    let last = path.path.segments.last()?;
    if last.ident != wrapper {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return None;
    };
    if arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

fn attribute_type(ty: &Type, name: &Ident) -> syn::Result<Ident> {
    let ty = inner_type(ty, "Option").unwrap_or(ty);
    let unsupported = || {
        syn::Error::new_spanned(
            ty,
            format!("unsupported searchable type for field `{name}`"),
        )
    };
    let Type::Path(path) = ty else {
        return Err(unsupported());
    };
    let Some(last) = path.path.segments.last() else {
        return Err(unsupported());
    };
    let kind = match last.ident.to_string().as_str() {
        "String" => "Text",
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => "Integer",
        "f32" | "f64" => "Float",
        "bool" => "Bool",
        "NaiveDateTime" | "DateTimeUtc" => "DateTime",
        "DateTime" => {
            let Some(Type::Path(zone)) = inner_type(ty, "DateTime") else {
                return Err(unsupported());
            };
            if !zone.path.segments.last().is_some_and(|s| s.ident == "Utc") {
                return Err(unsupported());
            }
            "DateTime"
        }
        "Vec" => {
            let Some(Type::Path(value)) = inner_type(ty, "Vec") else {
                return Err(unsupported());
            };
            if !value
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "String")
            {
                return Err(unsupported());
            }
            "TextList"
        }
        _ => return Err(unsupported()),
    };
    Ok(Ident::new(kind, name.span()))
}

/// Expand a derive separately from the proc-macro entry so invalid declarations are testable.
pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "Searchable models cannot be generic",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Searchable requires a model struct",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "Searchable requires named fields",
        ));
    };
    let opts = options(&input)?;
    let model = &input.ident;
    let index = opts.index.as_ref().map_or_else(
        || quote!(<#model as ::suprnova::eloquent::EloquentModel>::TABLE),
        |name| quote!(#name),
    );
    let mut declarations = Vec::new();
    let mut values = Vec::new();
    let mut embeds = Vec::new();
    let mut title: Option<String> = None;
    let mut order = 0usize;
    let mut names = std::collections::BTreeSet::new();
    let mut filterable_names = std::collections::BTreeSet::new();
    for field in &fields.named {
        let Some(ident) = &field.ident else { continue };
        if ident == "__eager" || ident == "__pivot" {
            continue;
        }
        let roles = roles(field)?;
        if !roles.declared() {
            continue;
        }
        let name = ident.unraw().to_string();
        let ty = attribute_type(&field.ty, ident)?;
        if (roles.text || roles.embed || roles.title) && ty != "Text" && ty != "TextList" {
            return Err(syn::Error::new_spanned(
                field,
                format!("searchable text, embed and title require text for field `{name}`"),
            ));
        }
        if roles.title {
            if ty != "Text" || title.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "searchable title must name one text field",
                ));
            }
            title = Some(name.clone());
        }
        if roles.embed {
            embeds.push(name.clone());
        }
        let searchable = if roles.text {
            let weight = roles.weight.unwrap_or(1.0);
            let position = order;
            order += 1;
            quote!(Some(::suprnova::search::SearchableDecl { weight: #weight, order: #position }))
        } else {
            quote!(None)
        };
        let filterable = roles.filterable;
        if filterable {
            filterable_names.insert(name.clone());
        }
        let sortable = roles.sortable;
        let facet = roles.facet;
        declarations.push(quote!(::suprnova::search::AttributeDecl {
            name: #name.into(), ty: ::suprnova::search::AttributeType::#ty,
            searchable: #searchable, filterable: #filterable, sortable: #sortable, facet: #facet,
        }));
        let finite_check = if ty == "Float" {
            let invalid = if inner_type(&field.ty, "Option").is_some() {
                quote!(self.#ident.is_some_and(|value| !value.is_finite()))
            } else {
                quote!(!self.#ident.is_finite())
            };
            quote! {
                if #invalid {
                    return Err(::suprnova::FrameworkError::param(format!("search attribute `{}`: expected a finite float", #name)));
                }
            }
        } else {
            TokenStream::new()
        };
        values.push(quote! {
            #finite_check
            let value = ::suprnova::serde_json::to_value(&self.#ident).map_err(|error|
                ::suprnova::FrameworkError::param(format!("search attribute `{}`: {error}", #name)))?;
            attributes.insert(#name.into(), ::suprnova::search::AttributeValue::from_json(
                &value, ::suprnova::search::AttributeType::#ty, #name)?);
        });
        names.insert(name);
    }
    for geo in &opts.geo {
        let name = geo.name.value();
        if name.is_empty() || !names.insert(name.clone()) {
            return Err(syn::Error::new_spanned(
                &geo.name,
                "searchable geo name must be nonempty and unique",
            ));
        }
        let coordinate = |column: &LitStr| -> syn::Result<Ident> {
            let found = fields
                .named
                .iter()
                .find(|field| field.ident.as_ref().is_some_and(|id| *id == column.value()));
            let Some(field) = found else {
                return Err(syn::Error::new_spanned(
                    column,
                    format!("searchable geo field `{}` does not exist", column.value()),
                ));
            };
            let Some(ident) = &field.ident else {
                return Err(syn::Error::new_spanned(
                    column,
                    "searchable geo requires named fields",
                ));
            };
            if attribute_type(&field.ty, ident)? != "Float"
                || inner_type(&field.ty, "Option").is_some()
            {
                return Err(syn::Error::new_spanned(
                    column,
                    format!("searchable geo field `{ident}` must be f32 or f64"),
                ));
            }
            Ok(ident.clone())
        };
        let lat = coordinate(&geo.lat)?;
        let lng = coordinate(&geo.lng)?;
        declarations.push(quote!(::suprnova::search::AttributeDecl {
            name: #name.into(), ty: ::suprnova::search::AttributeType::Geo,
            searchable: None, filterable: true, sortable: true, facet: false,
        }));
        values.push(quote! {
            let point = ::suprnova::search::GeoPoint { lat: self.#lat as f64, lng: self.#lng as f64 };
            point.validate()?;
            attributes.insert(#name.into(), ::suprnova::search::AttributeValue::Geo(point));
        });
    }
    let soft_delete = opts.soft_delete;
    if soft_delete {
        if names.contains("__soft_deleted") {
            return Err(syn::Error::new_spanned(
                model,
                "__soft_deleted is reserved for searchable soft deletion",
            ));
        }
        declarations.push(quote!(::suprnova::search::AttributeDecl {
            name: "__soft_deleted".into(),
            ty: ::suprnova::search::AttributeType::Bool,
            searchable: None,
            filterable: true,
            sortable: false,
            facet: false,
        }));
        values.push(quote! {
            let column = <Self as ::suprnova::eloquent::EloquentModel>::SOFT_DELETES_COLUMN;
            let deleted = !column.is_empty() && <Self as ::suprnova::eloquent::Model>::field_value(self, column)
                .is_some_and(|value| !value.is_null());
            attributes.insert("__soft_deleted".into(), ::suprnova::search::AttributeValue::Bool(deleted));
        });
    }
    let title_setting = title
        .as_ref()
        .map_or_else(|| quote!(None), |title| quote!(Some(#title.into())));
    let embed_source = if embeds.is_empty() {
        quote!(None)
    } else {
        quote!(Some(::suprnova::search::EmbedSource { attributes: vec![#(#embeds.into()),*], title: #title_setting }))
    };
    let title_value = title.as_ref().map_or_else(
        || quote!(None),
        |title| quote!(attributes.get(#title).and_then(|value| value.as_text()).map(str::to_owned)),
    );
    let scope = opts
        .scope
        .as_ref()
        .map_or_else(|| quote!(None), |scope| quote!(Some(#scope)));
    let settings = opts.settings.as_ref().map_or_else(
        || quote!(::suprnova::search::IndexSettings::default),
        |settings| quote!(#settings),
    );
    for geo in &opts.geo {
        filterable_names.insert(geo.name.value());
    }
    for field in &opts.scope_fields {
        if !filterable_names.contains(&field.value()) {
            return Err(syn::Error::new_spanned(
                field,
                format!(
                    "scope field `{}` must be a filterable searchable field",
                    field.value()
                ),
            ));
        }
    }
    let scope_fields = &opts.scope_fields;
    let touches = &opts.touches;
    let predicate = opts.should_index.as_ref().map(|method| quote! {
        fn should_be_searchable(&self) -> Result<bool, ::suprnova::FrameworkError> { self.#method() }
    });
    let chunk_hook = opts.with.as_ref().map(|method| quote! {
        async fn make_searchable_using(models: Vec<Self>) -> Result<Vec<Self>, ::suprnova::FrameworkError> {
            Self::#method(models).await
        }
    });
    Ok(quote! {
        #[::suprnova::async_trait]
        impl ::suprnova::search::Searchable for #model {
            fn descriptor() -> &'static ::suprnova::search::IndexDescriptor {
                static DESCRIPTOR: ::std::sync::OnceLock<::suprnova::search::IndexDescriptor> = ::std::sync::OnceLock::new();
                DESCRIPTOR.get_or_init(|| ::suprnova::search::IndexDescriptor {
                    name: #index,
                    model_table: <Self as ::suprnova::eloquent::EloquentModel>::TABLE,
                    key_column: <Self as ::suprnova::eloquent::EloquentModel>::PRIMARY_KEY,
                    attributes: vec![#(#declarations),*], embed_source: #embed_source,
                    soft_delete: #soft_delete, search_touches: vec![#(#touches),*],
                    settings: #settings, scope: #scope, scope_fields: vec![#(#scope_fields),*],
                })
            }

            fn to_search_document(&self) -> Result<::suprnova::search::SearchDocument, ::suprnova::FrameworkError> {
                let mut attributes = ::std::collections::BTreeMap::new();
                #(#values)*
                let key_value = <Self as ::suprnova::eloquent::Model>::primary_key_value_json(self);
                let key = ::suprnova::search::indexing::document_key(&key_value)?;
                let embed_names: Vec<&str> = vec![#(#embeds),*];
                let embed_parts: Vec<String> = embed_names.iter().filter_map(|name| attributes.get(*name))
                    .filter_map(|value| match value {
                        ::suprnova::search::AttributeValue::Text(text) => Some(text.clone()),
                        ::suprnova::search::AttributeValue::List(items) => Some(items.iter().filter_map(|v| v.as_text()).collect::<Vec<_>>().join(" ")),
                        _ => None,
                    }).collect();
                let title = #title_value;
                let mut document = ::suprnova::search::SearchDocument {
                    index: #index.into(), key, source_version: 0, attributes,
                    embed_text: if embed_parts.is_empty() { None } else { Some(embed_parts.join("\n")) }, title,
                };
                let updated_at = if <Self as ::suprnova::eloquent::EloquentModel>::HAS_TIMESTAMPS {
                    <Self as ::suprnova::eloquent::Model>::field_value(self, <Self as ::suprnova::eloquent::EloquentModel>::UPDATED_AT_COLUMN)
                } else { None };
                document.source_version = ::suprnova::search::indexing::source_version(&document, updated_at)?;
                document.validate(<Self as ::suprnova::search::Searchable>::descriptor())?;
                Ok(document)
            }

            #predicate
            #chunk_hook
        }

        impl #model {
            async fn __suprnova_load_search_documents(keys: Vec<String>) -> Result<Vec<::suprnova::search::SearchDocument>, ::suprnova::FrameworkError> {
                if keys.is_empty() { return Ok(Vec::new()) }
                let key_column = <Self as ::suprnova::eloquent::Model>::primary_key_name();
                let keys = keys.iter().map(|key| ::suprnova::search::indexing::model_key_value::<<Self as ::suprnova::eloquent::EloquentModel>::Key>(key, key_column))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut query = <Self as ::suprnova::eloquent::Model>::query();
                if let Some(connection) = ::suprnova::search::indexing::job_connection() { query = query.on(connection); }
                if #soft_delete { query = query.__disable_named_scope("soft_deletes"); }
                let rows = query.where_in(key_column, keys).get().await?;
                let models = <Self as ::suprnova::search::Searchable>::make_searchable_using(rows.into_vec()).await?;
                let mut documents = Vec::with_capacity(models.len());
                for model in models {
                    if <Self as ::suprnova::search::Searchable>::should_be_searchable(&model)? {
                        documents.push(<Self as ::suprnova::search::Searchable>::to_search_document(&model)?);
                    }
                }
                Ok(documents)
            }
        }

        ::suprnova::inventory::submit! {
            ::suprnova::search::SearchableEntry {
                search: |plan, grammar, viewer| ::std::boxed::Box::pin(
                    ::suprnova::search::builder::search_projected::<#model>(plan, grammar, viewer)
                ),
                type_id: || ::std::any::TypeId::of::<#model>(),
                model_name: || ::std::any::type_name::<#model>(),
                connection: <#model as ::suprnova::eloquent::EloquentModel>::default_connection_name,
                import_all: || ::std::boxed::Box::pin(::suprnova::search::indexing::make_all_searchable::<#model>()),
                index: #index,
                descriptor: <#model as ::suprnova::search::Searchable>::descriptor,
                load_document: |key| {
                    let key = key.to_owned();
                    ::std::boxed::Box::pin(async move {
                        let documents = #model::__suprnova_load_search_documents(vec![key]).await?;
                        Ok(documents.into_iter().next())
                    })
                },
                load_documents: |keys| ::std::boxed::Box::pin(#model::__suprnova_load_search_documents(keys)),
                evaluate_scope: |viewer, document| {
                    ::suprnova::search::evaluate_scope(
                        <#model as ::suprnova::search::Searchable>::descriptor(), viewer, document,
                    )
                },
                document: |model| {
                    let model = model.downcast_ref::<#model>().ok_or_else(|| ::suprnova::FrameworkError::param("search model type"))?;
                    <#model as ::suprnova::search::Searchable>::to_search_document(model)
                },
                should_index: |model| {
                    let model = model.downcast_ref::<#model>().ok_or_else(|| ::suprnova::FrameworkError::param("search model type"))?;
                    <#model as ::suprnova::search::Searchable>::should_be_searchable(model)
                },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_emits_typed_roles_inventory_and_hooks() {
        let input = syn::parse_quote! {
            #[searchable(index = "posts", soft_delete, touches("author"), should_index = "published", with = "eager", scope = "visible", scope_fields("category"), geo(name = "location", lat = "lat", lng = "lng"))]
            struct Post {
                id: i64,
                #[searchable(text(weight = 2.5), embed, title)] title: String,
                #[searchable(facet, sortable)] category: Option<String>,
                #[searchable(filterable)] tags: Vec<String>,
                lat: f64, lng: f32,
                __eager: Unsupported,
            }
        };
        let output = expand(input).unwrap().to_string();
        assert!(output.contains("inventory :: submit"));
        assert!(output.contains("make_searchable_using"));
        assert!(output.contains("should_be_searchable"));
        assert!(output.contains("AttributeType :: TextList"));
        assert!(output.contains("AttributeType :: Geo"));
        assert!(!output.contains("Unsupported"));
        syn::parse2::<syn::File>(
            expand(syn::parse_quote!(
                struct Post {
                    #[searchable(text)]
                    title: String,
                }
            ))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn scope_and_scope_fields_come_together_over_filterable_fields() {
        for (input, message) in [
            (
                syn::parse_quote!(
                    #[searchable(scope = "visible")]
                    struct Post {
                        #[searchable(filterable)]
                        owner: String,
                    }
                ),
                "scope_fields",
            ),
            (
                syn::parse_quote!(
                    #[searchable(scope_fields("owner"))]
                    struct Post {
                        #[searchable(filterable)]
                        owner: String,
                    }
                ),
                "scope_fields",
            ),
            (
                syn::parse_quote!(
                    #[searchable(scope = "visible", scope_fields("title"))]
                    struct Post {
                        #[searchable(text)]
                        title: String,
                    }
                ),
                "title",
            ),
        ] {
            let error = expand(input).unwrap_err().to_string();
            assert!(error.contains(message), "{error}");
        }
    }

    #[test]
    fn unsupported_declared_type_names_the_field() {
        let error = expand(syn::parse_quote!(
            struct Post {
                #[searchable(filterable)]
                labels: Vec<i32>,
            }
        ))
        .unwrap_err();
        assert!(error.to_string().contains("labels"));
    }

    #[test]
    fn rejects_invalid_roles_weights_geo_and_generic_models() {
        for input in [
            syn::parse_quote!(
                struct Post {
                    #[searchable(text, weight = 0)]
                    title: String,
                }
            ),
            syn::parse_quote!(
                struct Post {
                    #[searchable(text)]
                    age: i64,
                }
            ),
            syn::parse_quote!(
                struct Post {
                    #[searchable(weight = 2)]
                    title: String,
                }
            ),
            syn::parse_quote!(
                struct Post {
                    #[searchable(unknown)]
                    title: String,
                }
            ),
            syn::parse_quote!(
                #[searchable(geo(name = "location", lat = "missing", lng = "lng"))]
                struct Post {
                    lng: f64,
                }
            ),
            syn::parse_quote!(
                struct Post<T> {
                    #[searchable(text)]
                    title: T,
                }
            ),
            syn::parse_quote!(
                enum Post {
                    First,
                }
            ),
            syn::parse_quote!(
                struct Post(String);
            ),
        ] {
            assert!(expand(input).is_err());
        }
    }

    #[test]
    fn accepts_every_supported_scalar_and_optional_type() {
        for ty in [
            "String",
            "i8",
            "i16",
            "i32",
            "i64",
            "u8",
            "u16",
            "u32",
            "u64",
            "f32",
            "f64",
            "bool",
            "chrono::DateTime<chrono::Utc>",
            "chrono::NaiveDateTime",
            "DateTimeUtc",
            "Vec<String>",
        ] {
            let name: Ident = syn::parse_quote!(value);
            let ty: Type = syn::parse_str(ty).unwrap();
            assert!(attribute_type(&ty, &name).is_ok());
            let optional: Type = syn::parse_quote!(Option<#ty>);
            assert!(attribute_type(&optional, &name).is_ok());
        }
    }
}
