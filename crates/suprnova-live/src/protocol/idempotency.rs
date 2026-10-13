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
const SEMANTIC_REQUEST_PROFILE_V1: &str = "suprnova.live.semantic-request.v1";
const CHILD_PARAMETERS_AUTHORITY_PROFILE_V2: &[u8] =
    b"suprnova.live.idempotency.child-parameters-authority.v2\0";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    child_parameters_authority_digest: Option<&'value ContentDigest>,
    model_proposals: &'value BTreeMap<ModelField, CanonicalValue>,
    semantic_extensions: &'value BTreeMap<String, CanonicalValue>,
}

#[derive(Serialize)]
struct SemanticRequestDocument<'value, Operation> {
    profile: &'static str,
    protocol_version: u16,
    runtime_contract_version: u16,
    snapshot_schema_version: u16,
    component: &'value ComponentName,
    base_revision: Revision,
    idempotency_identity: &'value IdempotencyKey,
    operations: &'value [Operation],
    child_parameters: Option<&'value [u8]>,
    model_proposals: &'value BTreeMap<ModelField, CanonicalValue>,
    semantic_extensions: &'value BTreeMap<String, CanonicalValue>,
}

pub(crate) fn semantic_request_digest_v1(
    request: &VersionedUpdateRequest,
) -> Result<ContentDigest, ProtocolError> {
    match request {
        VersionedUpdateRequest::V1(request) => digest_semantic_request(
            request.protocol_version(),
            request.runtime_contract_version(),
            request.snapshot_schema_version(),
            request.component(),
            request.base_revision(),
            request.idempotency_key(),
            request.operations(),
            None,
            request.model_proposals(),
            request.extensions(),
        ),
        VersionedUpdateRequest::V2(request) => digest_semantic_request(
            request.protocol_version(),
            request.runtime_contract_version(),
            request.snapshot_schema_version(),
            request.component(),
            request.base_revision(),
            request.idempotency_key(),
            request.operations(),
            request
                .child_parameters()
                .map(super::ChildParameterAdmissionCarrier::canonical_bytes),
            request.model_proposals(),
            request.extensions(),
        ),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "every argument is one named field in the closed semantic request profile"
)]
fn digest_semantic_request<Operation: Serialize>(
    protocol_version: u16,
    runtime_contract_version: u16,
    snapshot_schema_version: u16,
    component: &ComponentName,
    base_revision: Revision,
    idempotency_identity: &IdempotencyKey,
    operations: &[Operation],
    child_parameters: Option<&[u8]>,
    model_proposals: &BTreeMap<ModelField, CanonicalValue>,
    semantic_extensions: &BTreeMap<String, CanonicalValue>,
) -> Result<ContentDigest, ProtocolError> {
    let document = SemanticRequestDocument {
        profile: SEMANTIC_REQUEST_PROFILE_V1,
        protocol_version,
        runtime_contract_version,
        snapshot_schema_version,
        component,
        base_revision,
        idempotency_identity,
        operations,
        child_parameters,
        model_proposals,
        semantic_extensions,
    };
    let canonical = serde_json_canonicalizer::to_vec(&document)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidEnvelope))?;
    let digest = Sha256::digest(canonical);
    ContentDigest::from_bytes(&digest)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))
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
            None,
            request.model_proposals(),
            request.extensions(),
        ),
        VersionedUpdateRequest::V2(request) => {
            let child_parameters_authority_digest = request
                .child_parameters()
                .map(|carrier| digest_child_parameters_authority(carrier.canonical_bytes()))
                .transpose()?;
            digest_document(
                input,
                request.component(),
                request.base_revision(),
                request.idempotency_key(),
                request.operations(),
                child_parameters_authority_digest.as_ref(),
                request.model_proposals(),
                request.extensions(),
            )
        }
    }
}

fn digest_child_parameters_authority(
    canonical_carrier: &[u8],
) -> Result<ContentDigest, ProtocolError> {
    let mut digest = Sha256::new();
    digest.update(CHILD_PARAMETERS_AUTHORITY_PROFILE_V2);
    digest.update(canonical_carrier);
    ContentDigest::from_bytes(&digest.finalize())
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))
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
    child_parameters_authority_digest: Option<&ContentDigest>,
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
        child_parameters_authority_digest,
        model_proposals,
        semantic_extensions,
    };
    let canonical = serde_json_canonicalizer::to_vec(&document)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidEnvelope))?;
    let digest = Sha256::digest(canonical);
    ContentDigest::from_bytes(&digest)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;

    use super::semantic_request_digest_v1;
    use crate::conformance::fixture_directory_v2;
    use crate::limits::InputLimits;
    use crate::protocol::{ProtocolLimitConfig, ProtocolLimits, parse_versioned_update_request};

    #[test]
    fn child_envelope_and_lifecycle_operation_are_semantic_request_significant() {
        let fixture: Value = serde_json::from_slice(
            &fs::read(fixture_directory_v2().join("protocol-success.json"))
                .expect("v2 fixture file"),
        )
        .expect("v2 fixture JSON");
        let cases = fixture["cases"].as_array().expect("fixture cases");
        let params = cases[0]["encoded"].as_str().expect("params request");
        let lazy = cases[1]["encoded"].as_str().expect("lazy request");
        let params = parse_versioned_update_request(params.as_bytes(), &limits())
            .expect("params request parses");
        let lazy = parse_versioned_update_request(lazy.as_bytes(), &limits())
            .expect("lazy request parses");
        assert_ne!(
            semantic_request_digest_v1(&params).expect("params digest"),
            semantic_request_digest_v1(&lazy).expect("lazy digest")
        );

        let mut changed: Value =
            serde_json::from_str(cases[0]["encoded"].as_str().expect("params request"))
                .expect("params request JSON");
        changed["child_parameters"]["envelope"]["body"]["parameters"]["query"] =
            Value::String("zig".to_owned());
        let changed =
            serde_json_canonicalizer::to_vec(&changed).expect("canonical changed request");
        let changed = parse_versioned_update_request(&changed, &limits())
            .expect("changed child request parses");
        assert_ne!(
            semantic_request_digest_v1(&params).expect("params digest"),
            semantic_request_digest_v1(&changed).expect("changed digest")
        );
    }

    fn limits() -> ProtocolLimits {
        ProtocolLimits::new(ProtocolLimitConfig {
            input: InputLimits::new(64 * 1024, 12, 512, 40 * 1024).expect("input limits"),
            max_snapshot_bytes: 32 * 1024,
            max_html_bytes: 32 * 1024,
            max_model_proposals: 8,
            max_operations: 8,
            max_arguments: 16,
            max_validation_entries: 16,
            max_events: 8,
            max_effects: 8,
            max_extensions: 8,
        })
        .expect("protocol limits")
    }
}
