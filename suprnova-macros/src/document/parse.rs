//! Parses `#[suprnova::document(...)]` and the struct it annotates.
//!
//! The attribute keeps the `#[model]` grammar where the two overlap:
//! `fillable`, `guarded` and `casts` go through the model parser's own
//! functions, and `primary_key`, `connection`, `timestamps`, `created_at`,
//! `updated_at`, `soft_deletes`, `soft_deletes_column`, `hidden` and
//! `visible` are spelled as on a model. `collection` takes the place of
//! `table`.

use proc_macro2::{Span, TokenStream};
use syn::parse::{Parse, ParseStream, Parser};
use syn::{
    Field, Fields, GenericArgument, Ident, ItemStruct, LitBool, LitStr, PathArguments, Result,
    Token, Type, parse2,
};

use crate::model::parse::{parse_casts_map, parse_str_array, pluralize_snake};

/// How an `#[embeds_one]` or `#[embeds_many]` field holds its documents.
#[derive(Debug, Clone)]
pub enum Embed {
    /// `#[embeds_one]` on an `Option<T>`: `T` is the embedded type.
    One(Type),
    /// `#[embeds_many]` on a `Vec<T>`: `T` is the embedded type.
    Many(Type),
}

/// One field of the document model.
#[derive(Debug, Clone)]
pub struct DocumentField {
    pub ident: Ident,
    pub ty: Type,
    /// The cast that writes and reads the field, declared in `casts` or
    /// chosen for a date-time or decimal type.
    pub cast: Option<Type>,
    pub embed: Option<Embed>,
    /// A missing `Vec` reads as an empty array.
    pub empty_when_missing: bool,
}

/// The parsed attribute and struct.
pub struct DocumentInput {
    /// The struct as emitted: the embed markers removed and, when no field
    /// is the key, an `id: ObjectId` field added first.
    pub item: ItemStruct,
    pub collection: String,
    pub connection: Option<String>,
    pub key: Ident,
    pub key_type: Type,
    /// Every field, the key first when the macro added it.
    pub fields: Vec<DocumentField>,
    pub fillable: Option<Vec<String>>,
    pub guarded: Option<Vec<String>>,
    pub timestamps: Option<(String, String)>,
    pub soft_deletes: Option<String>,
    pub hidden: Vec<String>,
    pub visible: Option<Vec<String>>,
}

