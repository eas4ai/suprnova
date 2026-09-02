use std::collections::BTreeMap;

use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens as _, format_ident, quote};
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;
use syn::{Data, DeriveInput, Fields, ItemStruct, Type, Visibility};

use super::attrs::{
    ComponentArgs, FieldKind, ModelTimingArgs, StreamModeArgs, StreamReconnectArgs,
    StreamTargetArgs, UrlModeArgs, contains_reference, parse_component_args, parse_field_args,
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
    let mut runtime_fields = Vec::with_capacity(fields.named.len());
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
        if let Some(upload) = field_args.upload {
            let policy = upload.policy;
            metadata = quote! {
                ::suprnova::live::__private::upload::attach_policy(
                    #metadata,
                    #policy(),
                )?
            };
        }
        generated_fields.push((name.clone(), metadata));
        runtime_fields.push(RuntimeField {
            ident: ident.clone(),
            name,
            ty: field.ty.clone(),
            kind: field_args.kind,
            codec,
        });
    }
    generated_fields.sort_by(|left, right| left.0.cmp(&right.0));
    let field_values = generated_fields
        .into_iter()
        .map(|(_, tokens)| tokens)
        .collect::<Vec<_>>();
    expand_definition(&input, &args, &field_values, &runtime_fields)
}

struct RuntimeField {
    ident: syn::Ident,
    name: String,
    ty: Type,
    kind: FieldKind,
    codec: TokenStream2,
}

