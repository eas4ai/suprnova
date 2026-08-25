//! Current host authorization and descriptor-scoped transport credentials.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::crypto::SnapshotKeyRing;
use crate::host::{HostScopeFacts, TrustedLiveRequestContext};
use crate::identity::{ComponentName, ContentDigest, UnixMillis};
use crate::metadata::ComponentMetadata;

use super::subscription::canonical_claim_budget_for_registration;
use super::{
    ASYNC_SUBSCRIPTION_PROTOCOL_V1, AuthorizationMemo, BoundedEventContracts, BoundedTopics,
    CapabilityVersion, PollFallbackPolicy, ReconnectPolicy, StreamName, StreamPosition,
    SubscriptionClaims, SubscriptionDescriptor, SubscriptionDescriptorCodec, SubscriptionError,
    SubscriptionErrorKind, SubscriptionEventContract, SubscriptionMetadata, TransportCredential,
    VerifiedSubscriptionDescriptor,
};

const MAX_TRUSTED_MOUNT_PARAMETERS: usize = 32;
const MAX_TOPIC_PARAMETER_NAME_BYTES: usize = 64;
const MAX_TOPIC_PARAMETER_VALUE_BYTES: usize = 128;

/// Executor-neutral future returned by host subscription ports.
pub type SubscriptionFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Current authorization boundary being evaluated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionAuthorizationOperation {
    /// Initial descriptor issuance.
    Issue,
    /// Establishment of a physical or multiplexed subscription.
    Connect,
    /// Descriptor and credential renewal before exclusive expiry.
    Renew,
}

/// Closed current-policy result returned by the host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionAuthorizationDecision {
    /// Current principal and resource policy permits this boundary.
    Allow,
    /// Current principal or resource policy denies this boundary.
    Deny,
}

/// Closed host result for a descriptor-scoped transport credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionCredentialDecision {
    /// The separate secret is valid for exactly this signed descriptor binding.
    Accept,
    /// The secret is missing, revoked, expired, or bound elsewhere.
    Reject,
}

/// Opaque stable digest of one exact signed descriptor.
#[derive(Clone, Eq, PartialEq)]
pub struct SubscriptionBinding(ContentDigest);

impl SubscriptionBinding {
    fn from_descriptor(descriptor: &SubscriptionDescriptor) -> Result<Self, SubscriptionError> {
        let digest: [u8; 32] = Sha256::digest(descriptor.as_str().as_bytes()).into();
        ContentDigest::from_bytes(&digest)
            .map(Self)
            .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::InvalidDescriptor))
    }

    /// Returns the non-secret stable binding value for host credential storage.
    #[must_use]
    pub fn to_base64url(&self) -> String {
        self.0.to_base64url()
    }
}

impl fmt::Debug for SubscriptionBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SubscriptionBinding>")
    }
}

/// Exact non-secret subscription scope additionally bound into one transport credential.
#[derive(Clone, Eq, PartialEq)]
pub struct SubscriptionCredentialScope {
    component: ComponentName,
    component_contract: ContentDigest,
    authorization_memo: AuthorizationMemo,
    stream: StreamName,
    topics: BoundedTopics,
    events: BoundedEventContracts,
}

impl SubscriptionCredentialScope {
    fn from_current(context: &TrustedLiveRequestContext, claims: &SubscriptionClaims) -> Self {
        Self {
            component: context.mount().component().clone(),
            component_contract: context.mount().contract_digest().clone(),
            authorization_memo: claims.authorization_memo().clone(),
            stream: claims.stream().clone(),
            topics: claims.topics().clone(),
            events: claims.events().clone(),
        }
    }

    /// Returns the registry component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        &self.component
    }

    /// Returns the canonical component contract digest.
    #[must_use]
    pub const fn component_contract(&self) -> &ContentDigest {
        &self.component_contract
    }

    /// Returns the signed current-identity authorization memo.
    #[must_use]
    pub const fn authorization_memo(&self) -> &AuthorizationMemo {
        &self.authorization_memo
    }

    /// Returns the registered stream identity.
    #[must_use]
    pub const fn stream(&self) -> &StreamName {
        &self.stream
    }

    /// Returns exact resolved topic scope.
    #[must_use]
    pub const fn topics(&self) -> &BoundedTopics {
        &self.topics
    }

    /// Returns full registered event contracts.
    #[must_use]
    pub const fn events(&self) -> &BoundedEventContracts {
        &self.events
    }
}