impl DocumentInput {
    /// Parse the attribute arguments `attr` and the struct `item`.
    pub fn parse(attr: TokenStream, item: TokenStream) -> Result<Self> {
        let mut item: ItemStruct = parse2(item)?;
        let attrs: DocumentAttrs = parse2(attr)?;
        let struct_name = item.ident.to_string();

        if !item.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &item.generics,
                "a document model cannot be generic: the macro stores one struct in one collection",
            ));
        }
        let Fields::Named(named) = &mut item.fields else {
            return Err(syn::Error::new_spanned(
                &item.ident,
                "a document model needs named fields: each field is one field of the document",
            ));
        };

        if attrs.fillable.is_some() && attrs.guarded.is_some() {
            return Err(syn::Error::new(
                Span::call_site(),
                "cannot specify both `fillable` and `guarded` on the same document model",
            ));
        }
        if attrs
            .hidden
            .as_ref()
            .is_some_and(|hidden| !hidden.is_empty())
            && attrs
                .visible
                .as_ref()
                .is_some_and(|visible| !visible.is_empty())
        {
            return Err(syn::Error::new(
                Span::call_site(),
                "cannot specify both `hidden` and `visible` on the same document model",
            ));
        }

        // The embed markers are the macro's, not real attributes: read them
        // and take them off the field.
        let mut embeds: Vec<(Ident, Option<Embed>)> = Vec::new();
        for field in named.named.iter_mut() {
            let ident = field_ident(field)?;
            let embed = take_embed(field)?;
            embeds.push((ident, embed));
        }

        // The key: the field `primary_key` names, else `id`, else an
        // `ObjectId` the macro adds.
        let key_name = attrs
            .primary_key
            .as_ref()
            .map_or_else(|| "id".to_owned(), LitStr::value);
        let declared_key = named
            .named
            .iter()
            .find(|field| field.ident.as_ref().is_some_and(|ident| ident == &key_name));
        let (key, key_type) = match (declared_key, &attrs.primary_key) {
            (Some(field), _) => (field_ident(field)?, field.ty.clone()),
            (None, Some(name)) => {
                return Err(syn::Error::new(
                    name.span(),
                    format!(
                        "`primary_key = \"{}\"` names no field of `{struct_name}`",
                        name.value()
                    ),
                ));
            }
            (None, None) => {
                let field: Field = syn::Field::parse_named.parse2(quote::quote! {
                    /// The document's key, stored as `_id`: an `ObjectId` the
                    /// model generates when it is made.
                    pub id: ::suprnova::bson::oid::ObjectId
                })?;
                named.named.insert(0, field);
                embeds.insert(0, (Ident::new("id", Span::call_site()), None));
                (
                    Ident::new("id", Span::call_site()),
                    syn::parse_quote!(::suprnova::bson::oid::ObjectId),
                )
            }
        };

        let field_names: Vec<String> = embeds.iter().map(|(ident, _)| ident.to_string()).collect();
        let is_field = |name: &str| field_names.iter().any(|field| field == name);

        let check_names = |list: &Option<Vec<String>>, what: &str| -> Result<()> {
            for name in list.iter().flatten() {
                if !is_field(name) {
                    return Err(syn::Error::new(
                        Span::call_site(),
                        format!("`{what}` names `{name}`, which is no field of `{struct_name}`"),
                    ));
                }
            }
            Ok(())
        };
        check_names(&attrs.fillable, "fillable")?;
        check_names(&attrs.guarded, "guarded")?;
        check_names(&attrs.hidden, "hidden")?;
        check_names(&attrs.visible, "visible")?;

        let declared_casts = attrs.casts.clone().unwrap_or_default();
        for (field, _) in &declared_casts {
            if !is_field(&field.to_string()) {
                return Err(syn::Error::new(
                    field.span(),
                    format!("`casts` names `{field}`, which is no field of `{struct_name}`"),
                ));
            }
            if *field == key {
                return Err(syn::Error::new(
                    field.span(),
                    "the key takes no cast: it is written and read as its own type",
                ));
            }
        }

        let mut fields = Vec::new();
        for (field, (_, embed)) in named.named.iter().zip(embeds) {
            let ident = field_ident(field)?;
            let embed = match embed {
                None => None,
                Some(Embed::One(_)) => match option_inner(&field.ty) {
                    Some(inner) => Some(Embed::One(inner.clone())),
                    None => {
                        return Err(syn::Error::new_spanned(
                            &field.ty,
                            format!(
                                "`#[embeds_one]` needs an `Option<T>` field, so `{ident}` can \
                                 hold no document"
                            ),
                        ));
                    }
                },
                Some(Embed::Many(_)) => match vec_inner(&field.ty) {
                    Some(inner) => Some(Embed::Many(inner.clone())),
                    None => {
                        return Err(syn::Error::new_spanned(
                            &field.ty,
                            format!(
                                "`#[embeds_many]` needs a `Vec<T>` field: `{ident}` is not one"
                            ),
                        ));
                    }
                },
            };
            if embed.is_some() && ident == key {
                return Err(syn::Error::new(
                    ident.span(),
                    "the key cannot be an embedded document",
                ));
            }
            let declared = declared_casts
                .iter()
                .find(|(name, _)| *name == ident)
                .map(|(_, cast)| cast.clone());
            let cast = if ident == key {
                None
            } else {
                declared.or_else(|| default_cast(&field.ty))
            };
            fields.push(DocumentField {
                empty_when_missing: vec_inner(&field.ty).is_some(),
                ident,
                ty: field.ty.clone(),
                cast,
                embed,
            });
        }

        let field_type = |name: &str| {
            fields
                .iter()
                .find(|field| field.ident == name)
                .map(|field| field.ty.clone())
        };

        // Timestamps, as on `#[model]`: on when both fields exist, off when
        // neither does, an error when only one does.
        let created_at = attrs
            .created_at
            .clone()
            .unwrap_or_else(|| "created_at".into());
        let updated_at = attrs
            .updated_at
            .clone()
            .unwrap_or_else(|| "updated_at".into());
        let timestamps = if attrs.timestamps.unwrap_or(true) {
            match (field_type(&created_at), field_type(&updated_at)) {
                (Some(created), Some(updated)) => {
                    for (name, ty) in [(&created_at, &created), (&updated_at, &updated)] {
                        if !is_date_time(unwrap_option(ty)) {
                            return Err(syn::Error::new_spanned(
                                ty,
                                format!(
                                    "the timestamp `{name}` must be a date-time: \
                                     `bson::DateTime` or `chrono::DateTime<Utc>`, or either in \
                                     an `Option`"
                                ),
                            ));
                        }
                    }
                    Some((created_at, updated_at))
                }
                (None, None) => None,
                _ => {
                    return Err(syn::Error::new_spanned(
                        &item.ident,
                        format!(
                            "the model has only one of `{created_at}` and `{updated_at}`: managed \
                             timestamps need both fields, or set `timestamps = false`"
                        ),
                    ));
                }
            }
        } else {
            None
        };

        let soft_deletes = if attrs.soft_deletes.unwrap_or(false) {
            let column = attrs
                .soft_deletes_column
                .clone()
                .unwrap_or_else(|| "deleted_at".into());
            match field_type(&column) {
                Some(ty) if option_inner(&ty).is_some_and(is_date_time) => Some(column),
                Some(ty) => {
                    return Err(syn::Error::new_spanned(
                        ty,
                        format!(
                            "the soft-delete field `{column}` must be an optional date-time: \
                             `Option<bson::DateTime>` or `Option<chrono::DateTime<Utc>>`"
                        ),
                    ));
                }
                None => {
                    return Err(syn::Error::new_spanned(
                        &item.ident,
                        format!(
                            "`soft_deletes` needs the field `{column}`, an \
                             `Option<bson::DateTime>` or `Option<chrono::DateTime<Utc>>`"
                        ),
                    ));
                }
            }
        } else {
            None
        };

        Ok(Self {
            collection: attrs
                .collection
                .unwrap_or_else(|| pluralize_snake(&struct_name)),
            connection: attrs.connection,
            key,
            key_type,
            fields,
            fillable: attrs.fillable,
            guarded: attrs.guarded,
            timestamps,
            soft_deletes,
            hidden: attrs.hidden.unwrap_or_default(),
            visible: attrs.visible,
            item,
        })
    }
}