fn expand_definition(
    input: &DeriveInput,
    args: &ComponentArgs,
    fields: &[TokenStream2],
    runtime_fields: &[RuntimeField],
) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    let view_ident = format_ident!("__SuprnovaLiveViewFor{ident}");
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
    let stream_blocks = args.streams.iter().map(stream_block_tokens);
    let visible_view_fields = runtime_fields
        .iter()
        .filter(|field| field.kind != FieldKind::Secret)
        .map(|field| {
            let ident = &field.ident;
            let ty = &field.ty;
            quote!(#ident: &'__snv_live #ty)
        });
    let visible_view_values = runtime_fields
        .iter()
        .filter(|field| field.kind != FieldKind::Secret)
        .map(|field| {
            let ident = &field.ident;
            quote!(#ident: &self.#ident)
        });
    let default_fields = runtime_fields.iter().map(|field| {
        let ident = &field.ident;
        quote!(#ident: ::std::default::Default::default())
    });
    let hydrated_fields = runtime_fields.iter().map(|field| {
        let ident = &field.ident;
        let ty = &field.ty;
        if matches!(
            field.kind,
            FieldKind::State | FieldKind::Public | FieldKind::Model | FieldKind::Locked
        ) {
            let name = &field.name;
            let codec = &field.codec;
            quote! {
                #ident: ::suprnova::live::__private::component::generated::decode_field::<#ty>(
                    fields.get(#name).ok_or_else(
                        ::suprnova::live::__private::component::ComponentError::contract_failure,
                    )?,
                    #codec,
                )?
            }
        } else {
            quote!(#ident: ::std::default::Default::default())
        }
    });
    let expected_hydrated_fields = runtime_fields
        .iter()
        .filter(|field| {
            matches!(
                field.kind,
                FieldKind::State | FieldKind::Public | FieldKind::Model | FieldKind::Locked
            )
        })
        .count();
    let model_bindings = runtime_fields
        .iter()
        .filter(|field| matches!(field.kind, FieldKind::Model | FieldKind::Transient))
        .map(|field| {
            let ident = &field.ident;
            let name = &field.name;
            if let Some(inner) = option_inner(&field.ty) {
                quote! {
                    let path = ::suprnova::live::__private::state::ModelPath::parse(#name)
                        .map_err(|_| {
                            ::suprnova::live::__private::component::ComponentError::contract_failure()
                        })?;
                    let _application = proposals.apply_optional::<Self, #inner, _>(
                        &path,
                        self,
                        |component, value| component.#ident = value,
                    );
                }
            } else {
                let ty = &field.ty;
                quote! {
                    let path = ::suprnova::live::__private::state::ModelPath::parse(#name)
                        .map_err(|_| {
                            ::suprnova::live::__private::component::ComponentError::contract_failure()
                        })?;
                    let _application = proposals.apply_required::<Self, #ty, _>(
                        &path,
                        self,
                        |component, value| component.#ident = value,
                    );
                }
            }
        });
    let public_dehydration = runtime_fields
        .iter()
        .filter(|field| field.kind == FieldKind::Public)
        .map(dehydrate_field_tokens);
    let instanced_dehydration = runtime_fields
        .iter()
        .filter(|field| {
            matches!(
                field.kind,
                FieldKind::State | FieldKind::Public | FieldKind::Model | FieldKind::Locked
            )
        })
        .map(dehydrate_field_tokens);

    let tokens = quote! {
        #[doc(hidden)]
        #[allow(
            dead_code,
            non_camel_case_types,
            reason = "generated checked Live view is private macro support"
        )]
        #[derive(::suprnova::live::__private::askama::Template)]
        #[template(
            askama = ::suprnova::live::__private::askama,
            path = #view
        )]
        struct #view_ident<'__snv_live> {
            component: &'__snv_live #ident,
            #(#visible_view_fields,)*
        }

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
                #[allow(
                    unused_mut,
                    reason = "components without streams leave the contract vectors untouched"
                )]
                let mut events: ::std::vec::Vec<
                    ::suprnova::live::__private::metadata::EventMetadata,
                > = ::std::vec![#(#events),*];
                #[allow(
                    unused_mut,
                    reason = "components without streams leave the contract vectors untouched"
                )]
                let mut subscriptions: ::std::vec::Vec<
                    ::suprnova::live::__private::async_updates::SubscriptionMetadata,
                > = ::std::vec::Vec::new();
                #(#stream_blocks)*
                ::suprnova::live::__private::metadata::ComponentMetadata::new_with_async_contracts(
                    ::suprnova::live::__private::identity::ComponentName::parse(#name)
                        .expect("macro-validated Live component identity"),
                    ::suprnova::live::__private::identity::ViewName::parse(#view)
                        .expect("macro-validated Live view identity"),
                    versions,
                    ::std::vec![#(#fields),*],
                    actions,
                    events,
                    ::std::vec![#(#effects),*],
                    subscriptions,
                    #refresh_on_promote,
                )
            }
        }

        impl ::suprnova::live::__private::component::generated::GeneratedComponentState
            for #ident
        {
            fn default_mount_state() -> ::std::result::Result<
                Self,
                ::suprnova::live::__private::component::ComponentError,
            > {
                ::std::result::Result::Ok(Self {
                    #(#default_fields,)*
                })
            }

            fn hydrate_state(
                state: &::suprnova::live::__private::canonical::CanonicalValue,
            ) -> ::std::result::Result<
                Self,
                ::suprnova::live::__private::component::ComponentError,
            > {
                let ::suprnova::live::__private::canonical::CanonicalValue::Object(fields) = state
                else {
                    return ::std::result::Result::Err(
                        ::suprnova::live::__private::component::ComponentError::contract_failure(),
                    );
                };
                if fields.len() != #expected_hydrated_fields {
                    return ::std::result::Result::Err(
                        ::suprnova::live::__private::component::ComponentError::contract_failure(),
                    );
                }
                ::std::result::Result::Ok(Self {
                    #(#hydrated_fields,)*
                })
            }

            fn bind_generated_models(
                &mut self,
                proposals: &::suprnova::live::__private::state::ProposalBatch,
            ) -> ::std::result::Result<
                (),
                ::suprnova::live::__private::component::ComponentError,
            > {
                #(#model_bindings)*
                ::std::result::Result::Ok(())
            }

            fn render_generated_view(
                &self,
                _context: &::suprnova::live::__private::component::RenderContext<'_>,
                metadata: &::suprnova::live::__private::metadata::ComponentMetadata,
            ) -> ::std::result::Result<
                ::suprnova::live::__private::view::IslandRender,
                ::suprnova::live::__private::component::ComponentError,
            > {
                let template = #view_ident {
                    component: self,
                    #(#visible_view_values,)*
                };
                ::suprnova::live::__private::component::generated::render_component_view(
                    metadata,
                    &template,
                )
            }

            fn dehydrate_generated_state(
                &self,
                exposure: ::suprnova::live::__private::snapshot::state::StateExposure,
            ) -> ::std::result::Result<
                ::suprnova::live::__private::canonical::CanonicalValue,
                ::suprnova::live::__private::component::ComponentError,
            > {
                let mut fields = ::std::collections::BTreeMap::new();
                match exposure {
                    ::suprnova::live::__private::snapshot::state::StateExposure::PublicSeed => {
                        #(#public_dehydration)*
                    }
                    ::suprnova::live::__private::snapshot::state::StateExposure::Instanced => {
                        #(#instanced_dehydration)*
                    }
                }
                ::std::result::Result::Ok(
                    ::suprnova::live::__private::canonical::CanonicalValue::Object(fields),
                )
            }
        }
    };
    enforce_runtime_path_contract(&tokens)?;
    Ok(tokens)
}

