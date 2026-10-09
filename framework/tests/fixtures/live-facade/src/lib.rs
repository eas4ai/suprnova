use suprnova::live::testing::ActionAssertion;
use suprnova::live::{
    ActionOutcome, ActionResult, ComponentContract, LiveConfig, LiveConfigError, LiveRegistry,
    LiveRegistryBuilder, RegistryError,
};
use suprnova::view::{
    TemplateFailure, TrustedHtml, TrustedMarkupError, TrustedMarkupReason, ViewTemplate,
};

/// Builds application-owned Live configuration through the stable facade.
pub fn live_config() -> Result<LiveConfig, LiveConfigError> {
    LiveConfig::builder()
        .max_request_bytes(256 * 1024)
        .max_response_bytes(192 * 1024)
        .build()
}

/// Registers one macro-produced component contract without naming engine types.
pub fn register<C: ComponentContract>(
    builder: LiveRegistryBuilder,
) -> Result<LiveRegistry, RegistryError> {
    Ok(builder.register::<C>()?.build())
}

/// Exercises the application-facing action and testing contracts.
pub fn assert_render_outcome() {
    let result = ActionResult::render();
    ActionAssertion::new(&result).assert_rendered();
    assert_eq!(result.outcome(), &ActionOutcome::Render);
}

/// Constructs checked unescaped markup through the Suprnova-owned view boundary.
pub fn trusted_markup() -> Result<TrustedHtml, TrustedMarkupError> {
    TrustedHtml::framework_static(
        "<strong>framework-owned</strong>",
        TrustedMarkupReason::new("static fixture markup")?,
    )
}

/// Renders through the checked view contract without naming its engine failure type.
pub fn render_checked<T: ViewTemplate>(
    template: &T,
    output: &mut dyn std::fmt::Write,
) -> Result<(), TemplateFailure> {
    template.render_view(output)
}