fn field_ident(field: &Field) -> Result<Ident> {
    field
        .ident
        .clone()
        .ok_or_else(|| syn::Error::new_spanned(field, "a document model needs named fields"))
}

/// Read and remove the `#[embeds_one]` or `#[embeds_many]` marker of
/// `field`. The type is checked later; the variant carries a placeholder.
fn take_embed(field: &mut Field) -> Result<Option<Embed>> {
    let mut found: Option<Embed> = None;
    let mut kept = Vec::with_capacity(field.attrs.len());
    for attr in field.attrs.drain(..) {
        let kind = if attr.path().is_ident("embeds_one") {
            Some(Embed::One(field.ty.clone()))
        } else if attr.path().is_ident("embeds_many") {
            Some(Embed::Many(field.ty.clone()))
        } else {
            None
        };
        match kind {
            None => kept.push(attr),
            Some(kind) => {
                if !matches!(attr.meta, syn::Meta::Path(_)) {
                    return Err(syn::Error::new_spanned(
                        &attr,
                        "`#[embeds_one]` and `#[embeds_many]` take no arguments",
                    ));
                }
                if found.is_some() {
                    return Err(syn::Error::new_spanned(
                        &attr,
                        "a field embeds one document or many, not both",
                    ));
                }
                found = Some(kind);
            }
        }
    }
    field.attrs = kept;
    Ok(found)
}

/// The last path segment of `ty` and its one generic type argument, when
/// it has exactly one.
fn single_argument<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != name {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    match arguments.args.iter().collect::<Vec<_>>().as_slice() {
        [GenericArgument::Type(inner)] => Some(inner),
        _ => None,
    }
}

/// `T` of `Option<T>`.
pub fn option_inner(ty: &Type) -> Option<&Type> {
    single_argument(ty, "Option")
}

