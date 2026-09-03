//! Dependency generation counters observed while rendering.
//!
//! This module currently defines only [`GenerationSet`], the bounded map
//! [`crate::render_cache::entry::EntryHeader`] needs to encode, decode, and
//! inspect stored entries. A later iteration extends this module with
//! declaration and coherence behavior.

use std::collections::BTreeMap;

/// Dependency generations observed during a render: one 32-byte dependency
/// digest mapped to the generation counter value observed for it.
pub type GenerationSet = BTreeMap<[u8; 32], u64>;