impl fmt::Debug for SubscriptionCredentialScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SubscriptionCredentialScope:redacted>")
    }
}

/// Exact current facts supplied to the host authorization provider.
#[derive(Clone, Copy)]
pub struct SubscriptionAuthorizationRequest<'a> {
    operation: SubscriptionAuthorizationOperation,
    component: &'a ComponentName,
    host_scope: &'a HostScopeFacts,
    stream: &'a StreamName,
    topics: &'a BoundedTopics,
    binding: &'a SubscriptionBinding,
}

impl<'a> SubscriptionAuthorizationRequest<'a> {
    fn new(
        operation: SubscriptionAuthorizationOperation,
        context: &'a TrustedLiveRequestContext,
        claims: &'a SubscriptionClaims,
        binding: &'a SubscriptionBinding,
    ) -> Self {
        Self {
            operation,
            component: context.mount().component(),
            host_scope: context.host_scope_facts(),
            stream: claims.stream(),
            topics: claims.topics(),
            binding,
        }
    }

    /// Returns the current authorization boundary.
    #[must_use]
    pub const fn operation(self) -> SubscriptionAuthorizationOperation {
        self.operation
    }

    /// Returns the registry-verified component identity.
    #[must_use]
    pub const fn component(self) -> &'a ComponentName {
        self.component
    }

    /// Returns current normalized principal, session, tenant, and aggregate scope.
    #[must_use]
    pub const fn host_scope(self) -> &'a HostScopeFacts {
        self.host_scope
    }

    /// Returns the signed registered stream identity.
    #[must_use]
    pub const fn stream(self) -> &'a StreamName {
        self.stream
    }

    /// Returns signed trusted topic scopes.
    #[must_use]
    pub const fn topics(self) -> &'a BoundedTopics {
        self.topics
    }

    /// Returns the exact signed descriptor binding.
    #[must_use]
    pub const fn binding(self) -> &'a SubscriptionBinding {
        self.binding
    }
}

impl fmt::Debug for SubscriptionAuthorizationRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubscriptionAuthorizationRequest")
            .field("operation", &self.operation)
            .field("component", &self.component.as_str())
            .field("stream", &self.stream.as_str())
            .field("topic_count", &self.topics.as_slice().len())
            .field("scope", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Host-owned current principal and resource policy for subscriptions.
pub trait SubscriptionAuthorizationPort: Send + Sync {
    /// Rechecks current principal/session/tenant/component/stream/topic authority.
    fn authorize<'a>(
        &'a self,
        request: SubscriptionAuthorizationRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<SubscriptionAuthorizationDecision, SubscriptionError>>;
}

/// Bounded canonical mount parameters obtained only through the trusted host registry port.
#[derive(Clone, Eq, PartialEq)]
pub struct TrustedMountParameters(BTreeMap<String, String>);

impl TrustedMountParameters {
    /// Validates stable parameter names and single-topic-segment values before storage.
    pub fn new(values: Vec<(String, String)>) -> Result<Self, SubscriptionError> {
        if values.len() > MAX_TRUSTED_MOUNT_PARAMETERS {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::UnregisteredSubscription,
            ));
        }
        let mut parameters = BTreeMap::new();
        for (name, value) in values {
            if !valid_topic_parameter(&name, MAX_TOPIC_PARAMETER_NAME_BYTES)
                || !valid_topic_parameter(&value, MAX_TOPIC_PARAMETER_VALUE_BYTES)
                || parameters.insert(name, value).is_some()
            {
                return Err(SubscriptionError::new(
                    SubscriptionErrorKind::UnregisteredSubscription,
                ));
            }
        }
        Ok(Self(parameters))
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

impl fmt::Debug for TrustedMountParameters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TrustedMountParameters:redacted>")
    }
}

/// Current registry-owned subscription contract after trusted topic resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentSubscriptionRegistration {
    component: ComponentName,
    component_contract: ContentDigest,
    stream: StreamName,
    topics: BoundedTopics,
    events: BoundedEventContracts,
    reconnect: ReconnectPolicy,
    canonical_claim_budget_bytes: usize,
}