/// `T` of `Vec<T>`.
pub fn vec_inner(ty: &Type) -> Option<&Type> {
    single_argument(ty, "Vec")
}

fn unwrap_option(ty: &Type) -> &Type {
    option_inner(ty).unwrap_or(ty)
}

fn last_segment(ty: &Type) -> Option<&syn::PathSegment> {
    match ty {
        Type::Path(path) => path.path.segments.last(),
        _ => None,
    }
}

/// `chrono::DateTime<Utc>`: a `DateTime` whose time-zone argument is
/// `Utc`. Another zone has no BSON datetime form the cast reads back, so it
/// keeps serde's string.
fn is_chrono_date_time(ty: &Type) -> bool {
    single_argument(ty, "DateTime")
        .and_then(last_segment)
        .is_some_and(|zone| zone.ident == "Utc")
}

/// `bson::DateTime`: a `DateTime` without arguments.
fn is_bson_date_time(ty: &Type) -> bool {
    last_segment(ty).is_some_and(|segment| {
        segment.ident == "DateTime" && matches!(segment.arguments, PathArguments::None)
    })
}

fn is_date_time(ty: &Type) -> bool {
    is_chrono_date_time(ty) || is_bson_date_time(ty)
}

/// `rust_decimal::Decimal`.
fn is_decimal(ty: &Type) -> bool {
    last_segment(ty).is_some_and(|segment| {
        segment.ident == "Decimal" && matches!(segment.arguments, PathArguments::None)
    })
}

/// The cast a field of type `ty` gets without one declared: a chrono date
/// time is stored as a BSON datetime and a decimal as a `Decimal128`, the
/// BSON types for them. Every other type goes through serde as it is.
fn default_cast(ty: &Type) -> Option<Type> {
    let inner = unwrap_option(ty);
    if is_chrono_date_time(inner) {
        Some(syn::parse_quote!(::suprnova::mongodb::AsBsonDateTime))
    } else if is_decimal(inner) {
        Some(syn::parse_quote!(::suprnova::mongodb::AsDecimal128))
    } else {
        None
    }
}

/// The attribute's `name = value` list.
#[derive(Default)]
struct DocumentAttrs {
    collection: Option<String>,
    connection: Option<String>,
    primary_key: Option<LitStr>,
    fillable: Option<Vec<String>>,
    guarded: Option<Vec<String>>,
    casts: Option<Vec<(Ident, Type)>>,
    timestamps: Option<bool>,
    created_at: Option<String>,
    updated_at: Option<String>,
    soft_deletes: Option<bool>,
    soft_deletes_column: Option<String>,
    hidden: Option<Vec<String>>,
    visible: Option<Vec<String>>,
}

impl Parse for DocumentAttrs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut out = DocumentAttrs::default();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let name = key.to_string();
            let is_flag = input.is_empty() || input.peek(Token![,]);
            match (name.as_str(), is_flag) {
                ("soft_deletes", true) => out.soft_deletes = Some(true),
                ("timestamps", true) => out.timestamps = Some(true),
                (_, true) => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("`{name}` needs a value: `{name} = ...`"),
                    ));
                }
                (_, false) => {
                    input.parse::<Token![=]>()?;
                    match name.as_str() {
                        "collection" => out.collection = Some(identifier_string(input, &name)?),
                        "connection" => out.connection = Some(input.parse::<LitStr>()?.value()),
                        "primary_key" => {
                            let literal = input.parse::<LitStr>()?;
                            if syn::parse_str::<Ident>(&literal.value()).is_err() {
                                return Err(syn::Error::new(
                                    literal.span(),
                                    "`primary_key` names a field: it must be a Rust identifier",
                                ));
                            }
                            out.primary_key = Some(literal);
                        }
                        "fillable" => out.fillable = Some(parse_str_array(input)?),
                        "guarded" => out.guarded = Some(parse_str_array(input)?),
                        "casts" => out.casts = Some(parse_casts_map(input)?),
                        "timestamps" => out.timestamps = Some(input.parse::<LitBool>()?.value),
                        "created_at" => out.created_at = Some(input.parse::<LitStr>()?.value()),
                        "updated_at" => out.updated_at = Some(input.parse::<LitStr>()?.value()),
                        "soft_deletes" => out.soft_deletes = Some(input.parse::<LitBool>()?.value),
                        "soft_deletes_column" => {
                            out.soft_deletes_column = Some(input.parse::<LitStr>()?.value())
                        }
                        "hidden" => out.hidden = Some(parse_str_array(input)?),
                        "visible" => out.visible = Some(parse_str_array(input)?),
                        other => {
                            return Err(syn::Error::new(
                                key.span(),
                                format!("unknown `#[document]` attribute: `{other}`"),
                            ));
                        }
                    }
                }
            }
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(out)
    }
}

