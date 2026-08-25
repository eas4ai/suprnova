//! Current host authorization and descriptor-scoped transport credentials.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::crypto::SnapshotKeyRing;
use crate::host::{HostScopeFacts, TrustedLiveRequestContext};
use crate::identity::{ComponentName, ContentDigest, UnixMillis};
use crate::metadata::ComponentMetadata;

use super::{
    ASYNC_SUBSCRIPTION_PROTOCOL_V1, AuthorizationMemo, BoundedTopics, CapabilityVersion,
    PollFallbackPolicy, StreamName, StreamPosition, SubscriptionClaims, SubscriptionDescriptor,
    SubscriptionDescriptorCodec, SubscriptionError, SubscriptionErrorKind, SubscriptionMetadata,
    TransportCredential, VerifiedSubscriptionDescriptor,
};

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

/// Exact descriptor binding supplied to the separate credential provider.
#[derive(Clone, Copy)]
pub struct SubscriptionCredentialRequest<'a> {
    operation: SubscriptionAuthorizationOperation,
    binding: &'a SubscriptionBinding,
    expires_at: UnixMillis,
    presented: Option<&'a TransportCredential>,
}

impl<'a> SubscriptionCredentialRequest<'a> {
    fn issue(
        operation: SubscriptionAuthorizationOperation,
        binding: &'a SubscriptionBinding,
        expires_at: UnixMillis,
    ) -> Self {
        Self {
            operation,
            binding,
            expires_at,
            presented: None,
        }
    }

