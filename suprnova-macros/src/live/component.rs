use std::collections::BTreeMap;

use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens as _, quote};
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;
use syn::{Data, DeriveInput, Fields, ItemStruct, Type, Visibility};

use super::attrs::{
    ComponentArgs, FieldKind, ModelTimingArgs, UrlModeArgs, contains_reference,
    parse_component_args, parse_field_args,
};
use super::expand::enforce_runtime_path_contract;

pub(crate) fn rewrite_struct_helper(
    args: TokenStream2,
    mut item: ItemStruct,
) -> syn::Result<TokenStream2> {
    item.attrs
        .push(syn::parse_quote!(#[__suprnova_live(#args)]));
    Ok(quote!(#item))
}

pub(crate) fn derive(input: DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            input.generics.span(),
            "Live components cannot be generic",
        ));
    }
    if !matches!(input.vis, Visibility::Public(_)) {
        return Err(syn::Error::new(
            input.ident.span(),
            "Live component types must be public for explicit startup registration",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.ident.span(),
            "LiveComponent can only be derived for a struct",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new(
            data.fields.span(),
            "Live component state must use named fields",
        ));
    };
    let args = parse_component_args(&input.attrs)?;
    let mut generated_fields = Vec::with_capacity(fields.named.len());
    let mut url_keys = BTreeMap::new();
    for field in &fields.named {
        if contains_reference(&field.ty) {
            return Err(syn::Error::new(
                field.ty.span(),
                "Live component state must be owned and cannot contain references or pointers",
            ));
        }
        let field_args = parse_field_args(&field.attrs)?;
        let ident = field.ident.as_ref().expect("named fields have identifiers");
        let name = ident.unraw().to_string();
        let category = field_category_tokens(field_args.kind);
        let codec = field_codec_tokens(&field.ty);
        let model_codec = model_codec_tokens(&field.ty);
        let mut metadata = quote! {
            ::suprnova::live::__private::metadata::FieldMetadata::new(
                ::suprnova::live::__private::identity::ModelField::parse(#name)
                    .expect("macro-validated Live field identity"),
                #category,
                #codec,
                true,
            )
        };
        if let Some(timing) = field_args.timing {
            let timing = timing_tokens(timing);
            metadata = quote! {
                (#metadata).with_model_binding(#model_codec, #timing)?
            };
        }
        if field_args.kind == FieldKind::Session {
            metadata = quote! {
                (#metadata).with_session_binding(#model_codec)?
            };
        }
        if let Some(url) = field_args.url {
            if !url_codec_supported(&field.ty) {
                return Err(syn::Error::new(
                    url.span,
                    "URL-bound state requires a supported scalar codec",
                ));
            }
            let key = url
                .key
                .unwrap_or_else(|| syn::LitStr::new(&name, ident.span()));
            let key_value = key.value();
            if url_keys.insert(key_value, url.span).is_some() {
                return Err(syn::Error::new(url.span, "duplicate Live URL query key"));
            }
            let mode = match url.mode {
                UrlModeArgs::Reflect => quote!(Reflect),
                UrlModeArgs::Navigate => quote!(Navigate),
            };
            let omit_default = url.omit_default;
            let url_category = field_category_tokens(field_args.kind);
            metadata = quote! {
                (#metadata).with_url_binding(
                    ::suprnova::live::__private::state::UrlBinding::new(
                        #key,
                        #url_category,
                        #model_codec,
                        ::suprnova::live::__private::state::UrlBindingMode::#mode,
                        #omit_default,
                    ).expect("macro-validated Live URL binding"),
                )?
            };
        }
        generated_fields.push((name.clone(), metadata));
    }
    generated_fields.sort_by(|left, right| left.0.cmp(&right.0));
    let field_values = generated_fields
        .into_iter()
        .map(|(_, tokens)| tokens)
        .collect::<Vec<_>>();
    expand_definition(&input, &args, &field_values)
}

fn expand_definition(
    input: &DeriveInput,
    args: &ComponentArgs,
    fields: &[TokenStream2],
) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    let name = &args.name;
    let view = &args.view;
    let component_version = args.component_version;
    let state_schema_version = args.state_schema_version;
    let action_schema_version = args.action_schema_version;
    let checker_contract_version = args.checker_contract_version;
    let minimum_protocol_version = args.minimum_protocol_version;
    let refresh_on_promote = args.refresh_on_promote;
    let events = args.events.iter().map(|path| {
        quote!(::suprnova::live::__private::metadata::EventMetadata::from_payload::<#path>()?)
    });
    let effects = args.effects.iter().map(|path| {
        quote!(::suprnova::live::__private::metadata::EffectMetadata::from_payload::<#path>()?)
    });

    let tokens = quote! {
        impl ::suprnova::live::__private::metadata::LiveComponentDefinitionMetadata for #ident {
            fn component_metadata(
                actions: ::std::vec::Vec<::suprnova::live::__private::metadata::ActionMetadata>,
            ) -> ::std::result::Result<
                ::suprnova::live::__private::metadata::ComponentMetadata,
                ::suprnova::live::__private::metadata::MetadataError,
            > {
                let versions = ::suprnova::live::__private::metadata::ContractVersions::new(
                    #component_version,
                    #state_schema_version,
                    #action_schema_version,
                    #checker_contract_version,
                    #minimum_protocol_version,
                )?;
                ::suprnova::live::__private::metadata::ComponentMetadata::new_with_browser_contracts(
                    ::suprnova::live::__private::identity::ComponentName::parse(#name)
                        .expect("macro-validated Live component identity"),
                    ::suprnova::live::__private::identity::ViewName::parse(#view)
                        .expect("macro-validated Live view identity"),
                    versions,
                    ::std::vec![#(#fields),*],
                    actions,
                    ::std::vec![#(#events),*],
                    ::std::vec![#(#effects),*],
                    #refresh_on_promote,
                )
            }
        }
    };
    enforce_runtime_path_contract(&tokens)?;
    Ok(tokens)
}

fn field_category_tokens(kind: FieldKind) -> TokenStream2 {
    let variant = match kind {
        FieldKind::State => quote!(State),
        FieldKind::Public => quote!(Public),
        FieldKind::Model => quote!(Model),
        FieldKind::Locked => quote!(Locked),
        FieldKind::ServerOnly => quote!(ServerOnly),
        FieldKind::Session => quote!(Session),
        FieldKind::Transient => quote!(Transient),
        FieldKind::Secret => quote!(Secret),
    };
    quote!(::suprnova::live::__private::snapshot::state::FieldCategory::#variant)
}

fn field_codec_tokens(ty: &Type) -> TokenStream2 {
    let normalized = ty.to_token_stream().to_string().replace(' ', "");
    let variant = match normalized.as_str() {
        "i64" => quote!(I64Decimal),
        "u64" => quote!(U64Decimal),
        "Vec<u8>" | "std::vec::Vec<u8>" | "::std::vec::Vec<u8>" => quote!(BytesBase64Url),
        _ => quote!(Json),
    };
    quote!(::suprnova::live::__private::snapshot::state::StateCodec::#variant)
}

pub(crate) fn model_codec_tokens(ty: &Type) -> TokenStream2 {
    let Type::Path(path) = ty else {
        return quote!(::suprnova::live::__private::state::ModelCodec::Json);
    };
    let Some(segment) = path.path.segments.last() else {
        return quote!(::suprnova::live::__private::state::ModelCodec::Json);
    };
    let ident = segment.ident.unraw().to_string();
    let scalar = match ident.as_str() {
        "String" => Some(quote!(String)),
        "bool" => Some(quote!(Boolean)),
        "i64" => Some(quote!(I64)),
        "u64" => Some(quote!(U64)),
        "f32" | "f64" => Some(quote!(F64)),
        "Date" => Some(quote!(Date)),
        "OffsetDateTime" => Some(quote!(DateTime)),
        "Uuid" => Some(quote!(Uuid)),
        _ => None,
    };
    if let Some(scalar) = scalar {
        return quote!(::suprnova::live::__private::state::ModelCodec::#scalar);
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return quote!(::suprnova::live::__private::state::ModelCodec::Json);
    };
    let types = arguments.args.iter().filter_map(|argument| match argument {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    });
    let types = types.collect::<Vec<_>>();
    match (ident.as_str(), types.as_slice()) {
        ("Option", [inner]) => model_codec_tokens(inner),
        ("Vec", [inner]) => {
            let inner = model_codec_tokens(inner);
            quote!(::suprnova::live::__private::state::ModelCodec::list(#inner))
        }
        ("BTreeMap" | "HashMap", [_key, value]) => {
            let value = model_codec_tokens(value);
            quote!(::suprnova::live::__private::state::ModelCodec::map(#value))
        }
        _ => quote!(::suprnova::live::__private::state::ModelCodec::Json),
    }
}

fn url_codec_supported(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    let ident = segment.ident.unraw().to_string();
    if matches!(
        ident.as_str(),
        "String" | "bool" | "i64" | "u64" | "Date" | "OffsetDateTime" | "Uuid"
    ) {
        return true;
    }
    if ident != "Option" {
        return false;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    let mut types = arguments.args.iter().filter_map(|argument| match argument {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    });
    let Some(inner) = types.next() else {
        return false;
    };
    types.next().is_none() && url_codec_supported(inner)
}

fn timing_tokens(timing: ModelTimingArgs) -> TokenStream2 {
    match timing {
        ModelTimingArgs::Immediate => {
            quote!(::suprnova::live::__private::state::BindingTiming::Immediate)
        }
        ModelTimingArgs::Change => {
            quote!(::suprnova::live::__private::state::BindingTiming::Change)
        }
        ModelTimingArgs::Blur => {
            quote!(::suprnova::live::__private::state::BindingTiming::Blur)
        }
        ModelTimingArgs::Submit => {
            quote!(::suprnova::live::__private::state::BindingTiming::Submit)
        }
        ModelTimingArgs::Debounce(milliseconds) => quote! {
            ::suprnova::live::__private::state::BindingTiming::debounce(#milliseconds)
                .expect("macro-validated Live debounce")
        },
    }
}
