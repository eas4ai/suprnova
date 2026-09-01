//! Hidden runtime bridge implemented only by generated component code.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::action::ActionResult;
use crate::canonical::CanonicalValue;
use crate::child::{EligibleChildParametersV2, VerifiedChildParametersV1};
use crate::limits::InputLimits;
use crate::metadata::ComponentMetadata;
use crate::snapshot::state::{StateCodec, StateExposure};
use crate::state::ModelCodec;
use crate::state::ProposalBatch;
use crate::view::{AssetSet, IslandRender, ViewRenderer, ViewTemplate};

use super::{
    ComponentError, ComponentFactory, ComponentHooks, ComponentInstance, HydrationContext,
    LiveFuture, MountContext, RenderContext,
};

/// State and checked-view operations generated from a Live component struct.
#[doc(hidden)]
pub trait GeneratedComponentState: Send + Sized + 'static {
    /// Constructs the component's field-wise default mount state.
    fn default_mount_state() -> Result<Self, ComponentError>;

    /// Reconstructs snapshot-backed fields and initializes non-snapshot fields safely.
    fn hydrate_state(state: &CanonicalValue) -> Result<Self, ComponentError>;

    /// Applies only previously authorized typed model proposals.
    fn bind_generated_models(&mut self, proposals: &ProposalBatch) -> Result<(), ComponentError>;

    /// Renders the generated checked Askama view through the engine boundary.
    fn render_generated_view(
        &self,
        context: &RenderContext<'_>,
        metadata: &ComponentMetadata,
    ) -> Result<IslandRender, ComponentError>;

    /// Serializes only fields eligible for the requested snapshot exposure.
    fn dehydrate_generated_state(
        &self,
        exposure: StateExposure,
    ) -> Result<CanonicalValue, ComponentError>;
}

/// Lifecycle operations generated from the component's `#[live]` implementation.
#[doc(hidden)]
pub trait GeneratedComponentRuntime: GeneratedComponentState {
    /// Runs the repeatable mount constructor.
    fn mount_generated<'a>(
        context: &'a MountContext<'a>,
    ) -> LiveFuture<'a, Result<Self, ComponentError>>;

    /// Runs the generated hydration hook.
    fn hydrated_generated<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Runs the generated pre-render hook.
    fn rendering_generated<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Runs the generated post-render hook.
    fn rendered_generated<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Runs the generated pre-dehydration hook.
    fn dehydrating_generated<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    /// Runs the generated parent-parameter hook.
    fn params_changed_generated<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
        _parameters: &'a VerifiedChildParametersV1,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Err(ComponentError::contract_failure()) })
    }

    /// Runs the modern generated parent-parameter hook after server eligibility.
    fn params_changed_v2_generated<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
        _parameters: &'a EligibleChildParametersV2,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Err(ComponentError::contract_failure()) })
    }

    /// Runs the generated lazy-completion hook.
    fn lazy_complete_generated<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Err(ComponentError::contract_failure()) })
    }

    /// Runs the generated teardown hook.
    fn teardown_generated<'a>(&'a mut self) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Converts a generated mount constructor result into the closed component error surface.
#[doc(hidden)]
pub trait IntoComponentResult<T> {
    /// Performs the closed conversion.
    fn into_component_result(self) -> Result<T, ComponentError>;
}

impl<T> IntoComponentResult<T> for T {
    fn into_component_result(self) -> Result<T, ComponentError> {
        Ok(self)
    }
}

impl<T> IntoComponentResult<T> for Result<T, ComponentError> {
    fn into_component_result(self) -> Result<T, ComponentError> {
        self
    }
}

/// Converts a generated lifecycle-hook result into the closed component error surface.
#[doc(hidden)]
pub trait IntoComponentHookResult {
    /// Performs the closed conversion.
    fn into_component_hook_result(self) -> Result<(), ComponentError>;
}