impl CurrentSubscriptionRegistration {
    /// Resolves one current stream from component metadata and trusted mount parameters.
    pub fn from_registered(
        component: &ComponentMetadata,
        stream: &StreamName,
        mount_parameters: &TrustedMountParameters,
    ) -> Result<Self, SubscriptionError> {
        let metadata = component
            .subscriptions()
            .iter()
            .find(|metadata| metadata.stream() == stream)
            .ok_or_else(|| {
                SubscriptionError::new(SubscriptionErrorKind::UnregisteredSubscription)
            })?;
        let topics = resolve_topics(metadata.topics(), mount_parameters)?;
        let events = registered_event_contracts(component, metadata)?;
        let canonical_claim_budget_bytes = canonical_claim_budget_for_registration(
            metadata.stream(),
            &topics,
            &events,
            metadata.reconnect(),
        )?;
        Ok(Self {
            component: component.identity().clone(),
            component_contract: component.contract_digest().clone(),
            stream: metadata.stream().clone(),
            topics,
            events,
            reconnect: metadata.reconnect(),
            canonical_claim_budget_bytes,
        })
    }

    /// Returns the current registry component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        &self.component
    }

    /// Returns the current canonical component contract digest.
    #[must_use]
    pub const fn component_contract(&self) -> &ContentDigest {
        &self.component_contract
    }

    /// Returns the current registered stream identity.
    #[must_use]
    pub const fn stream(&self) -> &StreamName {
        &self.stream
    }

    /// Returns topics resolved only from registered templates and trusted parameters.
    #[must_use]
    pub const fn topics(&self) -> &BoundedTopics {
        &self.topics
    }

    /// Returns current full registered event contracts.
    #[must_use]
    pub const fn events(&self) -> &BoundedEventContracts {
        &self.events
    }

    /// Returns current registered reconnect behavior.
    #[must_use]
    pub const fn reconnect(&self) -> ReconnectPolicy {
        self.reconnect
    }

    /// Returns the calculated worst-case canonical claims bytes for this registration.
    #[must_use]
    pub const fn canonical_claim_budget_bytes(&self) -> usize {
        self.canonical_claim_budget_bytes
    }
}

/// Exact current registry lookup requested by the subscription service.
#[derive(Clone, Copy)]
pub struct SubscriptionRegistryRequest<'a> {
    operation: SubscriptionAuthorizationOperation,
    component: &'a ComponentName,
    component_contract: &'a ContentDigest,
    stream: &'a StreamName,
}

impl<'a> SubscriptionRegistryRequest<'a> {
    fn new(
        operation: SubscriptionAuthorizationOperation,
        context: &'a TrustedLiveRequestContext,
        stream: &'a StreamName,
    ) -> Self {
        Self {
            operation,
            component: context.mount().component(),
            component_contract: context.mount().contract_digest(),
            stream,
        }
    }

    /// Returns the current lifecycle boundary.
    #[must_use]
    pub const fn operation(self) -> SubscriptionAuthorizationOperation {
        self.operation
    }

    /// Returns the validated mounted component identity.
    #[must_use]
    pub const fn component(self) -> &'a ComponentName {
        self.component
    }

    /// Returns the validated mounted component contract digest.
    #[must_use]
    pub const fn component_contract(self) -> &'a ContentDigest {
        self.component_contract
    }

    /// Returns the signed or requested registered stream identity.
    #[must_use]
    pub const fn stream(self) -> &'a StreamName {
        self.stream
    }
}

impl fmt::Debug for SubscriptionRegistryRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubscriptionRegistryRequest")
            .field("operation", &self.operation)
            .field("component", &self.component.as_str())
            .field("component_contract", &self.component_contract)
            .field("stream", &self.stream.as_str())
            .finish()
    }
}

/// Host-owned current component registry and trusted mount-parameter resolver.
pub trait SubscriptionRegistryPort: Send + Sync {
    /// Re-resolves one current subscription contract for every lifecycle boundary.
    fn resolve<'a>(
        &'a self,
        request: SubscriptionRegistryRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<CurrentSubscriptionRegistration, SubscriptionError>>;
}

/// Host-minted authoritative stream position used only for descriptor issuance.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AuthoritativeStreamPosition(StreamPosition);

impl AuthoritativeStreamPosition {
    /// Wraps a position obtained from the host continuity authority.
    #[must_use]
    pub const fn from_host_continuity(position: StreamPosition) -> Self {
        Self(position)
    }

    fn into_position(self) -> StreamPosition {
        self.0
    }
}

