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
/// The digest is the whole key, so equality, ordering, and hashing compare
/// it alone and a key parsed back from storage with
/// [`Self::from_base64url`] lands on the same map slot as the key it was
/// published under. Inspectable dimensions are not carried here; they are
/// described on demand from the input by
/// [`RenderKeyDimensions::describe`], which is what lets derivation run
/// without cloning any request material.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RenderKey {
    digest: [u8; 32],
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
        let epoch = input.epoch.to_be_bytes();
        let variance_len = input.variance.canonical_len();
        let digest = keys
            .mac_with(crate::crypto::SnapshotPurpose::RenderKeyV1, |mac| {
                mac.part(&[&[0][..], &[KEY_FORMAT_VERSION][..]]);
                mac.part(&[&[1][..], input.route.as_bytes()]);
                for (name, value) in &input.params {
                    mac.part(&[&[2][..], name.as_bytes()]);
                    mac.part(&[&[3][..], value.as_bytes()]);
                }
                for (name, value) in &input.query {
                    mac.part(&[&[4][..], name.as_bytes()]);
                    mac.part(&[&[5][..], value.as_bytes()]);
                }
                mac.part(&[&[6][..], input.host.as_deref().unwrap_or("").as_bytes()]);
                mac.part(&[&[7][..], input.media.as_bytes()]);
                mac.part(&[
                    &[8][..],
                    input.encoding.as_deref().unwrap_or("identity").as_bytes(),
                ]);
                mac.part(&[&[9][..], input.build.as_str().as_bytes()]);
                mac.part(&[&[10][..], &epoch[..]]);
                mac.part_streamed(1 + variance_len, |sink| {
                    sink(&[11]);
                    input.variance.write_canonical(sink);
                });
            })
            .map_err(|_| invalid())?;
        Ok(Self { digest })
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
    /// [`Self::to_base64url`]. A key is the digest and nothing else, so a
    /// parsed one carries no dimensions at all: no part of the request that
    /// derived it is recoverable from the text, which is all a lookup
    /// needs. A caller that must still describe such a key has
    /// [`RenderKeyDimensions::opaque`] to stand in for the input it does
    /// not hold.
    pub fn from_base64url(text: &str) -> Result<Self, RenderCacheError> {
        let invalid = || RenderCacheError::new(RenderCacheErrorKind::KeyInvalid);
        let encoded = text
            .strip_prefix(&format!("rk{KEY_FORMAT_VERSION}."))
            .ok_or_else(invalid)?;
        let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
        let digest: [u8; 32] = decoded.try_into().map_err(|_| invalid())?;
        Ok(Self { digest })
    }

    /// The raw digest.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
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
/// the redacted marker `<private-material>` (the `Debug` form of
/// [`PrivateMaterial`](super::variance::PrivateMaterial), which never
/// prints any part of the material), and an anonymous private dimension as
/// `anonymous`.
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
    /// Describes one key input for inspection. This is deliberately separate
    /// from [`RenderKey::derive`]: describing clones request material, and
    /// derivation must not.
    #[must_use]
    pub fn describe(input: &RenderKeyInput) -> Self {
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

    /// A marker a caller uses when it has no key input to describe, such
    /// as after parsing a key back from its encoded text. Nothing here was
    /// read out of a key, and no key carries these values; they are fixed
    /// placeholders that give an inspection the shape it expects.
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
