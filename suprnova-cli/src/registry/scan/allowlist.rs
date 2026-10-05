//! The documented public API of Suprnova with the capability each item
//! carries (REG-006, REG-030), generated from `feature-map/surface.jsonl`
//! by `feature-map/tools/registry_allowlist.py` into a data file the CLI
//! embeds. A test fails when the data file and the feature map disagree.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use super::super::{Capability, RegistryError, Result};

/// The generated data file, one JSON object per admitted path.
const EMBEDDED: &str = include_str!("allowlist.jsonl");

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

/// How the allowlist admits one full path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission<'a> {
    /// The path names an item, through its canonical path or an alias.
    Item {
        /// The item's canonical path.
        canonical: &'a str,
        /// The item.
        item: &'a AllowedItem,
    },
    /// The path sits under a re-exported crate or type, which admits every
    /// path below it with its own capability.
    Prefix {
        /// The re-export that covers the path.
        root: &'a str,
        /// The re-export's entry.
        item: &'a AllowedItem,
    },
    /// The path is a module that holds admitted items; naming it does
    /// nothing by itself.
    Module,
    /// The path names an item Suprnova documents but the allowlist refuses,
    /// because its capability cannot be decided or naming it changes the
    /// application (a container binding, for example).
    Refused {
        /// The refused item's canonical path.
        canonical: &'a str,
    },
}

/// Every admitted path, by its canonical full path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Allowlist {
    items: BTreeMap<String, AllowedItem>,
    /// Alias to canonical path, built from `items`.
    aliases: BTreeMap<String, String>,
    /// Canonical paths that admit everything below them (re-exported crates
    /// and external types).
    prefixes: BTreeSet<String>,
    /// The Suprnova traits a type implements, by canonical path, so a
    /// trait method called on the type can be found.
    implements: BTreeMap<String, Vec<String>>,
    /// Every proper ancestor of an admitted path or alias.
    modules: BTreeSet<String>,
    /// The item kind (`fn`, `struct`, `trait`, ...) by canonical path.
    kinds: BTreeMap<String, String>,
    /// Documented items the list refuses, by every path that names them,
    /// to their canonical path.
    refused: BTreeMap<String, String>,
    /// The type each function returns, by canonical path, as the feature
    /// map's extractor wrote it.
    returns: BTreeMap<String, String>,
}

impl Allowlist {
    /// Builds an allowlist from items by canonical path.
    pub fn from_items(items: BTreeMap<String, AllowedItem>) -> Self {
        Self::assemble(
            items,
            BTreeSet::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
    }

    fn assemble(
        items: BTreeMap<String, AllowedItem>,
        prefixes: BTreeSet<String>,
        implements: BTreeMap<String, Vec<String>>,
        kinds: BTreeMap<String, String>,
        refused: BTreeMap<String, String>,
    ) -> Self {
        let mut aliases = BTreeMap::new();
        let mut modules = BTreeSet::new();
        for (canonical, item) in &items {
            for alias in &item.aliases {
                aliases
                    .entry(alias.clone())
                    .or_insert_with(|| canonical.clone());
            }
            for path in std::iter::once(canonical).chain(item.aliases.iter()) {
                let mut end = path.len();
                while let Some(index) = path[..end].rfind("::") {
                    modules.insert(path[..index].to_string());
                    end = index;
                }
            }
        }
        Allowlist {
            items,
            aliases,
            prefixes,
            implements,
            modules,
            kinds,
            refused,
            returns: BTreeMap::new(),
        }
    }

    /// Parses the generated data file.
    pub fn parse(text: &str) -> Result<Self> {
        let mut items = BTreeMap::new();
        let mut prefixes = BTreeSet::new();
        let mut implements = BTreeMap::new();
        let mut kinds = BTreeMap::new();
        let mut refused = BTreeMap::new();
        let mut returns = BTreeMap::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: Entry = serde_json::from_str(line).map_err(|error| {
                RegistryError::Invalid(format!("allowlist line {}: {error}", index + 1))
            })?;
            if !entry.implements.is_empty() {
                implements.insert(entry.path.clone(), entry.implements.clone());
            }
            if let Some(returned) = &entry.returns {
                returns.insert(entry.path.clone(), returned.clone());
            }
            if entry.refused {
                for alias in &entry.aliases {
                    refused.insert(alias.clone(), entry.path.clone());
                }
                refused.insert(entry.path.clone(), entry.path.clone());
                if entry.prefix {
                    prefixes.insert(entry.path.clone());
                }
                kinds.insert(entry.path, entry.kind);
                continue;
            }
            if entry.prefix {
                prefixes.insert(entry.path.clone());
            }
            kinds.insert(entry.path.clone(), entry.kind);
            items.insert(
                entry.path,
                AllowedItem {
                    capability: entry.capability,
                    hidden: entry.hidden,
                    aliases: entry.aliases,
                },
            );
        }
        let mut list = Self::assemble(items, prefixes, implements, kinds, refused);
        list.returns = returns;
        Ok(list)
    }