impl fmt::Debug for AuthoritativeStreamPosition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AuthoritativeStreamPosition>")
    }
}

/// Current resolved subscription scope supplied to the host continuity authority.
#[derive(Clone, Copy)]
pub struct SubscriptionBaselineRequest<'a> {
    component: &'a ComponentName,
    component_contract: &'a ContentDigest,
    host_scope: &'a HostScopeFacts,
    stream: &'a StreamName,
    topics: &'a BoundedTopics,
    events: &'a BoundedEventContracts,
}

impl<'a> SubscriptionBaselineRequest<'a> {
    fn new(
        context: &'a TrustedLiveRequestContext,
        registration: &'a CurrentSubscriptionRegistration,
    ) -> Self {
        Self {
            component: registration.component(),
            component_contract: registration.component_contract(),
            host_scope: context.host_scope_facts(),
            stream: registration.stream(),
            topics: registration.topics(),
            events: registration.events(),
        }
    }

    /// Returns the current registry component identity.
    #[must_use]
    pub const fn component(self) -> &'a ComponentName {
        self.component
    }

    /// Returns the current component contract digest.
    #[must_use]
    pub const fn component_contract(self) -> &'a ContentDigest {
        self.component_contract
    }

    /// Returns the current normalized host scope.
    #[must_use]
    pub const fn host_scope(self) -> &'a HostScopeFacts {
        self.host_scope
    }

    /// Returns the current registered stream.
    #[must_use]
    pub const fn stream(self) -> &'a StreamName {
        self.stream
    }

    /// Returns the current resolved topics.
    #[must_use]
    pub const fn topics(self) -> &'a BoundedTopics {
        self.topics
    }

    /// Returns the current full event contracts.
    #[must_use]
    pub const fn events(self) -> &'a BoundedEventContracts {
        self.events
    }
}

impl fmt::Debug for SubscriptionBaselineRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SubscriptionBaselineRequest:redacted>")
    }
}

/// Host-owned continuity authority for the first required stream event.
pub trait SubscriptionContinuityPort: Send + Sync {
    /// Returns the authoritative baseline for the exact current subscription scope.
    fn authoritative_baseline<'a>(
        &'a self,
        request: SubscriptionBaselineRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<AuthoritativeStreamPosition, SubscriptionError>>;
}

/// Exact descriptor binding supplied to the separate credential provider.
#[derive(Clone, Copy)]
pub struct SubscriptionCredentialRequest<'a> {
    operation: SubscriptionAuthorizationOperation,
    binding: &'a SubscriptionBinding,
    scope: &'a SubscriptionCredentialScope,
    expires_at: UnixMillis,
    now: UnixMillis,
    presented: Option<&'a TransportCredential>,
}

impl<'a> SubscriptionCredentialRequest<'a> {
    fn issue(
        operation: SubscriptionAuthorizationOperation,
        binding: &'a SubscriptionBinding,
        scope: &'a SubscriptionCredentialScope,
        expires_at: UnixMillis,
        now: UnixMillis,
    ) -> Self {
        Self {
            operation,
            binding,
            scope,
            expires_at,
            now,
            presented: None,
        }
    }

    fn verify(
        operation: SubscriptionAuthorizationOperation,
        binding: &'a SubscriptionBinding,
        scope: &'a SubscriptionCredentialScope,
        expires_at: UnixMillis,
        now: UnixMillis,
        presented: &'a TransportCredential,
    ) -> Self {
        Self {
            operation,
            binding,
            scope,
            expires_at,
            now,
            presented: Some(presented),
        }
    }

    /// Returns the exact operation.
    #[must_use]
    pub const fn operation(self) -> SubscriptionAuthorizationOperation {
        self.operation
    }

    /// Returns the descriptor binding that scopes the credential.
    #[must_use]
    pub const fn binding(self) -> &'a SubscriptionBinding {
        self.binding
    }

    /// Returns the exact current subscription scope.
    #[must_use]
    pub const fn scope(self) -> &'a SubscriptionCredentialScope {
        self.scope
    }

    /// Returns the descriptor's exclusive expiry.
    #[must_use]
    pub const fn expires_at(self) -> UnixMillis {
        self.expires_at
    }

    /// Returns the current host time used for exclusive credential expiry.
    #[must_use]
    pub const fn now(self) -> UnixMillis {
        self.now
    }

    /// Returns the separately presented bearer only at verification boundaries.
    #[must_use]
    pub const fn presented(self) -> Option<&'a TransportCredential> {
        self.presented
    }
}

