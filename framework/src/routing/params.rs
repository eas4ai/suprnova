//! Optional route parameters and parameter constraints.
//!
//! # Optional parameters
//!
//! `/posts/{id?}` matches `/posts` and `/posts/42`. The matcher has no
//! optional segment, so the router registers the pattern once for every
//! length it can have, all pointing at one handler: `/posts` and
//! `/posts/{id}`. On the short form the handler finds no `id`.
//!
//! Every form is stored under the pattern as it was written. The pattern is
//! what a route's middleware, its name and its constraints are keyed by, so
//! they apply to every form alike.
//!
//! An optional parameter has nothing but optional parameters behind it. In
//! `/a/{x?}/b` the matcher could not tell `/a/b` from `/a/{x}` with `x = b`.
//!
//! # Constraints
//!
//! A [`ParamConstraint`] says what a parameter may hold: digits only, a
//! UUID, one of a list, a pattern. The router checks it when the path has
//! matched. A value the constraint refuses is a route that did not match,
//! which is a 404, and the handler is not run.

use crate::error::FrameworkError;
use std::sync::Arc;

/// The `where_*` methods of a route builder. The builder has `constrain`,
/// and this adds the spellings that read like Laravel's. Each is
/// `constrain` with the constraint named, and stops the boot where
/// `constrain` does.
macro_rules! where_methods {
    () => {
        /// Allow only ASCII digits in `param`. A request with anything
        /// else there is a 404. Mirrors Laravel's `whereNumber`.
        pub fn where_number(self, param: &str) -> Self {
            self.constrain(param, $crate::routing::ParamConstraint::Number)
        }

        /// Allow only ASCII letters in `param`. Mirrors `whereAlpha`.
        pub fn where_alpha(self, param: &str) -> Self {
            self.constrain(param, $crate::routing::ParamConstraint::Alpha)
        }

        /// Allow only ASCII letters and digits in `param`. Mirrors
        /// `whereAlphaNumeric`.
        pub fn where_alpha_numeric(self, param: &str) -> Self {
            self.constrain(param, $crate::routing::ParamConstraint::AlphaNumeric)
        }

        /// Allow only a hyphenated UUID in `param`. Mirrors `whereUuid`.
        pub fn where_uuid(self, param: &str) -> Self {
            self.constrain(param, $crate::routing::ParamConstraint::Uuid)
        }

        /// Allow only a ULID in `param`. Mirrors `whereUlid`.
        pub fn where_ulid(self, param: &str) -> Self {
            self.constrain(param, $crate::routing::ParamConstraint::Ulid)
        }

        /// Allow only these values in `param`. Mirrors `whereIn`.
        pub fn where_in<I, S>(self, param: &str, values: I) -> Self
        where
            I: IntoIterator<Item = S>,
            S: Into<String>,
        {
            self.constrain(param, $crate::routing::ParamConstraint::one_of(values))
        }

        /// Allow only a value the regular expression matches from its
        /// first character to its last. Mirrors Laravel's `where`.
        ///
        /// `\d` and `\w` match the digits and letters of every script
        /// here, where Laravel's match ASCII alone. Write `[0-9]` for the
        /// ASCII digits, or use `where_number`.
        ///
        /// # Panics
        ///
        /// When `expression` is not a regular expression, at boot.
        /// `ParamConstraint::pattern` returns the error instead.
        pub fn where_pattern(self, param: &str, expression: &str) -> Self {
            let constraint = $crate::routing::ParamConstraint::pattern(expression)
                .unwrap_or_else(|e| panic!("{e}"));
            self.constrain(param, constraint)
        }
    };
}
pub(crate) use where_methods;

/// The patterns to register for `pattern`, shortest first: `pattern` alone
/// when it has no optional parameter, and one pattern for every length
/// otherwise. The `?` is gone from every one of them.
pub(crate) fn expand_optional(pattern: &str) -> Result<Vec<String>, FrameworkError> {
    if !pattern.contains("?}") {
        return Ok(vec![pattern.to_owned()]);
    }
    let absolute = pattern.starts_with('/');
    // The forms are put together from the segments, so a slash with
    // nothing behind it would be lost: `/posts/{id?}/` would register
    // `/posts/{id}`, and a request for `/posts/42/` would find nothing.
    let inner = pattern.strip_prefix('/').unwrap_or(pattern);
    if inner.split('/').any(str::is_empty) {
        return Err(FrameworkError::internal(format!(
            "route `{pattern}` has an optional parameter and an empty segment: a slash \
             at the end, or two in a row. Write the pattern without it"
        )));
    }
    let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let is_optional = |segment: &str| segment.starts_with('{') && segment.ends_with("?}");
    let first_optional = segments
        .iter()
        .position(|s| is_optional(s))
        .unwrap_or(segments.len());
    if let Some(after) = segments[first_optional..].iter().find(|s| !is_optional(s)) {
        return Err(FrameworkError::internal(format!(
            "route `{pattern}` has `{after}` behind an optional parameter. An optional \
             parameter can only be followed by optional parameters: the router could not \
             tell a request that leaves it out from one that fills it"
        )));
    }
    if segments.iter().any(|s| s.contains("?}") && !is_optional(s)) {
        return Err(FrameworkError::internal(format!(
            "route `{pattern}` has an optional parameter that is not a whole segment"
        )));
    }
    let join = |parts: &[String]| {
        let joined = parts.join("/");
        match (absolute, joined.is_empty()) {
            (true, _) => format!("/{joined}"),
            (false, true) => "/".to_owned(),
            (false, false) => joined,
        }
    };
    let required: Vec<String> = segments[..first_optional]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let mut expanded = vec![join(&required)];
    let mut parts = required;
    for optional in &segments[first_optional..] {
        let name = &optional[1..optional.len() - 2];
        parts.push(format!("{{{name}}}"));
        expanded.push(join(&parts));
    }
    Ok(expanded)
}

