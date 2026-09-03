//! Explicit variance dimensions, opaque private key material, and the
//! classification that only preserves or reduces sharing.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::policy::RepresentationClass;
use super::{RenderCacheError, RenderCacheErrorKind};
use crate::crypto::SnapshotKeyRing;

/// Upper bound on one public dimension value.
pub const MAX_DIMENSION_VALUE_BYTES: usize = 256;
/// Upper bound on dimensions in one descriptor.
pub const MAX_DIMENSIONS: usize = 24;

/// A request or application dimension allowed to change bytes or metadata.
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum VarianceDimension {
    /// Trusted host, where deployment policy makes it meaningful.
    Host,
    /// Negotiated locale.
    Locale,
    /// Negotiated media type.
    Media,
    /// Negotiated content encoding.
    Encoding,
    /// Tenant, as opaque material.
    Tenant,
    /// Principal, as opaque material bound to the permission version.
    Principal,
    /// Feature flag set version.
    FeatureVersion,
    /// Configuration version.
    ConfigVersion,
    /// An explicit application dimension; the name is bounded and lower case.
    Application(String),
}

impl VarianceDimension {
    /// The `Vary` header a dimension implies, if any.
    #[must_use]
    pub fn vary_header(&self) -> Option<&'static str> {
        match self {
            Self::Locale => Some("Accept-Language"),
            Self::Media => Some("Accept"),
            Self::Encoding => Some("Accept-Encoding"),
            _ => None,
        }
    }

    fn canonical_name(&self) -> String {
        match self {
            Self::Host => "host".to_owned(),
            Self::Locale => "locale".to_owned(),
            Self::Media => "media".to_owned(),
            Self::Encoding => "encoding".to_owned(),
            Self::Tenant => "tenant".to_owned(),
            Self::Principal => "principal".to_owned(),
            Self::FeatureVersion => "feature_version".to_owned(),
            Self::ConfigVersion => "config_version".to_owned(),
            Self::Application(name) => format!("app:{name}"),
        }
    }
}

/// Opaque 32-byte private key material derived with purpose separation.
#[derive(Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct PrivateMaterial([u8; 32]);

impl PrivateMaterial {
    fn derive(keys: &SnapshotKeyRing, label: &str, parts: &[&[u8]]) -> Self {
        let mut all: Vec<&[u8]> = Vec::with_capacity(parts.len() + 1);
        all.push(label.as_bytes());
        all.extend_from_slice(parts);
        Self(
            keys.mac(crate::crypto::SnapshotPurpose::RenderVarianceV1, &all)
                .expect("active key derives"),
        )
    }

    /// Principal material bound to an opaque internal id and permission version.
    #[must_use]
    pub fn principal(keys: &SnapshotKeyRing, internal_id: &str, permission_version: u64) -> Self {
        Self::derive(
            keys,
            "suprnova-live/render-variance/principal/v1",
            &[internal_id.as_bytes(), &permission_version.to_be_bytes()],
        )
    }

    /// Tenant material bound to an opaque internal id.
    #[must_use]
    pub fn tenant(keys: &SnapshotKeyRing, internal_id: &str) -> Self {
        Self::derive(
            keys,
            "suprnova-live/render-variance/tenant/v1",
            &[internal_id.as_bytes()],
        )
    }

    /// The raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for PrivateMaterial {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<private-material>")
    }
}

impl serde::Serialize for PrivateMaterial {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&URL_SAFE_NO_PAD.encode(self.0))
    }
}

impl<'de> serde::Deserialize<'de> for PrivateMaterial {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(text.as_bytes())
            .map_err(serde::de::Error::custom)?;
        let array: [u8; 32] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("private material must be exactly 32 bytes"))?;
        Ok(Self(array))
    }
}

/// One dimension's normalized value.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DimensionValue {
    /// A bounded public value that may appear in inspection output.
    Public(String),
    /// Opaque private material.
    Private(PrivateMaterial),
    /// Explicitly anonymous for a private dimension; distinct from absent.
    Anonymous,
}

/// The complete declared variance of one representation.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct VarianceDescriptor {
    dimensions: BTreeMap<VarianceDimension, DimensionValue>,
}