impl fmt::Debug for SubscriptionCredentialRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubscriptionCredentialRequest")
            .field("operation", &self.operation)
            .field("binding", &self.binding)
            .field("scope", &self.scope)
            .field("expires_at", &self.expires_at)
            .field("now", &self.now)
            .field("presented", &self.presented.map(|_| "<redacted>"))
            .finish()
    }
}

/// Host-owned store or issuer for descriptor-scoped non-loggable credentials.
pub trait SubscriptionCredentialPort: Send + Sync {
    /// Mints a unique, cryptographically unpredictable secret bound to the exact descriptor.
    ///
    /// Implementations must never reuse an unexpired bearer, including across
    /// processes or restarts that share the same credential authority.
    fn issue<'a>(
        &'a self,
        request: SubscriptionCredentialRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<TransportCredential, SubscriptionError>>;

    /// Atomically verifies and consumes a presented operation-scoped secret.
    ///
    /// The host credential authority owns replay prevention. A successful
    /// consume must make every later consume of the same bearer fail across
    /// all processes and restarts sharing that authority.
    fn verify_and_consume<'a>(
        &'a self,
        request: SubscriptionCredentialRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<SubscriptionCredentialDecision, SubscriptionError>>;
}

/// Non-authoritative bounded inputs selecting one current registered stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionIssueRequest {
    stream: StreamName,
    capability: CapabilityVersion,
    expires_at: UnixMillis,
    fallback_poll: PollFallbackPolicy,
}

impl SubscriptionIssueRequest {
    /// Selects a stream; the service independently re-resolves current registry authority.
    #[must_use]
    pub const fn new(
        stream: StreamName,
        capability: CapabilityVersion,
        expires_at: UnixMillis,
        fallback_poll: PollFallbackPolicy,
    ) -> Self {
        Self {
            stream,
            capability,
            expires_at,
            fallback_poll,
        }
    }
}

/// Newly signed public descriptor paired with its separately secret credential.
pub struct IssuedSubscription {
    descriptor: SubscriptionDescriptor,
    transport_credential: TransportCredential,
    expires_at: UnixMillis,
}

impl IssuedSubscription {
    /// Returns the signed non-secret descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &SubscriptionDescriptor {
        &self.descriptor
    }

    /// Returns the separate non-loggable bearer.
    #[must_use]
    pub const fn transport_credential(&self) -> &TransportCredential {
        &self.transport_credential
    }

    /// Returns the exclusive expiry shared by descriptor and credential binding.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }
}

impl fmt::Debug for IssuedSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<IssuedSubscription:redacted>")
    }
}

/// Connect-authorized subscription with a separate operation-scoped renewal credential.
pub struct AuthorizedSubscription {
    verified: VerifiedSubscriptionDescriptor,
    renewal_credential: TransportCredential,
}

impl AuthorizedSubscription {
    /// Returns exact integrity-verified and currently authorized claims.
    #[must_use]
    pub const fn verified(&self) -> &VerifiedSubscriptionDescriptor {
        &self.verified
    }

    /// Returns the separate secret accepted only at the renewal boundary.
    #[must_use]
    pub const fn renewal_credential(&self) -> &TransportCredential {
        &self.renewal_credential
    }
}

impl fmt::Debug for AuthorizedSubscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AuthorizedSubscription:redacted>")
    }
}

/// Trusted issue/connect/renew coordinator over host-owned current policy.
pub struct SubscriptionService {
    codec: SubscriptionDescriptorCodec,
    last_issued_credential: Mutex<Option<[u8; 32]>>,
}

impl SubscriptionService {
    /// Creates a service backed by the bounded rotating descriptor key ring.
    #[must_use]
    pub const fn new(keys: SnapshotKeyRing) -> Self {
        Self {
            codec: SubscriptionDescriptorCodec::new(keys),
            last_issued_credential: Mutex::new(None),
        }
    }