/// The names of the parameters of `pattern`, in order, without the `?` of
/// an optional one. A catch-all `{*rest}` is named `rest`.
pub(crate) fn param_names(pattern: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else {
            break;
        };
        let name = rest[..close].trim_start_matches('*').trim_end_matches('?');
        names.push(name.to_owned());
        rest = &rest[close + 1..];
    }
    names
}

/// What a route parameter may hold. Attach one to a route with
/// `.where_number("id")` and its siblings, or with `.constrain(...)`, which
/// every route builder has. A `Router` route has `.try_constrain(...)` as
/// well, which returns the error.
///
/// Mirrors Laravel's `whereNumber`, `whereAlpha`, `whereAlphaNumeric`,
/// `whereUuid`, `whereUlid`, `whereIn` and `where`.
#[derive(Debug, Clone)]
pub enum ParamConstraint {
    /// One or more ASCII digits.
    Number,
    /// One or more ASCII letters.
    Alpha,
    /// One or more ASCII letters and digits.
    AlphaNumeric,
    /// A UUID in its hyphenated form, in either case.
    Uuid,
    /// A ULID: 26 characters of Crockford base 32, in either case.
    Ulid,
    /// One of these values, compared exactly.
    In(Vec<String>),
    /// A value the pattern matches from its first character to its last.
    /// Build it with [`ParamConstraint::pattern`].
    Pattern(WholeValuePattern),
}

/// A regular expression that matches a value from its first character to
/// its last.
///
/// [`ParamConstraint::pattern`] is the one way to build it, so a pattern
/// that matches a part of a value, `[0-9]{4}` inside `x2024abc`, cannot
/// become a constraint.
#[derive(Debug, Clone)]
pub struct WholeValuePattern {
    expression: String,
    whole_value: Arc<regex::Regex>,
}

impl WholeValuePattern {
    /// The expression as it was given to [`ParamConstraint::pattern`].
    pub fn as_str(&self) -> &str {
        &self.expression
    }

    /// Whether the expression matches `value` from its first character to
    /// its last.
    pub fn matches(&self, value: &str) -> bool {
        self.whole_value.is_match(value)
    }
}

impl ParamConstraint {
    /// A constraint from a regular expression. The expression has to match
    /// the whole value: `[0-9]+` refuses `12a`, as it does in Laravel.
    ///
    /// `\d` and `\w` match the digits and letters of every script, where
    /// Laravel's match ASCII alone. Write `[0-9]` for the ASCII digits.
    ///
    /// # Errors
    ///
    /// When `expression` is not a regular expression.
    pub fn pattern(expression: &str) -> Result<Self, FrameworkError> {
        let refused = |e: regex::Error| {
            FrameworkError::internal(format!(
                "`{expression}` is not a pattern a route parameter can be held to: {e}"
            ))
        };
        // The expression has to be one on its own. `a)|(b` is none, and
        // between `^(?:` and `)$` it would be one that matches whatever
        // starts with `a` or ends with `b`.
        regex::Regex::new(expression).map_err(refused)?;
        let whole_value = regex::Regex::new(&format!("^(?:{expression})$")).map_err(refused)?;
        Ok(Self::Pattern(WholeValuePattern {
            expression: expression.to_owned(),
            whole_value: Arc::new(whole_value),
        }))
    }

    /// A constraint that allows exactly these values.
    pub fn one_of<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::In(values.into_iter().map(Into::into).collect())
    }

    /// Whether `value` is one the constraint allows.
    pub fn allows(&self, value: &str) -> bool {
        match self {
            Self::Number => !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
            Self::Alpha => !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphabetic()),
            Self::AlphaNumeric => {
                !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric())
            }
            Self::Uuid => is_hyphenated_uuid(value),
            Self::Ulid => is_ulid(value),
            Self::In(values) => values.iter().any(|allowed| allowed == value),
            Self::Pattern(pattern) => pattern.matches(value),
        }
    }
}