fn dehydrate_field_tokens(field: &RuntimeField) -> TokenStream2 {
    let ident = &field.ident;
    let name = &field.name;
    let codec = &field.codec;
    quote! {
        fields.insert(
            #name.to_owned(),
            ::suprnova::live::__private::component::generated::encode_field(
                &self.#ident,
                #codec,
            )?,
        );
    }
}

fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    match arguments.args.first()? {
        syn::GenericArgument::Type(inner) if arguments.args.len() == 1 => Some(inner),
        _ => None,
    }
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

fn stream_block_tokens(stream: &super::attrs::StreamArgs) -> TokenStream2 {
    let stream_name = &stream.name;
    let topics = stream
        .topics
        .iter()
        .map(|topic| quote!(::suprnova::live::__private::async_updates::TopicName::parse(#topic)?));
    let targets = stream
        .targets
        .iter()
        .map(|target| match target {
            StreamTargetArgs::SelfIsland => {
                quote!(::suprnova::live::__private::async_updates::EventTarget::SelfIsland)
            }
            StreamTargetArgs::Parent => {
                quote!(::suprnova::live::__private::async_updates::EventTarget::Parent)
            }
            StreamTargetArgs::Child => {
                quote!(::suprnova::live::__private::async_updates::EventTarget::Child)
            }
            StreamTargetArgs::Document => {
                quote!(::suprnova::live::__private::async_updates::EventTarget::Document)
            }
        })
        .collect::<Vec<_>>();
    let fanout = stream.fanout;
    let stream_events = stream.events.iter().map(|path| {
        let targets = targets.iter();
        quote! {
            ::suprnova::live::__private::metadata::EventMetadata::from_payload_with_contract::<#path>(
                ::suprnova::live::__private::async_updates::EventSource::Stream,
                ::suprnova::live::__private::async_updates::BoundedTargets::new(
                    ::std::vec![#(#targets),*],
                )?,
                ::suprnova::live::__private::async_updates::EventOrder::PerSourceSequence,
                ::suprnova::live::__private::async_updates::EventCyclePolicy::ForbidRepeatedIsland,
                #fanout,
            )?
        }
    });
    let modes = stream.modes.iter().map(|mode| match mode {
        StreamModeArgs::ServerSentEvents => {
            quote!(::suprnova::live::__private::async_updates::SubscriptionMode::ServerSentEvents)
        }
        StreamModeArgs::WebSocket => {
            quote!(::suprnova::live::__private::async_updates::SubscriptionMode::WebSocket)
        }
    });
    let reconnect = match stream.reconnect {
        StreamReconnectArgs::RefreshOnReconnect => {
            quote!(::suprnova::live::__private::async_updates::ReconnectPolicy::RefreshOnReconnect)
        }
        StreamReconnectArgs::ResumeOrRefresh { maximum_attempts } => quote! {
            ::suprnova::live::__private::async_updates::ReconnectPolicy::ResumeOrRefresh {
                maximum_attempts: ::core::num::NonZeroU8::new(#maximum_attempts)
                    .expect("macro-validated stream resume attempts"),
            }
        },
    };
    quote! {
        {
            let stream_events: ::std::vec::Vec<
                ::suprnova::live::__private::metadata::EventMetadata,
            > = ::std::vec![#(#stream_events),*];
            subscriptions.push(
                ::suprnova::live::__private::async_updates::SubscriptionMetadata::new(
                    ::suprnova::live::__private::async_updates::StreamName::parse(#stream_name)?,
                    ::suprnova::live::__private::async_updates::BoundedTopics::new(
                        ::std::vec![#(#topics),*],
                    )?,
                    ::suprnova::live::__private::async_updates::BoundedEventNames::new(
                        stream_events
                            .iter()
                            .map(|event| event.name().clone())
                            .collect(),
                    )?,
                    ::suprnova::live::__private::async_updates::SubscriptionModes::new(
                        ::std::vec![#(#modes),*],
                    )?,
                    #reconnect,
                ),
            );
            events.extend(stream_events);
        }
    }
}