    /// Reauthorizes and issues a signed descriptor plus separately scoped credential.
    pub async fn issue(
        &self,
        context: &TrustedLiveRequestContext,
        request: SubscriptionIssueRequest,
        now: UnixMillis,
    ) -> Result<IssuedSubscription, SubscriptionError> {
        ensure_current_context(context, now)?;
        let registration = resolve_current(
            context,
            SubscriptionAuthorizationOperation::Issue,
            &request.stream,
        )
        .await?;
        let baseline = authoritative_baseline(context, &registration).await?;
        let memo = authorization_memo(context)?;
        let claims = SubscriptionClaims::new(
            registration.stream().clone(),
            ASYNC_SUBSCRIPTION_PROTOCOL_V1,
            request.capability,
            registration.topics().clone(),
            registration.events().clone(),
            memo,
            baseline,
            request.expires_at,
            registration.reconnect(),
            request.fallback_poll,
        )?;
        let descriptor = self.codec.sign(&claims, now)?;
        let binding = SubscriptionBinding::from_descriptor(&descriptor)?;
        authorize(
            context,
            SubscriptionAuthorizationRequest::new(
                SubscriptionAuthorizationOperation::Issue,
                context,
                &claims,
                &binding,
            ),
        )
        .await?;
        let credential_scope = SubscriptionCredentialScope::from_current(context, &claims);
        let credentials = context
            .capabilities()
            .subscription_credentials()
            .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::CredentialUnavailable))?;
        let credential = credentials
            .issue(SubscriptionCredentialRequest::issue(
                SubscriptionAuthorizationOperation::Connect,
                &binding,
                &credential_scope,
                claims.expires_at(),
                now,
            ))
            .await?;
        self.reject_immediately_repeated_credential(&credential)?;
        Ok(IssuedSubscription {
            descriptor,
            transport_credential: credential,
            expires_at: claims.expires_at(),
        })
    }

    /// Verifies descriptor scope/expiry and credential before current connect policy.
    pub async fn connect(
        &self,
        context: &TrustedLiveRequestContext,
        descriptor: &SubscriptionDescriptor,
        credential: &TransportCredential,
        now: UnixMillis,
    ) -> Result<AuthorizedSubscription, SubscriptionError> {
        let (verified, binding) = self
            .verify_current(
                context,
                descriptor,
                SubscriptionAuthorizationOperation::Connect,
                now,
            )
            .await?;
        let credential_scope =
            SubscriptionCredentialScope::from_current(context, verified.claims());
        verify_credential(
            context,
            SubscriptionAuthorizationOperation::Connect,
            &binding,
            &credential_scope,
            verified.expires_at(),
            now,
            credential,
        )
        .await?;
        authorize(
            context,
            SubscriptionAuthorizationRequest::new(
                SubscriptionAuthorizationOperation::Connect,
                context,
                verified.claims(),
                &binding,
            ),
        )
        .await?;
        let credentials = context
            .capabilities()
            .subscription_credentials()
            .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::CredentialUnavailable))?;
        let renewal_credential = credentials
            .issue(SubscriptionCredentialRequest::issue(
                SubscriptionAuthorizationOperation::Renew,
                &binding,
                &credential_scope,
                verified.expires_at(),
                now,
            ))
            .await?;
        self.reject_immediately_repeated_credential(&renewal_credential)?;
        Ok(AuthorizedSubscription {
            verified,
            renewal_credential,
        })
    }

    /// Reauthorizes and rotates both descriptor expiry and separate credential.
    #[allow(
        clippy::too_many_arguments,
        reason = "renewal keeps authority inputs explicit"
    )]
    pub async fn renew(
        &self,
        context: &TrustedLiveRequestContext,
        descriptor: &SubscriptionDescriptor,
        credential: &TransportCredential,
        expires_at: UnixMillis,
        now: UnixMillis,
    ) -> Result<IssuedSubscription, SubscriptionError> {
        let (verified, old_binding) = self
            .verify_current(
                context,
                descriptor,
                SubscriptionAuthorizationOperation::Renew,
                now,
            )
            .await?;
        let old_scope = SubscriptionCredentialScope::from_current(context, verified.claims());
        verify_credential(
            context,
            SubscriptionAuthorizationOperation::Renew,
            &old_binding,
            &old_scope,
            verified.expires_at(),
            now,
            credential,
        )
        .await?;
        authorize(
            context,
            SubscriptionAuthorizationRequest::new(
                SubscriptionAuthorizationOperation::Renew,
                context,
                verified.claims(),
                &old_binding,
            ),
        )
        .await?;
        let old = verified.claims();
        let claims = SubscriptionClaims::new(
            old.stream().clone(),
            old.protocol(),
            old.capability(),
            old.topics().clone(),
            old.events().clone(),
            old.authorization_memo().clone(),
            old.baseline(),
            expires_at,
            old.reconnect(),
            old.fallback_poll(),
        )?;
        let descriptor = self.codec.sign(&claims, now)?;
        let binding = SubscriptionBinding::from_descriptor(&descriptor)?;
        let credential_scope = SubscriptionCredentialScope::from_current(context, &claims);
        let credentials = context
            .capabilities()
            .subscription_credentials()
            .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::CredentialUnavailable))?;
        let credential = credentials
            .issue(SubscriptionCredentialRequest::issue(
                SubscriptionAuthorizationOperation::Connect,
                &binding,
                &credential_scope,
                expires_at,
                now,
            ))
            .await?;
        self.reject_immediately_repeated_credential(&credential)?;
        Ok(IssuedSubscription {
            descriptor,
            transport_credential: credential,
            expires_at,
        })
    }

    async fn verify_current(
        &self,
        context: &TrustedLiveRequestContext,
        descriptor: &SubscriptionDescriptor,
        operation: SubscriptionAuthorizationOperation,
        now: UnixMillis,
    ) -> Result<(VerifiedSubscriptionDescriptor, SubscriptionBinding), SubscriptionError> {
        ensure_current_context(context, now)?;
        let verified = self.codec.verify(descriptor, now)?;
        let claims = verified.claims();
        let registration = resolve_current(context, operation, claims.stream()).await?;
        if claims.topics() != registration.topics()
            || claims.events() != registration.events()
            || claims.reconnect() != registration.reconnect()
            || claims.authorization_memo() != &authorization_memo(context)?
        {
            return Err(SubscriptionError::new(SubscriptionErrorKind::ScopeMismatch));
        }
        Ok((verified, SubscriptionBinding::from_descriptor(descriptor)?))
    }

    fn reject_immediately_repeated_credential(
        &self,
        credential: &TransportCredential,
    ) -> Result<(), SubscriptionError> {
        let digest: [u8; 32] = Sha256::digest(credential.expose_authorization_bearer()).into();
        let mut previous = self
            .last_issued_credential
            .lock()
            .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::CredentialUnavailable))?;
        if previous.as_ref() == Some(&digest) {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::InvalidCredential,
            ));
        }
        *previous = Some(digest);
        Ok(())
    }
}

