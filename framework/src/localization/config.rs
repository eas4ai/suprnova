//! Localization configuration — env-driven with a programmatic builder,
//! same shape as `SessionConfig` / `CacheConfig`.

use super::locale::Locale;
use crate::error::FrameworkError;
use std::collections::{HashMap, HashSet};

/// One source the locale middleware consults, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detect {
    /// The session key (default name `locale`).
    Session,
    /// The cookie (default name `locale`).
    Cookie,
    /// `Accept-Language`, negotiated against the available catalogs.
    Header,
}

/// Configuration for the localization subsystem.
#[derive(Debug, Clone)]
pub struct LocalizationConfig {
    /// Locale used when detection finds nothing. Env: `APP_LOCALE`.
    pub default_locale: Locale,
    /// Locale consulted when a key is missing from the current locale's
    /// catalog. Env: `APP_FALLBACK_LOCALE`.
    pub fallback_locale: Locale,
    /// Whether Fluent wraps interpolations in Unicode isolation marks
    /// (U+2068/U+2069). Off by default — see the manual's divergence
    /// note; turn on when shipping RTL locales.
    pub use_isolating: bool,
    /// Detection order; first hit wins.
    pub detection: Vec<Detect>,
    /// Session key holding a locale override.
    pub session_key: String,
    /// Cookie name holding a locale override.
    pub cookie_name: String,
    /// Per-locale fallback parents (`child -> parent`), walked before
    /// `fallback_locale` when a key is missing from the child's catalog —
    /// e.g. `pt-PT` gaining Brazilian Portuguese as an intermediate
    /// fallback ahead of `en`. Env: `APP_LOCALE_PARENTS`, a comma-separated
    /// list of `child=parent` pairs (`pt-PT=pt-BR,en-AU=en-GB`). Empty or
    /// unset means no per-locale overrides — only `fallback_locale`
    /// applies.
    pub parents: HashMap<Locale, Locale>,
}

impl LocalizationConfig {
    /// Build from `APP_LOCALE` / `APP_FALLBACK_LOCALE`, defaulting both
    /// to `en`. Fails loudly on a malformed locale value.
    pub fn from_env() -> Result<Self, FrameworkError> {
        let default_locale =
            Locale::parse(&std::env::var("APP_LOCALE").unwrap_or_else(|_| "en".into()))?;
        let fallback_locale =
            Locale::parse(&std::env::var("APP_FALLBACK_LOCALE").unwrap_or_else(|_| "en".into()))?;
        let parents = parse_parents(&std::env::var("APP_LOCALE_PARENTS").unwrap_or_default())?;
        Ok(Self {
            default_locale,
            fallback_locale,
            use_isolating: false,
            detection: vec![Detect::Session, Detect::Cookie, Detect::Header],
            session_key: "locale".into(),
            cookie_name: "locale".into(),
            parents,
        })
    }

    /// Override the default locale.
    pub fn default_locale(mut self, locale: Locale) -> Self {
        self.default_locale = locale;
        self
    }

    /// Override the fallback locale.
    pub fn fallback_locale(mut self, locale: Locale) -> Self {
        self.fallback_locale = locale;
        self
    }

    /// Turn Unicode isolation marks on/off (default off).
    pub fn use_isolating(mut self, on: bool) -> Self {
        self.use_isolating = on;
        self
    }

    /// Replace the detection chain.
    pub fn detection(mut self, order: Vec<Detect>) -> Self {
        self.detection = order;
        self
    }

    /// Rename the session key consulted for a locale override.
    pub fn session_key(mut self, key: impl Into<String>) -> Self {
        self.session_key = key.into();
        self
    }

    /// Rename the cookie consulted for a locale override.
    pub fn cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = name.into();
        self
    }

    /// Add (or overwrite) a single fallback parent: `child` resolves
    /// through `parent` before `fallback_locale`. Last write wins here —
    /// unlike `parse_parents`, a programmatic builder call is not an
    /// ambiguous config, just a later override.
    pub fn parent(mut self, child: Locale, parent: Locale) -> Self {
        self.parents.insert(child, parent);
        self
    }
}

