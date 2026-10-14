//! The `relations = { ... }` of `#[suprnova::document]` (PAR-185): the
//! declarations, read before and resolved after the struct's fields, and
//! the code each one adds: a relation method, a `<name>_loaded` accessor,
//! and an arm of the model's eager loader.
//!
//! The grammar is the `#[model]` one: `name: Kind<Target> { option = "..." }`.
//! `HasOne`, `HasMany`, `BelongsTo` and `BelongsToMany` relate documents;
//! `BelongsToModel` relates a document to an SQL `#[model]`. Every key a
//! relation reads is checked at compile time against the field that holds
//! it on the other side, through `suprnova::mongodb::RelationKey`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Result, Token, Type};

use super::parse::{DocumentField, DocumentInput, vec_inner};
use crate::model::parse::to_snake;

/// What a document relation relates to, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    /// One document of the target whose foreign key holds this one's key.
    HasOne,
    /// The documents of the target whose foreign key holds this one's key.
    HasMany,
    /// The document of the target this one's foreign key names.
    BelongsTo,
    /// The documents of the target related many-to-many, each side
    /// keeping the other's keys in an array.
    BelongsToMany,
    /// The SQL model this document's foreign key names.
    BelongsToModel,
}

impl RelationKind {
    const ALL: &'static str = "HasOne, HasMany, BelongsTo, BelongsToMany, BelongsToModel";

    fn from_ident(name: &str) -> Option<Self> {
        Some(match name {
            "HasOne" => Self::HasOne,
            "HasMany" => Self::HasMany,
            "BelongsTo" => Self::BelongsTo,
            "BelongsToMany" => Self::BelongsToMany,
            "BelongsToModel" => Self::BelongsToModel,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::HasOne => "HasOne",
            Self::HasMany => "HasMany",
            Self::BelongsTo => "BelongsTo",
            Self::BelongsToMany => "BelongsToMany",
            Self::BelongsToModel => "BelongsToModel",
        }
    }

    /// The options the kind takes.
    fn options(self) -> &'static [&'static str] {
        match self {
            Self::BelongsToMany => &["pivot_foreign_key", "pivot_related_key"],
            _ => &["fk", "lk"],
        }
    }

    /// Whether the relation loads several models.
    fn is_many(self) -> bool {
        matches!(self, Self::HasMany | Self::BelongsToMany)
    }
}

/// One relation, its keys resolved.
#[derive(Debug, Clone)]
pub struct DocumentRelation {
    pub name: Ident,
    pub kind: RelationKind,
    pub target: Type,
    /// `HasOne` and `HasMany`: the field of the target that holds this
    /// document's key. `BelongsTo` and `BelongsToModel`: the field of this
    /// document that holds the owner's key. `BelongsToMany`: the array of
    /// the target that holds this document's keys.
    pub foreign_key: String,
    /// `HasOne` and `HasMany`: the field of this document the foreign key
    /// holds, `None` for the key. `BelongsTo`: the field of the owner,
    /// `None` for its key. `BelongsToModel`: the owner's column, `None`
    /// for its primary key. `BelongsToMany`: the array of this document
    /// that holds the target's keys.
    pub local_key: Option<String>,
}

/// One relation as written, before the struct's fields are known.
pub struct RelationDecl {
    name: Ident,
    kind: RelationKind,
    target: Type,
    options: Vec<(Ident, LitStr)>,
}

/// The `{ name: Kind<Target> { ... }, ... }` after `relations =`.
pub fn parse_relations(input: ParseStream) -> Result<Vec<RelationDecl>> {
    let content;
    syn::braced!(content in input);
    let mut out = Vec::new();
    while !content.is_empty() {
        out.push(content.parse::<RelationDecl>()?);
        if content.is_empty() {
            break;
        }
        content.parse::<Token![,]>()?;
    }
    Ok(out)
}