async fn authoritative_baseline(
    context: &TrustedLiveRequestContext,
    registration: &CurrentSubscriptionRegistration,
) -> Result<StreamPosition, SubscriptionError> {
    let continuity = context
        .capabilities()
        .subscription_continuity()
        .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::AuthorizationUnavailable))?;
    continuity
        .authoritative_baseline(SubscriptionBaselineRequest::new(context, registration))
        .await
        .map(AuthoritativeStreamPosition::into_position)
}

async fn resolve_current(
    context: &TrustedLiveRequestContext,
    operation: SubscriptionAuthorizationOperation,
    stream: &StreamName,
) -> Result<CurrentSubscriptionRegistration, SubscriptionError> {
    let registry = context
        .capabilities()
        .subscription_registry()
        .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::AuthorizationUnavailable))?;
    let registration = registry
        .resolve(SubscriptionRegistryRequest::new(operation, context, stream))
        .await?;
    if registration.component() != context.mount().component()
        || registration.component_contract() != context.mount().contract_digest()
        || registration.stream() != stream
    {
        return Err(SubscriptionError::new(SubscriptionErrorKind::ScopeMismatch));
    }
    Ok(registration)
}

fn registered_event_contracts(
    component: &ComponentMetadata,
    subscription: &SubscriptionMetadata,
) -> Result<BoundedEventContracts, SubscriptionError> {
    let events = subscription
        .events()
        .as_slice()
        .iter()
        .map(|name| {
            component
                .events()
                .iter()
                .find(|event| event.name() == name)
                .ok_or_else(|| {
                    SubscriptionError::new(SubscriptionErrorKind::UnregisteredSubscription)
                })
                .and_then(SubscriptionEventContract::from_registered)
        })
        .collect::<Result<Vec<_>, _>>()?;
    BoundedEventContracts::new(events)
}

