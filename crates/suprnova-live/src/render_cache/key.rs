//! Canonical, bounded, versioned lookup identity.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};

use super::variance::{DimensionValue, VarianceDescriptor};
use super::{RenderCacheError, RenderCacheErrorKind};
use crate::crypto::SnapshotKeyRing;
use crate::identity::{BuildId, RouteIdentity};

/// Key format version; a change here cannot collide with prior entries.
pub const KEY_FORMAT_VERSION: u8 = 1;
/// Upper bound on route parameters plus declared query parameters.
pub const MAX_PARAMS: usize = 32;
/// Upper bound on one parameter value.
pub const MAX_PARAM_BYTES: usize = 512;
/// Upper bound on the human-readable route pattern shown in inspection.
const MAX_ROUTE_PATTERN_BYTES: usize = 256;
/// Domain separator for the deterministic test-only route digest.
const TEST_ROUTE_DOMAIN: &[u8] = b"suprnova-live/render-key/test-route/v1";

/// Everything that identifies one representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderKeyInput {
    /// Canonical route identity from the router.
    pub route: RouteIdentity,
    /// Human-readable route pattern, shown only in inspection; the digest
    /// in `route` is what participates in the key. Must be the router's
    /// registered route pattern (for example `/catalog/{category}`), never
    /// the resolved request path (`/catalog/shoes`): a concrete path can
    /// carry request material into inspection output, which this type
    /// exists to prevent.
    pub route_pattern: String,
    /// Normalized route parameters.
    pub params: BTreeMap<String, String>,
    /// Declared query parameters, normalized by the route's query policy.
    pub query: BTreeMap<String, String>,
    /// Trusted host when deployment policy makes it meaningful.
    pub host: Option<String>,
    /// Negotiated media type.
    pub media: String,
    /// Negotiated content encoding.
    pub encoding: Option<String>,
    /// Application and view build.
    pub build: BuildId,
    /// Authority epoch.
    pub epoch: u64,
    /// Declared variance.
    pub variance: VarianceDescriptor,
}

/// A purpose-separated digest of one representation identity.
///
/// Equality, ordering, and hashing compare `digest` alone: `dimensions` is
/// inspection metadata, not part of the key's identity. Two keys with the
/// same digest must be the same key regardless of how they were built, since
/// [`Self::from_base64url`] recovers only the digest and deliberately
/// carries [`RenderKeyDimensions::opaque`] instead of the original
/// dimensions; a lookup by a key parsed back from storage must land on the
/// same map slot as the key it was published under.
#[derive(Clone)]
pub struct RenderKey {
    digest: [u8; 32],
    dimensions: RenderKeyDimensions,
}

impl PartialEq for RenderKey {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
    }
}

impl Eq for RenderKey {}

impl std::hash::Hash for RenderKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.digest.hash(state);
    }
}

impl PartialOrd for RenderKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RenderKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.digest.cmp(&other.digest)
    }
}

impl RenderKey {
    /// Derives the key; fails closed on any bound.
    pub fn derive(
        input: &RenderKeyInput,
        keys: &SnapshotKeyRing,
    ) -> Result<Self, RenderCacheError> {
        let invalid = || RenderCacheError::new(RenderCacheErrorKind::KeyInvalid);
        if input.params.len() + input.query.len() > MAX_PARAMS {
            return Err(invalid());
        }
        for (name, value) in input.params.iter().chain(input.query.iter()) {
            if name.is_empty() || name.len() > 64 || value.len() > MAX_PARAM_BYTES {
                return Err(invalid());
            }
        }
        if input.media.len() > 128 || input.host.as_ref().is_some_and(|host| host.len() > 253) {
            return Err(invalid());
        }
        if input.route_pattern.is_empty()
            || input.route_pattern.len() > MAX_ROUTE_PATTERN_BYTES
            || input
                .route_pattern
                .bytes()
                .any(|byte| byte.is_ascii_control())
        {
            return Err(invalid());
        }
        let mut parts: Vec<Vec<u8>> = Vec::new();
        let mut feed = |tag: u8, bytes: &[u8]| {
            let mut part = Vec::with_capacity(bytes.len() + 1);
            part.push(tag);
            part.extend_from_slice(bytes);
            parts.push(part);
        };
        feed(0, &[KEY_FORMAT_VERSION]);
        feed(1, input.route.as_bytes());
        for (name, value) in &input.params {
            feed(2, name.as_bytes());
            feed(3, value.as_bytes());
        }
        for (name, value) in &input.query {
            feed(4, name.as_bytes());
            feed(5, value.as_bytes());
        }
        feed(6, input.host.as_deref().unwrap_or("").as_bytes());
        feed(7, input.media.as_bytes());
        feed(
            8,
            input.encoding.as_deref().unwrap_or("identity").as_bytes(),
        );
        feed(9, input.build.as_str().as_bytes());
        feed(10, &input.epoch.to_be_bytes());
        feed(11, &input.variance.canonical_bytes());
        let borrowed: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
        let digest = keys
            .mac(crate::crypto::SnapshotPurpose::RenderKeyV1, &borrowed)
            .map_err(|_| invalid())?;
        Ok(Self {
            digest,
            dimensions: RenderKeyDimensions::from_input(input),
        })
    }