    /// The item a full path names, through an alias when needed.
    pub fn lookup<'a>(&'a self, path: &str) -> Option<(&'a str, &'a AllowedItem)> {
        if let Some((canonical, item)) = self.items.get_key_value(path) {
            return Some((canonical.as_str(), item));
        }
        let canonical = self.aliases.get(path)?;
        self.items
            .get_key_value(canonical.as_str())
            .map(|(canonical, item)| (canonical.as_str(), item))
    }

    /// How the list admits a full path, or `None` when it does not.
    pub fn admit<'a>(&'a self, path: &str) -> Option<Admission<'a>> {
        if let Some((canonical, item)) = self.lookup(path) {
            return Some(Admission::Item { canonical, item });
        }
        if let Some(canonical) = self.refused.get(path) {
            return Some(Admission::Refused { canonical });
        }
        let mut end = path.len();
        while let Some(index) = path[..end].rfind("::") {
            let ancestor = &path[..index];
            if let Some((root, item)) = self.lookup(ancestor)
                && self.prefixes.contains(root)
            {
                return Some(Admission::Prefix { root, item });
            }
            if let Some(canonical) = self.refused.get(ancestor)
                && self.prefixes.contains(canonical.as_str())
            {
                return Some(Admission::Refused { canonical });
            }
            end = index;
        }
        if self.modules.contains(path) {
            return Some(Admission::Module);
        }
        None
    }

    /// The method or associated item `name` of a type, through the type's
    /// own path, its aliases and the Suprnova traits it implements.
    pub fn member<'a>(&'a self, type_path: &str, name: &str) -> Option<Admission<'a>> {
        let canonical = match self.lookup(type_path) {
            Some((canonical, _)) => Some(canonical.to_string()),
            None => self.refused.get(type_path).cloned(),
        };
        let mut owners: Vec<String> = Vec::new();
        owners.extend(canonical.iter().cloned());
        owners.push(type_path.to_string());
        if let Some(canonical) = &canonical {
            owners.extend(
                self.implements
                    .get(canonical)
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        owners
            .iter()
            .filter_map(|owner| self.admit(&format!("{owner}::{name}")))
            .find(|found| !matches!(found, Admission::Module))
    }

    /// The kind of an admitted item (`fn`, `struct`, `trait`, ...), when the
    /// data file recorded one.
    pub fn kind(&self, canonical: &str) -> Option<&str> {
        self.kinds.get(canonical).map(String::as_str)
    }

    /// The type a function returns, written with full paths, when the
    /// feature map recorded it (`_` for a type that names no single type).
    pub fn returns(&self, canonical: &str) -> Option<&str> {
        self.returns.get(canonical).map(String::as_str)
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

/// One line of the data file.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    kind: String,
    capability: Option<Capability>,
    hidden: bool,
    prefix: bool,
    refused: bool,
    aliases: Vec<String>,
    implements: Vec<String>,
    #[serde(default)]
    returns: Option<String>,
}

/// The effect-free part of `std` a component may name (REG-030): the
/// prelude's types and traits, collections, formatting, the async
/// vocabulary (`Future`, `Pin`, `Poll`), and nothing that reaches files,
/// the network, the environment, processes or threads.
pub const STD_ALLOWED: &[&str] = &[
    "std::array",
    "std::borrow",
    "std::cell",
    "std::char",
    "std::clone",
    "std::cmp",
    "std::collections",
    "std::convert",
    "std::default",
    "std::fmt",
    "std::future",
    "std::hash",
    "std::iter",
    "std::marker",
    "std::mem::drop",
    "std::mem::replace",
    "std::mem::swap",
    "std::mem::take",
    "std::num",
    "std::ops",
    "std::option",
    "std::pin",
    "std::primitive",
    "std::result",
    "std::slice",
    "std::str",
    "std::string",
    "std::task",
    "std::vec",
    "std::boxed",
    "std::rc",
    "std::sync::Arc",
    "std::time::Duration",
];

/// The allowlist embedded in this binary, for the framework version this
/// CLI was built with.
pub fn embedded() -> Result<&'static Allowlist> {
    static PARSED: OnceLock<std::result::Result<Allowlist, RegistryError>> = OnceLock::new();
    PARSED
        .get_or_init(|| Allowlist::parse(EMBEDDED))
        .as_ref()
        .map_err(Clone::clone)
}

/// The data file's text, for the test that compares it with a fresh
/// generation from the feature map.
pub fn embedded_text() -> &'static str {
    EMBEDDED
}
