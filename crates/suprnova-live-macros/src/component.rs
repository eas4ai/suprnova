use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens as _, quote};
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;
use syn::{Data, DeriveInput, Fields, ItemStruct, Type, Visibility};

use crate::attrs::{
    ComponentArgs, FieldKind, contains_reference, parse_component_args, parse_field_args,
};
use crate::expand::enforce_runtime_path_contract;

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
        let _url_is_reserved_for_task_4 = field_args.url;
        generated_fields.push((
            name.clone(),
            quote! {
                ::suprnova::live::__private::metadata::FieldMetadata::new(
                    ::suprnova::live::__private::identity::ModelField::parse(#name)
                        .expect("macro-validated Live field identity"),
                    #category,
                    #codec,
                    true,
                )
            },
        ));
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
