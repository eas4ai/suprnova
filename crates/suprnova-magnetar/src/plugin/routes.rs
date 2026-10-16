//! Named, framework-neutral route descriptors.

use super::wire::Method;

/// A route declared by a plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteDescriptor {
    /// HTTP method.
    pub method: Method,
    /// Path template, with `{parameter}` segments.
    pub path: String,
    /// Stable host-facing route name.
    pub name: String,
    /// Feature that owns this route, when feature mirroring is used.
    pub feature: Option<String>,
    /// Whether the route is present in this build/configuration.
    pub enabled: bool,
}

impl RouteDescriptor {
    /// Declare an enabled route.
    pub fn new(method: Method, path: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            name: name.into(),
            feature: None,
            enabled: true,
        }
    }
    /// Attach the Cargo feature name owning this route group.
    pub fn with_feature(mut self, feature: impl Into<String>) -> Self {
        self.feature = Some(feature.into());
        self
    }
    /// Mark a descriptor unavailable without inventing a placeholder route.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Match a request path and extract `{parameter}` segments.
    pub fn match_path(&self, path: &str) -> Option<Vec<(String, String)>> {
        if !self.enabled {
            return None;
        }
        let template = self.path.trim_matches('/').split('/').collect::<Vec<_>>();
        let actual = path.trim_matches('/').split('/').collect::<Vec<_>>();
        if template.len() != actual.len() {
            return None;
        }
        let mut captures = Vec::new();
        for (expected, got) in template.into_iter().zip(actual) {
            if let Some(name) = expected.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
                if name.is_empty() {
                    return None;
                }
                captures.push((name.to_owned(), got.to_owned()));
            } else if expected != got {
                return None;
            }
        }
        Some(captures)
    }

    /// Return whether two enabled templates can match the same request.
    pub fn overlaps(&self, other: &Self) -> bool {
        if !self.enabled || !other.enabled || self.method != other.method {
            return false;
        }
        let left = self.path.trim_matches('/').split('/').collect::<Vec<_>>();
        let right = other.path.trim_matches('/').split('/').collect::<Vec<_>>();
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .all(|(a, b)| a.starts_with('{') || b.starts_with('{') || a == &b)
    }
}