impl IntoComponentHookResult for () {
    fn into_component_hook_result(self) -> Result<(), ComponentError> {
        Ok(())
    }
}

impl IntoComponentHookResult for Result<(), ComponentError> {
    fn into_component_hook_result(self) -> Result<(), ComponentError> {
        self
    }
}

struct GeneratedComponentFactory<C> {
    metadata: Arc<ComponentMetadata>,
    marker: std::marker::PhantomData<fn() -> C>,
}

impl<C> ComponentFactory for GeneratedComponentFactory<C>
where
    C: GeneratedComponentRuntime,
{
    fn mount<'a>(
        &'a self,
        context: &'a MountContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            let component = C::mount_generated(context).await?;
            Ok(Box::new(GeneratedComponentInstance {
                component,
                metadata: Arc::clone(&self.metadata),
            }) as Box<dyn ComponentInstance>)
        })
    }

    fn hydrate<'a>(
        &'a self,
        context: &'a HydrationContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            let component = C::hydrate_state(context.state())?;
            Ok(Box::new(GeneratedComponentInstance {
                component,
                metadata: Arc::clone(&self.metadata),
            }) as Box<dyn ComponentInstance>)
        })
    }
}

struct GeneratedComponentInstance<C> {
    component: C,
    metadata: Arc<ComponentMetadata>,
}

impl<C> ComponentInstance for GeneratedComponentInstance<C>
where
    C: GeneratedComponentRuntime,
{
    fn metadata(&self) -> &ComponentMetadata {
        self.metadata.as_ref()
    }

    fn action_target(&mut self) -> &mut dyn crate::action::ActionTarget {
        &mut self.component
    }

    fn hydrated<'a>(
        &'a mut self,
        context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        self.component.hydrated_generated(context)
    }

    fn bind_models(&mut self, proposals: &ProposalBatch) -> Result<(), ComponentError> {
        self.component.bind_generated_models(proposals)
    }

    fn params_changed<'a>(
        &'a mut self,
        context: &'a RenderContext<'a>,
        parameters: &'a VerifiedChildParametersV1,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        self.component.params_changed_generated(context, parameters)
    }

    fn params_changed_v2<'a>(
        &'a mut self,
        context: &'a RenderContext<'a>,
        parameters: &'a EligibleChildParametersV2,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        self.component
            .params_changed_v2_generated(context, parameters)
    }

    fn lazy_complete<'a>(
        &'a mut self,
        context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        self.component.lazy_complete_generated(context)
    }

    fn before_action<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
        _action: &'a crate::identity::ActionName,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    fn after_action<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
        _action: &'a crate::identity::ActionName,
        _result: &'a ActionResult,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async { Ok(()) })
    }

    fn rendering<'a>(
        &'a mut self,
        context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        self.component.rendering_generated(context)
    }

    fn render<'a>(
        &'a self,
        context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<IslandRender, ComponentError>> {
        let rendered = self
            .component
            .render_generated_view(context, self.metadata.as_ref());
        Box::pin(async move { rendered })
    }

    fn rendered<'a>(
        &'a mut self,
        context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        self.component.rendered_generated(context)
    }

    fn dehydrating<'a>(
        &'a mut self,
        context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        self.component.dehydrating_generated(context)
    }

    fn dehydrate(&self, exposure: StateExposure) -> Result<CanonicalValue, ComponentError> {
        self.component.dehydrate_generated_state(exposure)
    }

    fn dehydrate_memo(&self) -> Result<CanonicalValue, ComponentError> {
        Ok(CanonicalValue::Object(BTreeMap::new()))
    }

    fn teardown<'a>(&'a mut self) -> LiveFuture<'a, Result<(), ComponentError>> {
        self.component.teardown_generated()
    }
}