    /// `rk1.` followed by the base64url digest; at most 48 characters.
    #[must_use]
    pub fn to_base64url(&self) -> String {
        format!(
            "rk{KEY_FORMAT_VERSION}.{}",
            URL_SAFE_NO_PAD.encode(self.digest)
        )
    }

    /// Parses `rk{KEY_FORMAT_VERSION}.<digest>`, the exact inverse of
    /// [`Self::to_base64url`]. The encoded text carries no recoverable
    /// request identity, so the returned key's dimensions are the opaque
    /// marker; this is sufficient for lookup and stored-entry inspection.
    pub fn from_base64url(text: &str) -> Result<Self, RenderCacheError> {
        let invalid = || RenderCacheError::new(RenderCacheErrorKind::KeyInvalid);
        let encoded = text
            .strip_prefix(&format!("rk{KEY_FORMAT_VERSION}."))
            .ok_or_else(invalid)?;
        let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
        let digest: [u8; 32] = decoded.try_into().map_err(|_| invalid())?;
        Ok(Self {
            digest,
            dimensions: RenderKeyDimensions::opaque(),
        })
    }

    /// The raw digest.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Safe decoded dimensions for inspection.
    #[must_use]
    pub const fn dimensions(&self) -> &RenderKeyDimensions {
        &self.dimensions
    }

    /// Test-only fixture key derived from a route pattern with empty
    /// parameters and query, media `text/html`, build `test`, and epoch 1.
    #[doc(hidden)]
    #[must_use]
    pub fn for_test(keys: &SnapshotKeyRing, pattern: &str) -> Self {
        let input = RenderKeyInput {
            route: route_identity_for_pattern(pattern),
            route_pattern: pattern.to_owned(),
            params: BTreeMap::new(),
            query: BTreeMap::new(),
            host: None,
            media: "text/html".to_owned(),
            encoding: None,
            build: BuildId::parse("test").expect("'test' is a valid build id"),
            epoch: 1,
            variance: VarianceDescriptor::new(),
        };
        Self::derive(&input, keys).expect("bounded fixture input always derives")
    }
}

/// Deterministic, domain-separated digest used only to give
/// [`RenderKey::for_test`] fixtures a stable [`RouteIdentity`] from a route
/// pattern string. Not used by any non-test code path.
fn route_identity_for_pattern(pattern: &str) -> RouteIdentity {
    let mut hasher = Sha256::new();
    hasher.update(TEST_ROUTE_DOMAIN);
    hasher.update(pattern.as_bytes());
    RouteIdentity::from_bytes(&hasher.finalize()).expect("sha-256 output is exactly 32 bytes")
}

impl std::fmt::Debug for RenderKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.to_base64url())
    }
}

/// Inspectable key dimensions: public values verbatim, private values as
/// four-byte digest prefixes.
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct RenderKeyDimensions {
    route: String,
    params: BTreeMap<String, String>,
    query: BTreeMap<String, String>,
    host: Option<String>,
    media: String,
    encoding: Option<String>,
    build: String,
    epoch: u64,
    variance: BTreeMap<String, String>,
}

impl RenderKeyDimensions {
    fn from_input(input: &RenderKeyInput) -> Self {
        Self {
            route: input.route_pattern.clone(),
            params: input.params.clone(),
            query: input.query.clone(),
            host: input.host.clone(),
            media: input.media.clone(),
            encoding: input.encoding.clone(),
            build: input.build.as_str().to_owned(),
            epoch: input.epoch,
            variance: input
                .variance
                .dimensions()
                .iter()
                .map(|(dimension, value)| {
                    let shown = match value {
                        DimensionValue::Public(text) => text.clone(),
                        DimensionValue::Private(material) => format!("{material:?}"),
                        DimensionValue::Anonymous => "anonymous".to_owned(),
                    };
                    (format!("{dimension:?}"), shown)
                })
                .collect(),
        }
    }

    /// Marker dimensions for a key parsed from its encoded text alone;
    /// carries no recoverable request identity, only enough shape to be
    /// inspected.
    #[must_use]
    pub fn opaque() -> Self {
        Self {
            route: "<stored>".to_owned(),
            params: BTreeMap::new(),
            query: BTreeMap::new(),
            host: None,
            media: String::new(),
            encoding: None,
            build: String::new(),
            epoch: 0,
            variance: BTreeMap::new(),
        }
    }

    /// The route pattern.
    #[must_use]
    pub fn route(&self) -> &str {
        &self.route
    }
    /// Route parameters.
    #[must_use]
    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }
    /// Declared query parameters.
    #[must_use]
    pub fn query(&self) -> &BTreeMap<String, String> {
        &self.query
    }
    /// Variance dimensions as inspectable text.
    #[must_use]
    pub fn variance(&self) -> &BTreeMap<String, String> {
        &self.variance
    }
}
