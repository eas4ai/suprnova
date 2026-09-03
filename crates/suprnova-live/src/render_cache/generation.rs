//! Dependency generation counters observed while rendering.
//!
//! This module is a stub: [`GenerationSet`] exists only so
//! [`crate::render_cache::entry::EntryHeader`] can encode, decode, and
//! inspect stored entries. A later iteration replaces this alias-shaped
//! type with a real struct that declares and reconciles dependencies.
//!
//! The wire form is part of the entry contract and MUST survive that
//! replacement, since entries already on disk are decoded against it: each
//! 32-byte dependency digest key is a lowercase 64-character hex string,
//! matching how the same digests are stored elsewhere, mapped to its
//! generation counter value. Any key that is not exactly 64 hex characters
//! fails to decode.

use std::collections::BTreeMap;

/// Dependency generations observed during a render: one 32-byte dependency
/// digest mapped to the generation counter value observed for it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GenerationSet(BTreeMap<[u8; 32], u64>);

impl GenerationSet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the generation counter observed for one dependency digest; a
    /// repeated digest overwrites its prior value.
    pub fn insert(&mut self, dependency: [u8; 32], generation: u64) {
        self.0.insert(dependency, generation);
    }

    /// Number of dependencies observed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when nothing was observed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl serde::Serialize for GenerationSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let hex_keyed: BTreeMap<String, u64> = self
            .0
            .iter()
            .map(|(digest, generation)| (to_hex(digest), *generation))
            .collect();
        hex_keyed.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for GenerationSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let hex_keyed = BTreeMap::<String, u64>::deserialize(deserializer)?;
        let mut set = BTreeMap::new();
        for (key, generation) in hex_keyed {
            let digest = from_hex(&key).ok_or_else(|| {
                serde::de::Error::custom("dependency digest must be 64 hex characters")
            })?;
            set.insert(digest, generation);
        }
        Ok(Self(set))
    }
}

fn to_hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(64);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("formatting into a String cannot fail");
    }
    text
}

fn from_hex(text: &str) -> Option<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, slot) in digest.iter_mut().enumerate() {
        let high = hex_value(bytes[index * 2])?;
        let low = hex_value(bytes[index * 2 + 1])?;
        *slot = (high << 4) | low;
    }
    Some(digest)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