/// Builds descriptor-owned hooks for one generated concrete component type.
#[doc(hidden)]
#[must_use]
pub fn component_hooks<C>(metadata: ComponentMetadata) -> ComponentHooks
where
    C: GeneratedComponentRuntime,
{
    ComponentHooks::new(Arc::new(GeneratedComponentFactory::<C> {
        metadata: Arc::new(metadata),
        marker: std::marker::PhantomData,
    }))
}

/// Renders one generated Askama component view through bounded engine validation.
#[doc(hidden)]
pub fn render_component_view<T: ViewTemplate + ?Sized>(
    metadata: &ComponentMetadata,
    template: &T,
) -> Result<IslandRender, ComponentError> {
    ViewRenderer::new(crate::view::RenderLimits::standard())
        .and_then(|renderer| {
            renderer.render_component_fragment(
                metadata.view().clone(),
                template,
                AssetSet::empty(),
                Vec::new(),
            )
        })
        .map_err(|_| ComponentError::contract_failure())
}

/// Encodes one generated JSON-codec field under the engine's fixed input bound.
#[doc(hidden)]
pub fn encode_json_field<T: Serialize>(value: &T) -> Result<CanonicalValue, ComponentError> {
    crate::snapshot::state::encode_json(value, &InputLimits::default())
        .map_err(|_| ComponentError::contract_failure())
}

/// Decodes one generated JSON-codec field from verified canonical state.
#[doc(hidden)]
pub fn decode_json_field<T: DeserializeOwned>(value: &CanonicalValue) -> Result<T, ComponentError> {
    crate::snapshot::state::decode_json(value).map_err(|_| ComponentError::contract_failure())
}

/// Decodes one generated mount parameter with its registered model codec.
#[doc(hidden)]
pub fn decode_model_field<T: DeserializeOwned + 'static>(
    value: &CanonicalValue,
    codec: &ModelCodec,
) -> Result<T, ComponentError> {
    codec
        .decode(value, &InputLimits::default())
        .map_err(|_| ComponentError::contract_failure())
}

/// Encodes one generated field with its registered exact state codec.
#[doc(hidden)]
pub fn encode_field<T: Serialize>(
    value: &T,
    codec: StateCodec,
) -> Result<CanonicalValue, ComponentError> {
    match codec {
        StateCodec::Json => encode_json_field(value),
        StateCodec::I64Decimal => {
            let value: i64 = decode_json_field(&encode_json_field(value)?)?;
            Ok(crate::snapshot::state::encode_i64(value))
        }
        StateCodec::U64Decimal => {
            let value: u64 = decode_json_field(&encode_json_field(value)?)?;
            Ok(crate::snapshot::state::encode_u64(value))
        }
        StateCodec::BytesBase64Url => {
            let value: Vec<u8> = decode_json_field(&encode_json_field(value)?)?;
            crate::snapshot::state::encode_bytes(&value, InputLimits::default().max_bytes())
                .map_err(|_| ComponentError::contract_failure())
        }
    }
}

/// Decodes one generated field with its registered exact state codec.
#[doc(hidden)]
pub fn decode_field<T: DeserializeOwned>(
    value: &CanonicalValue,
    codec: StateCodec,
) -> Result<T, ComponentError> {
    match codec {
        StateCodec::Json => decode_json_field(value),
        StateCodec::I64Decimal => {
            let value = crate::snapshot::state::decode_i64(value)
                .map_err(|_| ComponentError::contract_failure())?;
            decode_json_field(&encode_json_field(&value)?)
        }
        StateCodec::U64Decimal => {
            let value = crate::snapshot::state::decode_u64(value)
                .map_err(|_| ComponentError::contract_failure())?;
            decode_json_field(&encode_json_field(&value)?)
        }
        StateCodec::BytesBase64Url => {
            let value =
                crate::snapshot::state::decode_bytes(value, InputLimits::default().max_bytes())
                    .map_err(|_| ComponentError::contract_failure())?;
            decode_json_field(&encode_json_field(&value)?)
        }
    }
}