impl Parse for RelationDecl {
    fn parse(input: ParseStream) -> Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let kind_ident: Ident = input.parse()?;
        let kind = RelationKind::from_ident(&kind_ident.to_string()).ok_or_else(|| {
            syn::Error::new(
                kind_ident.span(),
                format!(
                    "unknown document relation kind `{kind_ident}`; a document relates with {}",
                    RelationKind::ALL
                ),
            )
        })?;
        input.parse::<Token![<]>()?;
        let target: Type = input.parse()?;
        if input.peek(Token![,]) {
            return Err(syn::Error::new(
                kind_ident.span(),
                format!("`{kind_ident}` takes one generic argument, the related model"),
            ));
        }
        input.parse::<Token![>]>()?;
        let mut options = Vec::new();
        if input.peek(syn::token::Brace) {
            let content;
            syn::braced!(content in input);
            while !content.is_empty() {
                let option: Ident = content.parse()?;
                if !kind.options().iter().any(|allowed| option == allowed) {
                    return Err(syn::Error::new(
                        option.span(),
                        format!(
                            "`{option}` is no option of `{}`; it takes {}",
                            kind.name(),
                            kind.options().join(", ")
                        ),
                    ));
                }
                content.parse::<Token![=]>()?;
                let value: LitStr = content.parse()?;
                if syn::parse_str::<Ident>(&value.value()).is_err() {
                    return Err(syn::Error::new(
                        value.span(),
                        format!("`{option}` names a field: it must be a Rust identifier"),
                    ));
                }
                if options
                    .iter()
                    .any(|(seen, _): &(Ident, LitStr)| *seen == option)
                {
                    return Err(syn::Error::new(
                        option.span(),
                        format!("`{option}` is given twice"),
                    ));
                }
                options.push((option, value));
                if content.is_empty() {
                    break;
                }
                content.parse::<Token![,]>()?;
            }
        }
        Ok(Self {
            name,
            kind,
            target,
            options,
        })
    }
}

impl RelationDecl {
    fn option(&self, name: &str) -> Option<&LitStr> {
        self.options
            .iter()
            .find(|(option, _)| option == name)
            .map(|(_, value)| value)
    }
}

/// The `DocumentModel` methods a relation name would hide: the model's
/// inherent method of the relation's name would win over them.
const RESERVED: &[&str] = &[
    "all",
    "attributes",
    "collection",
    "create",
    "decrement",
    "delete",
    "fill",
    "fillable",
    "find",
    "find_or_fail",
    "force_delete",
    "fresh",
    "from_document",
    "increment",
    "key",
    "make",
    "observe",
    "only_trashed",
    "pull",
    "push",
    "push_unique",
    "query",
    "refresh",
    "relation_loaded",
    "restore",
    "save",
    "to_document",
    "update",
    "with_trashed",
];