/// Parse `APP_LOCALE_PARENTS`-shaped input: comma-separated `child=parent`
/// pairs, e.g. `"pt-PT=pt-BR,en-AU=en-GB"`. Segments are trimmed and blank
/// segments (from stray commas, or an empty string overall) are skipped,
/// so `""` parses to an empty map. Each side is trimmed and parsed with
/// [`Locale::parse`]. Errs loudly on a malformed pair (no `=`, an empty
/// child or parent, an invalid locale), a child named more than once
/// (ambiguous config must be loud, not last-wins), or a parent chain that
/// cycles back on itself (a child can never be its own ancestor).
pub(crate) fn parse_parents(raw: &str) -> Result<HashMap<Locale, Locale>, FrameworkError> {
    let mut parents = HashMap::new();
    for segment in raw.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        let (child, parent) = segment.split_once('=').ok_or_else(|| {
            FrameworkError::param(format!(
                "`APP_LOCALE_PARENTS` entry `{segment}` is missing `=` — expected `child=parent`"
            ))
        })?;
        let child = child.trim();
        let parent = parent.trim();
        if child.is_empty() || parent.is_empty() {
            return Err(FrameworkError::param(format!(
                "`APP_LOCALE_PARENTS` entry `{segment}` has an empty child or parent — expected `child=parent`"
            )));
        }
        let child = Locale::parse(child)?;
        let parent = Locale::parse(parent)?;
        if parents.insert(child.clone(), parent).is_some() {
            return Err(FrameworkError::param(format!(
                "`APP_LOCALE_PARENTS` names `{}` as a child more than once — ambiguous config, not last-wins",
                child.as_str()
            )));
        }
    }
    if let Some(cycle) = parents_cycle(&parents) {
        let path = cycle
            .iter()
            .map(|l| format!("`{}`", l.as_str()))
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(FrameworkError::param(format!(
            "`APP_LOCALE_PARENTS` has a fallback cycle: {path}"
        )));
    }
    Ok(parents)
}

