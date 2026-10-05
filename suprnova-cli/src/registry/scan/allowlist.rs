//! The documented public API of Suprnova with the capability each item
//! carries (REG-006, REG-030), generated from `feature-map/surface.jsonl`
//! by `feature-map/tools/registry_allowlist.py` into a data file the CLI
//! embeds. A test fails when the data file and the feature map disagree.

use std::collections::BTreeMap;

use super::super::{Capability, RegistryError, Result};

/// One admitted item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedItem {
    /// The capability naming this item carries, or none.
    pub capability: Option<Capability>,
    /// Whether the item is `#[doc(hidden)]`; hidden items are refused.
    pub hidden: bool,
    /// Other full paths that name the same item (re-exports).
    pub aliases: Vec<String>,
}

/// Every admitted path, by its canonical full path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Allowlist {
    items: BTreeMap<String, AllowedItem>,
}

impl Allowlist {
    /// Builds an allowlist from items by canonical path.
    pub fn from_items(items: BTreeMap<String, AllowedItem>) -> Self {
        Allowlist { items }
    }

    /// The item a full path names, through an alias when needed.
    pub fn lookup<'a>(&'a self, path: &str) -> Option<(&'a str, &'a AllowedItem)> {
        if let Some((canonical, item)) = self.items.get_key_value(path) {
            return Some((canonical.as_str(), item));
        }
        self.items
            .iter()
            .find(|(_, item)| item.aliases.iter().any(|alias| alias == path))
            .map(|(canonical, item)| (canonical.as_str(), item))
    }

    /// How many items the list holds.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The effect-free part of `std` a component may name (REG-030): the
/// prelude's types and traits, collections, formatting, and nothing that
/// reaches files, the network, the environment, processes or threads.
pub const STD_ALLOWED: &[&str] = &[
    "std::borrow",
    "std::cmp",
    "std::collections",
    "std::convert",
    "std::default",
    "std::fmt",
    "std::hash",
    "std::iter",
    "std::marker",
    "std::mem::replace",
    "std::mem::swap",
    "std::mem::take",
    "std::num",
    "std::ops",
    "std::option",
    "std::result",
    "std::str",
    "std::string",
    "std::vec",
    "std::boxed",
    "std::rc",
    "std::sync::Arc",
    "std::time::Duration",
];

/// The allowlist embedded in this binary, for the framework version this
/// CLI was built with.
pub fn embedded() -> Result<&'static Allowlist> {
    Err(RegistryError::NotBuilt("the embedded allowlist"))
}
