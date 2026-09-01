use std::collections::BTreeMap;

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;
use syn::{
    Attribute, FnArg, GenericArgument, ImplItem, ImplItemFn, ItemImpl, Meta, Pat, PathArguments,
    Receiver, Type, Visibility,
};

use super::attrs::{
    ActionAuthorizationArgs, ActionTransactionArgs, ActionValidationArgs, contains_reference,
    is_field_helper, is_method_helper, parse_action_args, parse_validation_hook_args,
    validate_registered_name,
};
use super::component::model_codec_tokens;
use super::expand::enforce_runtime_path_contract;

pub(crate) fn expand(args: TokenStream2, mut item: ItemImpl) -> syn::Result<TokenStream2> {
    if !args.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "the outer #[live] impl attribute does not accept arguments",
        ));
    }
    if item.trait_.is_some() {
        return Err(syn::Error::new(
            item.impl_token.span(),
            "#[live] requires an inherent impl",
        ));
    }
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            item.generics.span(),
            "a Live component impl cannot be generic",
        ));
    }

    let mut actions = BTreeMap::<String, RegisteredAction>::new();
    let mut singleton_helpers = BTreeMap::<String, Span>::new();
    let mut mount = None::<RegisteredMount>;
    let mut lifecycle_hooks = BTreeMap::<String, RegisteredLifecycleHook>::new();
    let mut computed_methods = Vec::<RegisteredComputed>::new();
    let mut supports_params_changed = false;
    let mut supports_lazy_complete = false;
    let mut component_validation_hooks = Vec::new();
    let mut action_validation_hooks = BTreeMap::new();
    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            reject_helpers_on_non_method(impl_item)?;
            continue;
        };
        let helper = take_method_helper(method)?;
        let Some((helper_name, attribute)) = helper else {
            continue;
        };
        validate_common_signature(method)?;
        match helper_name.as_str() {
            "action" => {
                validate_action_signature(method)?;
                method.attrs.push(syn::parse_quote!(
                    #[doc = "Live may invoke this action body again after an uncommitted attempt. Nontransactional external effects require application idempotency, compensation, or an established outbox/delivery contract."]
                ));
                let args = parse_action_args(&attribute)?;
                let name = args
                    .name
                    .as_ref()
                    .map_or_else(|| method.sig.ident.unraw().to_string(), syn::LitStr::value);
                let literal = args
                    .name
                    .unwrap_or_else(|| syn::LitStr::new(&name, method.sig.ident.span()));
                validate_registered_name(&literal)?;
                let (authorization_parameter, arguments) = extract_action_parameters(method)?;
                let registered = RegisteredAction {
                    version: args.version,
                    method: method.sig.ident.clone(),
                    arguments,
                    asynchronous: method.sig.asyncness.is_some(),
                    authorization: args.authorization,
                    validation: args.validation,
                    transaction: args.transaction,
                    authorization_parameter,
                };
                if actions.insert(name, registered).is_some() {
                    return Err(syn::Error::new(
                        attribute.span(),
                        "duplicate registered Live action name",
                    ));
                }
            }
            "mount" => {
                ensure_singleton(&mut singleton_helpers, &helper_name, attribute.span())?;
                validate_mount_signature(method)?;
                mount = Some(RegisteredMount {
                    method: method.sig.ident.clone(),
                    parameters: extract_mount_parameters(method)?,
                    asynchronous: method.sig.asyncness.is_some(),
                });
                ensure_path_helper(&attribute)?;
            }
            "computed" => {
                validate_computed_signature(method)?;
                computed_methods.push(extract_computed(method)?);
                ensure_path_helper(&attribute)?;
            }
            "validate" => {
                validate_receiver_method(method, false)?;
                let hook = parse_validation_hook_args(&attribute)?;
                if let Some(action) = hook.action {
                    let action_name = action.value();
                    let arguments = extract_validation_parameters(method)?;
                    let hook = RegisteredValidationHook {
                        method: method.sig.ident.clone(),
                        arguments,
                        asynchronous: method.sig.asyncness.is_some(),
                        span: attribute.span(),
                    };
                    if action_validation_hooks.insert(action_name, hook).is_some() {
                        return Err(syn::Error::new(
                            action.span(),
                            "an action may declare only one typed validation hook",
                        ));
                    }
                } else {
                    if method.sig.inputs.len() != 1 {
                        return Err(syn::Error::new(
                            method.sig.ident.span(),
                            "component validation methods accept only `&self` or `&mut self`",
                        ));
                    }
                    component_validation_hooks
                        .push((method.sig.ident.clone(), method.sig.asyncness.is_some()));
                }
            }
            _ => {
                ensure_singleton(&mut singleton_helpers, &helper_name, attribute.span())?;
                validate_receiver_method(method, true)?;
                if method.sig.inputs.len() != 1 {
                    return Err(syn::Error::new(
                        method.sig.ident.span(),
                        "Live lifecycle hooks accept only `&mut self`",
                    ));
                }
                ensure_path_helper(&attribute)?;
                supports_params_changed |= helper_name == "params_changed";
                supports_lazy_complete |= helper_name == "lazy_complete";
                lifecycle_hooks.insert(
                    helper_name,
                    RegisteredLifecycleHook {
                        method: method.sig.ident.clone(),
                        asynchronous: method.sig.asyncness.is_some(),
                    },
                );
            }
        }
    }

    validate_validation_hooks(
        &actions,
        &component_validation_hooks,
        &action_validation_hooks,
    )?;

    let self_ty = &item.self_ty;
    let action_values = actions
        .iter()
        .map(|(name, action)| {
            let version = action.version;
            let authorization = match action.authorization {
                ActionAuthorizationArgs::Public => quote!(Public),
                ActionAuthorizationArgs::Current => quote!(Current),
            };
            let validation = match action.validation {
                ActionValidationArgs::None => quote!(None),
                ActionValidationArgs::Whole => quote!(WholeComponent),
                ActionValidationArgs::Arguments => quote!(ActionArguments),
                ActionValidationArgs::All => quote!(ComponentAndArguments),
            };
            let transaction = match action.transaction {
                ActionTransactionArgs::None => quote!(None),
                ActionTransactionArgs::Required => quote!(Required),
            };
            let argument_fields = action.arguments.iter().map(|argument| {
                let name = argument.name.to_string();
                let codec = model_codec_tokens(&argument.ty);
                let required = argument.required;
                quote! {
                    ::suprnova::live::__private::action::ActionArgumentField::new(
                        ::suprnova::live::__private::identity::ModelField::parse(#name)
                            .expect("macro-validated Live action argument identity"),
                        #codec,
                        #required,
                    ).expect("macro-validated Live action argument field")
                }
            });
            quote! {
                ::suprnova::live::__private::metadata::ActionMetadata::new_with_contract(
                    ::suprnova::live::__private::identity::ActionName::parse(#name)
                        .expect("macro-validated Live action identity"),
                    #version,
                    ::suprnova::live::__private::action::ActionArgumentSchema::new(
                        ::std::vec![#(#argument_fields),*],
                    ).expect("macro-validated Live action argument schema"),
                    ::suprnova::live::__private::action::AuthorizationRequirement::#authorization,
                    ::suprnova::live::__private::validation::ValidationSelection::#validation,
                    ::suprnova::live::__private::action::TransactionPolicy::#transaction,
                )?
            }
        })
        .collect::<Vec<_>>();
    let action_entries = actions
        .values()
        .enumerate()
        .map(|(index, action)| {
            let method = &action.method;
            let decodes = action.arguments.iter().map(|argument| {
                let ident = &argument.name;
                let ty = &argument.ty;
                let name = ident.to_string();
                quote! {
                    let #ident: #ty = arguments.decode::<#ty>(#name)?;
                }
            });
            let mut invocation_arguments = Vec::new();
            if action.authorization_parameter.is_some() {
                invocation_arguments.push(quote!(authorization));
            }
            invocation_arguments.extend(
                action
                    .arguments
                    .iter()
                    .map(|argument| {
                        let name = &argument.name;
                        quote!(#name)
                    }),
            );
            let invocation = if action.asynchronous {
                quote!(target.#method(#(#invocation_arguments),*).await)
            } else {
                quote!(target.#method(#(#invocation_arguments),*))
            };
            quote! {
                ::suprnova::live::__private::action::ActionEntry::new(
                    metadata.actions()[#index].clone(),
                    |target, authorization, arguments| {
                        ::std::boxed::Box::pin(async move {
                            let target = target
                                .as_any_mut()
                                .downcast_mut::<#self_ty>()
                                .ok_or_else(
                                    ::suprnova::live::__private::action::ActionError::dispatcher_contract
                                )?;
                            #(#decodes)*
                            let _ = authorization;
                            let output = #invocation;
                            ::suprnova::live::__private::action::IntoActionResult::into_action_result(
                                output,
                            )
                        })
                    },
                )
            }
        })
        .collect::<Vec<_>>();
    let parameter_values =
        mount
            .iter()
            .flat_map(|mount| mount.parameters.iter())
            .map(|parameter| {
                let name = &parameter.name;
                let codec = model_codec_tokens(&parameter.ty);
                quote! {
                    ::suprnova::live::__private::component::composition::ChildParameterField::new(
                        ::suprnova::live::__private::identity::ModelField::parse(#name)
                            .expect("macro-validated Live mount parameter identity"),
                        #codec,
                        true,
                    )
                }
            });
    let parameter_values = parameter_values.collect::<Vec<_>>();
    let validation_port = if component_validation_hooks.is_empty()
        && action_validation_hooks.is_empty()
    {
        quote! {}
    } else {
        let component_validation_calls =
            component_validation_hooks.iter().map(|(method, asynchronous)| {
            let invocation = if *asynchronous {
                quote!(target.#method().await)
            } else {
                quote!(target.#method())
            };
            quote! {
                {
                    let target = request
                        .target_mut()
                        .and_then(|target| target.as_any_mut().downcast_mut::<#self_ty>())
                        .ok_or_else(
                            ::suprnova::live::__private::validation::ValidationPortError::unavailable,
                        )?;
                    issues.extend(
                        ::suprnova::live::__private::validation::into_validation_issues(
                            #invocation,
                        )?,
                    );
                }
            }
        });
        let has_component_validation = !component_validation_hooks.is_empty();
        let action_validation_arms = action_validation_hooks.iter().map(|(name, hook)| {
            let name = syn::LitStr::new(name, hook.span);
            let method = &hook.method;
            let decodes = hook.arguments.iter().map(|argument| {
                let ident = &argument.name;
                let ty = &argument.ty;
                let name = ident.unraw().to_string();
                quote! {
                    let #ident: #ty = request.decode_argument::<#ty>(#name)?;
                }
            });
            let arguments = hook.arguments.iter().map(|argument| &argument.name);
            let invocation = if hook.asynchronous {
                quote!(target.#method(#(#arguments),*).await)
            } else {
                quote!(target.#method(#(#arguments),*))
            };
            quote! {
                #name => {
                    #(#decodes)*
                    let target = request
                        .target_mut()
                        .and_then(|target| target.as_any_mut().downcast_mut::<#self_ty>())
                        .ok_or_else(
                            ::suprnova::live::__private::validation::ValidationPortError::unavailable,
                        )?;
                    issues.extend(
                        ::suprnova::live::__private::validation::into_validation_issues(
                            #invocation,
                        )?,
                    );
                }
            }
        });
        quote! {
            fn validation_port() -> ::std::option::Option<
                ::std::sync::Arc<
                    dyn ::suprnova::live::__private::validation::ValidationPort,
                >,
            > {
                struct GeneratedValidation;

                impl ::suprnova::live::__private::validation::ValidationPort
                    for GeneratedValidation
                {
                    fn validate<'a>(
                        &'a self,
                        mut request: ::suprnova::live::__private::validation::ValidationRequest<'a>,
                    ) -> ::suprnova::live::__private::validation::ValidationFuture<
                        'a,
                        ::std::result::Result<
                            ::std::vec::Vec<
                                ::suprnova::live::__private::validation::ValidationIssue,
                            >,
                            ::suprnova::live::__private::validation::ValidationPortError,
                        >,
                    > {
                        ::std::boxed::Box::pin(async move {
                            let selection = request.selection().clone();
                            let mut issues = ::std::vec::Vec::new();
                            let validate_component = ::std::matches!(
                                selection,
                                ::suprnova::live::__private::validation::ValidationSelection::Selected(_)
                                    | ::suprnova::live::__private::validation::ValidationSelection::WholeComponent
                                    | ::suprnova::live::__private::validation::ValidationSelection::ComponentAndArguments
                            );
                            if validate_component {
                                if !#has_component_validation {
                                    return ::std::result::Result::Err(
                                        ::suprnova::live::__private::validation::ValidationPortError::unavailable(),
                                    );
                                }
                                #(#component_validation_calls)*
                                if let ::suprnova::live::__private::validation::ValidationSelection::Selected(selected) = &selection {
                                    issues.retain(|issue| {
                                        selected.iter().any(|selected| {
                                            let candidate = issue.path().as_str();
                                            let selected = selected.as_str();
                                            candidate == selected
                                                || candidate.strip_prefix(selected).is_some_and(
                                                    |suffix| suffix.starts_with('.'),
                                                )
                                        })
                                    });
                                }
                            }

                            let validate_arguments = ::std::matches!(
                                selection,
                                ::suprnova::live::__private::validation::ValidationSelection::ActionArguments
                                    | ::suprnova::live::__private::validation::ValidationSelection::ComponentAndArguments
                            );
                            if validate_arguments {
                                let action = request
                                    .action()
                                    .ok_or_else(
                                        ::suprnova::live::__private::validation::ValidationPortError::unavailable,
                                    )?
                                    .as_str()
                                    .to_owned();
                                match action.as_str() {
                                    #(#action_validation_arms)*
                                    _ => {
                                        return ::std::result::Result::Err(
                                            ::suprnova::live::__private::validation::ValidationPortError::unavailable(),
                                        );
                                    }
                                }
                            }
                            ::std::result::Result::Ok(issues)
                        })
                    }
                }

                ::std::option::Option::Some(::std::sync::Arc::new(GeneratedValidation))
            }
        }
    };
    let mount_runtime = generate_mount_runtime(&mount);
    let hydrated_runtime =
        generate_context_lifecycle_hook(&lifecycle_hooks, "hydrate", "hydrated_generated");
    let rendering_runtime =
        generate_context_lifecycle_hook(&lifecycle_hooks, "rendering", "rendering_generated");
    let rendered_runtime =
        generate_context_lifecycle_hook(&lifecycle_hooks, "rendered", "rendered_generated");
    let dehydrating_runtime =
        generate_context_lifecycle_hook(&lifecycle_hooks, "dehydrate", "dehydrating_generated");
    let params_changed_runtime = generate_params_changed_hook(&lifecycle_hooks);
    let lazy_complete_runtime = generate_context_lifecycle_hook(
        &lifecycle_hooks,
        "lazy_complete",
        "lazy_complete_generated",
    );
    let teardown_runtime = generate_teardown_hook(&lifecycle_hooks);
    let component_ident = self_type_ident(self_ty)?;
    let view_ident = format_ident!("__SuprnovaLiveViewFor{component_ident}");
    let computed_forwarders = computed_methods.iter().map(|computed| {
        let method = &computed.method;
        let inputs = &computed.inputs;
        let arguments = &computed.arguments;
        let output = &computed.output;
        quote! {
            fn #method(&self, #(#inputs),*) #output {
                self.component.#method(#(#arguments),*)
            }
        }
    });
    let computed_view_impl = if computed_methods.is_empty() {
        quote! {}
    } else {
        quote! {
            impl<'__snv_live> #view_ident<'__snv_live> {
                #(#computed_forwarders)*
            }
        }
    };
    let tokens = quote! {
        #item

        #computed_view_impl

        impl ::suprnova::live::__private::component::generated::GeneratedComponentRuntime
            for #self_ty
        {
            #mount_runtime
            #hydrated_runtime
            #rendering_runtime
            #rendered_runtime
            #dehydrating_runtime
            #params_changed_runtime
            #lazy_complete_runtime
            #teardown_runtime
        }

        impl ::suprnova::live::__private::metadata::LiveComponentContract for #self_ty {
            fn descriptor() -> ::std::result::Result<
                ::suprnova::live::__private::registry::ComponentDescriptor,
                ::suprnova::live::__private::metadata::MetadataError,
            > {
                let metadata = <Self as
                    ::suprnova::live::__private::metadata::LiveComponentDefinitionMetadata
                >::component_metadata(::std::vec![#(#action_values),*])?;
                let action_table =
                    ::suprnova::live::__private::action::ActionTable::new(
                        ::std::vec![#(#action_entries),*],
                    ).expect("macro-validated Live action table");
                let parameter_schema =
                    ::suprnova::live::__private::component::composition::ChildParameterSchema::new(
                        metadata.versions().state_schema(),
                        ::std::vec![#(#parameter_values),*],
                    ).expect("macro-validated Live mount parameter schema");
                let hooks =
                    ::suprnova::live::__private::component::generated::component_hooks::<Self>(
                        metadata.clone(),
                    );
                ::std::result::Result::Ok(
                    ::suprnova::live::__private::registry::ComponentDescriptor::with_hooks(
                        metadata,
                        hooks,
                    )
                        .with_composition(
                            parameter_schema,
                            #supports_params_changed,
                            #supports_lazy_complete,
                        )
                        .with_actions(action_table)
                        .expect("macro-validated Live action metadata equivalence"),
                )
            }

            fn descriptor_with_hooks(
                hooks: ::suprnova::live::__private::component::ComponentHooks,
            ) -> ::std::result::Result<
                ::suprnova::live::__private::registry::ComponentDescriptor,
                ::suprnova::live::__private::metadata::MetadataError,
            > {
                let metadata = <Self as
                    ::suprnova::live::__private::metadata::LiveComponentDefinitionMetadata
                >::component_metadata(::std::vec![#(#action_values),*])?;
                let action_table =
                    ::suprnova::live::__private::action::ActionTable::new(
                        ::std::vec![#(#action_entries),*],
                    ).expect("macro-validated Live action table");
                let parameter_schema =
                    ::suprnova::live::__private::component::composition::ChildParameterSchema::new(
                        metadata.versions().state_schema(),
                        ::std::vec![#(#parameter_values),*],
                    ).expect("macro-validated Live mount parameter schema");
                ::std::result::Result::Ok(
                    ::suprnova::live::__private::registry::ComponentDescriptor::with_hooks(
                        metadata,
                        hooks,
                    ).with_composition(
                        parameter_schema,
                        #supports_params_changed,
                        #supports_lazy_complete,
                    )
                    .with_actions(action_table)
                    .expect("macro-validated Live action metadata equivalence"),
                )
            }

            #validation_port
        }
    };
    enforce_runtime_path_contract(&tokens)?;
    Ok(tokens)
}

fn generate_mount_runtime(mount: &Option<RegisteredMount>) -> TokenStream2 {
    let (expected, decodes, invocation) = if let Some(mount) = mount {
        let expected = mount.parameters.len();
        let decodes = mount.parameters.iter().map(|parameter| {
            let name = &parameter.name;
            let ident = &parameter.ident;
            let ty = &parameter.ty;
            let codec = model_codec_tokens(ty);
            quote! {
                let #ident: #ty =
                    ::suprnova::live::__private::component::generated::decode_model_field(
                        parameters.get(#name).ok_or_else(
                            ::suprnova::live::__private::component::ComponentError::contract_failure,
                        )?,
                        &#codec,
                    )?;
            }
        });
        let method = &mount.method;
        let arguments = mount.parameters.iter().map(|parameter| &parameter.ident);
        let invocation = if mount.asynchronous {
            quote!(Self::#method(#(#arguments),*).await)
        } else {
            quote!(Self::#method(#(#arguments),*))
        };
        (
            expected,
            quote!(#(#decodes)*),
            quote! {
                let output = #invocation;
                ::suprnova::live::__private::component::generated::IntoComponentResult::into_component_result(
                    output,
                )
            },
        )
    } else {
        (
            0,
            quote! {},
            quote! {
                <Self as
                    ::suprnova::live::__private::component::generated::GeneratedComponentState
                >::default_mount_state()
            },
        )
    };

    quote! {
        fn mount_generated<'a>(
            context: &'a ::suprnova::live::__private::component::MountContext<'a>,
        ) -> ::suprnova::live::__private::component::LiveFuture<
            'a,
            ::std::result::Result<
                Self,
                ::suprnova::live::__private::component::ComponentError,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let ::suprnova::live::__private::canonical::CanonicalValue::Object(parameters) =
                    context.parameters()
                else {
                    return ::std::result::Result::Err(
                        ::suprnova::live::__private::component::ComponentError::contract_failure(),
                    );
                };
                if parameters.len() != #expected {
                    return ::std::result::Result::Err(
                        ::suprnova::live::__private::component::ComponentError::contract_failure(),
                    );
                }
                #decodes
                #invocation
            })
        }
    }
}

fn generate_context_lifecycle_hook(
    hooks: &BTreeMap<String, RegisteredLifecycleHook>,
    source: &str,
    generated: &str,
) -> TokenStream2 {
    let Some(hook) = hooks.get(source) else {
        return quote! {};
    };
    let generated = syn::Ident::new(generated, hook.method.span());
    let method = &hook.method;
    let invocation = if hook.asynchronous {
        quote!(self.#method().await)
    } else {
        quote!(self.#method())
    };
    quote! {
        fn #generated<'a>(
            &'a mut self,
            _context: &'a ::suprnova::live::__private::component::RenderContext<'a>,
        ) -> ::suprnova::live::__private::component::LiveFuture<
            'a,
            ::std::result::Result<
                (),
                ::suprnova::live::__private::component::ComponentError,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let output = #invocation;
                ::suprnova::live::__private::component::generated::IntoComponentHookResult::into_component_hook_result(
                    output,
                )
            })
        }
    }
}

fn generate_params_changed_hook(hooks: &BTreeMap<String, RegisteredLifecycleHook>) -> TokenStream2 {
    let Some(hook) = hooks.get("params_changed") else {
        return quote! {};
    };
    let method = &hook.method;
    let invocation = if hook.asynchronous {
        quote!(self.#method().await)
    } else {
        quote!(self.#method())
    };
    quote! {
        fn params_changed_generated<'a>(
            &'a mut self,
            _context: &'a ::suprnova::live::__private::component::RenderContext<'a>,
            _parameters: &'a ::suprnova::live::__private::child::VerifiedChildParametersV1,
        ) -> ::suprnova::live::__private::component::LiveFuture<
            'a,
            ::std::result::Result<
                (),
                ::suprnova::live::__private::component::ComponentError,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let output = #invocation;
                ::suprnova::live::__private::component::generated::IntoComponentHookResult::into_component_hook_result(
                    output,
                )
            })
        }
    }
}

fn generate_teardown_hook(hooks: &BTreeMap<String, RegisteredLifecycleHook>) -> TokenStream2 {
    let Some(hook) = hooks.get("teardown") else {
        return quote! {};
    };
    let method = &hook.method;
    let invocation = if hook.asynchronous {
        quote!(self.#method().await)
    } else {
        quote!(self.#method())
    };
    quote! {
        fn teardown_generated<'a>(
            &'a mut self,
        ) -> ::suprnova::live::__private::component::LiveFuture<
            'a,
            ::std::result::Result<
                (),
                ::suprnova::live::__private::component::ComponentError,
            >,
        > {
            ::std::boxed::Box::pin(async move {
                let output = #invocation;
                ::suprnova::live::__private::component::generated::IntoComponentHookResult::into_component_hook_result(
                    output,
                )
            })
        }
    }
}

fn self_type_ident(self_ty: &Type) -> syn::Result<&syn::Ident> {
    let Type::Path(path) = self_ty else {
        return Err(syn::Error::new(
            self_ty.span(),
            "Live component impl target must be a named type",
        ));
    };
    path.path
        .segments
        .last()
        .map(|segment| &segment.ident)
        .ok_or_else(|| syn::Error::new(self_ty.span(), "Live component impl target is empty"))
}

fn take_method_helper(method: &mut ImplItemFn) -> syn::Result<Option<(String, Attribute)>> {
    let mut helper = None;
    let mut retained = Vec::with_capacity(method.attrs.len());
    for attribute in method.attrs.drain(..) {
        let Some(name) = attribute
            .path()
            .get_ident()
            .map(|ident| ident.unraw().to_string())
        else {
            retained.push(attribute);
            continue;
        };
        if is_field_helper(&name) {
            return Err(syn::Error::new(
                attribute.span(),
                "field helper cannot be placed on a Live impl method",
            ));
        }
        if is_method_helper(&name) {
            if helper.replace((name, attribute)).is_some() {
                return Err(syn::Error::new(
                    method.sig.ident.span(),
                    "a Live method may declare only one method helper",
                ));
            }
        } else {
            retained.push(attribute);
        }
    }
    method.attrs = retained;
    Ok(helper)
}

fn reject_helpers_on_non_method(item: &ImplItem) -> syn::Result<()> {
    let attributes = match item {
        ImplItem::Const(item) => &item.attrs,
        ImplItem::Type(item) => &item.attrs,
        ImplItem::Macro(item) => &item.attrs,
        ImplItem::Verbatim(_) | _ => return Ok(()),
    };
    if let Some(attribute) = attributes.iter().find(|attribute| {
        attribute.path().get_ident().is_some_and(|ident| {
            let name = ident.unraw().to_string();
            is_method_helper(&name) || is_field_helper(&name)
        })
    }) {
        return Err(syn::Error::new(
            attribute.span(),
            "Live helpers can only be placed on their documented item kind",
        ));
    }
    Ok(())
}

fn validate_common_signature(method: &ImplItemFn) -> syn::Result<()> {
    let signature = &method.sig;
    if signature.constness.is_some()
        || signature.unsafety.is_some()
        || signature.abi.is_some()
        || signature.variadic.is_some()
        || !signature.generics.params.is_empty()
        || signature.generics.where_clause.is_some()
    {
        return Err(syn::Error::new(
            signature.span(),
            "Live methods cannot be const, unsafe, extern, variadic, or generic",
        ));
    }
    if !matches!(method.vis, Visibility::Public(_)) {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "registered Live methods must be public",
        ));
    }
    Ok(())
}

fn validate_action_signature(method: &ImplItemFn) -> syn::Result<()> {
    let receiver = method.sig.inputs.first().and_then(receiver);
    if !receiver.is_some_and(|value| value.reference.is_some() && value.mutability.is_some()) {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "Live actions require `&mut self` as their first argument",
        ));
    }
    Ok(())
}

struct RegisteredAction {
    version: u16,
    method: syn::Ident,
    arguments: Vec<ActionParameter>,
    asynchronous: bool,
    authorization: ActionAuthorizationArgs,
    validation: ActionValidationArgs,
    transaction: ActionTransactionArgs,
    authorization_parameter: Option<syn::Ident>,
}

struct ActionParameter {
    name: syn::Ident,
    ty: Type,
    required: bool,
}

struct RegisteredMount {
    method: syn::Ident,
    parameters: Vec<MountParameter>,
    asynchronous: bool,
}

struct MountParameter {
    name: String,
    ident: syn::Ident,
    ty: Type,
}

struct RegisteredLifecycleHook {
    method: syn::Ident,
    asynchronous: bool,
}

struct RegisteredComputed {
    method: syn::Ident,
    inputs: Vec<FnArg>,
    arguments: Vec<syn::Ident>,
    output: syn::ReturnType,
}

struct RegisteredValidationHook {
    method: syn::Ident,
    arguments: Vec<ActionParameter>,
    asynchronous: bool,
    span: Span,
}

fn validate_validation_hooks(
    actions: &BTreeMap<String, RegisteredAction>,
    component_hooks: &[(syn::Ident, bool)],
    action_hooks: &BTreeMap<String, RegisteredValidationHook>,
) -> syn::Result<()> {
    for (name, hook) in action_hooks {
        let action = actions.get(name).ok_or_else(|| {
            syn::Error::new(
                hook.span,
                "typed validation hook names an unknown registered action",
            )
        })?;
        let same_contract = action.arguments.len() == hook.arguments.len()
            && action
                .arguments
                .iter()
                .zip(&hook.arguments)
                .all(|(action, hook)| {
                    let action_ty = &action.ty;
                    let hook_ty = &hook.ty;
                    action.name == hook.name
                        && quote!(#action_ty).to_string() == quote!(#hook_ty).to_string()
                });
        if !same_contract {
            return Err(syn::Error::new(
                hook.span,
                "typed validation hook arguments must exactly match the registered action",
            ));
        }
    }

    for (name, action) in actions {
        let component_required = matches!(
            action.validation,
            ActionValidationArgs::Whole | ActionValidationArgs::All
        );
        if component_required && component_hooks.is_empty() {
            return Err(syn::Error::new(
                action.method.span(),
                "component validation selection requires a bare #[validate] hook",
            ));
        }
        let arguments_required = matches!(
            action.validation,
            ActionValidationArgs::Arguments | ActionValidationArgs::All
        );
        if arguments_required && !action_hooks.contains_key(name) {
            return Err(syn::Error::new(
                action.method.span(),
                "argument validation selection requires #[validate(action = \"...\")]",
            ));
        }
    }
    Ok(())
}

fn extract_action_parameters(
    method: &ImplItemFn,
) -> syn::Result<(Option<syn::Ident>, Vec<ActionParameter>)> {
    let mut authorization = None;
    let mut parameters = Vec::new();
    for (index, argument) in method.sig.inputs.iter().skip(1).enumerate() {
        let FnArg::Typed(argument) = argument else {
            return Err(syn::Error::new(
                argument.span(),
                "action arguments must be typed named values",
            ));
        };
        let Pat::Ident(pattern) = argument.pat.as_ref() else {
            return Err(syn::Error::new(
                argument.pat.span(),
                "action arguments must use simple stable names",
            ));
        };
        if is_authorized_action_reference(&argument.ty) {
            if index != 0 || authorization.is_some() {
                return Err(syn::Error::new(
                    argument.span(),
                    "the authorized action context must be the first action parameter",
                ));
            }
            authorization = Some(pattern.ident.clone());
            continue;
        }
        if pattern.by_ref.is_some()
            || pattern.mutability.is_some()
            || pattern.subpat.is_some()
            || contains_reference(&argument.ty)
        {
            return Err(syn::Error::new(
                argument.span(),
                "action arguments must be immutable owned values",
            ));
        }
        parameters.push(ActionParameter {
            name: pattern.ident.clone(),
            ty: argument.ty.as_ref().clone(),
            required: option_inner(&argument.ty).is_none(),
        });
    }
    Ok((authorization, parameters))
}

fn extract_validation_parameters(method: &ImplItemFn) -> syn::Result<Vec<ActionParameter>> {
    let (authorization, parameters) = extract_action_parameters(method)?;
    if authorization.is_some() {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "validation hooks cannot receive authorization capabilities",
        ));
    }
    Ok(parameters)
}

fn is_authorized_action_reference(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    if reference.mutability.is_some() {
        return false;
    }
    let Type::Path(path) = reference.elem.as_ref() else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "AuthorizedAction")
}

fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    match arguments.args.first()? {
        GenericArgument::Type(inner) if arguments.args.len() == 1 => Some(inner),
        _ => None,
    }
}

fn validate_mount_signature(method: &ImplItemFn) -> syn::Result<()> {
    if method
        .sig
        .inputs
        .iter()
        .any(|argument| matches!(argument, FnArg::Receiver(_)))
    {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "mount must be an associated constructor without a self receiver",
        ));
    }
    Ok(())
}

fn extract_mount_parameters(method: &ImplItemFn) -> syn::Result<Vec<MountParameter>> {
    method
        .sig
        .inputs
        .iter()
        .map(|argument| {
            let FnArg::Typed(argument) = argument else {
                return Err(syn::Error::new(
                    argument.span(),
                    "mount parameters must be typed named arguments",
                ));
            };
            let Pat::Ident(pattern) = argument.pat.as_ref() else {
                return Err(syn::Error::new(
                    argument.pat.span(),
                    "mount parameters must use simple stable names",
                ));
            };
            if pattern.by_ref.is_some()
                || pattern.mutability.is_some()
                || pattern.subpat.is_some()
                || contains_reference(&argument.ty)
            {
                return Err(syn::Error::new(
                    argument.span(),
                    "mount parameters must be immutable owned values",
                ));
            }
            Ok(MountParameter {
                name: pattern.ident.unraw().to_string(),
                ident: pattern.ident.clone(),
                ty: argument.ty.as_ref().clone(),
            })
        })
        .collect()
}

fn validate_computed_signature(method: &ImplItemFn) -> syn::Result<()> {
    if method.sig.asyncness.is_some() {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "computed values must be synchronous",
        ));
    }
    let receiver = method.sig.inputs.first().and_then(receiver);
    if !receiver.is_some_and(|value| value.reference.is_some() && value.mutability.is_none()) {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "computed values require `&self`",
        ));
    }
    Ok(())
}