/// Detect a cycle in a `child -> parent` map. Walks the parent chain from
/// each key in turn (iteration order is `HashMap`-dependent, so which key
/// is tried first — and therefore which cycle is reported when several
/// exist — is not deterministic) with a fresh per-walk `HashSet`, stopping
/// the first time a locale is revisited within that walk. The returned
/// path is the walk taken, in order, with the repeated locale appended
/// once more at the end — its *last* two elements are always equal (e.g.
/// `pt-PT -> pt-BR -> pt-PT` when the walk starts on a cycle member), but
/// its *first* element need not be part of the cycle at all: for a
/// feed-in shape like `a=b, b=c, c=b`, starting the walk from `a` returns
/// `[a, b, c, b]`, whose first element `a` merely leads into the cycle
/// rather than participating in it. Returns `None` when every chain
/// terminates (leaves the map, i.e. reaches a locale with no configured
/// parent).
pub(crate) fn parents_cycle(parents: &HashMap<Locale, Locale>) -> Option<Vec<Locale>> {
    for start in parents.keys() {
        let mut seen = HashSet::new();
        let mut path = Vec::new();
        let mut current = start.clone();
        loop {
            if !seen.insert(current.clone()) {
                path.push(current);
                return Some(path);
            }
            path.push(current.clone());
            match parents.get(&current) {
                Some(next) => current = next.clone(),
                None => break,
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_overrides_env_defaults() {
        // No env manipulation: construct the default directly to stay
        // parallel-safe, then exercise the builder.
        let cfg = LocalizationConfig {
            default_locale: Locale::parse("en").unwrap(),
            fallback_locale: Locale::parse("en").unwrap(),
            use_isolating: false,
            detection: vec![Detect::Session, Detect::Cookie, Detect::Header],
            session_key: "locale".into(),
            cookie_name: "locale".into(),
            parents: Default::default(),
        }
        .default_locale(Locale::parse("es").unwrap())
        .use_isolating(true)
        .detection(vec![Detect::Header]);
        assert_eq!(cfg.default_locale.as_str(), "es");
        assert!(cfg.use_isolating);
        assert_eq!(cfg.detection, vec![Detect::Header]);
    }

    fn locale(s: &str) -> Locale {
        Locale::parse(s).unwrap()
    }

    #[test]
    fn parse_parents_accepts_pairs_and_whitespace() {
        let map = parse_parents(" pt-PT = pt-BR , en-AU=en-GB ").unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&locale("pt-PT")), Some(&locale("pt-BR")));
        assert_eq!(map.get(&locale("en-AU")), Some(&locale("en-GB")));

        let empty = parse_parents("").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn parse_parents_rejects_malformed() {
        assert!(parse_parents("pt-PT").is_err(), "missing `=` must err");
        assert!(parse_parents("=pt-BR").is_err(), "empty child must err");
        assert!(parse_parents("pt-PT=").is_err(), "empty parent must err");
        assert!(
            parse_parents("pt PT=pt-BR").is_err(),
            "invalid locale must err"
        );
    }

    #[test]
    fn parse_parents_rejects_duplicate_children() {
        let err = parse_parents("pt-PT=pt-BR,pt-PT=en")
            .expect_err("duplicate child must be ambiguous, not last-wins");
        let message = format!("{err}");
        assert!(
            message.contains("pt-PT"),
            "error should name the duplicated child: {message}"
        );
    }

    // The brief's illustrative cycle input is `"a=b,b=a"`, but a bare
    // single-letter subtag is not valid BCP-47 (`unic-langid`'s `Language`
    // subtag requires 2-8 alphabetic characters) — `Locale::parse("a")`
    // errs before cycle detection ever runs. Substituting real two-letter
    // codes (`en`/`es`) exercises the same two-node cycle shape while
    // actually reaching `parents_cycle`; `"es=es"` (the brief's self-cycle
    // case) is valid as given.
    #[test]
    fn parse_parents_rejects_cycles() {
        let err =
            parse_parents("en=es,es=en").expect_err("a two-node cycle must err naming the cycle");
        let message = format!("{err}");
        assert!(
            message.contains("en") && message.contains("es") && message.contains("->"),
            "error should name the cycle path: {message}"
        );

        let err = parse_parents("es=es").expect_err("a self-cycle must err naming the cycle");
        let message = format!("{err}");
        assert!(
            message.contains("es") && message.contains("->"),
            "error should name the self-cycle: {message}"
        );
    }

    #[test]
    fn parents_cycle_detects_and_clears() {
        let mut chain = HashMap::new();
        chain.insert(locale("en"), locale("es"));
        chain.insert(locale("es"), locale("fr"));
        assert_eq!(
            parents_cycle(&chain),
            None,
            "a chain that terminates is not a cycle"
        );

        let mut cyclic = HashMap::new();
        cyclic.insert(locale("en"), locale("es"));
        cyclic.insert(locale("es"), locale("en"));
        let path = parents_cycle(&cyclic).expect("a=b,b=a must be detected as a cycle");
        assert_eq!(path.first(), path.last(), "path must close on itself");
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn builder_parent_adds_pairs() {
        // Constructed directly rather than via `from_env` to stay
        // parallel-safe, same rationale as `builder_overrides_env_defaults`.
        let cfg = LocalizationConfig {
            default_locale: locale("en"),
            fallback_locale: locale("en"),
            use_isolating: false,
            detection: vec![Detect::Session, Detect::Cookie, Detect::Header],
            session_key: "locale".into(),
            cookie_name: "locale".into(),
            parents: Default::default(),
        }
        .parent(locale("pt-PT"), locale("pt-BR"));
        assert_eq!(cfg.parents.get(&locale("pt-PT")), Some(&locale("pt-BR")));
    }
}
