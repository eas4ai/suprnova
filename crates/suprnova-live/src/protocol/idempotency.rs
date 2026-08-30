//! Versioned semantic idempotency digest independent of transport presentation.

use std::collections::BTreeMap;

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::canonical::CanonicalValue;
use crate::identity::{
    ComponentName, ContentDigest, IdempotencyKey, InstanceId, ModelField, Revision,
    ScopeFingerprint,
};

use super::{ProtocolError, ProtocolErrorKind, VersionedUpdateRequest};

const PROFILE_V1: &str = "suprnova.live.idempotency.v1";

/// Trusted semantic inputs combined with one fully parsed versioned request.
pub struct SemanticIdempotencyInputV1<'request> {
    scope: ScopeFingerprint,
    instance: InstanceId,
    component_contract: ContentDigest,
    authority: ContentDigest,
    request: &'request VersionedUpdateRequest,
}

impl<'request> SemanticIdempotencyInputV1<'request> {
    /// Creates the v1 semantic digest input after authority verification.
    #[must_use]
    pub const fn new(
        scope: ScopeFingerprint,
        instance: InstanceId,
        component_contract: ContentDigest,
        authority: ContentDigest,
        request: &'request VersionedUpdateRequest,
    ) -> Self {
        Self {
            scope,
            instance,
            component_contract,
            authority,
            request,
        }
    }
}

#[derive(Serialize)]
struct ComponentAuthority<'value> {
    name: &'value ComponentName,
    contract_digest: &'value ContentDigest,
}

#[derive(Serialize)]
struct DigestDocument<'value, Operation> {
    profile: &'static str,
    scope: &'value ScopeFingerprint,
    instance: &'value InstanceId,
    base_revision: Revision,
    component: ComponentAuthority<'value>,
    idempotency_identity: &'value IdempotencyKey,
    authority_digest: &'value ContentDigest,
    operations: &'value [Operation],
    model_proposals: &'value BTreeMap<ModelField, CanonicalValue>,
    semantic_extensions: &'value BTreeMap<String, CanonicalValue>,
}

/// Computes the canonical SHA-256 semantic idempotency profile v1 digest.
pub fn semantic_idempotency_digest_v1(
    input: &SemanticIdempotencyInputV1<'_>,
) -> Result<ContentDigest, ProtocolError> {
    match input.request {
        VersionedUpdateRequest::V1(request) => digest_document(
            input,
            request.component(),
            request.base_revision(),
            request.idempotency_key(),
            request.operations(),
            request.model_proposals(),
            request.extensions(),
        ),
        VersionedUpdateRequest::V2(request) => digest_document(
            input,
            request.component(),
            request.base_revision(),
            request.idempotency_key(),
            request.operations(),
            request.model_proposals(),
            request.extensions(),
        ),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "every argument is one named field in the locked semantic digest profile"
)]
fn digest_document<Operation: Serialize>(
    input: &SemanticIdempotencyInputV1<'_>,
    component: &ComponentName,
    base_revision: Revision,
    idempotency_identity: &IdempotencyKey,
    operations: &[Operation],
    model_proposals: &BTreeMap<ModelField, CanonicalValue>,
    semantic_extensions: &BTreeMap<String, CanonicalValue>,
) -> Result<ContentDigest, ProtocolError> {
    let document = DigestDocument {
        profile: PROFILE_V1,
        scope: &input.scope,
        instance: &input.instance,
        base_revision,
        component: ComponentAuthority {
            name: component,
            contract_digest: &input.component_contract,
        },
        idempotency_identity,
        authority_digest: &input.authority,
        operations,
        model_proposals,
        semantic_extensions,
    };
    let canonical = serde_json_canonicalizer::to_vec(&document)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidEnvelope))?;
    let digest = Sha256::digest(canonical);
    ContentDigest::from_bytes(&digest)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))
}
