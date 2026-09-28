//! Multi-level `?include=` parse tree for JSON:API compound documents.

use crate::data::RequestIncludeSet;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Parsed tree representation of a multi-level `?include=` query.
///
/// `?include=author.posts.tags,comments` parses to:
/// ```text
/// {
///   author: { posts: { tags: {} } },
///   comments: {}
/// }
/// ```
///
/// Children are stored in a `BTreeMap` so iteration order is the
/// deterministic lexicographic order of include names. The order is
/// observable only when validating includes against a resource's
/// allowlist: if multiple invalid include paths are present in one
/// request, the first one rejected is now stable across runs (instead
/// of varying with `HashMap`'s randomised iteration). The JSON:API
/// response itself does not surface this order - `included` is a set,
/// and the spec assigns no semantic meaning to its member order.
///
/// Paths are capped at [`current_max_relationship_depth`] segments while
/// they are parsed, so a deeply nested request costs bounded work no
/// matter how long its query string is.
#[derive(Debug, Default, Clone)]
pub struct IncludeTree {
    /// Sub-includes keyed by their dotted segment. The leaf set is
    /// the set of paths actually requested by the client.
    pub children: BTreeMap<String, IncludeTree>,
}

/// Default ceiling on `?include=` path depth, matching Laravel's
/// `JsonApiResource::$maxRelationshipDepth`.
pub const DEFAULT_MAX_RELATIONSHIP_DEPTH: usize = 5;

/// The configured cap, stored as `depth + 1` so `0` reads as "never
/// configured".
///
/// The "0 means use the default" sentinel the upload caps use
/// (`crate::http::upload`) cannot work here: `0` is a meaningful setting
/// that turns every include off, and the sentinel would swallow it.
static MAX_RELATIONSHIP_DEPTH: AtomicUsize = AtomicUsize::new(0);

/// Cap how many dotted segments a `?include=` path may carry.
///
/// `?include=author.posts.author.posts…` is attacker-controlled recursion
/// on a cyclic relationship graph: each segment multiplies the work one
/// request performs, bounded only by the query string's own length. The
/// cap truncates every path while it is parsed, so the segments past it
/// are never walked.
///
/// Truncation only ever removes nodes from the tree - it can never add
/// one - and every level still checks its own default-deny allowlist
/// before descending, so a truncated path can never reach data the full
/// path could not. A segment past the cap is dropped before the allowlist
/// sees it, so a request naming an unknown relationship out there is
/// served with the segments that survived instead of rejected.
///
/// Call this once at boot, from `bootstrap::register()`. `0` turns every
/// include off. Thread-safe and idempotent; the most recent value wins for
/// any subsequent request.
pub fn max_relationship_depth(depth: usize) {
    MAX_RELATIONSHIP_DEPTH.store(depth.saturating_add(1), Ordering::SeqCst);
}

/// The depth cap in force, or [`DEFAULT_MAX_RELATIONSHIP_DEPTH`] when
/// [`max_relationship_depth`] has never been called.
///
/// A cap of `usize::MAX` reads back as `usize::MAX - 1` - an artifact of
/// the `depth + 1` encoding, and unreachable in practice, because no query
/// string can carry that many segments.
pub fn current_max_relationship_depth() -> usize {
    match MAX_RELATIONSHIP_DEPTH.load(Ordering::SeqCst) {
        0 => DEFAULT_MAX_RELATIONSHIP_DEPTH,
        stored => stored - 1,
    }
}

impl IncludeTree {
    /// Build from `RequestIncludeSet`. Each include name is split on `.`
    /// and the segments accumulate into a nested tree, truncated to
    /// [`current_max_relationship_depth`] segments.
    pub fn from_include_set(set: &RequestIncludeSet) -> Self {
        Self::from_include_set_with_depth(set, current_max_relationship_depth())
    }

    /// [`Self::from_include_set`] with an explicit cap, so the truncation
    /// is testable without touching the process-global setting.
    pub(crate) fn from_include_set_with_depth(set: &RequestIncludeSet, depth: usize) -> Self {
        let mut root = Self::default();
        for path in &set.include {
            let mut node = &mut root;
            for segment in path.split('.').take(depth) {
                node = node.children.entry(segment.to_string()).or_default();
            }
        }
        root
    }

    /// Empty tree - no relationships requested.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// Lookup a child subtree by name. Returns `None` when the name
    /// is not present in this branch.
    pub fn subtree(&self, name: &str) -> Option<&IncludeTree> {
        self.children.get(name)
    }

    /// Iterate over (name, subtree) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &IncludeTree)> {
        self.children.iter().map(|(k, v)| (k.as_str(), v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(include: &[&str]) -> RequestIncludeSet {
        RequestIncludeSet {
            include: include.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        }
    }

    /// Walk `tree` along `path`, returning how many segments were
    /// actually present.
    fn depth_of(tree: &IncludeTree, path: &[&str]) -> usize {
        let mut node = tree;
        let mut reached = 0;
        for segment in path {
            match node.subtree(segment) {
                Some(child) => {
                    reached += 1;
                    node = child;
                }
                None => break,
            }
        }
        reached
    }

    #[test]
    fn a_path_shorter_than_the_cap_is_untouched() {
        let tree = IncludeTree::from_include_set_with_depth(&set(&["author.posts"]), 5);
        assert_eq!(depth_of(&tree, &["author", "posts"]), 2);
    }

    #[test]
    fn a_path_exactly_as_long_as_the_cap_keeps_every_segment() {
        let tree = IncludeTree::from_include_set_with_depth(
            &set(&["author.posts.author.posts.author"]),
            5,
        );
        assert_eq!(
            depth_of(&tree, &["author", "posts", "author", "posts", "author"]),
            5,
            "a path exactly at the cap is not truncated"
        );
    }

    #[test]
    fn a_path_longer_than_the_cap_is_truncated() {
        let tree = IncludeTree::from_include_set_with_depth(
            &set(&["author.posts.author.posts.author.posts"]),
            5,
        );
        assert_eq!(
            depth_of(
                &tree,
                &["author", "posts", "author", "posts", "author", "posts"]
            ),
            5,
            "the sixth segment must not be in the tree at all"
        );
    }

    #[test]
    fn a_cap_of_zero_drops_every_include() {
        let tree = IncludeTree::from_include_set_with_depth(&set(&["author", "author.posts"]), 0);
        assert!(tree.is_empty(), "depth 0 means no relationships at all");
    }

    #[test]
    fn truncation_keeps_the_shared_prefix_of_sibling_paths() {
        let tree = IncludeTree::from_include_set_with_depth(
            &set(&["author.posts.tags", "author.posts.author"]),
            2,
        );
        assert_eq!(depth_of(&tree, &["author", "posts", "tags"]), 2);
        assert_eq!(depth_of(&tree, &["author", "posts", "author"]), 2);
    }

    #[test]
    fn the_default_cap_is_five() {
        assert_eq!(DEFAULT_MAX_RELATIONSHIP_DEPTH, 5);
    }
}
