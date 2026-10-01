//! Closed semantic action outcomes without arbitrary JavaScript or raw URLs.

use std::error::Error;
use std::fmt;

use crate::canonical::{CanonicalValue, to_canonical_bytes};
use crate::identity::{BrowserOperationName, RouteIdentity};
use crate::limits::InputLimits;
use crate::registry::ComponentDescriptor;

use super::{EmissionKind, RegisteredEmission};

const MAX_OUTCOME_ITEMS: usize = 128;

/// Closed action outcome understood by the Live protocol.
#[derive(Clone, Debug, PartialEq)]
pub enum ActionOutcome {
    /// Render fresh island HTML and successor state.
    Render,
    /// Keep current island HTML while still completing state/revision semantics.
    NoRender,
    /// Perform ordinary document navigation to a registered route.
    Redirect(RouteIntent),
}

impl ActionOutcome {
    /// Returns whether this outcome requires fresh island HTML.
    #[must_use]
    pub const fn requires_render(&self) -> bool {
        matches!(self, Self::Render)
    }

    /// Returns whether ordinary real-route navigation wins over island rendering.
    #[must_use]
    pub const fn redirects(&self) -> bool {
        matches!(self, Self::Redirect(_))
    }
}

/// Registered route identity plus bounded typed route parameters.
#[derive(Clone, PartialEq)]
pub struct RouteIntent {
    route: RouteIdentity,
    parameters: CanonicalValue,
}

impl RouteIntent {
    /// Creates an ordinary route intent without accepting a raw external URL.
    pub fn new(
        route: RouteIdentity,
        parameters: CanonicalValue,
        limits: &InputLimits,
    ) -> Result<Self, OutcomeError> {
        if !matches!(parameters, CanonicalValue::Object(_)) {
            return Err(OutcomeError::new(OutcomeErrorKind::InvalidPayload));
        }
        to_canonical_bytes(&parameters, limits)
            .map_err(|_| OutcomeError::new(OutcomeErrorKind::InvalidPayload))?;
        Ok(Self { route, parameters })
    }

    /// Returns the host-resolved route identity.
    #[must_use]
    pub const fn route(&self) -> &RouteIdentity {
        &self.route
    }

    /// Returns bounded route parameters for the host router.
    #[must_use]
    pub const fn parameters(&self) -> &CanonicalValue {
        &self.parameters
    }
}

impl fmt::Debug for RouteIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<RouteIntent:redacted>")
    }
}

/// Same-route replace-only URL reflection.
#[derive(Clone, PartialEq)]
pub struct UrlIntent {
    query: CanonicalValue,
}

impl UrlIntent {
    /// Creates a same-route `replaceState` intent from bounded typed query state.
    pub fn replace_same_route(
        query: CanonicalValue,
        limits: &InputLimits,
    ) -> Result<Self, OutcomeError> {
        if !matches!(query, CanonicalValue::Object(_)) {
            return Err(OutcomeError::new(OutcomeErrorKind::InvalidPayload));
        }
        to_canonical_bytes(&query, limits)
            .map_err(|_| OutcomeError::new(OutcomeErrorKind::InvalidPayload))?;
        Ok(Self { query })
    }

    /// Returns bounded same-route query state.
    #[must_use]
    pub const fn query(&self) -> &CanonicalValue {
        &self.query
    }
}

impl fmt::Debug for UrlIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UrlIntent:redacted>")
    }
}

/// Bounded ordinary session flash intent.
#[derive(Clone, PartialEq)]
pub struct FlashIntent {
    key: BrowserOperationName,
    value: CanonicalValue,
}

impl FlashIntent {
    /// Creates a bounded registered-key flash value for the host session adapter.
    pub fn new(
        key: BrowserOperationName,
        value: CanonicalValue,
        limits: &InputLimits,
    ) -> Result<Self, OutcomeError> {
        to_canonical_bytes(&value, limits)
            .map_err(|_| OutcomeError::new(OutcomeErrorKind::InvalidPayload))?;
        Ok(Self { key, value })
    }

    /// Returns the stable flash key.
    #[must_use]
    pub const fn key(&self) -> &BrowserOperationName {
        &self.key
    }

    /// Returns the bounded flash value for the host session adapter.
    #[must_use]
    pub const fn value(&self) -> &CanonicalValue {
        &self.value
    }
}

impl fmt::Debug for FlashIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FlashIntent")
            .field("key", &self.key.as_str())
            .finish_non_exhaustive()
    }
}

/// Bounded metadata applied around one semantic action outcome.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutcomeMetadata {
    flash: Vec<FlashIntent>,
    events: Vec<RegisteredEmission>,
    effects: Vec<RegisteredEmission>,
    url: Option<UrlIntent>,
}