    fn verify(
        operation: SubscriptionAuthorizationOperation,
        binding: &'a SubscriptionBinding,
        expires_at: UnixMillis,
        presented: &'a TransportCredential,
    ) -> Self {
        Self {
            operation,
            binding,
            expires_at,
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

    /// Returns the descriptor's exclusive expiry.
    #[must_use]
    pub const fn expires_at(self) -> UnixMillis {
        self.expires_at
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
            .field("expires_at", &self.expires_at)
            .field("presented", &self.presented.map(|_| "<redacted>"))
            .finish()
    }
}

/// Host-owned store or issuer for descriptor-scoped non-loggable credentials.
pub trait SubscriptionCredentialPort: Send + Sync {
    /// Mints a separate secret bound to the exact signed descriptor.
    fn issue<'a>(
        &'a self,
        request: SubscriptionCredentialRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<TransportCredential, SubscriptionError>>;

    /// Verifies a presented secret against the exact descriptor binding.
    fn verify<'a>(
        &'a self,
        request: SubscriptionCredentialRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<SubscriptionCredentialDecision, SubscriptionError>>;
}

/// Trusted registry-selected inputs for initial descriptor issuance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionIssueRequest {
    component: ComponentName,
    component_contract: ContentDigest,
    metadata: SubscriptionMetadata,
    capability: CapabilityVersion,
    baseline: StreamPosition,
    expires_at: UnixMillis,
    fallback_poll: PollFallbackPolicy,
}

impl SubscriptionIssueRequest {
    /// Selects one stream from registry-digested component metadata.
    ///
    /// The service compares the retained digest and component identity to the
    /// validated mount before signing, so standalone directive-created metadata
    /// cannot become descriptor authority.
    pub fn from_registered(
        component: &ComponentMetadata,
        stream: &StreamName,
        capability: CapabilityVersion,
        baseline: StreamPosition,
        expires_at: UnixMillis,
        fallback_poll: PollFallbackPolicy,
    ) -> Result<Self, SubscriptionError> {
        let metadata = component
            .subscriptions()
            .iter()
            .find(|metadata| metadata.stream() == stream)
            .cloned()
            .ok_or_else(|| {
                SubscriptionError::new(SubscriptionErrorKind::UnregisteredSubscription)
            })?;
        Ok(Self {
            component: component.identity().clone(),
            component_contract: component.contract_digest().clone(),
            metadata,
            capability,
            baseline,
            expires_at,
            fallback_poll,
        })
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

/// Connect-authorized subscription retaining only verified public claims.
#[derive(Clone, Eq, PartialEq)]
pub struct AuthorizedSubscription {
    verified: VerifiedSubscriptionDescriptor,
}

impl AuthorizedSubscription {
    /// Returns exact integrity-verified and currently authorized claims.
    #[must_use]
    pub const fn verified(&self) -> &VerifiedSubscriptionDescriptor {
        &self.verified
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
}

impl SubscriptionService {
    /// Creates a service backed by the bounded rotating descriptor key ring.
    #[must_use]
    pub const fn new(keys: SnapshotKeyRing) -> Self {
        Self {
            codec: SubscriptionDescriptorCodec::new(keys),
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
        if request.component != *context.mount().component()
            || request.component_contract != *context.mount().contract_digest()
        {
            return Err(SubscriptionError::new(
                SubscriptionErrorKind::UnregisteredSubscription,
            ));
        }
        let memo = authorization_memo(context)?;
        let claims = SubscriptionClaims::new(
            request.metadata.stream().clone(),
            ASYNC_SUBSCRIPTION_PROTOCOL_V1,
            request.capability,
            request.metadata.topics().clone(),
            request.metadata.events().clone(),
            memo,
            request.baseline,
            request.expires_at,
            request.metadata.reconnect(),
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
        let credentials = context
            .capabilities()
            .subscription_credentials()
            .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::CredentialUnavailable))?;
        let credential = credentials
            .issue(SubscriptionCredentialRequest::issue(
                SubscriptionAuthorizationOperation::Issue,
                &binding,
                claims.expires_at(),
            ))
            .await?;
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
        expected: &SubscriptionMetadata,
        now: UnixMillis,
    ) -> Result<AuthorizedSubscription, SubscriptionError> {
        let (verified, binding) = self.verify_current(context, descriptor, expected, now)?;
        verify_credential(
            context,
            SubscriptionAuthorizationOperation::Connect,
            &binding,
            verified.expires_at(),
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
        Ok(AuthorizedSubscription { verified })
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
        expected: &SubscriptionMetadata,
        expires_at: UnixMillis,
        now: UnixMillis,
    ) -> Result<IssuedSubscription, SubscriptionError> {
        let (verified, old_binding) = self.verify_current(context, descriptor, expected, now)?;
        verify_credential(
            context,
            SubscriptionAuthorizationOperation::Renew,
            &old_binding,
            verified.expires_at(),
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
        let credentials = context
            .capabilities()
            .subscription_credentials()
            .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::CredentialUnavailable))?;
        let credential = credentials
            .issue(SubscriptionCredentialRequest::issue(
                SubscriptionAuthorizationOperation::Renew,
                &binding,
                expires_at,
            ))
            .await?;
        Ok(IssuedSubscription {
            descriptor,
            transport_credential: credential,
            expires_at,
        })
    }

    fn verify_current(
        &self,
        context: &TrustedLiveRequestContext,
        descriptor: &SubscriptionDescriptor,
        expected: &SubscriptionMetadata,
        now: UnixMillis,
    ) -> Result<(VerifiedSubscriptionDescriptor, SubscriptionBinding), SubscriptionError> {
        ensure_current_context(context, now)?;
        let verified = self.codec.verify(descriptor, now)?;
        let claims = verified.claims();
        if claims.stream() != expected.stream()
            || claims.topics() != expected.topics()
            || claims.events() != expected.events()
            || claims.reconnect() != expected.reconnect()
            || claims.authorization_memo() != &authorization_memo(context)?
        {
            return Err(SubscriptionError::new(SubscriptionErrorKind::ScopeMismatch));
        }
        Ok((verified, SubscriptionBinding::from_descriptor(descriptor)?))
    }
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
    expires_at: UnixMillis,
    credential: &TransportCredential,
) -> Result<(), SubscriptionError> {
    let credentials = context
        .capabilities()
        .subscription_credentials()
        .ok_or_else(|| SubscriptionError::new(SubscriptionErrorKind::CredentialUnavailable))?;
    match credentials
        .verify(SubscriptionCredentialRequest::verify(
            operation, binding, expires_at, credential,
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