/// The last segment of `ty`'s path, for default key names.
fn type_name(ty: &Type) -> String {
    match ty {
        Type::Path(path) => path.path.segments.last().map_or_else(
            || quote!(#ty).to_string(),
            |segment| segment.ident.to_string(),
        ),
        _ => quote!(#ty).to_string(),
    }
}

/// Resolve the declarations against the document's fields: default each
/// key, and refuse a relation whose own keys are no fields of `struct_name`.
pub fn resolve(
    decls: Vec<RelationDecl>,
    struct_name: &str,
    fields: &[DocumentField],
) -> Result<Vec<DocumentRelation>> {
    let field = |name: &str| fields.iter().find(|field| field.ident == name);
    let mut relations: Vec<DocumentRelation> = Vec::with_capacity(decls.len());
    for decl in decls {
        let name = decl.name.to_string();
        if relations.iter().any(|relation| relation.name == decl.name) {
            return Err(syn::Error::new(
                decl.name.span(),
                format!("the relation `{name}` is declared twice"),
            ));
        }
        if fields
            .iter()
            .any(|field| field.embed.is_some() && field.ident == decl.name)
        {
            return Err(syn::Error::new(
                decl.name.span(),
                format!(
                    "`{name}` names an embedded field, whose method is its embed relation; \
                     give the relation another name"
                ),
            ));
        }
        if RESERVED.contains(&name.as_str()) {
            return Err(syn::Error::new(
                decl.name.span(),
                format!(
                    "a relation named `{name}` would hide the model's own `{name}`; give it \
                     another name"
                ),
            ));
        }
        let own_field = |key: &str, what: &str| -> Result<()> {
            match field(key) {
                Some(_) => Ok(()),
                None => Err(syn::Error::new(
                    decl.name.span(),
                    format!(
                        "the relation `{name}` reads {what} `{key}`, which is no field of \
                         `{struct_name}`"
                    ),
                )),
            }
        };
        let fk = decl.option("fk").map(LitStr::value);
        let lk = decl.option("lk").map(LitStr::value);
        let target = type_name(&decl.target);
        let (foreign_key, local_key) = match decl.kind {
            RelationKind::HasOne | RelationKind::HasMany => {
                if let Some(lk) = &lk {
                    own_field(lk, "its local key")?;
                }
                (
                    fk.unwrap_or_else(|| format!("{}_id", to_snake(struct_name))),
                    lk,
                )
            }
            RelationKind::BelongsTo | RelationKind::BelongsToModel => {
                let fk = fk.unwrap_or_else(|| format!("{}_id", to_snake(&target)));
                own_field(&fk, "its foreign key")?;
                (fk, lk)
            }
            RelationKind::BelongsToMany => {
                let foreign = decl
                    .option("pivot_foreign_key")
                    .map_or_else(|| format!("{}_ids", to_snake(struct_name)), LitStr::value);
                let local = decl
                    .option("pivot_related_key")
                    .map_or_else(|| format!("{}_ids", to_snake(&target)), LitStr::value);
                own_field(&local, "the keys of its related documents from")?;
                if field(&local).is_some_and(|field| vec_inner(&field.ty).is_none()) {
                    return Err(syn::Error::new(
                        decl.name.span(),
                        format!(
                            "the relation `{name}` keeps the related keys in `{local}`, which \
                             must be a `Vec` of them"
                        ),
                    ));
                }
                (foreign, Some(local))
            }
        };
        relations.push(DocumentRelation {
            name: decl.name,
            kind: decl.kind,
            target: decl.target,
            foreign_key,
            local_key,
        });
    }
    Ok(relations)
}

// --- Emission ---------------------------------------------------------------------

/// The stored value of this document's field `name`, read from
/// `receiver`, through the field's cast.
fn stored(input: &DocumentInput, receiver: &TokenStream, name: &str) -> TokenStream {
    let struct_name = input.item.ident.to_string();
    let ident = format_ident!("{name}");
    match input.fields.iter().find(|field| field.ident == name) {
        Some(DocumentField {
            ty,
            cast: Some(cast),
            ..
        }) => quote! {
            ::suprnova::mongodb::__write_cast_field::<#ty, #cast>(
                &#receiver.#ident, #struct_name, #name,
            )
        },
        _ => quote! {
            ::suprnova::mongodb::__write_field(&#receiver.#ident, #struct_name, #name)
        },
    }
}

/// The value this document's side of `relation` compares: its key, or
/// the field the relation names.
fn local_value(
    input: &DocumentInput,
    relation: &DocumentRelation,
    receiver: TokenStream,
) -> TokenStream {
    match (relation.kind, &relation.local_key) {
        (RelationKind::HasOne | RelationKind::HasMany, Some(field)) => {
            stored(input, &receiver, field)
        }
        (RelationKind::BelongsTo | RelationKind::BelongsToModel, _) => {
            stored(input, &receiver, &relation.foreign_key)
        }
        _ => quote! { ::suprnova::mongodb::__document_key::<Self>(#receiver) },
    }
}

/// The compile-time key checks of `relation`, spanned at its name so the
/// error points at the declaration.
fn key_checks(relation: &DocumentRelation) -> TokenStream {
    let span = relation.name.span();
    let target = &relation.target;
    let fk = format_ident!("{}", relation.foreign_key);
    let key_of = |model: TokenStream| {
        quote! { |model: &#model| <#model as ::suprnova::mongodb::DocumentModel>::key(model) }
    };
    match relation.kind {
        RelationKind::HasOne | RelationKind::HasMany => {
            let parent = match &relation.local_key {
                Some(field) => {
                    let field = format_ident!("{field}");
                    quote! { |model: &Self| &model.#field }
                }
                None => key_of(quote!(Self)),
            };
            quote_spanned! {span=>
                ::suprnova::mongodb::__relation_keys::<Self, #target, _, _, _, _>(
                    #parent,
                    |child: &#target| &child.#fk,
                );
            }
        }
        RelationKind::BelongsTo => {
            let owner = match &relation.local_key {
                Some(field) => {
                    let field = format_ident!("{field}");
                    quote! { |owner: &#target| &owner.#field }
                }
                None => key_of(quote!(#target)),
            };
            quote_spanned! {span=>
                ::suprnova::mongodb::__relation_keys::<#target, Self, _, _, _, _>(
                    #owner,
                    |child: &Self| &child.#fk,
                );
            }
        }
        RelationKind::BelongsToMany => {
            let local = format_ident!(
                "{}",
                relation
                    .local_key
                    .as_deref()
                    .unwrap_or(&relation.foreign_key)
            );
            let parent = key_of(quote!(Self));
            let related = key_of(quote!(#target));
            quote_spanned! {span=>
                ::suprnova::mongodb::__relation_keys::<Self, #target, _, _, _, _>(
                    #parent,
                    |related: &#target| &related.#fk,
                );
                ::suprnova::mongodb::__relation_keys::<#target, Self, _, _, _, _>(
                    #related,
                    |parent: &Self| &parent.#local,
                );
            }
        }
        RelationKind::BelongsToModel => match &relation.local_key {
            Some(column) => {
                let column = format_ident!("{column}");
                quote_spanned! {span=>
                    ::suprnova::mongodb::__relation_keys::<#target, Self, _, _, _, _>(
                        |owner: &#target| &owner.#column,
                        |child: &Self| &child.#fk,
                    );
                }
            }
            None => quote_spanned! {span=>
                ::suprnova::mongodb::__relation_model_key::<#target, Self, _, _>(
                    |child: &Self| &child.#fk,
                );
            },
        },
    }
}

/// The relation methods and the `<name>_loaded` accessors, in one
/// `impl` block; nothing for a document without relations.
pub fn emit(input: &DocumentInput) -> TokenStream {
    if input.relations.is_empty() {
        return TokenStream::new();
    }
    let ident = &input.item.ident;
    let struct_name = ident.to_string();
    let methods = input.relations.iter().map(|relation| {
        let name = &relation.name;
        let name_str = name.to_string();
        let target = &relation.target;
        let target_name = type_name(target);
        let fk = relation.foreign_key.as_str();
        let checks = key_checks(relation);
        let local = local_value(input, relation, quote!(self));
        let loaded = format_ident!("{name}_loaded");
        let (method, accessor) = match relation.kind {
            RelationKind::HasMany | RelationKind::HasOne => {
                let (ty, what) = if relation.kind == RelationKind::HasMany {
                    (quote!(HasManyDocuments), "documents")
                } else {
                    (quote!(HasOneDocument), "document")
                };
                let doc = format!(
                    " The `{target_name}` {what} whose `{fk}` holds this document's {}.",
                    relation
                        .local_key
                        .as_deref()
                        .map_or_else(|| "key".to_owned(), |field| format!("`{field}`"))
                );
                (
                    quote! {
                        #[doc = #doc]
                        pub fn #name(&self) -> ::suprnova::mongodb::#ty<#target> {
                            #checks
                            ::suprnova::mongodb::#ty::<#target>::__new(#fk, #local)
                        }
                    },
                    accessor(relation, &loaded, &name_str),
                )
            }
            RelationKind::BelongsTo => {
                let owner_key = match &relation.local_key {
                    Some(field) => quote!(#field),
                    None => quote!(<#target as ::suprnova::mongodb::DocumentModel>::KEY_FIELD),
                };
                let doc = format!(" The `{target_name}` document this document's `{fk}` names.");
                (
                    quote! {
                        #[doc = #doc]
                        pub fn #name(&self) -> ::suprnova::mongodb::BelongsToDocument<#target> {
                            #checks
                            ::suprnova::mongodb::BelongsToDocument::<#target>::__new(
                                #fk, #owner_key, #local,
                            )
                        }
                    },
                    accessor(relation, &loaded, &name_str),
                )
            }
            RelationKind::BelongsToMany => {
                let related = relation.local_key.as_deref().unwrap_or_default();
                let doc = format!(
                    " The `{target_name}` documents related to this one: their `{fk}` holds \
                     its key, and its `{related}` holds theirs."
                );
                (
                    quote! {
                        #[doc = #doc]
                        pub fn #name(&self) -> ::suprnova::mongodb::BelongsToManyDocuments<Self, #target> {
                            #checks
                            ::suprnova::mongodb::BelongsToManyDocuments::<Self, #target>::__new(
                                #local, #fk, #related,
                            )
                        }
                    },
                    accessor(relation, &loaded, &name_str),
                )
            }
            RelationKind::BelongsToModel => {
                let owner_key = match &relation.local_key {
                    Some(column) => quote!(#column),
                    None => quote!(<#target as ::suprnova::eloquent::EloquentModel>::PRIMARY_KEY),
                };
                let fk_ident = format_ident!("{fk}");
                let doc = format!(
                    " The SQL `{target_name}` this document's `{fk}` names, read through the \
                     model's connection."
                );
                (
                    quote! {
                        #[doc = #doc]
                        pub fn #name(&self) -> ::suprnova::mongodb::BelongsToModel<#target> {
                            #checks
                            ::suprnova::mongodb::BelongsToModel::<#target>::__new(
                                #fk,
                                #owner_key,
                                ::suprnova::mongodb::__present_json(&self.#fk_ident, #struct_name, #fk),
                            )
                        }
                    },
                    accessor(relation, &loaded, &name_str),
                )
            }
        };
        quote! { #method #accessor }
    });
    quote! {
        impl #ident {
            #(#methods)*
        }
    }
}

/// The `<name>_loaded` accessor: `None` until `with` loads the relation.
fn accessor(relation: &DocumentRelation, loaded: &Ident, name: &str) -> TokenStream {
    let target = &relation.target;
    if relation.kind.is_many() {
        let doc = format!(
            " The `{name}` that `with([\"{name}\"])` loaded, or `None` when no query loaded them."
        );
        quote! {
            #[doc = #doc]
            pub fn #loaded(&self) -> ::core::option::Option<&[#target]> {
                self.__eager.loaded_many::<#target>(#name)
            }
        }
    } else {
        let doc = format!(
            " The `{name}` that `with([\"{name}\"])` loaded, or `None` when no query loaded it \
             or it found none."
        );
        quote! {
            #[doc = #doc]
            pub fn #loaded(&self) -> ::core::option::Option<&#target> {
                self.__eager.get_one::<#target>(#name)
            }
        }
    }
}

/// The items of the `DocumentModel` impl a document with relations
/// overrides: the relation list, `relation_loaded`, and the eager loader.
pub fn emit_model_items(input: &DocumentInput) -> TokenStream {
    if input.relations.is_empty() {
        return TokenStream::new();
    }
    let names: Vec<String> = input
        .relations
        .iter()
        .map(|relation| relation.name.to_string())
        .collect();
    let arms = input
        .relations
        .iter()
        .map(|relation| eager_arm(input, relation));
    quote! {
        const RELATIONS: &'static [&'static str] = &[#(#names),*];

        fn relation_loaded(&self, relation: &str) -> bool {
            self.__eager.has(relation)
        }

        async fn __eager_load(
            path: &str,
            models: &mut [&mut Self],
        ) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
            let (head, rest) = ::suprnova::mongodb::__split_relation_path(path);
            match head {
                #(#arms)*
                _ => ::core::result::Result::Err(
                    ::suprnova::mongodb::__unknown_relation::<Self>(path),
                ),
            }
        }
    }
}