impl VarianceDescriptor {
    /// An empty descriptor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares one dimension value; bounded and unique.
    pub fn declare(
        &mut self,
        dimension: VarianceDimension,
        value: DimensionValue,
    ) -> Result<(), RenderCacheError> {
        if self.dimensions.len() >= MAX_DIMENSIONS {
            return Err(RenderCacheError::new(RenderCacheErrorKind::VarianceInvalid));
        }
        if let VarianceDimension::Application(name) = &dimension
            && (name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
        {
            return Err(RenderCacheError::new(RenderCacheErrorKind::VarianceInvalid));
        }
        if let DimensionValue::Public(text) = &value
            && (text.len() > MAX_DIMENSION_VALUE_BYTES
                || text.bytes().any(|b| b.is_ascii_control()))
        {
            return Err(RenderCacheError::new(RenderCacheErrorKind::VarianceInvalid));
        }
        if self.dimensions.contains_key(&dimension) {
            return Err(RenderCacheError::new(RenderCacheErrorKind::VarianceInvalid));
        }
        self.dimensions.insert(dimension, value);
        Ok(())
    }

    /// The declared dimensions in canonical order.
    #[must_use]
    pub fn dimensions(&self) -> &BTreeMap<VarianceDimension, DimensionValue> {
        &self.dimensions
    }

    /// `Vary` header names implied by the declared dimensions, sorted.
    #[must_use]
    pub fn vary_headers(&self) -> Vec<&'static str> {
        let mut headers: Vec<&'static str> = self
            .dimensions
            .keys()
            .filter_map(VarianceDimension::vary_header)
            .collect();
        headers.sort_unstable();
        headers.dedup();
        headers
    }

    /// Canonical bytes that join the lookup key: length-prefixed name and
    /// value pairs in dimension order; private values as their digests.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (dimension, value) in &self.dimensions {
            let name = dimension.canonical_name();
            out.extend_from_slice(&(name.len() as u32).to_be_bytes());
            out.extend_from_slice(name.as_bytes());
            match value {
                DimensionValue::Public(text) => {
                    out.push(1);
                    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
                    out.extend_from_slice(text.as_bytes());
                }
                DimensionValue::Private(material) => {
                    out.push(2);
                    out.extend_from_slice(material.as_bytes());
                }
                DimensionValue::Anonymous => out.push(3),
            }
        }
        out
    }
}

/// What rendering observed about identity and context.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObservedContext {
    /// Principal material when a signed-in principal was observed.
    pub principal: Option<PrivateMaterial>,
    /// Tenant material when a tenant was observed.
    pub tenant: Option<PrivateMaterial>,
    /// A session value was read (not merely a session id).
    pub session_read: bool,
    /// A private authorization decision was evaluated.
    pub authorization_read: bool,
    /// Secret configuration or feature context was read.
    pub secret_context_read: bool,
    /// Request context outside the declared variance affected rendering.
    pub undeclared_reads: Vec<String>,
}

/// Why classification narrowed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassificationReason {
    /// A signed-in principal was observed.
    PrincipalObserved,
    /// A tenant was observed.
    TenantObserved,
    /// A session value was read.
    SessionValueRead,
    /// A private authorization decision was evaluated.
    AuthorizationRead,
    /// Secret context was read.
    SecretContextRead,
    /// Undeclared request context affected output.
    UndeclaredContext,
}

/// The classification decision with its inspectable reasons.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassificationOutcome {
    /// The effective class.
    pub class: RepresentationClass,
    /// Every reason that narrowed the declared class, in evaluation order.
    pub reasons: Vec<ClassificationReason>,
}

/// Starts from the route's permitted class and narrows on observed state.
#[must_use]
pub fn classify(
    declared: RepresentationClass,
    observed: &ObservedContext,
) -> ClassificationOutcome {
    let mut class = declared;
    let mut reasons = Vec::new();
    let mut narrow = |to: RepresentationClass,
                      reason: ClassificationReason,
                      class: &mut RepresentationClass| {
        let next = class.narrowest(to);
        if next != *class
            || to == RepresentationClass::Uncacheable && *class != RepresentationClass::Uncacheable
        {
            *class = next;
        }
        reasons.push(reason);
    };
    if observed.principal.is_some() {
        narrow(
            RepresentationClass::PrivateCached,
            ClassificationReason::PrincipalObserved,
            &mut class,
        );
    }
    if observed.tenant.is_some() {
        narrow(
            RepresentationClass::PrivateCached,
            ClassificationReason::TenantObserved,
            &mut class,
        );
    }
    if observed.session_read {
        narrow(
            RepresentationClass::Uncacheable,
            ClassificationReason::SessionValueRead,
            &mut class,
        );
    }
    if observed.authorization_read {
        narrow(
            RepresentationClass::PrivateCached,
            ClassificationReason::AuthorizationRead,
            &mut class,
        );
    }
    if observed.secret_context_read {
        narrow(
            RepresentationClass::Uncacheable,
            ClassificationReason::SecretContextRead,
            &mut class,
        );
    }
    if !observed.undeclared_reads.is_empty() {
        narrow(
            RepresentationClass::Uncacheable,
            ClassificationReason::UndeclaredContext,
            &mut class,
        );
    }
    ClassificationOutcome { class, reasons }
}