/// A collection name: not empty, and without `$` or a NUL, which MongoDB
/// refuses in collection names.
fn identifier_string(input: ParseStream, name: &str) -> Result<String> {
    let literal = input.parse::<LitStr>()?;
    let value = literal.value();
    if value.is_empty() || value.contains('$') || value.contains('\0') {
        return Err(syn::Error::new(
            literal.span(),
            format!("`{name}` must be a collection name: not empty, without `$` or NUL"),
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn parse(attr: TokenStream, item: TokenStream) -> Result<DocumentInput> {
        DocumentInput::parse(attr, item)
    }

    fn error(attr: TokenStream, item: TokenStream) -> String {
        match parse(attr, item) {
            Ok(_) => panic!("expected a parse error"),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn a_struct_without_a_key_gets_an_object_id_named_id() {
        let input = parse(
            quote!(collection = "users"),
            quote!(
                pub struct User {
                    pub name: String,
                }
            ),
        )
        .expect("parse");
        assert_eq!(input.key, "id");
        assert_eq!(input.fields[0].ident, "id");
        assert_eq!(input.fields.len(), 2);
        assert_eq!(input.collection, "users");
        assert!(input.timestamps.is_none());
    }

    #[test]
    fn the_collection_defaults_to_the_plural_snake_case_name() {
        let input = parse(
            quote!(),
            quote!(
                pub struct BlogPost {
                    pub title: String,
                }
            ),
        )
        .expect("parse");
        assert_eq!(input.collection, "blog_posts");
    }

    #[test]
    fn primary_key_names_the_key_field() {
        let input = parse(
            quote!(primary_key = "sku"),
            quote!(
                pub struct Product {
                    pub sku: String,
                    pub title: String,
                }
            ),
        )
        .expect("parse");
        assert_eq!(input.key, "sku");
        assert_eq!(input.fields.len(), 2, "no field is added");
        let message = error(
            quote!(primary_key = "code"),
            quote!(
                pub struct Product {
                    pub sku: String,
                }
            ),
        );
        assert!(message.contains("code"), "{message}");
    }

    #[test]
    fn fillable_and_guarded_together_are_refused() {
        let message = error(
            quote!(fillable = ["name"], guarded = ["name"]),
            quote!(
                pub struct User {
                    pub name: String,
                }
            ),
        );
        assert!(
            message.contains("fillable") && message.contains("guarded"),
            "{message}"
        );
    }

    #[test]
    fn a_list_naming_no_field_is_refused_naming_it() {
        let message = error(
            quote!(hidden = ["pasword"]),
            quote!(
                pub struct User {
                    pub password: String,
                }
            ),
        );
        assert!(message.contains("pasword"), "{message}");
    }

    #[test]
    fn embeds_need_option_and_vec_fields() {
        let input = parse(
            quote!(),
            quote!(
                pub struct User {
                    #[embeds_one]
                    pub profile: Option<Profile>,
                    #[embeds_many]
                    pub addresses: Vec<Address>,
                }
            ),
        )
        .expect("parse");
        assert!(matches!(input.fields[1].embed, Some(Embed::One(_))));
        assert!(matches!(input.fields[2].embed, Some(Embed::Many(_))));
        assert!(input.fields[2].empty_when_missing);
        let syn::Fields::Named(named) = &input.item.fields else {
            panic!("named fields")
        };
        assert!(
            named
                .named
                .iter()
                .all(|field| field.attrs.iter().all(|attr| {
                    !attr.path().is_ident("embeds_one") && !attr.path().is_ident("embeds_many")
                })),
            "the markers are removed"
        );

        let message = error(
            quote!(),
            quote!(
                pub struct User {
                    #[embeds_many]
                    pub addresses: Option<Address>,
                }
            ),
        );
        assert!(message.contains("Vec"), "{message}");
        let message = error(
            quote!(),
            quote!(
                pub struct User {
                    #[embeds_one]
                    pub profile: Profile,
                }
            ),
        );
        assert!(message.contains("Option"), "{message}");
    }

    #[test]
    fn timestamps_need_both_fields_as_date_times() {
        let input = parse(
            quote!(),
            quote!(
                pub struct User {
                    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
                    pub updated_at: Option<bson::DateTime>,
                }
            ),
        )
        .expect("parse");
        assert_eq!(
            input.timestamps,
            Some(("created_at".to_owned(), "updated_at".to_owned()))
        );
        let message = error(
            quote!(),
            quote!(
                pub struct User {
                    pub created_at: Option<bson::DateTime>,
                }
            ),
        );
        assert!(message.contains("updated_at"), "{message}");
        let message = error(
            quote!(),
            quote!(
                pub struct User {
                    pub created_at: String,
                    pub updated_at: String,
                }
            ),
        );
        assert!(message.contains("date-time"), "{message}");
        let input = parse(
            quote!(timestamps = false),
            quote!(
                pub struct User {
                    pub created_at: Option<bson::DateTime>,
                }
            ),
        )
        .expect("off");
        assert!(input.timestamps.is_none());
    }

    #[test]
    fn soft_deletes_need_an_optional_date_time_field() {
        let input = parse(
            quote!(soft_deletes),
            quote!(
                pub struct User {
                    pub deleted_at: Option<bson::DateTime>,
                }
            ),
        )
        .expect("parse");
        assert_eq!(input.soft_deletes.as_deref(), Some("deleted_at"));
        let message = error(
            quote!(soft_deletes),
            quote!(
                pub struct User {
                    pub name: String,
                }
            ),
        );
        assert!(message.contains("deleted_at"), "{message}");
        let message = error(
            quote!(soft_deletes),
            quote!(
                pub struct User {
                    pub deleted_at: bson::DateTime,
                }
            ),
        );
        assert!(message.contains("optional"), "{message}");
    }

    #[test]
    fn date_times_and_decimals_get_their_bson_casts() {
        let input = parse(
            quote!(casts = { note = MyCast }),
            quote!(
                pub struct User {
                    pub joined: Option<chrono::DateTime<chrono::Utc>>,
                    pub local: chrono::DateTime<chrono::FixedOffset>,
                    pub balance: rust_decimal::Decimal,
                    pub seen: bson::DateTime,
                    pub note: String,
                }
            ),
        )
        .expect("parse");
        let cast = |name: &str| {
            input
                .fields
                .iter()
                .find(|field| field.ident == name)
                .and_then(|field| field.cast.as_ref())
                .map(|cast| quote!(#cast).to_string().replace(' ', ""))
        };
        assert_eq!(
            cast("joined").as_deref(),
            Some("::suprnova::mongodb::AsBsonDateTime")
        );
        assert_eq!(
            cast("balance").as_deref(),
            Some("::suprnova::mongodb::AsDecimal128")
        );
        assert_eq!(cast("seen"), None, "a BSON datetime needs no cast");
        assert_eq!(cast("local"), None, "another zone keeps serde's form");
        assert_eq!(cast("note").as_deref(), Some("MyCast"));
        assert_eq!(cast("id"), None);
    }

    #[test]
    fn unknown_attributes_generics_and_tuple_structs_are_refused() {
        let message = error(
            quote!(table = "users"),
            quote!(
                pub struct User {
                    pub name: String,
                }
            ),
        );
        assert!(message.contains("table"), "{message}");
        let message = error(
            quote!(),
            quote!(
                pub struct User<T> {
                    pub name: T,
                }
            ),
        );
        assert!(message.contains("generic"), "{message}");
        let message = error(
            quote!(),
            quote!(
                pub struct User(String);
            ),
        );
        assert!(message.contains("named fields"), "{message}");
        let message = error(
            quote!(collection = ""),
            quote!(
                pub struct User {
                    pub name: String,
                }
            ),
        );
        assert!(message.contains("collection"), "{message}");
    }
}
