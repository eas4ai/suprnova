//! Whether a constant URL stays on the application's origin (REG-031,
//! REG-032). The test follows the WHATWG URL parser where it decides the
//! origin: leading and trailing controls and spaces are stripped, tabs and
//! newlines anywhere are removed, a backslash counts as a slash, and any
//! scheme at all, `javascript:` and `data:` included, leaves the origin.
//! Only a relative reference (a path, a query, a fragment or nothing) is
//! admitted.

/// Why a constant URL was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UrlRefusal {
    /// `javascript:` runs script.
    Javascript,
    /// `data:` carries a document of its own.
    Data,
    /// Another scheme, `https:` included: the scan cannot know the
    /// application's host, so an absolute URL may leave it.
    Scheme,
    /// `//host` or `\\host`: another host on the same scheme.
    NetworkPath,
}

impl UrlRefusal {
    /// What the refusal means, for a finding.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            UrlRefusal::Javascript => "a `javascript:` URL runs script",
            UrlRefusal::Data => "a `data:` URL carries a document of its own",
            UrlRefusal::Scheme => "an absolute URL may leave the application's origin",
            UrlRefusal::NetworkPath => "a URL starting with two slashes names another host",
        }
    }
}

/// The URL as the URL parser reads it: outer controls and spaces trimmed,
/// tabs and newlines removed.
fn normalized(value: &str) -> String {
    value
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect()
}

/// The scheme a URL starts with, lowercased, when it has one.
fn scheme(value: &str) -> Option<String> {
    let mut chars = value.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    for (index, c) in chars {
        if c == ':' {
            return Some(value[..index].to_ascii_lowercase());
        }
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return None;
        }
    }
    None
}

/// Checks a constant URL.
pub(crate) fn check_constant(value: &str) -> Result<(), UrlRefusal> {
    let value = normalized(value);
    let mut chars = value.chars();
    if let (Some(first), Some(second)) = (chars.next(), chars.next())
        && matches!(first, '/' | '\\')
        && matches!(second, '/' | '\\')
    {
        return Err(UrlRefusal::NetworkPath);
    }
    match scheme(&value).as_deref() {
        None => Ok(()),
        Some("javascript") => Err(UrlRefusal::Javascript),
        Some("data") => Err(UrlRefusal::Data),
        Some(_) => Err(UrlRefusal::Scheme),
    }
}

/// Whether a constant names a resource on another origin or runs script:
/// `javascript:`, `data:`, `vbscript:`, a network path, or a scheme that
/// fetches (`http`, `https`, `ws`, `wss`, `ftp`, `file`, `blob`). A string
/// passed where the scan cannot tell whether it becomes a URL is refused
/// only when it is one of these, so a label such as `Note: ...` still
/// passes.
pub(crate) fn names_another_origin(value: &str) -> bool {
    match check_constant(value) {
        Ok(()) => false,
        Err(UrlRefusal::Javascript | UrlRefusal::Data | UrlRefusal::NetworkPath) => true,
        Err(UrlRefusal::Scheme) => matches!(
            scheme(&normalized(value)).as_deref(),
            Some(
                "http"
                    | "https"
                    | "ws"
                    | "wss"
                    | "ftp"
                    | "file"
                    | "blob"
                    | "vbscript"
                    | "filesystem"
            )
        ),
    }
}

/// Whether a constant prefix already fixes a URL to the application's
/// origin, whatever follows it: a path from the root, a relative path whose
/// first segment ends before any colon, a query or a fragment.
pub(crate) fn prefix_commits_to_origin(prefix: &str) -> bool {
    let prefix = normalized(prefix);
    let mut chars = prefix.chars();
    match chars.next() {
        Some('?' | '#') => true,
        Some('/' | '\\') => chars
            .next()
            .is_some_and(|second| !matches!(second, '/' | '\\')),
        Some(_) => {
            let end = prefix.find(['/', '\\', '?', '#']);
            match end {
                Some(end) => !prefix[..end].contains(':') && check_constant(&prefix).is_ok(),
                None => false,
            }
        }
        None => false,
    }
}

/// The URLs a `srcset` value names: every comma- or space-separated token
/// that is not a width, density or height descriptor.
pub(crate) fn srcset_urls(value: &str) -> Vec<&str> {
    value
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|token| !token.is_empty())
        .filter(|token| !is_descriptor(token))
        .collect()
}

fn is_descriptor(token: &str) -> bool {
    let Some(unit) = token.chars().last() else {
        return false;
    };
    if !matches!(unit, 'w' | 'x' | 'h') {
        return false;
    }
    let number = &token[..token.len() - 1];
    !number.is_empty() && number.chars().all(|c| c.is_ascii_digit() || c == '.')
}

#[cfg(test)]
mod tests {
    use super::{UrlRefusal, check_constant, prefix_commits_to_origin, srcset_urls};

    #[test]
    fn relative_references_stay_on_the_origin() {
        for url in [
            "",
            "/",
            "/a/b?c#d",
            "sort",
            "./x",
            "../x",
            "?q=1",
            "#top",
            "java%0ascript:x",
        ] {
            assert_eq!(check_constant(url), Ok(()), "{url}");
        }
    }

    #[test]
    fn schemes_and_network_paths_are_refused_in_every_spelling() {
        let cases = [
            ("javascript:alert(1)", UrlRefusal::Javascript),
            (" JaVaScRiPt:alert(1)", UrlRefusal::Javascript),
            ("java\tscript:alert(1)", UrlRefusal::Javascript),
            ("\u{1}javascript:x", UrlRefusal::Javascript),
            ("data:text/html,x", UrlRefusal::Data),
            ("DATA:x", UrlRefusal::Data),
            ("https://evil.test", UrlRefusal::Scheme),
            ("mailto:a@b", UrlRefusal::Scheme),
            ("//evil.test", UrlRefusal::NetworkPath),
            ("\\\\evil.test", UrlRefusal::NetworkPath),
            ("/\\evil.test", UrlRefusal::NetworkPath),
            (" \n//evil.test", UrlRefusal::NetworkPath),
        ];
        for (url, refusal) in cases {
            assert_eq!(check_constant(url), Err(refusal), "{url:?}");
        }
    }

    #[test]
    fn only_a_committed_prefix_fixes_the_origin() {
        assert!(prefix_commits_to_origin("/users/"));
        assert!(prefix_commits_to_origin("?page="));
        assert!(prefix_commits_to_origin("users/"));
        assert!(!prefix_commits_to_origin("/"));
        assert!(!prefix_commits_to_origin("java"));
        assert!(!prefix_commits_to_origin("https://"));
        assert!(!prefix_commits_to_origin(""));
    }

    #[test]
    fn srcset_names_each_candidate_url() {
        assert_eq!(
            srcset_urls("a.png 1x, //evil/b.png 2x"),
            vec!["a.png", "//evil/b.png"]
        );
        assert_eq!(srcset_urls("a.png,b.png 300w"), vec!["a.png", "b.png"]);
    }
}