/// The eager loader's arm for `relation`: one query for every model, then
/// the rest of the path on what it loaded.
fn eager_arm(input: &DocumentInput, relation: &DocumentRelation) -> TokenStream {
    let name = relation.name.to_string();
    let target = &relation.target;
    let (store, take, put_back) = if relation.kind.is_many() {
        (
            quote! { |model, rows| model.__eager.set_many::<#target>(#name, rows) },
            quote! { |model| model.__eager.take_many::<#target>(#name) },
            quote! { |model, rows| model.__eager.set_many::<#target>(#name, rows) },
        )
    } else {
        (
            quote! { |model, rows| model.__eager.set_one::<#target>(#name, rows.into_iter().next()) },
            quote! { |model| model.__eager.take_one::<#target>(#name).map(|row| ::std::vec![row]) },
            quote! { |model, mut rows| model.__eager.set_one::<#target>(#name, rows.pop()) },
        )
    };
    if relation.kind == RelationKind::BelongsToModel {
        let fk = relation.foreign_key.as_str();
        let fk_ident = format_ident!("{fk}");
        let struct_name = input.item.ident.to_string();
        let owner_key = match &relation.local_key {
            Some(column) => quote!(#column),
            None => quote!(<#target as ::suprnova::eloquent::EloquentModel>::PRIMARY_KEY),
        };
        return quote! {
            #name => {
                let keys = models
                    .iter()
                    .map(|model| ::suprnova::mongodb::__present_json(&model.#fk_ident, #struct_name, #fk))
                    .collect::<::core::result::Result<::std::vec::Vec<_>, ::suprnova::FrameworkError>>()?;
                ::suprnova::mongodb::__load_models::<Self, #target>(
                    &mut *models,
                    keys,
                    #owner_key,
                    |model, row| model.__eager.set_one::<#target>(#name, row),
                )
                .await?;
                if let ::core::option::Option::Some(rest) = rest {
                    ::suprnova::mongodb::__load_nested_models::<Self, #target>(
                        &mut *models, #take, #put_back, rest,
                    )
                    .await?;
                }
                ::core::result::Result::Ok(())
            }
        };
    }
    let local = local_value(input, relation, quote!(model));
    let field = match relation.kind {
        RelationKind::BelongsTo => match &relation.local_key {
            Some(field) => quote!(#field),
            None => quote!(<#target as ::suprnova::mongodb::DocumentModel>::KEY_FIELD),
        },
        _ => {
            let fk = relation.foreign_key.as_str();
            quote!(#fk)
        }
    };
    quote! {
        #name => {
            let keys = models
                .iter()
                .map(|model| ::suprnova::mongodb::__present(#local))
                .collect::<::core::result::Result<::std::vec::Vec<_>, ::suprnova::FrameworkError>>()?;
            ::suprnova::mongodb::__load_documents::<Self, #target>(&mut *models, keys, #field, #store)
                .await?;
            if let ::core::option::Option::Some(rest) = rest {
                ::suprnova::mongodb::__load_nested_documents::<Self, #target>(
                    &mut *models, #take, #put_back, rest, false,
                )
                .await?;
            }
            ::core::result::Result::Ok(())
        }
    }
}

/// The hidden field that holds what `with` loaded, added to a document
/// that declares relations.
pub fn cache_field() -> Result<syn::Field> {
    syn::parse::Parser::parse2(
        syn::Field::parse_named,
        quote! {
            /// The relations `with(..)` loaded on this document.
            #[doc(hidden)]
            pub __eager: ::suprnova::EagerLoadCache
        },
    )
}

#[cfg(test)]
mod tests {
    use super::super::parse::DocumentInput;
    use super::*;
    use proc_macro2::TokenStream;
    use quote::quote;

    fn parse(attr: TokenStream, item: TokenStream) -> DocumentInput {
        match DocumentInput::parse(attr, item) {
            Ok(input) => input,
            Err(error) => panic!("expected a parse, got: {error}"),
        }
    }

    fn error(attr: TokenStream, item: TokenStream) -> String {
        match DocumentInput::parse(attr, item) {
            Ok(_) => panic!("expected a parse error"),
            Err(error) => error.to_string(),
        }
    }

    fn writer() -> TokenStream {
        quote!(
            pub struct Writer {
                pub name: String,
                pub user_id: Option<ObjectId>,
                pub account_id: i64,
                pub role_ids: Vec<ObjectId>,
                pub uuid: String,
            }
        )
    }

    #[test]
    fn each_kind_parses_with_the_default_keys() {
        let input = parse(
            quote!(relations = {
                posts: HasMany<Post>,
                profile: HasOne<crate::models::Profile>,
                user: BelongsTo<User>,
                roles: BelongsToMany<Role>,
                account: BelongsToModel<Account>,
            }),
            writer(),
        );
        let relation = |name: &str| {
            input
                .relations
                .iter()
                .find(|relation| relation.name == name)
                .unwrap_or_else(|| panic!("the relation `{name}`"))
        };
        assert_eq!(input.relations.len(), 5);

        let posts = relation("posts");
        assert_eq!(posts.kind, RelationKind::HasMany);
        assert_eq!(posts.foreign_key, "writer_id", "the child names the parent");
        assert_eq!(posts.local_key, None, "the key");

        let profile = relation("profile");
        assert_eq!(profile.kind, RelationKind::HasOne);
        assert_eq!(profile.foreign_key, "writer_id");

        let user = relation("user");
        assert_eq!(user.kind, RelationKind::BelongsTo);
        assert_eq!(user.foreign_key, "user_id", "this document names the owner");
        assert_eq!(user.local_key, None, "the owner's key");

        let roles = relation("roles");
        assert_eq!(roles.kind, RelationKind::BelongsToMany);
        assert_eq!(roles.foreign_key, "writer_ids", "the role's array");
        assert_eq!(roles.local_key.as_deref(), Some("role_ids"), "this array");

        let account = relation("account");
        assert_eq!(account.kind, RelationKind::BelongsToModel);
        assert_eq!(account.foreign_key, "account_id");
        assert_eq!(account.local_key, None, "the SQL model's primary key");

        assert_eq!(
            input
                .relations
                .iter()
                .map(|relation| relation.name.to_string())
                .collect::<Vec<_>>(),
            ["posts", "profile", "user", "roles", "account"],
            "declaration order"
        );
    }

    #[test]
    fn options_name_the_keys() {
        let input = parse(
            quote!(relations = {
                posts: HasMany<Post> { fk = "author_id", lk = "uuid" },
                owner: BelongsTo<User> { fk = "user_id", lk = "uuid" },
                groups: BelongsToMany<Role> {
                    pivot_foreign_key = "members",
                    pivot_related_key = "role_ids",
                },
                account: BelongsToModel<Account> { lk = "number" },
            }),
            writer(),
        );
        let posts = &input.relations[0];
        assert_eq!(posts.foreign_key, "author_id");
        assert_eq!(posts.local_key.as_deref(), Some("uuid"));
        let owner = &input.relations[1];
        assert_eq!(owner.foreign_key, "user_id");
        assert_eq!(owner.local_key.as_deref(), Some("uuid"));
        let groups = &input.relations[2];
        assert_eq!(groups.foreign_key, "members");
        assert_eq!(groups.local_key.as_deref(), Some("role_ids"));
        let account = &input.relations[3];
        assert_eq!(account.foreign_key, "account_id");
        assert_eq!(account.local_key.as_deref(), Some("number"));
    }

    #[test]
    fn a_relation_needs_its_own_keys_as_fields_of_the_document() {
        let message = error(quote!(relations = { team: BelongsTo<Team> }), writer());
        assert!(
            message.contains("team_id") && message.contains("Writer"),
            "{message}"
        );
        let message = error(
            quote!(relations = { account: BelongsToModel<Account> { fk = "acct" } }),
            writer(),
        );
        assert!(message.contains("acct"), "{message}");
        let message = error(quote!(relations = { tags: BelongsToMany<Tag> }), writer());
        assert!(
            message.contains("tag_ids") && message.contains("Writer"),
            "{message}"
        );
        let message = error(
            quote!(relations = { tags: BelongsToMany<Tag> { pivot_related_key = "uuid" } }),
            writer(),
        );
        assert!(
            message.contains("uuid") && message.contains("Vec"),
            "{message}"
        );
        let message = error(
            quote!(relations = { posts: HasMany<Post> { lk = "slug" } }),
            writer(),
        );
        assert!(message.contains("slug"), "{message}");
    }

    #[test]
    fn unknown_kinds_misplaced_options_and_duplicates_are_refused() {
        let message = error(quote!(relations = { posts: MorphMany<Post> }), writer());
        assert!(
            message.contains("MorphMany") && message.contains("BelongsToModel"),
            "the error lists the kinds: {message}"
        );
        let message = error(
            quote!(relations = { posts: HasMany<Post> { pivot_foreign_key = "x" } }),
            writer(),
        );
        assert!(
            message.contains("pivot_foreign_key") && message.contains("HasMany"),
            "{message}"
        );
        let message = error(
            quote!(relations = { roles: BelongsToMany<Role> { fk = "x" } }),
            writer(),
        );
        assert!(
            message.contains("fk") && message.contains("BelongsToMany"),
            "{message}"
        );
        let message = error(
            quote!(relations = { posts: HasMany<Post>, posts: HasOne<Post> }),
            writer(),
        );
        assert!(message.contains("posts"), "{message}");
        let message = error(quote!(relations = { posts: HasMany<Post, Tag> }), writer());
        assert!(message.contains("one"), "{message}");
        let message = error(
            quote!(relations = { addresses: HasMany<Address> }),
            quote!(
                pub struct Writer {
                    #[embeds_many]
                    pub addresses: Vec<Address>,
                }
            ),
        );
        assert!(
            message.contains("addresses") && message.contains("embed"),
            "{message}"
        );
    }

    #[test]
    fn relations_add_the_eager_cache_field_beside_the_stored_fields() {
        let has_cache = |input: &DocumentInput| match &input.item.fields {
            syn::Fields::Named(named) => named
                .named
                .iter()
                .any(|field| field.ident.as_ref().is_some_and(|ident| ident == "__eager")),
            _ => false,
        };
        let input = parse(quote!(relations = { posts: HasMany<Post> }), writer());
        assert!(has_cache(&input), "a document with relations caches them");
        assert!(
            input.fields.iter().all(|field| field.ident != "__eager"),
            "the cache is no stored field"
        );
        let input = parse(quote!(), writer());
        assert!(
            !has_cache(&input),
            "a document without relations has no cache"
        );
        assert!(input.relations.is_empty());
    }
}