/// `8-4-4-4-12` hexadecimal digits. Stricter than `Uuid::parse_str`, which
/// also takes the simple, braced and URN forms: a route that asks for a
/// UUID asks for the form a URL carries.
fn is_hyphenated_uuid(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    let lengths = [8, 4, 4, 4, 12];
    groups.len() == lengths.len()
        && groups.iter().zip(lengths).all(|(group, length)| {
            group.len() == length && group.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

/// 26 characters of Crockford base 32, the first no greater than `7`: a
/// ULID is 128 bits, and 26 characters of 5 bits would hold 130.
fn is_ulid(value: &str) -> bool {
    let crockford = |b: u8| {
        b.is_ascii_digit()
            || matches!(b.to_ascii_uppercase(), b'A'..=b'H' | b'J' | b'K' | b'M' | b'N' | b'P'..=b'T' | b'V'..=b'Z')
    };
    value.len() == 26
        && value.bytes().all(crockford)
        && value.bytes().next().is_some_and(|first| first <= b'7')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_without_an_optional_parameter_is_registered_as_it_is() {
        assert_eq!(expand_optional("/posts/{id}").unwrap(), ["/posts/{id}"]);
        assert_eq!(expand_optional("/").unwrap(), ["/"]);
        assert_eq!(
            expand_optional("/files/{*rest}").unwrap(),
            ["/files/{*rest}"]
        );
    }

    #[test]
    fn an_optional_parameter_gives_a_pattern_for_every_length() {
        assert_eq!(
            expand_optional("/posts/{id?}").unwrap(),
            ["/posts", "/posts/{id}"]
        );
        assert_eq!(
            expand_optional("/archive/{year?}/{month?}").unwrap(),
            ["/archive", "/archive/{year}", "/archive/{year}/{month}"]
        );
        assert_eq!(expand_optional("/{locale?}").unwrap(), ["/", "/{locale}"]);
    }

    #[test]
    fn an_optional_parameter_with_a_required_segment_behind_it_is_refused() {
        let error = expand_optional("/posts/{id?}/comments").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("`comments` behind an optional parameter"),
            "{error}"
        );
        assert!(expand_optional("/a/{x?}/{y}").is_err());
        assert!(expand_optional("/a/pre-{x?}").is_err());
    }

    #[test]
    fn the_names_of_a_pattern_come_without_their_marks() {
        assert_eq!(param_names("/a/{x}/b/{y?}/{*rest}"), ["x", "y", "rest"]);
        assert!(param_names("/plain").is_empty());
    }

    #[test]
    fn each_constraint_allows_what_its_name_says() {
        let allows = |constraint: &ParamConstraint, value: &str| constraint.allows(value);

        assert!(allows(&ParamConstraint::Number, "42"));
        assert!(!allows(&ParamConstraint::Number, "4x2"));
        assert!(!allows(&ParamConstraint::Number, ""));
        assert!(!allows(&ParamConstraint::Number, "-1"));
        assert!(!allows(&ParamConstraint::Number, "٤٢"), "ASCII digits only");

        assert!(allows(&ParamConstraint::Alpha, "abc"));
        assert!(!allows(&ParamConstraint::Alpha, "abc1"));
        assert!(allows(&ParamConstraint::AlphaNumeric, "abc1"));
        assert!(!allows(&ParamConstraint::AlphaNumeric, "abc-1"));

        let uuid = ParamConstraint::Uuid;
        assert!(allows(&uuid, "550e8400-e29b-41d4-a716-446655440000"));
        assert!(allows(&uuid, "550E8400-E29B-41D4-A716-446655440000"));
        assert!(!allows(&uuid, "550e8400e29b41d4a716446655440000"));
        assert!(!allows(&uuid, "550e8400-e29b-41d4-a716-44665544000g"));

        let ulid = ParamConstraint::Ulid;
        assert!(allows(&ulid, "01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert!(
            !allows(&ulid, "81ARZ3NDEKTSV4RRFFQ69G5FAV"),
            "more than 128 bits"
        );
        assert!(
            !allows(&ulid, "01ARZ3NDEKTSV4RRFFQ69G5FAU"),
            "U is not in the alphabet"
        );
        assert!(!allows(&ulid, "01ARZ3NDEKTSV4RRFFQ69G5FA"));

        let status = ParamConstraint::one_of(["draft", "published"]);
        assert!(allows(&status, "draft"));
        assert!(!allows(&status, "Draft"));
    }

    #[test]
    fn a_pattern_has_to_match_the_whole_value() {
        let digits = ParamConstraint::pattern("[0-9]+").unwrap();
        assert!(digits.allows("2026"));
        assert!(!digits.allows("2026x"));
        assert!(!digits.allows("x2026"));

        // An alternation is held to the whole value too.
        let either = ParamConstraint::pattern("en|fr").unwrap();
        assert!(either.allows("fr"));
        assert!(!either.allows("french"));

        let error = ParamConstraint::pattern("[0-9").unwrap_err();
        assert!(error.to_string().contains("is not a pattern"), "{error}");
    }
}