impl OutcomeMetadata {
    /// Creates bounded metadata; type registration is rechecked by [`ActionResult::new`].
    pub fn new(
        flash: Vec<FlashIntent>,
        events: Vec<RegisteredEmission>,
        effects: Vec<RegisteredEmission>,
        url: Option<UrlIntent>,
    ) -> Result<Self, OutcomeError> {
        if flash.len() > MAX_OUTCOME_ITEMS
            || events.len() > MAX_OUTCOME_ITEMS
            || effects.len() > MAX_OUTCOME_ITEMS
        {
            return Err(OutcomeError::new(OutcomeErrorKind::TooManyItems));
        }
        if events
            .iter()
            .any(|emission| emission.kind() != EmissionKind::Event)
            || effects
                .iter()
                .any(|emission| emission.kind() != EmissionKind::Effect)
        {
            return Err(OutcomeError::new(OutcomeErrorKind::InvalidEmissionChannel));
        }
        Ok(Self {
            flash,
            events,
            effects,
            url,
        })
    }

    /// Returns session flash intents.
    #[must_use]
    pub fn flash(&self) -> &[FlashIntent] {
        &self.flash
    }

    /// Returns registered event emissions.
    #[must_use]
    pub fn events(&self) -> &[RegisteredEmission] {
        &self.events
    }

    /// Returns registered effect emissions.
    #[must_use]
    pub fn effects(&self) -> &[RegisteredEmission] {
        &self.effects
    }

    /// Returns same-route replace reflection when requested.
    #[must_use]
    pub const fn url(&self) -> Option<&UrlIntent> {
        self.url.as_ref()
    }
}

/// Validated semantic result returned by a registered action.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionResult {
    outcome: ActionOutcome,
    metadata: OutcomeMetadata,
}

impl ActionResult {
    /// Validates outcome precedence and the component's declared emission set.
    pub fn new(
        outcome: ActionOutcome,
        metadata: OutcomeMetadata,
        descriptor: &ComponentDescriptor,
    ) -> Result<Self, OutcomeError> {
        if outcome.redirects() && metadata.url.is_some() {
            return Err(OutcomeError::new(OutcomeErrorKind::IncompatibleOutcome));
        }
        if metadata
            .events
            .iter()
            .chain(&metadata.effects)
            .any(|emission| !emission.is_registered(descriptor))
        {
            return Err(OutcomeError::new(OutcomeErrorKind::UnregisteredEmission));
        }
        Ok(Self { outcome, metadata })
    }

    /// Creates the default render outcome with no side metadata.
    #[must_use]
    pub fn render() -> Self {
        Self::from_outcome(ActionOutcome::Render)
    }

    /// Creates an explicit no-render outcome with no side metadata.
    #[must_use]
    pub fn no_render() -> Self {
        Self::from_outcome(ActionOutcome::NoRender)
    }

    /// Wraps one already-typed outcome with empty side metadata.
    #[must_use]
    pub fn from_outcome(outcome: ActionOutcome) -> Self {
        Self {
            outcome,
            metadata: OutcomeMetadata::default(),
        }
    }

    /// Returns the closed semantic outcome.
    #[must_use]
    pub const fn outcome(&self) -> &ActionOutcome {
        &self.outcome
    }

    /// Returns validated flash/event/effect/URL metadata.
    #[must_use]
    pub const fn metadata(&self) -> &OutcomeMetadata {
        &self.metadata
    }
}

/// Closed semantic outcome construction failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutcomeErrorKind {
    /// A typed payload could not be encoded within its configured limits.
    InvalidPayload,
    /// An event/effect type was not declared by the current component descriptor.
    UnregisteredEmission,
    /// Too many flash, event, or effect items were requested.
    TooManyItems,
    /// A registered effect was placed in the event channel or vice versa.
    InvalidEmissionChannel,
    /// Redirect precedence conflicted with same-route URL reflection.
    IncompatibleOutcome,
}

/// Redacted semantic outcome failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct OutcomeError {
    kind: OutcomeErrorKind,
}

impl OutcomeError {
    pub(crate) const fn new(kind: OutcomeErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> OutcomeErrorKind {
        self.kind
    }
}

impl fmt::Display for OutcomeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            OutcomeErrorKind::InvalidPayload => "invalid_outcome_payload",
            OutcomeErrorKind::UnregisteredEmission => "unregistered_outcome_emission",
            OutcomeErrorKind::TooManyItems => "too_many_outcome_items",
            OutcomeErrorKind::InvalidEmissionChannel => "invalid_outcome_emission_channel",
            OutcomeErrorKind::IncompatibleOutcome => "incompatible_action_outcome",
        })
    }
}

impl fmt::Debug for OutcomeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for OutcomeError {}