fn resolve_topics(
    registered: &BoundedTopics,
    parameters: &TrustedMountParameters,
) -> Result<BoundedTopics, SubscriptionError> {
    let topics = registered
        .as_slice()
        .iter()
        .map(|topic| {
            let mut resolved = String::with_capacity(topic.as_str().len());
            for (index, segment) in topic.as_str().split('/').enumerate() {
                if index != 0 {
                    resolved.push('/');
                }
                if let Some(parameter) = segment.strip_prefix(':') {
                    if parameter.is_empty()
                        || !valid_topic_parameter(parameter, MAX_TOPIC_PARAMETER_NAME_BYTES)
                    {
                        return Err(SubscriptionError::new(
                            SubscriptionErrorKind::UnregisteredSubscription,
                        ));
                    }
                    resolved.push_str(parameters.get(parameter).ok_or_else(|| {
                        SubscriptionError::new(SubscriptionErrorKind::UnregisteredSubscription)
                    })?);
                } else {
                    resolved.push_str(segment);
                }
                if resolved.len() > 256 {
                    return Err(SubscriptionError::new(
                        SubscriptionErrorKind::UnregisteredSubscription,
                    ));
                }
            }
            super::TopicName::parse(&resolved).map_err(|_| {
                SubscriptionError::new(SubscriptionErrorKind::UnregisteredSubscription)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    BoundedTopics::new(topics)
        .map_err(|_| SubscriptionError::new(SubscriptionErrorKind::UnregisteredSubscription))
}

fn valid_topic_parameter(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

impl fmt::Debug for SubscriptionService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SubscriptionService:redacted>")
    }
}

async fn authorize(
    context: &TrustedLiveRequestContext,
    request: SubscriptionAuthorizationRequest<'_>,
) -> Result<(), SubscriptionError> {
    let authorization = context
        .capabilities()
        .subscription_authorization()
        .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::AuthorizationUnavailable))?;
    match authorization.authorize(request).await? {
        SubscriptionAuthorizationDecision::Allow => Ok(()),
        SubscriptionAuthorizationDecision::Deny => Err(SubscriptionError::new(
            SubscriptionErrorKind::AuthorizationDenied,
        )),
    }
}

async fn verify_credential(
    context: &TrustedLiveRequestContext,
    operation: SubscriptionAuthorizationOperation,
    binding: &SubscriptionBinding,
    scope: &SubscriptionCredentialScope,
    expires_at: UnixMillis,
    now: UnixMillis,
    credential: &TransportCredential,
) -> Result<(), SubscriptionError> {
    let credentials = context
        .capabilities()
        .subscription_credentials()
        .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::CredentialUnavailable))?;
    match credentials
        .verify_and_consume(SubscriptionCredentialRequest::verify(
            operation, binding, scope, expires_at, now, credential,
        ))
        .await?
    {
        SubscriptionCredentialDecision::Accept => Ok(()),
        SubscriptionCredentialDecision::Reject => Err(SubscriptionError::new(
            SubscriptionErrorKind::InvalidCredential,
        )),
    }
}

fn ensure_current_context(
    context: &TrustedLiveRequestContext,
    now: UnixMillis,
) -> Result<(), SubscriptionError> {
    if context.is_current(now) {
        Ok(())
    } else {
        Err(SubscriptionError::new(
            SubscriptionErrorKind::ContextExpired,
        ))
    }
}

fn authorization_memo(
    context: &TrustedLiveRequestContext,
) -> Result<AuthorizationMemo, SubscriptionError> {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/async-authorization-memo/v1\0");
    hash_part(&mut digest, context.mount().component().as_str().as_bytes());
    hash_part(&mut digest, context.mount().contract_digest().as_bytes());
    let scope = context.host_scope_facts();
    hash_part(&mut digest, scope.scope().as_bytes());
    hash_optional_digest(
        &mut digest,
        scope.session().map(|value| value.digest().as_bytes()),
    );
    hash_optional_digest(
        &mut digest,
        scope.principal().map(|value| value.digest().as_bytes()),
    );
    hash_optional_digest(
        &mut digest,
        scope.tenant().map(|value| value.digest().as_bytes()),
    );
    AuthorizationMemo::parse(&format!("v1:{}", URL_SAFE_NO_PAD.encode(digest.finalize())))
}

fn hash_optional_digest(digest: &mut Sha256, value: Option<&[u8]>) {
    match value {
        Some(value) => {
            digest.update([1]);
            hash_part(digest, value);
        }
        None => digest.update([0]),
    }
}

fn hash_part(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}
