//! The single manifest write is the publication commit point.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Versions retained for the current publication and its grace period.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// The origin prefix below `_pub/` selected by the last complete publication.
    pub live: String,
    /// Publication revisions, including the previous live revision until pruned.
    pub versions: BTreeMap<String, VersionEntry>,
}

/// A complete revision and the objects that must survive while it is retained.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionEntry {
    /// The Inertia asset version, independent of the publication revision.
    pub asset_version: String,
    /// Content identity of the mutable asset set; empty when this revision has none.
    #[serde(default)]
    pub asset_set_digest: String,
    /// Page URLs retain their set roots so partial publication preserves relative dependencies.
    #[serde(default)]
    pub page_asset_sets: BTreeMap<String, String>,
    /// Creation time in Unix milliseconds.
    pub published_at_ms: u64,
    /// Time this revision stopped being live; grace starts here.
    pub retired_at_ms: Option<u64>,
    /// Complete page and asset inventory.
    pub objects: Vec<ObjectEntry>,
}

/// An object's identity, delivery headers and invalidation dependencies.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectEntry {
    /// Storage key relative to the application's disk root.
    pub path: String,
    /// Canonical page URL or fingerprinted asset URL.
    pub url: String,
    /// Whether this is a referenced asset rather than an HTML or JSON page.
    #[serde(default)]
    pub asset: bool,
    /// SHA-256 of the uploaded bytes.
    pub digest: String,
    /// Inertia asset version used to render these bytes.
    pub asset_version: String,
    /// Digested dependency identities from the render collector.
    pub dependency_tags: Vec<String>,
    /// Headers sent by the origin and stored as object metadata.
    pub headers: BTreeMap<String, String>,
}
