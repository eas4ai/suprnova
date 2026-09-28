//! Canonical generated component, field, action, and version metadata.

mod browser;
mod component;
mod digest;
mod field;
mod generated;
mod method;
mod version;

use std::error::Error;
use std::fmt;

pub use browser::{
    EffectMetadata, EffectPayloadMetadata, EventMetadata, EventPayloadMetadata,
    PayloadContractIdentity,
};
pub use component::ComponentMetadata;
pub use field::FieldMetadata;
pub use generated::{LiveComponentContract, LiveComponentDefinitionMetadata};
pub use method::ActionMetadata;
pub use version::ContractVersions;

/// Closed reason component metadata construction failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataErrorKind {
    /// Generated payload metadata contained an invalid registered identity.
    InvalidIdentity,
    /// A component, state, action, checker, or protocol version was zero.
    InvalidVersion,
    /// The component requires a Live protocol this engine does not implement.
    UnsupportedProtocol,
    /// More fields were declared than the bounded metadata profile permits.
    TooManyFields,
    /// More actions were declared than the bounded metadata profile permits.
    TooManyActions,
    /// More browser events were declared than the bounded profile permits.
    TooManyEvents,
    /// More browser effects were declared than the bounded profile permits.
    TooManyEffects,
    /// More event target scopes were declared than the bounded profile permits.
    TooManyEventTargets,
    /// An event target declaration was empty or otherwise inconsistent.
    InvalidEventTarget,
    /// The same exact event target scope was declared more than once.
    DuplicateEventTarget,
    /// An event fanout was zero, below its explicit targets, or above the fixed ceiling.
    InvalidEventFanout,
    /// More asynchronous subscriptions were declared than the component permits.
    TooManySubscriptions,
    /// More topic scopes were declared than one subscription permits.
    TooManySubscriptionTopics,
    /// More typed event names were declared than one subscription permits.
    TooManySubscriptionEvents,
    /// More physical transport modes were declared than the closed transport set permits.
    TooManySubscriptionModes,
    /// The component declared the same field identity more than once.
    DuplicateField,
    /// The component declared the same action identity more than once.
    DuplicateAction,
    /// Two declared event payload types registered the same browser identity.
    DuplicateEvent,
    /// Two declared effect payload types registered the same browser identity.
    DuplicateEffect,
    /// The same stream identity was declared more than once.
    DuplicateSubscription,
    /// The same trusted topic scope was declared more than once.
    DuplicateSubscriptionTopic,
    /// The same typed stream event was declared more than once.
    DuplicateSubscriptionEvent,
    /// The same approved stream transport was declared more than once.
    DuplicateSubscriptionMode,
    /// A subscription omitted a required bounded declaration.
    InvalidSubscriptionMetadata,
    /// A subscription named an absent or component-authored event.
    UnknownSubscriptionEvent,
    /// A stream-authored event was not registered by any subscription.
    UnregisteredStreamEvent,
    /// Field model, timing, category, or URL metadata was internally inconsistent.
    InvalidBindingMetadata,
    /// Action argument, authorization, validation, or transaction metadata was inconsistent.
    InvalidActionMetadata,
    /// Upload field policy was attached to an ineligible field or unknown action.
    InvalidUploadMetadata,
    /// Two registered fields exposed the same URL query key.
    DuplicateUrlQueryKey,
    /// Canonical contract metadata could not be encoded within fixed bounds.
    ContractEncodingFailed,
}

impl MetadataErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidIdentity => "invalid_metadata_identity",
            Self::InvalidVersion => "invalid_metadata_version",
            Self::UnsupportedProtocol => "unsupported_component_protocol",
            Self::TooManyFields => "too_many_component_fields",
            Self::TooManyActions => "too_many_component_actions",
            Self::TooManyEvents => "too_many_component_events",
            Self::TooManyEffects => "too_many_component_effects",
            Self::TooManyEventTargets => "too_many_event_targets",
            Self::InvalidEventTarget => "invalid_event_target",
            Self::DuplicateEventTarget => "duplicate_event_target",
            Self::InvalidEventFanout => "invalid_event_fanout",
            Self::TooManySubscriptions => "too_many_component_subscriptions",
            Self::TooManySubscriptionTopics => "too_many_subscription_topics",
            Self::TooManySubscriptionEvents => "too_many_subscription_events",
            Self::TooManySubscriptionModes => "too_many_subscription_modes",
            Self::DuplicateField => "duplicate_component_field",
            Self::DuplicateAction => "duplicate_component_action",
            Self::DuplicateEvent => "duplicate_component_event",
            Self::DuplicateEffect => "duplicate_component_effect",
            Self::DuplicateSubscription => "duplicate_component_subscription",
            Self::DuplicateSubscriptionTopic => "duplicate_subscription_topic",
            Self::DuplicateSubscriptionEvent => "duplicate_subscription_event",
            Self::DuplicateSubscriptionMode => "duplicate_subscription_mode",
            Self::InvalidSubscriptionMetadata => "invalid_subscription_metadata",
            Self::UnknownSubscriptionEvent => "unknown_subscription_event",
            Self::UnregisteredStreamEvent => "unregistered_stream_event",
            Self::InvalidBindingMetadata => "invalid_component_binding_metadata",
            Self::InvalidActionMetadata => "invalid_component_action_metadata",
            Self::InvalidUploadMetadata => "invalid_component_upload_metadata",
            Self::DuplicateUrlQueryKey => "duplicate_component_url_query_key",
            Self::ContractEncodingFailed => "component_contract_encoding_failed",
        }
    }
}

/// Redacted component-metadata construction error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MetadataError {
    kind: MetadataErrorKind,
}

impl MetadataError {
    pub(crate) const fn new(kind: MetadataErrorKind) -> Self {
        Self { kind }
    }

    /// Creates the closed upload-metadata failure used by framework generated-code adapters.
    #[doc(hidden)]
    #[must_use]
    pub const fn invalid_upload_metadata() -> Self {
        Self::new(MetadataErrorKind::InvalidUploadMetadata)
    }

    /// Returns the closed construction failure.
    #[must_use]
    pub const fn kind(self) -> MetadataErrorKind {
        self.kind
    }
}

impl fmt::Display for MetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for MetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for MetadataError {}