fn extract_computed(method: &ImplItemFn) -> syn::Result<RegisteredComputed> {
    let mut inputs = Vec::new();
    let mut arguments = Vec::new();
    for argument in method.sig.inputs.iter().skip(1) {
        let FnArg::Typed(argument) = argument else {
            return Err(syn::Error::new(
                argument.span(),
                "computed arguments must be typed named values",
            ));
        };
        let Pat::Ident(pattern) = argument.pat.as_ref() else {
            return Err(syn::Error::new(
                argument.pat.span(),
                "computed arguments must use simple stable names",
            ));
        };
        if pattern.by_ref.is_some() || pattern.mutability.is_some() || pattern.subpat.is_some() {
            return Err(syn::Error::new(
                argument.pat.span(),
                "computed arguments must use immutable named values",
            ));
        }
        inputs.push(FnArg::Typed(argument.clone()));
        arguments.push(pattern.ident.clone());
    }
    Ok(RegisteredComputed {
        method: method.sig.ident.clone(),
        inputs,
        arguments,
        output: method.sig.output.clone(),
    })
}

fn validate_receiver_method(method: &ImplItemFn, require_mutable: bool) -> syn::Result<()> {
    let receiver = method.sig.inputs.first().and_then(receiver);
    let valid = receiver.is_some_and(|value| {
        value.reference.is_some() && (!require_mutable || value.mutability.is_some())
    });
    if !valid {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            if require_mutable {
                "lifecycle hooks require `&mut self`"
            } else {
                "validation methods require `&self` or `&mut self`"
            },
        ));
    }
    Ok(())
}

fn receiver(argument: &FnArg) -> Option<&Receiver> {
    match argument {
        FnArg::Receiver(receiver) => Some(receiver),
        FnArg::Typed(_) => None,
    }
}

fn ensure_singleton(
    helpers: &mut BTreeMap<String, Span>,
    name: &str,
    span: Span,
) -> syn::Result<()> {
    if helpers.insert(name.to_owned(), span).is_some() {
        return Err(syn::Error::new(
            span,
            format!("duplicate #[{name}] lifecycle helper"),
        ));
    }
    Ok(())
}

fn ensure_path_helper(attribute: &Attribute) -> syn::Result<()> {
    if matches!(attribute.meta, Meta::Path(_)) {
        Ok(())
    } else {
        Err(syn::Error::new(
            attribute.span(),
            "this Live method helper does not accept arguments",
        ))
    }
}
