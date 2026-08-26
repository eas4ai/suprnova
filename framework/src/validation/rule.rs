//! Rule objects - composable validators that work alongside (and
//! independently of) `#[derive(Validate)]`.
//!
//! Four traits cover the design space:
//!
//! - [`Rule`] - pure sync check on a single value. Built-ins:
//!   [`rules::Required`], [`rules::Email`], [`rules::Min`],
//!   [`rules::Max`], [`rules::Between`], [`rules::In`],
//!   [`rules::NotIn`], [`rules::InArray`], [`rules::Integer`],
//!   [`rules::Numeric`], [`rules::Boolean`], [`rules::Alpha`],
//!   [`rules::AlphaNum`], [`rules::AlphaDash`], [`rules::Url`],
//!   [`rules::UrlProtocols`], [`rules::HttpUrl`], [`rules::Uuid`],
//!   [`rules::Password`] (strength checks only - see [`AsyncRule`] below
//!   for its `uncompromised()` half).
//! - [`ValueRule`] - pure sync check on a JSON-shaped value (array or
//!   object), for rules a bare string can't carry enough structure for.
//!   Built-ins: [`rules::ArrayKeys`], [`rules::Distinct`],
//!   [`rules::Contains`], [`rules::DoesntContain`].
//! - [`ContextualRule`] - sync check that can read sibling fields
//!   (think Laravel `required_if:other,value`). Built-ins:
//!   [`rules::RequiredIf`], [`rules::RequiredWith`],
//!   [`rules::RequiredUnless`], [`rules::Same`],
//!   [`rules::Different`], [`rules::Confirmed`], [`rules::Gt`],
//!   [`rules::Gte`], [`rules::Lt`], [`rules::Lte`].
//! - [`AsyncRule`] - async check (DB queries - [`async_rules::Unique`]
//!   lives here; HTTP - [`rules::Password`]'s `uncompromised()` speaks
//!   the Have I Been Pwned k-anonymity API here).
//!
//! # Coherence
//!
//! No blanket `impl<R: Rule> ContextualRule for R` is provided. Each
//! built-in rule implements exactly **one** of `Rule` or
//! `ContextualRule` (and `Unique` implements `AsyncRule` only). Adding
//! a blanket would conflict with the explicit `ContextualRule` impls
//! on the conditional rules. Consumers writing their own rules should
//! pick a trait and stick to it.
//!
//! [`rules::Password`] is the one deliberate exception: it implements
//! both `Rule` and `AsyncRule` (never `ContextualRule`, so no coherence
//! conflict), because its strength checks are sync but its
//! `uncompromised()` check is HTTP. See its doc comment for why.
//! `ValueRule` is dispatched through a separate bridging trait
//! (`RuleCheck`, below `pub mod rules`) whose two blanket impls target
//! different type parameters (`RuleCheck<str>` vs
//! `RuleCheck<serde_json::Value>`), so they can never overlap.

use crate::error::ValidationErrors;
use crate::validation::message::ValidationMessage;
use std::collections::HashMap;

/// A synchronous validator over a single string value.
///
/// `Err(msg)` carries a [`ValidationMessage`]: a catalog key
/// (`validation-min`), the arguments that message interpolates
/// (`min: 8`), and a pre-rendered English fallback.
///
/// # The keyed-message contract
///
/// Rules never translate. They describe the failure; translation
/// happens once, at the serialization boundary
/// ([`ValidationErrors::to_json`](crate::ValidationErrors::to_json)),
/// against the locale in effect for the current request. That keeps
/// rules pure, keeps a rule usable outside a request (console, queue
/// worker), and lets the `localization` feature be compiled out
/// without touching a single rule.
///
/// Every built-in rule returns a keyed message whose id is
/// `validation-<rule name in kebab-case>` - [`rules::RequiredWithAll`]
/// emits `validation-required-with-all`. Those ids ship in the
/// framework's embedded English catalog and an app overrides any of
/// them by defining the same id in its own `lang/<locale>/*.ftl`.
///
/// Custom rules may do either:
///
/// ```rust
/// # use suprnova::{Rule, ValidationMessage};
/// struct Even;
/// impl Rule for Even {
///     fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
///         match value.parse::<i64>() {
///             // Keyless: the text is used verbatim, never translated.
///             Err(_) => Err("must be a number".into()),
///             Ok(n) if n % 2 != 0 => Err(ValidationMessage::keyed("validation-even")
///                 .arg("value", n)
///                 .fallback("must be even")),
///             Ok(_) => Ok(()),
///         }
///     }
/// }
/// ```
///
/// A keyless message (`"...".into()`) renders its text as-is in every
/// locale - the right choice for one-off, app-specific checks. A keyed
/// message needs its id defined in a catalog; when it isn't, rendering
/// falls back to the message's own English text, so a missing
/// translation degrades instead of breaking.
pub trait Rule {
    /// Check `value`. Return `Ok(())` if it passes, `Err(message)` if
    /// it fails.
    // `ValidationMessage` is 144 bytes - key, args map, fallback text,
    // and context prefix - which trips clippy's 128-byte heuristic for
    // returned errors. Boxing it would buy a heap allocation on every
    // failed check and an unwrap at every call site, to save stack bytes
    // on a path that runs once per invalid field. The struct stays
    // by-value deliberately.
    #[allow(clippy::result_large_err)]
    fn passes(&self, value: &str) -> Result<(), ValidationMessage>;

    /// Run the rule and push any failure message onto `errs` under
    /// the given field key. Returns `()` so calls can be chained
    /// without an `if let` per rule. Used by the [`validate!`] macro
    /// and convenient for hand-written `after_validation` bodies that
    /// accumulate errors across many checks.
    ///
    /// [`validate!`]: crate::validate
    fn check(&self, value: &str, errs: &mut ValidationErrors, field: &str) {
        if let Err(msg) = self.passes(value) {
            errs.add(field.to_string(), msg);
        }
    }
}

/// A synchronous validator over a JSON-shaped value - [`Rule`]'s sibling
/// for checks that need more structure than a string carries (allowed
/// keys on an object, duplicates in an array). Same keyed-message
/// contract as [`Rule`]; translation happens once, at
/// [`ValidationErrors::to_json`](crate::ValidationErrors::to_json).
///
/// The field a `ValueRule` runs against must hold `serde_json::Value`
/// (or `Option<serde_json::Value>` on a `?:`/`?=>` row) - [`Rule`] and
/// [`ContextualRule`] only ever see `&str`. A [`validate!`] row
/// dispatches to `Rule` or `ValueRule` automatically, by whichever
/// trait the rule's type implements.
///
/// Built-ins: [`rules::ArrayKeys`], [`rules::Distinct`],
/// [`rules::Contains`], [`rules::DoesntContain`].
///
/// [`validate!`]: crate::validate
pub trait ValueRule {
    /// Check `value`. `Ok(())` if it passes, `Err(message)` if it fails.
    #[allow(clippy::result_large_err)]
    fn passes(&self, value: &serde_json::Value) -> Result<(), ValidationMessage>;

    /// [`ValueRule`] analogue of [`Rule::check`].
    fn check(&self, value: &serde_json::Value, errs: &mut ValidationErrors, field: &str) {
        if let Err(msg) = self.passes(value) {
            errs.add(field.to_string(), msg);
        }
    }
}

/// Map of "field name → its current string value", supplied to rules
/// that need to read sibling fields during validation.
pub type FormContext = HashMap<String, String>;

/// A synchronous validator that needs visibility into other form
/// fields.
///
/// This is the trait Laravel's `required_if` / `required_with` /
/// `required_unless` style rules implement. The runner is expected to
/// build a [`FormContext`] keyed by field name and pass it in alongside
/// the value under test.
pub trait ContextualRule {
    /// Check `value` against `ctx`. Return `Ok(())` if it passes,
    /// `Err(message)` if it fails.
    ///
    /// Rules whose semantics depend on the name of the field being
    /// validated (for example, [`rules::Confirmed`], which looks up
    /// `<field>_confirmation` in `ctx`) cannot implement a meaningful
    /// `passes` because the name isn't available here. Such rules
    /// override [`Self::check_named`] instead and use `passes` only
    /// as a stub explaining the limitation.
    ///
    /// The returned [`ValidationMessage`] follows the same keyed
    /// contract as [`Rule::passes`].
    // By value for the same reason as `Rule::passes` - see the note there.
    #[allow(clippy::result_large_err)]
    fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage>;

    /// Run the rule and push any failure message onto `errs` under
    /// the given field key. The error-accumulating analogue of
    /// [`Self::passes`].
    ///
    /// Most rules don't need the field name - [`Self::check_named`]'s
    /// default impl calls into this method, ignoring `field`. Override
    /// `check_named` directly when the rule needs the name (see
    /// [`rules::Confirmed`]).
    ///
    /// [`validate!`]: crate::validate
    fn check(&self, value: &str, errs: &mut ValidationErrors, field: &str, ctx: &FormContext) {
        if let Err(msg) = self.passes(value, ctx) {
            errs.add(field.to_string(), msg);
        }
    }

    /// Like [`Self::check`], but the rule may use `field` (e.g.
    /// [`rules::Confirmed`] derives `<field>_confirmation` to look up
    /// in `ctx`). The [`validate!`] macro always dispatches through
    /// this method, threading the field ident via
    /// `stringify!($field)`. The default impl ignores `field` and
    /// forwards to [`Self::check`], so rules that don't care about the
    /// field name need not override it.
    ///
    /// [`validate!`]: crate::validate
    fn check_named(
        &self,
        value: &str,
        errs: &mut ValidationErrors,
        field: &str,
        ctx: &FormContext,
    ) {
        self.check(value, errs, field, ctx);
    }
}

/// Built-in synchronous rules - both pure ([`Rule`]) and contextual
/// ([`ContextualRule`]).
pub mod rules {
    use super::{AsyncRule, ContextualRule, FormContext, Rule, ValueRule};
    use crate::config::env::env;
    use crate::error::FrameworkError;
    use crate::http_client::Http;
    use crate::validation::message::ValidationMessage;
    use serde_json::Value;
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;
    use validator::ValidateEmail;

    /// Treat a value as "blank" when it is empty or whitespace-only.
    ///
    /// Matches Laravel's [`Validator::isImplicit`] heuristic: a string
    /// of only spaces is not considered present.
    fn is_blank(value: &str) -> bool {
        value.trim().is_empty()
    }

    /// Laravel `required` - value must be present and non-whitespace.
    pub struct Required;
    impl Rule for Required {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if is_blank(value) {
                Err(ValidationMessage::keyed("validation-required").fallback("required"))
            } else {
                Ok(())
            }
        }
    }

    /// Laravel `email` - defers to [`validator::ValidateEmail`] so
    /// semantics match `#[validate(email)]` on derived types.
    pub struct Email;
    impl Rule for Email {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if value.validate_email() {
                Ok(())
            } else {
                Err(ValidationMessage::keyed("validation-email").fallback("must be a valid email"))
            }
        }
    }

    /// Laravel `min:N` - value must be at least `N` characters long.
    ///
    /// Counts Unicode scalar values (`char`s), not bytes, so multi-byte
    /// characters count as a single character.
    pub struct Min(pub usize);
    impl Rule for Min {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if value.chars().count() >= self.0 {
                Ok(())
            } else {
                Err(ValidationMessage::keyed("validation-min")
                    .arg("min", self.0)
                    .fallback(format!("must be at least {} characters", self.0)))
            }
        }
    }

    /// Laravel `max:N` - value must be at most `N` characters long.
    ///
    /// Counts Unicode scalar values (`char`s), not bytes.
    pub struct Max(pub usize);
    impl Rule for Max {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if value.chars().count() <= self.0 {
                Ok(())
            } else {
                Err(ValidationMessage::keyed("validation-max")
                    .arg("max", self.0)
                    .fallback(format!("must be at most {} characters", self.0)))
            }
        }
    }

    /// Laravel `between:min,max` - value length is `min..=max` inclusive
    /// (counted in Unicode scalar values, not bytes).
    pub struct Between(pub usize, pub usize);
    impl Rule for Between {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            let len = value.chars().count();
            if len < self.0 || len > self.1 {
                Err(ValidationMessage::keyed("validation-between")
                    .arg("min", self.0)
                    .arg("max", self.1)
                    .fallback(format!(
                        "must be between {} and {} characters",
                        self.0, self.1
                    )))
            } else {
                Ok(())
            }
        }
    }

    /// Laravel `in:foo,bar,baz` - value must be one of the allowed
    /// strings (exact match, case-sensitive).
    pub struct In(pub &'static [&'static str]);
    impl Rule for In {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if self.0.contains(&value) {
                Ok(())
            } else {
                Err(ValidationMessage::keyed("validation-in")
                    .arg("values", self.0.join(", "))
                    .fallback(format!("must be one of {:?}", self.0)))
            }
        }
    }

    /// Laravel `not_in:foo,bar,baz` - value must NOT be in the
    /// forbidden list (exact match, case-sensitive).
    pub struct NotIn(pub &'static [&'static str]);
    impl Rule for NotIn {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if self.0.contains(&value) {
                Err(ValidationMessage::keyed("validation-not-in")
                    .arg("values", self.0.join(", "))
                    .fallback(format!("must not be one of {:?}", self.0)))
            } else {
                Ok(())
            }
        }
    }

    /// Laravel `integer` - value parses cleanly as an `i64`.
    pub struct Integer;
    impl Rule for Integer {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            value.parse::<i64>().map(|_| ()).map_err(|_| {
                ValidationMessage::keyed("validation-integer").fallback("must be an integer")
            })
        }
    }

    /// Laravel `numeric` - value parses as a **finite** `f64` (covers
    /// integers, floats, and scientific notation).
    ///
    /// Rust's `f64::from_str` accepts `"NaN"`, `"inf"`, `"-inf"`, and
    /// magnitudes that overflow to infinity; none of those are valid
    /// user-input numbers, so they are rejected here.
    pub struct Numeric;
    impl Rule for Numeric {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            match value.parse::<f64>() {
                Ok(n) if n.is_finite() => Ok(()),
                _ => {
                    Err(ValidationMessage::keyed("validation-numeric").fallback("must be numeric"))
                }
            }
        }
    }

    /// Laravel `boolean` - accepts `"true"`, `"false"`, `"0"`, `"1"`,
    /// `"yes"`, `"no"`, `"on"`, `"off"` (case-insensitive).
    pub struct Boolean;
    impl Rule for Boolean {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            match value.to_ascii_lowercase().as_str() {
                "true" | "false" | "0" | "1" | "yes" | "no" | "on" | "off" => Ok(()),
                _ => {
                    Err(ValidationMessage::keyed("validation-boolean")
                        .fallback("must be a boolean"))
                }
            }
        }
    }

    /// Laravel `alpha` - value must contain only alphabetic
    /// characters and be non-empty.
    ///
    /// **Unicode semantics:** uses [`char::is_alphabetic`] which
    /// accepts non-ASCII letters (`é`, `ñ`, `中`, etc.). This differs
    /// from Laravel 13's default `alpha`, which is ASCII-only - Laravel
    /// only matches Unicode if the `:ascii` suffix is omitted in newer
    /// versions. Suprnova picks the international default; if you need
    /// ASCII-only behaviour, gate with a custom rule.
    pub struct Alpha;
    impl Rule for Alpha {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if !value.is_empty() && value.chars().all(|c| c.is_alphabetic()) {
                Ok(())
            } else {
                Err(ValidationMessage::keyed("validation-alpha")
                    .fallback("must contain only letters"))
            }
        }
    }

    /// Laravel `alpha_num` - value is letters or digits only; must be
    /// non-empty. Uses Unicode-aware [`char::is_alphanumeric`]. For a
    /// rule that also permits `_` and `-`, use [`AlphaDash`].
    pub struct AlphaNum;
    impl Rule for AlphaNum {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if !value.is_empty() && value.chars().all(|c| c.is_alphanumeric()) {
                Ok(())
            } else {
                Err(ValidationMessage::keyed("validation-alpha-num")
                    .fallback("must be alphanumeric (letters and digits only)"))
            }
        }
    }

    /// Laravel `alpha_dash` - value is letters, digits, underscores,
    /// or hyphens; must be non-empty. Uses Unicode-aware
    /// [`char::is_alphanumeric`]. For letters and digits only, use
    /// [`AlphaNum`].
    pub struct AlphaDash;
    impl Rule for AlphaDash {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if !value.is_empty()
                && value
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                Ok(())
            } else {
                Err(ValidationMessage::keyed("validation-alpha-dash")
                    .fallback("must contain only letters, digits, dashes, and underscores"))
            }
        }
    }

    /// Laravel's default URL scheme allowlist, ported verbatim from
    /// `Illuminate\Support\Str::isUrl`'s `$protocolList`
    /// (`reference/framework-13.25.0/src/Illuminate/Support/Str.php:625`),
    /// with the regex escapes (`\+`, `\.`) unescaped back to literals.
    ///
    /// Why an allowlist and not a `javascript:`/`vbscript:` denylist: a
    /// denylist has to be right about every scheme that will ever exist,
    /// and browsers keep inventing them. Laravel's list is the contract
    /// this rule is meant to match, so a re-port against a future Laravel
    /// is a literal diff.
    ///
    /// Kept in Laravel's original (unsorted) order for exactly that
    /// reason, so lookup is a linear scan. 312 short string compares cost
    /// nothing next to the URL parse that precedes them, and the scan
    /// runs only on fields a form actually submitted.
    #[rustfmt::skip]
    pub(crate) static ALLOWED_SCHEMES: &[&str] = &[
        "aaa", "aaas", "about", "acap", "acct", "acd", "acr", "adiumxtra", "adt", "afp", "afs",
        "aim", "amss", "android", "appdata", "apt", "ark", "attachment", "aw", "barion", "beshare",
        "bitcoin", "bitcoincash", "blob", "bolo", "browserext", "calculator", "callto", "cap",
        "cast", "casts", "chrome", "chrome-extension", "cid", "coap", "coap+tcp", "coap+ws",
        "coaps", "coaps+tcp", "coaps+ws", "com-eventbrite-attendee", "content", "conti", "crid",
        "cvs", "dab", "data", "dav", "diaspora", "dict", "did", "dis", "dlna-playcontainer",
        "dlna-playsingle", "dns", "dntp", "dpp", "drm", "drop", "dtn", "dvb", "ed2k", "elsi",
        "example", "facetime", "fax", "feed", "feedready", "file", "filesystem", "finger",
        "first-run-pen-experience", "fish", "fm", "ftp", "fuchsia-pkg", "geo", "gg", "git",
        "gizmoproject", "go", "gopher", "graph", "gtalk", "h323", "ham", "hcap", "hcp", "http",
        "https", "hxxp", "hxxps", "hydrazone", "iax", "icap", "icon", "im", "imap", "info",
        "iotdisco", "ipn", "ipp", "ipps", "irc", "irc6", "ircs", "iris", "iris.beep", "iris.lwz",
        "iris.xpc", "iris.xpcs", "isostore", "itms", "jabber", "jar", "jms", "keyparc", "lastfm",
        "ldap", "ldaps", "leaptofrogans", "lorawan", "lvlt", "magnet", "mailserver", "mailto",
        "maps", "market", "message", "mid", "mms", "modem", "mongodb", "moz", "ms-access",
        "ms-browser-extension", "ms-calculator", "ms-drive-to", "ms-enrollment", "ms-excel",
        "ms-eyecontrolspeech", "ms-gamebarservices", "ms-gamingoverlay", "ms-getoffice", "ms-help",
        "ms-infopath", "ms-inputapp", "ms-lockscreencomponent-config", "ms-media-stream-id",
        "ms-mixedrealitycapture", "ms-mobileplans", "ms-officeapp", "ms-people", "ms-project",
        "ms-powerpoint", "ms-publisher", "ms-restoretabcompanion", "ms-screenclip",
        "ms-screensketch", "ms-search", "ms-search-repair", "ms-secondary-screen-controller",
        "ms-secondary-screen-setup", "ms-settings", "ms-settings-airplanemode",
        "ms-settings-bluetooth", "ms-settings-camera", "ms-settings-cellular",
        "ms-settings-cloudstorage", "ms-settings-connectabledevices",
        "ms-settings-displays-topology", "ms-settings-emailandaccounts", "ms-settings-language",
        "ms-settings-location", "ms-settings-lock", "ms-settings-nfctransactions",
        "ms-settings-notifications", "ms-settings-power", "ms-settings-privacy",
        "ms-settings-proximity", "ms-settings-screenrotation", "ms-settings-wifi",
        "ms-settings-workplace", "ms-spd", "ms-sttoverlay", "ms-transit-to", "ms-useractivityset",
        "ms-virtualtouchpad", "ms-visio", "ms-walk-to", "ms-whiteboard", "ms-whiteboard-cmd",
        "ms-word", "msnim", "msrp", "msrps", "mss", "mtqp", "mumble", "mupdate", "mvn", "news",
        "nfs", "ni", "nih", "nntp", "notes", "ocf", "oid", "onenote", "onenote-cmd",
        "opaquelocktoken", "openpgp4fpr", "pack", "palm", "paparazzi", "payto", "pkcs11",
        "platform", "pop", "pres", "prospero", "proxy", "pwid", "psyc", "pttp", "qb", "query",
        "redis", "rediss", "reload", "res", "resource", "rmi", "rsync", "rtmfp", "rtmp", "rtsp",
        "rtsps", "rtspu", "s3", "secondlife", "service", "session", "sftp", "sgn", "shttp",
        "sieve", "simpleledger", "sip", "sips", "skype", "smb", "sms", "smtp", "snews", "snmp",
        "soap.beep", "soap.beeps", "soldat", "spiffe", "spotify", "ssh", "steam", "stun", "stuns",
        "submit", "svn", "tag", "teamspeak", "tel", "teliaeid", "telnet", "tftp", "tg", "things",
        "thismessage", "tip", "tn3270", "tool", "ts3server", "turn", "turns", "tv", "udp",
        "unreal", "urn", "ut2004", "v-event", "vemmi", "ventrilo", "videotex", "vnc",
        "view-source", "wais", "webcal", "wpid", "ws", "wss", "wtai", "wyciwyg", "xcon",
        "xcon-userid", "xfire", "xmlrpc.beep", "xmlrpc.beeps", "xmpp", "xri", "ymsgr", "z39.50",
        "z39.50r", "z39.50s",
    ];

    /// The two schemes [`HttpUrl`] accepts. Named so the sugar impl and
    /// its doc comment can't drift apart.
    pub(crate) static HTTP_SCHEMES: &[&str] = &["http", "https"];

    /// Whether `value` has the literal bytes `<scheme>://` immediately
    /// followed by a non-empty host token that doesn't itself start with
    /// `/`. This is Laravel's `Str::isUrl` host requirement
    /// (`reference/framework-13.25.0/src/Illuminate/Support/Str.php:633`):
    /// the pattern's host group, opened at `Str.php:636`, has no `?` - an
    /// absent or empty host never matches, no matter how valid the
    /// scheme.
    ///
    /// This has to run against the **raw** input, not `url::Url`'s parsed
    /// authority, because the WHATWG URL parser `url::Url::parse` uses is
    /// forgiving of extra slashes for "special" schemes (`http`, `https`,
    /// `ftp`, `file`, `ws`, `wss`) in a way Laravel's PCRE match is not.
    /// `http:///foo` folds its third `/` straight into the host -
    /// `Url::parse("http:///foo").host_str()` comes back `Some("foo")`, a
    /// non-empty host - even though Laravel's regex fails to match at
    /// that position (the character right after `://` has to be a host
    /// character, and `/` isn't one). A non-empty `host_str()` alone is
    /// not the load-bearing check for that reason; the raw bytes are.
    ///
    /// Never panics on adversarial input: every slice goes through
    /// `str::get`, so a byte range that misses a UTF-8 char boundary (or
    /// runs past the end of `value`) reads as "no host" rather than
    /// indexing out of bounds.
    fn scheme_is_followed_by_a_host(value: &str, scheme: &str) -> bool {
        let scheme_len = scheme.len();
        let (Some(scheme_part), Some(colon_slashes), Some(rest)) = (
            value.get(..scheme_len),
            value.get(scheme_len..scheme_len + 3),
            value.get(scheme_len + 3..),
        ) else {
            return false;
        };
        scheme_part.eq_ignore_ascii_case(scheme)
            && colon_slashes == "://"
            && !rest.is_empty()
            && !rest.starts_with('/')
    }

    /// Laravel `url` - the value matches Laravel's `^(PROTOCOLS)://HOST`
    /// pattern (`Illuminate\Support\Str::isUrl`,
    /// `reference/framework-13.25.0/src/Illuminate/Support/Str.php:633`):
    /// its scheme must be on Laravel's allowlist (`ALLOWED_SCHEMES`), be
    /// followed by `://`, and that in turn must be followed by a
    /// non-empty host token - Laravel's host group (`Str.php:636`,
    /// `localhost | hostname | IPv4 | IPv6`) has no `?`, so an absent or
    /// empty host never matches even with a listed scheme.
    ///
    /// The allowlist is the security half of the rule. `url::Url::parse`
    /// accepts any syntactically valid scheme, `javascript:` included, so
    /// a plain "does it parse" check hands a stored-XSS sink to every
    /// field that later renders as an `href`. Laravel's `url` has always
    /// rejected `javascript:` for that reason; this matches it.
    ///
    /// The `://`-plus-host requirement is Laravel's, not an extra
    /// restriction added here: it's why `mailto:`, `data:`, and `tel:`
    /// are rejected even though those schemes are on the allowlist (no
    /// authority component at all), and why `file:///etc/passwd` is
    /// rejected too (an authority, but an empty host - nothing sits
    /// between the third and fourth `/`, and nothing isn't a host
    /// token). `file://` needs an actual hostname to pass, which the
    /// everyday `file:///path` form never has.
    ///
    /// For a narrower set, use [`Url::protocols`] (Laravel's
    /// `url:http,https`) or the [`HttpUrl`] shorthand.
    pub struct Url;

    impl Url {
        /// Accept only the listed schemes, mirroring Laravel's
        /// parameterised `url:http,https`. Each accepted value must still
        /// be followed by `://` and a non-empty host, exactly as for bare
        /// [`Url`].
        ///
        /// The list **replaces** `ALLOWED_SCHEMES` rather than
        /// intersecting with it (Laravel's `$protocols` argument does the
        /// same), so an app can accept its own custom scheme - a mobile
        /// deep link, say - without the framework holding an opinion
        /// about it.
        ///
        /// An empty list is not "reject everything": it falls back to
        /// the same default allowlist [`Url`] uses, matching Laravel's
        /// `empty($protocols) ? $default : implode('|', $protocols)`. So
        /// `Url::protocols(&[])` behaves exactly like bare `Url`.
        ///
        /// ```rust
        /// # use suprnova::{Rule, rules::Url};
        /// let rule = Url::protocols(&["https"]);
        /// assert!(rule.passes("https://example.com").is_ok());
        /// assert!(rule.passes("http://example.com").is_err());
        /// ```
        pub const fn protocols(protocols: &'static [&'static str]) -> UrlProtocols {
            UrlProtocols(protocols)
        }
    }

    impl Rule for Url {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            match url::Url::parse(value) {
                // `url::Url::scheme()` is already lowercased by the
                // parser, so a plain `contains` is a case-insensitive
                // compare against the lowercase allowlist.
                // `has_authority()` is a cheap first filter (mailto:/
                // data:/tel: never even reach `//`); the load-bearing
                // check is `scheme_is_followed_by_a_host`, the only one
                // of the three that also catches the `http:///foo`-style
                // empty-host forms `has_authority()` alone lets through.
                Ok(u)
                    if u.has_authority()
                        && ALLOWED_SCHEMES.contains(&u.scheme())
                        && scheme_is_followed_by_a_host(value, u.scheme()) =>
                {
                    Ok(())
                }
                _ => {
                    Err(ValidationMessage::keyed("validation-url").fallback("must be a valid URL"))
                }
            }
        }
    }

    /// Scheme-restricted URL rule, built by [`Url::protocols`]. Mirrors
    /// Laravel's `url:http,https`.
    ///
    /// Shares [`Url`]'s `validation-url` catalog key: Laravel renders one
    /// message for both the bare and the parameterised form, and a rule
    /// that named its own schemes in the message would tell an attacker
    /// which schemes to try.
    ///
    /// Like [`Url`], a value must be one of the listed schemes, followed
    /// by `://`, followed by a non-empty host. An empty list falls back
    /// to Laravel's full default allowlist (same `://`-plus-host
    /// requirement) rather than rejecting every value - see
    /// [`Url::protocols`].
    pub struct UrlProtocols(pub &'static [&'static str]);

    impl Rule for UrlProtocols {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            // An empty list means "no override" in Laravel - fall back to
            // the same default allowlist `Url` uses.
            let protocols: &[&str] = if self.0.is_empty() {
                ALLOWED_SCHEMES
            } else {
                self.0
            };
            match url::Url::parse(value) {
                Ok(u)
                    if u.has_authority()
                        && protocols.iter().any(|p| p.eq_ignore_ascii_case(u.scheme()))
                        && scheme_is_followed_by_a_host(value, u.scheme()) =>
                {
                    Ok(())
                }
                _ => {
                    Err(ValidationMessage::keyed("validation-url").fallback("must be a valid URL"))
                }
            }
        }
    }

    /// Laravel `url:http,https` under a name - the value parses as a URL
    /// **and** its scheme is `http` or `https`.
    ///
    /// Reach for this on callback, webhook, and avatar URLs. It's
    /// [`Url`] with the scheme list narrowed to two entries - nothing
    /// else changes, so `ftp://host/x` and `ssh://host` (real hosts,
    /// wrong scheme) are rejected the same way `http:///x` (right
    /// scheme, no host) is.
    pub struct HttpUrl;

    impl Rule for HttpUrl {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            // Literal sugar for `Url::protocols(&["http", "https"])`,
            // re-keyed to its own message so the wording (and any app's
            // catalog override of it) stays specific to this rule.
            Url::protocols(HTTP_SCHEMES).passes(value).map_err(|_| {
                ValidationMessage::keyed("validation-http-url")
                    .fallback("must be a valid http(s) URL")
            })
        }
    }

    /// Laravel `uuid` - value parses as a UUID in any of the formats
    /// the [`uuid`] crate's `parse_str` accepts (hyphenated, simple,
    /// braced, urn).
    pub struct Uuid;
    impl Rule for Uuid {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            uuid::Uuid::parse_str(value).map(|_| ()).map_err(|_| {
                ValidationMessage::keyed("validation-uuid").fallback("must be a valid UUID")
            })
        }
    }

    /// Statically-compiled Unicode-category patterns behind
    /// [`Password`]'s strength checks. Each is a direct Rust `regex`
    /// translation of the PCRE Laravel's `Password` rule uses
    /// (`reference/framework-13.25.0/src/Illuminate/Validation/Rules/Password.php:336-389`),
    /// kept literal so a re-port against a future Laravel is a plain
    /// diff rather than a re-derivation. Each pattern is compiled once
    /// and cached - `Regex::new` is too costly to redo on every
    /// `passes` call.
    fn password_mixed_case_pattern() -> &'static regex::Regex {
        static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
        PATTERN.get_or_init(|| {
            // Laravel: `/(\p{Ll}+.*\p{Lu})|(\p{Lu}+.*\p{Ll})/u` - a run of
            // lowercase letters followed eventually by an uppercase one,
            // or the reverse. Whichever letter class occurs first in the
            // string, one of the two alternatives matches as soon as both
            // classes are present at all - so in effect: "contains at
            // least one lowercase AND at least one uppercase letter."
            regex::Regex::new(r"(\p{Ll}+.*\p{Lu})|(\p{Lu}+.*\p{Ll})")
                .expect("password mixed-case pattern is a fixed, valid regex")
        })
    }

    fn password_letters_pattern() -> &'static regex::Regex {
        static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
        // Laravel: `/\pL/u` - any Unicode letter.
        PATTERN.get_or_init(|| {
            regex::Regex::new(r"\pL").expect("password letters pattern is a fixed, valid regex")
        })
    }

    fn password_numbers_pattern() -> &'static regex::Regex {
        static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
        // Laravel: `/\pN/u` - any Unicode number.
        PATTERN.get_or_init(|| {
            regex::Regex::new(r"\pN").expect("password numbers pattern is a fixed, valid regex")
        })
    }

    fn password_symbols_pattern() -> &'static regex::Regex {
        static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
        // Laravel: `/\p{Z}|\p{S}|\p{P}/u` - separator, symbol, or
        // punctuation. `\p{Z}` (separator) is what makes a plain space
        // count as a symbol - matching Laravel exactly.
        PATTERN.get_or_init(|| {
            regex::Regex::new(r"\p{Z}|\p{S}|\p{P}")
                .expect("password symbols pattern is a fixed, valid regex")
        })
    }

    /// Process-wide default configured by [`Password::defaults_with`].
    static DEFAULT_PASSWORD_RULE: OnceLock<fn() -> Password> = OnceLock::new();

    /// Laravel `Password::min(N)` rule object - password strength
    /// checks (length, letter mix, digits, symbols), plus an optional
    /// Have I Been Pwned [`uncompromised`](Self::uncompromised) check.
    ///
    /// Ported from `Illuminate\Validation\Rules\Password`
    /// (`reference/framework-13.25.0/src/Illuminate/Validation/Rules/Password.php`).
    /// Build with [`Password::min`] and chain the strength builders:
    ///
    /// ```rust
    /// use suprnova::{Password, Rule};
    ///
    /// let rule = Password::min(8).letters().mixed_case().numbers().symbols();
    /// assert!(Rule::passes(&rule, "Str0ng! Pass").is_ok());
    /// assert!(Rule::passes(&rule, "weak").is_err());
    /// ```
    ///
    /// # Two trait impls, one struct - the deliberate exception
    ///
    /// Every other built-in rule implements exactly one of `Rule` /
    /// `ContextualRule` / `AsyncRule` (see the module-level "Coherence"
    /// note). `Password` needs both: the strength checks are pure and
    /// synchronous, but [`Self::uncompromised`] needs an HTTP round trip,
    /// which [`crate::validate!`] - sync-only by design - cannot run. So
    /// `Password` implements [`Rule`] for the strength-only sync path
    /// (usable in `validate!` rows) and [`AsyncRule`] for the full check
    /// (strength, then HIBP), wired through `after_validation_async`
    /// like [`crate::validation::rule::async_rules::Unique`]. Both impls
    /// call the same private `strength_check`, so they can never
    /// disagree about what "strength" means.
    ///
    /// Calling [`Rule::passes`] on a `Password` with `uncompromised` set
    /// is a **loud error**, not a silent skip - see
    /// [`Self::uncompromised`] for why that matters.
    pub struct Password {
        min: usize,
        max: Option<usize>,
        letters: bool,
        mixed_case: bool,
        numbers: bool,
        symbols: bool,
        uncompromised: bool,
        uncompromised_threshold: u32,
        verifier: Option<Arc<dyn UncompromisedVerifier>>,
    }

    impl Password {
        /// Start a `Password` rule requiring at least `min` characters.
        /// Laravel floors this at `1` (`Password.php:243`): `min(0)`
        /// behaves exactly like `min(1)` rather than accepting an empty
        /// password.
        pub fn min(min: usize) -> Self {
            Self {
                min: min.max(1),
                max: None,
                letters: false,
                mixed_case: false,
                numbers: false,
                symbols: false,
                uncompromised: false,
                uncompromised_threshold: 0,
                verifier: None,
            }
        }

        /// Cap the password length at `max` characters.
        pub fn max(mut self, max: usize) -> Self {
            self.max = Some(max);
            self
        }

        /// Require at least one Unicode letter (Laravel: `/\pL/u`).
        pub fn letters(mut self) -> Self {
            self.letters = true;
            self
        }

        /// Require both an uppercase and a lowercase letter, in either
        /// order (Laravel: `/(\p{Ll}+.*\p{Lu})|(\p{Lu}+.*\p{Ll})/u`).
        pub fn mixed_case(mut self) -> Self {
            self.mixed_case = true;
            self
        }

        /// Require at least one Unicode digit (Laravel: `/\pN/u`).
        pub fn numbers(mut self) -> Self {
            self.numbers = true;
            self
        }

        /// Require at least one separator, symbol, or punctuation
        /// character (Laravel: `/\p{Z}|\p{S}|\p{P}/u`). `\p{Z}` is why a
        /// plain space satisfies this rule - Laravel treats whitespace
        /// as a symbol, and so does this port.
        pub fn symbols(mut self) -> Self {
            self.symbols = true;
            self
        }

        /// Require the password to not appear in the Have I Been Pwned
        /// breach corpus, checked via [`HibpVerifier`] (or a
        /// [`Self::verifier`] override) with threshold `0` - any
        /// appearance at all fails. Use
        /// [`Self::uncompromised_with_threshold`] to tolerate a small
        /// number of low-signal appearances instead.
        ///
        /// # This check is HTTP, and HTTP is async
        ///
        /// Setting this flag means the rule needs [`AsyncRule`], not
        /// [`Rule`]. If this `Password` only ever runs through
        /// [`Rule::passes`] (a `validate!` row, or a hand-written sync
        /// call), the check is **not silently skipped** - silently
        /// skipping a compromised-password check is the one unacceptable
        /// outcome here. `Rule::passes` instead returns a keyless,
        /// developer-facing error explaining that the rule must run via
        /// [`AsyncRule::passes`] (or [`AsyncRule::check_async`]) from
        /// `after_validation_async` instead.
        pub fn uncompromised(mut self) -> Self {
            self.uncompromised = true;
            self
        }

        /// Like [`Self::uncompromised`], but the password is only
        /// flagged once it has been seen more than `threshold` times in
        /// the breach corpus (Laravel: `uncompromised($threshold = 0)`).
        /// The comparison is strict - `count > threshold` - so
        /// `uncompromised_with_threshold(0)` behaves exactly like
        /// [`Self::uncompromised`]: any appearance at all fails.
        pub fn uncompromised_with_threshold(mut self, threshold: u32) -> Self {
            self.uncompromised = true;
            self.uncompromised_threshold = threshold;
            self
        }

        /// Override the verifier [`Self::uncompromised`] uses instead of
        /// the default [`HibpVerifier`] - for tests (a verifier that
        /// never touches the network) or to swap in a self-hosted breach
        /// corpus.
        pub fn verifier(mut self, verifier: Arc<dyn UncompromisedVerifier>) -> Self {
            self.verifier = Some(verifier);
            self
        }

        /// Configure the process-wide default returned by
        /// [`Self::defaults`] - Laravel's `Password::defaults(fn () =>
        /// ...)` (`Password.php:308-317`). Takes a plain `fn` pointer
        /// rather than a closure, so the stored default stays `Copy` and
        /// `Send + Sync` without boxing. Call it once from
        /// `bootstrap::register()`, at boot.
        ///
        /// Like [`crate::hashing::set_default_driver`], this is
        /// set-once. Unlike that function, this does not return a
        /// `Result`: Laravel's `defaults()` setter is a fire-and-forget
        /// configuration call, not a fallible one, so a repeat call
        /// instead logs `tracing::warn!` and keeps whatever was
        /// configured first.
        pub fn defaults_with(f: fn() -> Password) {
            if DEFAULT_PASSWORD_RULE.set(f).is_err() {
                tracing::warn!(
                    "Password::defaults_with called more than once; keeping the \
                     first configured default and ignoring this call"
                );
            }
        }

        /// The process-wide default `Password` rule: whatever
        /// [`Self::defaults_with`] configured, or `Password::min(8)` if
        /// it was never called.
        pub fn defaults() -> Self {
            match DEFAULT_PASSWORD_RULE.get() {
                Some(f) => f(),
                None => Password::min(8),
            }
        }

        /// The strength checks shared by both trait impls. Evaluation
        /// order matches Laravel's (min, max, mixed, letters, symbols,
        /// numbers); unlike Laravel, which collects every failure,
        /// Suprnova's `Rule` contract returns a single message, so this
        /// returns the FIRST failing check (see "Why Suprnova diverges"
        /// in `manual/validation.md`).
        // Same 128-byte clippy heuristic noted on `Rule::passes`'s doc
        // comment - `ValidationMessage` stays by-value; see that comment.
        #[allow(clippy::result_large_err)]
        fn strength_check(&self, value: &str) -> Result<(), ValidationMessage> {
            let len = value.chars().count();
            if len < self.min {
                return Err(ValidationMessage::keyed("validation-min")
                    .arg("min", self.min)
                    .fallback(format!("must be at least {} characters", self.min)));
            }
            if let Some(max) = self.max
                && len > max
            {
                return Err(ValidationMessage::keyed("validation-max")
                    .arg("max", max)
                    .fallback(format!("must be at most {} characters", max)));
            }
            if self.mixed_case && !password_mixed_case_pattern().is_match(value) {
                return Err(ValidationMessage::keyed("validation-password-mixed")
                    .fallback("must contain at least one uppercase and one lowercase letter"));
            }
            if self.letters && !password_letters_pattern().is_match(value) {
                return Err(ValidationMessage::keyed("validation-password-letters")
                    .fallback("must contain at least one letter"));
            }
            if self.symbols && !password_symbols_pattern().is_match(value) {
                return Err(ValidationMessage::keyed("validation-password-symbols")
                    .fallback("must contain at least one symbol"));
            }
            if self.numbers && !password_numbers_pattern().is_match(value) {
                return Err(ValidationMessage::keyed("validation-password-numbers")
                    .fallback("must contain at least one number"));
            }
            Ok(())
        }
    }

    impl Rule for Password {
        /// Strength checks only. If [`Password::uncompromised`] was
        /// called and every strength check passes, returns a keyless
        /// error explaining that the HIBP check needs [`AsyncRule`]
        /// instead - see the struct docs.
        #[allow(clippy::result_large_err)]
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            self.strength_check(value)?;
            if self.uncompromised {
                return Err(
                    "Password::uncompromised() requires an HTTP round trip and cannot run \
                     through the synchronous Rule::passes path; call it via AsyncRule::passes \
                     (or AsyncRule::check_async) from after_validation_async instead"
                        .into(),
                );
            }
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl AsyncRule for Password {
        /// Strength checks (delegating to the same `strength_check` that
        /// [`Rule::passes`] uses), then - only once every strength check
        /// passes and [`Password::uncompromised`] was called - the HIBP
        /// range check via [`Password::verifier`] or the default
        /// [`HibpVerifier`].
        async fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            self.strength_check(value)?;
            if !self.uncompromised {
                return Ok(());
            }
            let verified = match &self.verifier {
                Some(v) => v.verify(value, self.uncompromised_threshold).await,
                None => {
                    HibpVerifier::default()
                        .verify(value, self.uncompromised_threshold)
                        .await
                }
            };
            match verified {
                Ok(true) => Ok(()),
                Ok(false) => Err(
                    ValidationMessage::keyed("validation-password-uncompromised").fallback(
                        "has appeared in a data leak; please choose a different password",
                    ),
                ),
                // A conforming verifier fails open internally (see
                // `HibpVerifier::verify`) instead of returning `Err` for a
                // network problem, so a propagated `Err` means the verifier
                // implementation itself is broken. That is the operator's
                // problem, so its detail goes to the log and never into the
                // 422 body - a verbatim error would hand infrastructure
                // detail (hosts, ports, upstream failures) to the client and
                // route around the 5xx body sanitisation every other
                // operational fault gets. The user sees a fixed, translatable
                // message saying the check could not run, not that the
                // password is bad.
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "Password::uncompromised(): verifier returned Err; the check did not run"
                    );
                    Err(
                        ValidationMessage::keyed("validation-password-unverifiable").fallback(
                            "could not be checked against known data leaks; please try again",
                        ),
                    )
                }
            }
        }
    }

    /// Pluggable check behind [`Password::uncompromised`]. `Ok(true)`
    /// means the password is clean; `Ok(false)` means it was found
    /// compromised. [`HibpVerifier`] is the default - swap in your own
    /// via [`Password::verifier`] for tests or a self-hosted breach
    /// corpus.
    ///
    /// Mirrors Laravel's
    /// `Illuminate\Contracts\Validation\UncompromisedVerifier`.
    #[async_trait::async_trait]
    pub trait UncompromisedVerifier: Send + Sync {
        /// Check `value` against the breach corpus. `threshold` is the
        /// maximum tolerated appearance count - see
        /// [`Password::uncompromised_with_threshold`].
        ///
        /// # The `Err` contract
        ///
        /// `Err`'s `Display` text is logged at `error` level and never
        /// reaches the client: the user sees the fixed, translatable
        /// `validation-password-unverifiable` message instead (see
        /// `impl AsyncRule for Password`). Keep password material out of
        /// the error regardless - logs are persisted and searchable, the
        /// same discipline [`HibpVerifier`] applies to its own logging.
        ///
        /// `Err` is not a third way to "fail open" - it is treated as a
        /// genuine implementation bug and surfaces to the caller as a
        /// failure. [`HibpVerifier`]'s fail-open behavior (a network
        /// problem reports the password clean) lives entirely inside
        /// its own `verify`, which returns `Ok(true)` for that case and
        /// never `Err`. An implementation that instead returns `Err` on
        /// its own network failure **inverts that policy to fail
        /// closed** for every app that installs it via
        /// [`Password::verifier`] - return `Ok(true)` internally for
        /// anything you want Laravel's `NotPwnedVerifier` semantics for.
        async fn verify(&self, value: &str, threshold: u32) -> Result<bool, FrameworkError>;
    }

    /// Default [`UncompromisedVerifier`] - Have I Been Pwned's
    /// k-anonymity range API, ported from Laravel's
    /// `Illuminate\Validation\NotPwnedVerifier`.
    ///
    /// # k-anonymity: only a 5-character SHA-1 prefix ever leaves this process
    ///
    /// [`UncompromisedVerifier::verify`] never sends `value`, or even its
    /// full hash, over the network. It hashes `value` with SHA-1,
    /// uppercases the hex encoding, and sends only the **first 5
    /// characters** of that 40-character hash to
    /// `GET https://api.pwnedpasswords.com/range/{prefix}`. The API
    /// answers with every breached hash sharing that prefix (as
    /// `SUFFIX:COUNT` lines), and the match against the full hash
    /// happens locally, in this function. That is the whole point of
    /// k-anonymity: the service learns a 5-hex-character bucket shared
    /// by (on average) hundreds of distinct passwords - never the
    /// password, and never even its full hash.
    ///
    /// # Fails open
    ///
    /// A transport error, a request timeout, a non-2xx response, or an
    /// unreadable body all report the password **clean** (`Ok(true)`)
    /// rather than failing the check - matching Laravel's
    /// `NotPwnedVerifier` exactly, and required so a third-party outage
    /// can never block every login or signup in the app. Each of these
    /// cases logs a `tracing::warn!` - never the password, and never the
    /// prefix. That second guarantee needs its own care: `reqwest`'s
    /// transport-error `Display` (and this crate's own fail-on-real-calls
    /// message) both embed the full request URL, which *contains* the
    /// prefix, so the transport-error branches scrub it out of the error
    /// text before logging rather than logging `%e` verbatim.
    ///
    /// The one exception, ported from Laravel `:47`: an **empty**
    /// `value` is reported compromised (`Ok(false)`) without making any
    /// network call at all.
    pub struct HibpVerifier {
        timeout: Duration,
    }

    impl Default for HibpVerifier {
        /// Reads `HIBP_TIMEOUT_SECS` (default `30`, matching Laravel's
        /// `NotPwnedVerifier`) once, at construction.
        fn default() -> Self {
            Self {
                timeout: Duration::from_secs(env("HIBP_TIMEOUT_SECS", 30u64)),
            }
        }
    }

    /// Redact the k-anonymity prefix out of an error's `Display` text
    /// before it reaches a log line.
    ///
    /// Needed because the two errors [`HibpVerifier`]'s
    /// [`UncompromisedVerifier::verify`] impl logs on its fail-open paths
    /// both embed the *full request URL* in their own `Display` output -
    /// `reqwest::Error` always does, and so does this crate's own
    /// `Http::fail_on_real_calls` message (`http_client/mod.rs`'s "no
    /// fake matched outbound request to {url}") - and that URL contains
    /// `prefix`. Logging `%e` verbatim would leak it despite
    /// [`HibpVerifier`]'s own documented guarantee that the prefix never
    /// appears in a log line. Matches both the
    /// exact case `prefix` was built in (always uppercase, from
    /// `hex::encode_upper`) and a lowercase fallback, since nothing
    /// guarantees every future error's `Display` preserves that case.
    fn scrub_hibp_prefix(text: &str, prefix: &str) -> String {
        text.replace(prefix, "<prefix>")
            .replace(&prefix.to_ascii_lowercase(), "<prefix>")
    }

    #[async_trait::async_trait]
    impl UncompromisedVerifier for HibpVerifier {
        async fn verify(&self, value: &str, threshold: u32) -> Result<bool, FrameworkError> {
            // Laravel `NotPwnedVerifier::verify` (`:47`): empty is always
            // compromised, and never even reaches the network.
            if value.is_empty() {
                return Ok(false);
            }

            use digest::Digest;
            let mut hasher = sha1::Sha1::new();
            hasher.update(value.as_bytes());
            let full_hash = hex::encode_upper(hasher.finalize());
            // Safe: `full_hash` is always 40 ASCII hex characters.
            let prefix = &full_hash[..5];

            let response = match Http::get(format!("https://api.pwnedpasswords.com/range/{prefix}"))
                .header("Add-Padding", "true")
                .timeout(self.timeout)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        error = %scrub_hibp_prefix(&e.to_string(), prefix),
                        "HIBP range request failed; failing open (treating the password as uncompromised)"
                    );
                    return Ok(true);
                }
            };

            if !(200..300).contains(&response.status()) {
                tracing::warn!(
                    status = response.status(),
                    "HIBP range request returned a non-2xx status; failing open"
                );
                return Ok(true);
            }

            let body = match response.text().await {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(
                        error = %scrub_hibp_prefix(&e.to_string(), prefix),
                        "HIBP range response body could not be read as text; failing open"
                    );
                    return Ok(true);
                }
            };

            for raw_line in body.split('\n') {
                // HIBP serves CRLF; splitting on '\n' alone leaves a
                // trailing '\r' on every line.
                let line = raw_line.trim_end_matches('\r');
                let Some((suffix, count)) = line.split_once(':') else {
                    continue;
                };
                if format!("{prefix}{suffix}") != full_hash {
                    continue;
                }
                return match count.trim().parse::<u64>() {
                    // `Ok(bool)` here is "is this password clean?", the
                    // inverse of "count exceeds the threshold" - matched
                    // and over threshold means compromised, so `Ok(false)`.
                    Ok(count) => Ok(count <= u64::from(threshold)),
                    Err(_) => {
                        tracing::warn!(
                            "HIBP range response had an unparseable count for a matched \
                             suffix; treating the password as compromised"
                        );
                        Ok(false)
                    }
                };
            }
            Ok(true)
        }
    }

    /// Laravel `required_if:other,value` - the field is required only
    /// when sibling field `other` is exactly equal to `value`.
    ///
    /// When `other` matches: empty/whitespace value fails.
    /// When `other` does not match (or is missing): always passes.
    pub struct RequiredIf {
        /// Name of the sibling field whose value determines the requirement.
        pub other: &'static str,
        /// When `ctx[other]` equals this string, the field becomes required.
        pub value: &'static str,
    }
    impl ContextualRule for RequiredIf {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            let other_matches = ctx
                .get(self.other)
                .map(|v| v == self.value)
                .unwrap_or(false);
            if other_matches && is_blank(value) {
                Err(ValidationMessage::keyed("validation-required-if")
                    .arg("other", self.other)
                    .arg("value", self.value)
                    .fallback(format!("required when {} is {}", self.other, self.value)))
            } else {
                Ok(())
            }
        }
    }

    /// Laravel `required_with:foo,bar,baz` - the field is required
    /// when **any** of the listed sibling fields is present and
    /// non-blank.
    ///
    /// The slice may carry a single name (the common case) or many.
    /// Use [`RequiredWithAll`] for "required when ALL siblings present".
    pub struct RequiredWith {
        /// Names of the sibling fields whose presence triggers the
        /// requirement. The rule fires when at least one of them is
        /// present and non-blank.
        pub others: &'static [&'static str],
    }
    impl ContextualRule for RequiredWith {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            let any_present = self
                .others
                .iter()
                .any(|name| ctx.get(*name).map(|v| !is_blank(v)).unwrap_or(false));
            if any_present && is_blank(value) {
                Err(ValidationMessage::keyed("validation-required-with")
                    .arg("others", self.others.join(", "))
                    .fallback(format!(
                        "required when {} is present",
                        self.others.join(", ")
                    )))
            } else {
                Ok(())
            }
        }
    }

    /// Laravel `required_with_all:foo,bar,baz` - the field is required
    /// only when **every** listed sibling is present and non-blank.
    /// The complement of [`RequiredWith`].
    pub struct RequiredWithAll {
        /// Names of the sibling fields; every one of them must be
        /// present and non-blank to trigger the requirement.
        pub others: &'static [&'static str],
    }
    impl ContextualRule for RequiredWithAll {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            let all_present = !self.others.is_empty()
                && self
                    .others
                    .iter()
                    .all(|name| ctx.get(*name).map(|v| !is_blank(v)).unwrap_or(false));
            if all_present && is_blank(value) {
                Err(ValidationMessage::keyed("validation-required-with-all")
                    .arg("others", self.others.join(", "))
                    .fallback(format!(
                        "required when {} are all present",
                        self.others.join(", ")
                    )))
            } else {
                Ok(())
            }
        }
    }

    /// Laravel `required_unless:other,value` - the field is required
    /// unless sibling field `other` is exactly equal to `value`.
    ///
    /// When `other` matches `value`: always passes.
    /// Otherwise: empty/whitespace value fails.
    pub struct RequiredUnless {
        /// Name of the sibling field whose value waives the requirement.
        pub other: &'static str,
        /// When `ctx[other]` equals this string, the field is not required.
        pub value: &'static str,
    }
    impl ContextualRule for RequiredUnless {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            let other_matches = ctx
                .get(self.other)
                .map(|v| v == self.value)
                .unwrap_or(false);
            if !other_matches && is_blank(value) {
                Err(ValidationMessage::keyed("validation-required-unless")
                    .arg("other", self.other)
                    .arg("value", self.value)
                    .fallback(format!("required unless {} is {}", self.other, self.value)))
            } else {
                Ok(())
            }
        }
    }

    /// Laravel `same:other_field` - value must equal `ctx[other]`. Used
    /// for password-confirmation style flows where the two fields don't
    /// share the `<field>_confirmation` suffix convention.
    ///
    /// Missing `other` field is treated as a failure.
    pub struct Same {
        /// Name of the sibling field whose value the input must equal.
        pub other: &'static str,
    }
    impl ContextualRule for Same {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            match ctx.get(self.other) {
                Some(v) if v == value => Ok(()),
                _ => Err(ValidationMessage::keyed("validation-same")
                    .arg("other", self.other)
                    .fallback(format!("must match {}", self.other))),
            }
        }
    }

    /// Laravel `different:other_field` - value must differ from
    /// `ctx[other]`. If `other` is missing, the rule passes (there is
    /// nothing to be the same as).
    pub struct Different {
        /// Name of the sibling field whose value the input must differ from.
        pub other: &'static str,
    }
    impl ContextualRule for Different {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            match ctx.get(self.other) {
                Some(v) if v == value => Err(ValidationMessage::keyed("validation-different")
                    .arg("other", self.other)
                    .fallback(format!("must differ from {}", self.other))),
                _ => Ok(()),
            }
        }
    }

    /// Laravel `confirmed` - value must equal `ctx["<field>_confirmation"]`.
    ///
    /// Usage is through the [`crate::validate!`] macro, which threads
    /// the field ident into the rule via `stringify!($field)`:
    ///
    /// ```rust,no_run
    /// use suprnova::{validate, Confirmed};
    /// use std::collections::HashMap;
    /// # struct Form { password: String }
    /// # fn run(form: Form, ctx: HashMap<String, String>) {
    /// let _ = validate! { form =>
    ///     password => Confirmed => with ctx;
    /// };
    /// # }
    /// ```
    ///
    /// When `password` is being validated, `Confirmed` looks up
    /// `password_confirmation` in `ctx` and compares it to the field
    /// value. Missing confirmation field is treated as a failure.
    ///
    /// # Why a unit struct
    ///
    /// `Confirmed` is a unit struct because the field name it needs
    /// for the `<field>_confirmation` lookup is supplied automatically
    /// by the [`crate::validate!`] macro through
    /// [`ContextualRule::check_named`]. Earlier versions exposed the
    /// field as a struct member (`Confirmed { field: "password" }`),
    /// which made the field name appear twice in `validate!` rows. The
    /// unit-struct form is the canonical API.
    ///
    /// # Direct use (without the macro)
    ///
    /// If you call `Confirmed` outside the `validate!` macro, use
    /// [`ContextualRule::check_named`] directly, passing the field
    /// name. Calling [`ContextualRule::passes`] returns an error: the
    /// trait signature does not give the rule access to the field
    /// name, so there is no `<field>_confirmation` key to look up.
    pub struct Confirmed;
    impl ContextualRule for Confirmed {
        /// `Confirmed` needs the name of the field being validated in
        /// order to look up `<field>_confirmation` in `ctx`. The
        /// [`ContextualRule::passes`] signature does not carry that
        /// name, so this method always returns an error explaining
        /// how to invoke the rule correctly.
        ///
        /// The message is deliberately **keyless**: it reports API
        /// misuse to the developer, not a validation failure to the
        /// end user, so it must read the same in every locale.
        fn passes(&self, _value: &str, _ctx: &FormContext) -> Result<(), ValidationMessage> {
            Err(
                "Confirmed requires the field name; use the `validate!` macro or call `check_named` directly"
                    .into(),
            )
        }

        fn check_named(
            &self,
            value: &str,
            errs: &mut crate::error::ValidationErrors,
            field: &str,
            ctx: &FormContext,
        ) {
            let key = format!("{field}_confirmation");
            match ctx.get(&key) {
                Some(v) if v == value => {}
                _ => errs.add(
                    field.to_string(),
                    ValidationMessage::keyed("validation-confirmed")
                        .fallback("confirmation does not match"),
                ),
            }
        }
    }

    /// Laravel `array:keys` (#60918) - the value must be a JSON object
    /// whose keys are all drawn from the allowed list; none of them are
    /// required to be present, this only rejects keys *outside* it.
    ///
    /// A tuple struct like [`In`]/[`NotIn`]: `ArrayKeys(&["name", "email"])`.
    /// An empty allowed list can never usefully constrain an object, so
    /// `passes` reports it as a **keyless** message - a construction
    /// error to fix, not a translatable failure - the pattern
    /// [`Confirmed`] uses for "you called this wrong."
    pub struct ArrayKeys(pub &'static [&'static str]);
    impl ValueRule for ArrayKeys {
        fn passes(&self, value: &Value) -> Result<(), ValidationMessage> {
            if self.0.is_empty() {
                return Err(
                    "ArrayKeys requires at least one allowed key; an empty list can never \
                     usefully constrain an object"
                        .into(),
                );
            }
            let Some(obj) = value.as_object() else {
                return Err(ValidationMessage::keyed("validation-array-keys")
                    .arg("values", self.0.join(", "))
                    .arg("unexpected", String::new())
                    .fallback(format!(
                        "must only contain the following keys: {}",
                        self.0.join(", ")
                    )));
            };
            let unexpected: Vec<&str> = obj
                .keys()
                .map(String::as_str)
                .filter(|k| !self.0.contains(k))
                .collect();
            if unexpected.is_empty() {
                Ok(())
            } else {
                Err(ValidationMessage::keyed("validation-array-keys")
                    .arg("values", self.0.join(", "))
                    .arg("unexpected", unexpected.join(", "))
                    .fallback(format!(
                        "must only contain the following keys: {}",
                        self.0.join(", ")
                    )))
            }
        }
    }

    /// Laravel `distinct` / `distinct:ignore_case` / `distinct:strict` -
    /// the value must be a JSON array with no two elements equal.
    ///
    /// `ignore_case` lowercases `String`-vs-`String` pairs before
    /// comparing. `strict` governs numbers only: `true` requires the
    /// same internal representation (`1` ≠ `1.0`); `false` (the default
    /// meaning - no `Default` impl, name both fields) compares by
    /// numeric value. No flag ever equates two *different-typed*
    /// elements - JSON is already typed, unlike PHP's coercing `==`.
    /// Every comparison matches concrete variants or falls through to
    /// `Value`'s own total `PartialEq`, so it never panics.
    pub struct Distinct {
        /// Fold `String` elements to lowercase before comparing.
        pub ignore_case: bool,
        /// Require matching number representation (`1` ≠ `1.0`) instead
        /// of comparing numbers by value.
        pub strict: bool,
    }
    impl Distinct {
        /// True when `a` and `b` count as the same value under this
        /// rule's flags. Never panics.
        fn values_equal(&self, a: &Value, b: &Value) -> bool {
            match (a, b) {
                (Value::String(sa), Value::String(sb)) => {
                    if self.ignore_case {
                        sa.to_lowercase() == sb.to_lowercase()
                    } else {
                        sa == sb
                    }
                }
                (Value::Number(na), Value::Number(nb)) if !self.strict => {
                    match (na.as_f64(), nb.as_f64()) {
                        (Some(fa), Some(fb)) => fa == fb,
                        _ => na == nb,
                    }
                }
                _ => a == b,
            }
        }
    }
    impl ValueRule for Distinct {
        fn passes(&self, value: &Value) -> Result<(), ValidationMessage> {
            let fail = || {
                ValidationMessage::keyed("validation-distinct").fallback("has a duplicate value")
            };
            let Some(items) = value.as_array() else {
                return Err(fail());
            };
            for i in 0..items.len() {
                for j in (i + 1)..items.len() {
                    if self.values_equal(&items[i], &items[j]) {
                        return Err(fail());
                    }
                }
            }
            Ok(())
        }
    }

    /// Laravel `in_array:other.*` - the value must appear in a list taken
    /// from elsewhere in the form.
    ///
    /// Laravel names the other field in a rule string and the validator
    /// globs it out of the request data at run time. Suprnova has no
    /// rule-string parser - a rule is a value you construct - so you hand
    /// the list over directly and the compiler checks the field exists:
    /// `InArray(&self.allowed_roles)`. `S: AsRef<str>` on the impl so a
    /// `Vec<String>` field and a `&[&str]` literal both work.
    ///
    /// Comparison is exact `str` equality, like [`In`]. Nothing is
    /// coerced, so `"1"` matches only `"1"`.
    ///
    /// An empty haystack is ordinary data - a sibling field can be empty
    /// at run time - so the value fails with the normal keyed message
    /// rather than the construction error [`ArrayKeys`] reports for its
    /// empty allow-list.
    pub struct InArray<'a, S>(pub &'a [S]);
    impl<S: AsRef<str>> Rule for InArray<'_, S> {
        fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            if self.0.iter().any(|allowed| allowed.as_ref() == value) {
                Ok(())
            } else {
                // The haystack is submitted data. Naming its contents here
                // would reflect request input into a response body, which
                // is the same reason `validation-must-match` deliberately
                // drops its `$other` parameter.
                Err(ValidationMessage::keyed("validation-in-array")
                    .fallback("must be one of the allowed values"))
            }
        }
    }

    /// Laravel `contains:foo,bar` - the value must be a JSON array holding
    /// every listed parameter.
    ///
    /// An element matches a parameter only when the element is a JSON
    /// string equal to it: `["1"]` contains `"1"` and `[1]` does not. JSON
    /// is already typed, and PHP's coercing `in_array` is the bug class
    /// #61318/#61319 closed upstream - Suprnova does not reintroduce it
    /// for the sake of matching Laravel's `validateContains`, which is
    /// still loose. A value that is not an array fails, matching Laravel's
    /// own `! is_array($value)` guard.
    ///
    /// An empty parameter list can never usefully constrain an array, so
    /// `passes` reports it as a **keyless** message - a construction error
    /// to fix, not a translatable failure - the pattern [`ArrayKeys`] uses.
    pub struct Contains(pub &'static [&'static str]);
    impl ValueRule for Contains {
        fn passes(&self, value: &Value) -> Result<(), ValidationMessage> {
            if self.0.is_empty() {
                return Err(
                    "Contains requires at least one value; an empty list can never \
                     usefully constrain an array"
                        .into(),
                );
            }
            let fail = || {
                ValidationMessage::keyed("validation-contains")
                    .fallback("is missing a required value")
            };
            let Some(items) = value.as_array() else {
                return Err(fail());
            };
            let held = |wanted: &str| {
                items
                    .iter()
                    .any(|item| matches!(item, Value::String(s) if s == wanted))
            };
            if self.0.iter().all(|wanted| held(wanted)) {
                Ok(())
            } else {
                Err(fail())
            }
        }
    }

    /// Laravel `doesnt_contain:foo,bar` - the value must be a JSON array
    /// holding none of the listed parameters.
    ///
    /// Matching is the same exact string comparison [`Contains`] uses, so
    /// `[1]` does not contain the forbidden value `"1"`. A value that is
    /// not an array fails, matching Laravel's `! is_array($value)` guard -
    /// the rule states "this array holds none of these," and a non-array
    /// cannot make that true.
    ///
    /// An empty parameter list is a keyless construction error, as in
    /// [`Contains`].
    pub struct DoesntContain(pub &'static [&'static str]);
    impl ValueRule for DoesntContain {
        fn passes(&self, value: &Value) -> Result<(), ValidationMessage> {
            if self.0.is_empty() {
                return Err(
                    "DoesntContain requires at least one value; an empty list can never \
                     usefully constrain an array"
                        .into(),
                );
            }
            let fail = || {
                ValidationMessage::keyed("validation-doesnt-contain")
                    .fallback("contains a forbidden value")
            };
            let Some(items) = value.as_array() else {
                return Err(fail());
            };
            let held = |forbidden: &str| {
                items
                    .iter()
                    .any(|item| matches!(item, Value::String(s) if s == forbidden))
            };
            if self.0.iter().any(|forbidden| held(forbidden)) {
                Err(fail())
            } else {
                Ok(())
            }
        }
    }

    /// The right-hand operand of a comparison rule ([`Gt`], [`Gte`],
    /// [`Lt`], [`Lte`]), and the measure applied to both sides.
    ///
    /// Laravel infers the measure from the attribute's *other* rules
    /// (`getSize`, `ValidatesAttributes.php:2817-2835`). A Suprnova rule
    /// has no view of the other rules on its field, and guessing from the
    /// string's shape is the coercion habit the 13.27 strictness fixes
    /// were closing - so the measure is part of the operand. Pairing them
    /// in one enum also makes the meaningless combination unwritable:
    /// there is no literal-plus-length variant, because comparing a value
    /// against the length of the digits you typed is not a check anyone
    /// wants (use [`Min`] or [`Max`] for that), and Laravel refuses it too.
    pub enum CompareWith<'a> {
        /// A literal number. Both sides must be finite: the value has to
        /// parse as a finite `f64`, and a non-finite literal fails the
        /// field rather than comparing against infinity.
        Number(f64),
        /// A sibling field compared numerically. Both the value and the
        /// sibling must parse as finite `f64`.
        NumericField(&'a str),
        /// A sibling field compared by Unicode scalar count - the same
        /// measure [`Min`] and [`Max`] use.
        LengthField(&'a str),
    }

    /// Parse a finite `f64`, rejecting `NaN`, `inf`, and magnitudes that
    /// overflow to infinity - the contract [`Numeric`] already enforces,
    /// kept identical so the two rules agree on what a number is.
    fn finite_number(raw: &str) -> Option<f64> {
        match raw.trim().parse::<f64>() {
            Ok(n) if n.is_finite() => Some(n),
            _ => None,
        }
    }

    /// Measure both sides of a comparison and order them.
    ///
    /// `None` means the comparison cannot be made at all: an unparseable
    /// or non-finite operand, or a sibling field the form never supplied.
    /// Every caller turns `None` into its own failure message, so an
    /// unusable operand fails the field instead of panicking or passing.
    /// Both sides are finite by the time `partial_cmp` runs, so it never
    /// returns `None` there.
    fn compare_sides(
        value: &str,
        with: &CompareWith<'_>,
        ctx: &FormContext,
    ) -> Option<std::cmp::Ordering> {
        match with {
            CompareWith::Number(operand) if operand.is_finite() => {
                finite_number(value)?.partial_cmp(operand)
            }
            CompareWith::Number(_) => None,
            CompareWith::NumericField(other) => {
                let lhs = finite_number(value)?;
                let rhs = finite_number(ctx.get(*other)?)?;
                lhs.partial_cmp(&rhs)
            }
            CompareWith::LengthField(other) => {
                let rhs = ctx.get(*other)?.chars().count();
                Some(value.chars().count().cmp(&rhs))
            }
        }
    }

    /// How an operand is named in a failure message: a literal renders as
    /// its own number, a field operand as the field's **name**.
    ///
    /// Never the sibling's value. A validation message is rendered into a
    /// response body, and the sibling's value is submitted data - the same
    /// reason `validation-must-match` deliberately drops its `$other`
    /// parameter.
    fn operand_label(with: &CompareWith<'_>) -> String {
        match with {
            CompareWith::Number(n) => n.to_string(),
            CompareWith::NumericField(other) | CompareWith::LengthField(other) => {
                (*other).to_string()
            }
        }
    }

    /// Laravel `gt:field_or_value` - the value must be greater than its
    /// operand.
    ///
    /// [`CompareWith`] carries both the operand and the measure. An
    /// operand that cannot be measured - a non-numeric value under a
    /// numeric comparison, a sibling the form never sent, a non-finite
    /// literal - fails the field with this rule's message. Arrays and
    /// files have no comparison here: a rule only ever sees a string, and
    /// upload sizes are capped by the multipart parser instead.
    pub struct Gt<'a>(pub CompareWith<'a>);
    impl ContextualRule for Gt<'_> {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            match compare_sides(value, &self.0, ctx) {
                Some(std::cmp::Ordering::Greater) => Ok(()),
                _ => {
                    let other = operand_label(&self.0);
                    Err(ValidationMessage::keyed("validation-gt")
                        .arg("other", other.clone())
                        .fallback(format!("must be greater than {other}")))
                }
            }
        }
    }

    /// Laravel `gte:field_or_value` - the value must be greater than or
    /// equal to its operand. See [`Gt`] for the operand and failure rules.
    pub struct Gte<'a>(pub CompareWith<'a>);
    impl ContextualRule for Gte<'_> {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            match compare_sides(value, &self.0, ctx) {
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal) => Ok(()),
                _ => {
                    let other = operand_label(&self.0);
                    Err(ValidationMessage::keyed("validation-gte")
                        .arg("other", other.clone())
                        .fallback(format!("must be greater than or equal to {other}")))
                }
            }
        }
    }

    /// Laravel `lt:field_or_value` - the value must be less than its
    /// operand. See [`Gt`] for the operand and failure rules.
    pub struct Lt<'a>(pub CompareWith<'a>);
    impl ContextualRule for Lt<'_> {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            match compare_sides(value, &self.0, ctx) {
                Some(std::cmp::Ordering::Less) => Ok(()),
                _ => {
                    let other = operand_label(&self.0);
                    Err(ValidationMessage::keyed("validation-lt")
                        .arg("other", other.clone())
                        .fallback(format!("must be less than {other}")))
                }
            }
        }
    }

    /// Laravel `lte:field_or_value` - the value must be less than or equal
    /// to its operand. See [`Gt`] for the operand and failure rules.
    pub struct Lte<'a>(pub CompareWith<'a>);
    impl ContextualRule for Lte<'_> {
        fn passes(&self, value: &str, ctx: &FormContext) -> Result<(), ValidationMessage> {
            match compare_sides(value, &self.0, ctx) {
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal) => Ok(()),
                _ => {
                    let other = operand_label(&self.0);
                    Err(ValidationMessage::keyed("validation-lte")
                        .arg("other", other.clone())
                        .fallback(format!("must be less than or equal to {other}")))
                }
            }
        }
    }
}

/// Bridges [`Rule`] and [`ValueRule`] for [`validate!`]'s required- and
/// optional-shape rows: which trait's `check` runs is decided by which
/// trait `$rule`'s type implements - never by macro syntax - so one
/// field list mixes `Min(8)` and `ArrayKeys(&[...])` with no new syntax.
/// `Field` is `str` for [`Rule`], `serde_json::Value` for [`ValueRule`];
/// the two blanket impls target different `Field`s, so they can't
/// conflict even for a hypothetical rule implementing both (none does).
/// Not meant to be called directly - [`validate!`] reaches it through
/// `$crate`-qualified paths that need no trait imports at the call site.
///
/// [`validate!`]: crate::validate
#[doc(hidden)]
pub trait RuleCheck<Field: ?Sized> {
    /// Dispatch to whichever of [`Rule::check`] / [`ValueRule::check`]
    /// applies to `Self`.
    fn __check(&self, value: &Field, errs: &mut ValidationErrors, field: &str);
}

impl<R: Rule> RuleCheck<str> for R {
    fn __check(&self, value: &str, errs: &mut ValidationErrors, field: &str) {
        Rule::check(self, value, errs, field)
    }
}

impl<R: ValueRule> RuleCheck<serde_json::Value> for R {
    fn __check(&self, value: &serde_json::Value, errs: &mut ValidationErrors, field: &str) {
        ValueRule::check(self, value, errs, field)
    }
}

/// An asynchronous validator over a single string value.
///
/// Rules that need to hit a database, an HTTP service, or any other
/// `.await` point go here. [`async_rules::Unique`] is the canonical
/// built-in.
#[async_trait::async_trait]
pub trait AsyncRule: Send + Sync {
    /// Check `value`. Return `Ok(())` if it passes, `Err(message)` if
    /// it fails.
    ///
    /// The returned [`ValidationMessage`] follows the same keyed
    /// contract as [`Rule::passes`]. Infrastructure failures (a dead
    /// connection, a malformed identifier) surface as keyless messages:
    /// they are operator-facing, not user-facing text.
    async fn passes(&self, value: &str) -> Result<(), ValidationMessage>;

    /// Async analogue of [`Rule::check`]: run the rule and push any
    /// failure message onto `errs` under the given field key.
    ///
    /// The [`validate!`] macro does not currently weave async rules
    /// in (placing `.await` inside a declarative macro arm gets
    /// awkward); use this helper to accumulate async rule failures
    /// alongside your sync checks:
    ///
    /// ```rust,no_run
    /// # use suprnova::{Unique, AsyncRule, ValidationErrors};
    /// # struct CreateUserRequest { email: String }
    /// # impl CreateUserRequest {
    /// # async fn ex(&self) -> Result<(), ValidationErrors> {
    /// let mut errs = ValidationErrors::new();
    /// Unique::new("users", "email")
    ///     .check_async(&self.email, &mut errs, "email")
    ///     .await;
    /// errs.into_result()
    /// # }
    /// # }
    /// ```
    ///
    /// [`validate!`]: crate::validate
    async fn check_async(&self, value: &str, errs: &mut ValidationErrors, field: &str) {
        if let Err(msg) = self.passes(value).await {
            errs.add(field.to_string(), msg);
        }
    }
}

/// Built-in asynchronous rules.
pub mod async_rules {
    use super::AsyncRule;
    use crate::database::placeholder::placeholder;
    use crate::database::validate_identifier;
    use crate::validation::message::ValidationMessage;
    use crate::{DB, FrameworkError};
    use sea_orm::{ConnectionTrait, Statement, Value};

    /// Laravel `unique:table,column` - issues a single parameterized
    /// `COUNT(*)` against the configured DB connection and fails when a
    /// matching row exists.
    ///
    /// Construct with [`Unique::new`] and refine with the builder:
    ///
    /// ```rust,no_run
    /// # use suprnova::Unique;
    /// # let current_user_id = 1i64;
    /// # let tenant_id = 7i64;
    /// // `email` must be unique, ignoring the row currently being edited
    /// let _edit = Unique::new("users", "email").ignore(current_user_id);
    ///
    /// // `email` unique *per tenant*, compared case-insensitively
    /// let _scoped = Unique::new("users", "email")
    ///     .where_eq("tenant_id", tenant_id)
    ///     .case_insensitive();
    /// ```
    ///
    /// # This is an advisory check, not a guarantee
    ///
    /// `Unique` reads the table *before* the write, so it carries an
    /// unavoidable time-of-check/time-of-use race: two concurrent
    /// requests can both pass the `COUNT(*)` and then both insert, and
    /// the database ends up with duplicates. Laravel's `unique` rule has
    /// exactly the same property. The **only** real guarantee is a
    /// `UNIQUE` constraint (or unique index) on the column in the
    /// database schema.
    ///
    /// Use `Unique` for a fast, friendly pre-submit message (and so
    /// Precognition can validate the field), and back it with the DB
    /// constraint for correctness. When the constraint fires on the
    /// loser of a race, map that write error back to the same clean 422
    /// with
    /// [`FrameworkError::from_unique_violation`](crate::FrameworkError::from_unique_violation)
    /// instead of leaking a 500.
    ///
    /// # Safety on identifiers
    ///
    /// `table`, `column`, the exclusion key column, and any
    /// [`where_eq`](Self::where_eq) scope column are `&'static str`
    /// slices from source. SQL has no placeholder for identifiers, so
    /// they are interpolated into the query - but every one is first run
    /// through [`crate::database::validate_identifier`],
    /// the same allowlist the model-less query builder uses, so a typo or
    /// hostile literal errors instead of shaping an injection. The value
    /// under test, the excluded id, and scope values are all bound
    /// parameters.
    pub struct Unique {
        table: &'static str,
        column: &'static str,
        except: Option<(&'static str, Value)>,
        wheres: Vec<(&'static str, Value)>,
        case_insensitive: bool,
    }

    impl Unique {
        /// Start a uniqueness rule for `column` in `table`.
        pub fn new(table: &'static str, column: &'static str) -> Self {
            Self {
                table,
                column,
                except: None,
                wheres: Vec::new(),
                case_insensitive: false,
            }
        }

        /// Ignore the row whose `id` equals `id` - the "editing my own
        /// record" case, where a user's own email must not trip the rule
        /// on update. Uses the `id` primary-key column. Accepts anything
        /// that converts into a bound parameter, so integer, UUID, and
        /// string primary keys all work: `ignore(5)`, `ignore(uuid)`,
        /// `ignore("01H…")`.
        pub fn ignore(mut self, id: impl Into<Value>) -> Self {
            self.except = Some(("id", id.into()));
            self
        }

        /// Like [`ignore`](Self::ignore) but excludes on a custom key
        /// column instead of `id` (a non-`id` primary key, or excluding
        /// by another unique key).
        pub fn ignore_with_column(mut self, id_column: &'static str, id: impl Into<Value>) -> Self {
            self.except = Some((id_column, id.into()));
            self
        }

        /// Scope the uniqueness check to rows where `column = value`.
        /// Multiple calls AND together. This is Laravel's
        /// `Rule::unique(...)->where(col, val)` - e.g. an email that must
        /// be unique only *within a tenant*:
        /// `Unique::new("users", "email").where_eq("tenant_id", tenant_id)`.
        pub fn where_eq(mut self, column: &'static str, value: impl Into<Value>) -> Self {
            self.wheres.push((column, value.into()));
            self
        }

        /// Compare case-insensitively (`LOWER(column) = LOWER(?)`). Use
        /// for emails or usernames where `Foo@x.com` and `foo@x.com` must
        /// be treated as the same value.
        pub fn case_insensitive(mut self) -> Self {
            self.case_insensitive = true;
            self
        }
    }

    #[async_trait::async_trait]
    impl AsyncRule for Unique {
        async fn passes(&self, value: &str) -> Result<(), ValidationMessage> {
            // Identifiers can't be placeholder-bound; validate each one
            // through the shared allowlist before interpolation.
            let table = validate_identifier(self.table).map_err(|e| e.to_string())?;
            let column = validate_identifier(self.column).map_err(|e| e.to_string())?;

            let conn = DB::connection().map_err(|e| format!("db: {e}"))?;
            let backend = conn.inner().get_database_backend();

            let mut clauses: Vec<String> = Vec::new();
            let mut values: Vec<Value> = Vec::new();

            // Placeholders are rendered per backend: Postgres rejects `?`
            // outright, so a hard-coded one made this rule - and therefore
            // every `unique` validation, including the one on a sign-up
            // form's email - fail on Postgres. `next` stays in step with
            // `values`, which is what keeps `$1`/`$2`/… aligned with the
            // binds across all three clause groups below.
            let mut next = 1usize;
            let mut bind = |values: &mut Vec<Value>, v: Value| -> Result<String, FrameworkError> {
                values.push(v);
                let rendered = placeholder(backend, next)?;
                next += 1;
                Ok(rendered)
            };

            // Target-column predicate.
            let target =
                bind(&mut values, Value::from(value.to_string())).map_err(|e| e.to_string())?;
            if self.case_insensitive {
                clauses.push(format!("LOWER({column}) = LOWER({target})"));
            } else {
                clauses.push(format!("{column} = {target}"));
            }

            // Exclude the row being edited.
            if let Some((id_column, id)) = &self.except {
                let id_column = validate_identifier(id_column).map_err(|e| e.to_string())?;
                let ph = bind(&mut values, id.clone()).map_err(|e| e.to_string())?;
                clauses.push(format!("{id_column} <> {ph}"));
            }

            // Scoped uniqueness predicates (AND together).
            for (scope_col, scope_val) in &self.wheres {
                let scope_col = validate_identifier(scope_col).map_err(|e| e.to_string())?;
                let ph = bind(&mut values, scope_val.clone()).map_err(|e| e.to_string())?;
                clauses.push(format!("{scope_col} = {ph}"));
            }

            let sql = format!(
                "SELECT COUNT(*) AS c FROM {table} WHERE {}",
                clauses.join(" AND ")
            );

            let stmt = Statement::from_sql_and_values(backend, &sql, values);
            let row = conn
                .inner()
                .query_one_raw(stmt)
                .await
                .map_err(|e| format!("unique query: {e}"))?
                .ok_or_else(|| "unique query returned no rows".to_string())?;

            let count: i64 = row
                .try_get::<i64>("", "c")
                .map_err(|e| format!("unique decode: {e}"))?;

            if count == 0 {
                Ok(())
            } else {
                Err(ValidationMessage::keyed("validation-unique")
                    .arg("column", self.column)
                    .arg("table", self.table)
                    .fallback(format!("{} already exists for {}", self.column, self.table)))
            }
        }
    }
}

pub use async_rules::Unique;

/// Run a chain of validation rules over fields of `$self`, accumulating
/// errors into a single [`ValidationErrors`](crate::ValidationErrors).
/// Returns `Ok(())` if every rule passes, `Err(ValidationErrors)`
/// otherwise.
///
/// # Syntax
///
/// ```rust,no_run
/// use suprnova::{validate, Required, Email, Min, Max, RequiredIf, ValidationErrors};
/// # struct Form {
/// #     email: String,
/// #     password: String,
/// #     bio: Option<String>,
/// #     card_number: String,
/// #     billing_type: String,
/// # }
/// # impl Form {
/// fn after_validation(&self) -> Result<(), ValidationErrors> {
///     // Contextual rules read sibling values from a `FormContext` you
///     // build - a map of field name to its string value.
///     let mut ctx = std::collections::HashMap::new();
///     ctx.insert("billing_type".to_string(), self.billing_type.clone());
///     validate! { self =>
///         email       => Required, Email;
///         password    => Min(8), Max(72);
///         bio?:          Min(10), Max(500);
///         card_number => RequiredIf {
///             other: "billing_type",
///             value: "card",
///         } => with ctx;
///     }
/// }
/// # }
/// ```
///
/// Each row is one of three shapes:
///
/// - `field_ident => Rule1, Rule2, ... ;` - the field is treated as
///   required-shaped: the rule is invoked on `&self.field` directly.
///   This is the shape for `String`, `i64`, or any other concrete
///   type that derefs to `&str` (or implements [`Rule`] / [`ContextualRule`]
///   over the contained scalar).
/// - `field_ident ?: Rule1, Rule2, ... ;` - the field is `Option<T>`.
///   When `Some`, the rules run on the unwrapped inner value; when
///   `None`, every rule on the row is **skipped**. This matches Laravel's
///   "if present, validate" semantics for optional form fields and is
///   the right choice for every `Option<String>` (or `Option<i64>`, …)
///   field on a form. **Note:** because `None` skips, a
///   presence-conditional rule like `RequiredIf` on a `?:` row can never
///   fail an *absent* field - use `?=>` for that.
/// - `field_ident ?=> Rule1, Rule2, ... ;` - also for an `Option<String>`
///   field, but the rules run **even when `None`** (absence is treated as
///   the empty string). This is the row for presence-conditional rules
///   (`RequiredIf` / `RequiredWith` / `RequiredUnless`) that must be able
///   to fail an absent optional field. A present `Some` is evaluated too.
///
/// A rule in any row may be a [`Rule`] (over `&str`) or a [`ValueRule`]
/// (over `&serde_json::Value`) - which one runs is resolved by which
/// trait the rule's type implements, not by anything written in the
/// row, so the two kinds mix freely in one field list. A `ValueRule`
/// row needs the field's Rust type to actually be `serde_json::Value`
/// (or `Option<serde_json::Value>` on `?:`/`?=>`).
///
/// Each rule is either a plain [`Rule`] (no suffix) or a
/// [`ContextualRule`] followed by `=> with $ctx_ident`. The contextual
/// separator is `=> with` (not parenthesised) because `macro_rules!`
/// matches `$rule:expr` greedily - placing the suffix in parentheses
/// runs into Rust's `FOLLOW` set rules for `expr` fragments.
///
/// The macro expands to a fresh [`ValidationErrors`](crate::ValidationErrors),
/// calls [`Rule::check`] / [`ContextualRule::check_named`] for each
/// declared rule, then returns
/// [`ValidationErrors::into_result`](crate::ValidationErrors::into_result).
///
/// # `Option<T>` fields
///
/// Use the `?:` row separator (`bio?: Min(10), Max(500);`). The
/// macro expands to `if let Some(ref __val) = self.bio { ... }`, so
/// rules only run when the field is `Some`. Rules see the inner value
/// borrowed as the type they expect (typically `&str` via
/// `String: Deref<Target=str>` auto-deref). For non-string optional
/// types, the inner type must implement the rule's expected borrow
/// itself.
///
/// # Conditionally-required optional fields
///
/// `?:` is "if present, validate" - it can't *require* an absent field.
/// When an `Option<String>` field must be present under a condition on a
/// sibling field, use the `?=>` row instead:
///
/// ```rust,no_run
/// # use suprnova::{validate, RequiredIf, ValidationErrors};
/// # struct Form { card_number: Option<String>, billing_type: String }
/// # impl Form {
/// # fn ex(&self) -> Result<(), ValidationErrors> {
/// # let mut ctx = std::collections::HashMap::new();
/// # ctx.insert("billing_type".to_string(), self.billing_type.clone());
/// // card_number is required only when billing_type == "card"
/// validate! { self =>
///     card_number ?=> RequiredIf { other: "billing_type", value: "card" } => with ctx;
/// }
/// # }
/// # }
/// ```
///
/// `?=>` evaluates its rules even when the field is `None` (absence is
/// treated as the empty string), so `RequiredIf` can fail. It uses
/// `Option::as_deref`, so the field must be `Option<String>`-shaped.
///
/// # Async rules
///
/// The macro is sync-only. Call
/// [`AsyncRule::check_async`](crate::AsyncRule::check_async) inline
/// for async-backed checks; both styles accumulate into the same
/// [`ValidationErrors`](crate::ValidationErrors).
#[macro_export]
macro_rules! validate {
    ($self:ident => $($tt:tt)*) => {{
        let mut __errs = $crate::ValidationErrors::new();
        $crate::__validate_rows!(__errs, $self, $($tt)*);
        __errs.into_result()
    }};
}

/// Internal row walker used by [`validate!`]. Not part of the public
/// API even though `#[macro_export]` makes it reachable at the crate
/// root - `#[doc(hidden)]` keeps it out of rustdoc.
///
/// The walker consumes one row per recursive invocation. A row is one of
/// `field => rule1, rule2;` (required-shape), `field?: rule1, rule2;`
/// (optional-shape - runs rules only when the field is `Some`), or
/// `field ?=> rule1, rule2;` (conditional-presence - runs rules even when
/// the field is `None`, treating absence as `""`). Recursion terminates
/// when the input is empty (or only a stray `;` remains, supporting the
/// optional trailing semicolon style).
#[macro_export]
#[doc(hidden)]
macro_rules! __validate_rows {
    // Optional-shape row: `field?: Rule1, Rule2 => with ctx, ... ;`
    ($errs:ident, $self:ident, $field:ident ?: $($rule:expr $(=> with $ctx:ident)?),+ ; $($rest:tt)*) => {
        if let ::core::option::Option::Some(ref __val) = $self.$field {
            $(
                $crate::__validate_one_optional!($errs, $field, __val, $rule $(=> with $ctx)?);
            )+
        }
        $crate::__validate_rows!($errs, $self, $($rest)*);
    };
    // Conditional-presence optional row: `field ?=> Rule => with ctx, ... ;`
    //
    // The optional-typed sibling of the required contextual row
    // (`field => Rule => with ctx`): the rules run *even when the field is
    // `None`*, treating absence as the empty string. This is what lets a
    // presence-conditional rule (`RequiredIf` and friends) fail an absent
    // `Option<String>` field - the case `?:` cannot express because it
    // skips entirely on `None`. Uses `as_deref`, so the field must be
    // `Option<String>`-shaped (an `Option<i64>` is a loud compile error).
    ($errs:ident, $self:ident, $field:ident ?=> $($rule:expr $(=> with $ctx:ident)?),+ ; $($rest:tt)*) => {
        {
            let __val: &str = $self.$field.as_deref().unwrap_or("");
            $(
                $crate::__validate_one_optional!($errs, $field, __val, $rule $(=> with $ctx)?);
            )+
        }
        $crate::__validate_rows!($errs, $self, $($rest)*);
    };
    // Required-shape row: `field => Rule1, Rule2 => with ctx, ... ;`
    ($errs:ident, $self:ident, $field:ident => $($rule:expr $(=> with $ctx:ident)?),+ ; $($rest:tt)*) => {
        $(
            $crate::__validate_one!($errs, $self, $field, $rule $(=> with $ctx)?);
        )+
        $crate::__validate_rows!($errs, $self, $($rest)*);
    };
    // Terminal: input exhausted (with or without a trailing `;`).
    ($errs:ident, $self:ident, $(;)?) => {};
}

/// Internal dispatch macro used by [`validate!`] for required-shape
/// rows. Not part of the public API.
#[macro_export]
#[doc(hidden)]
macro_rules! __validate_one {
    ($errs:ident, $self:ident, $field:ident, $rule:expr => with $ctx:ident) => {
        $crate::validation::rule::ContextualRule::check_named(
            &$rule,
            &$self.$field,
            &mut $errs,
            ::core::stringify!($field),
            &$ctx,
        );
    };
    ($errs:ident, $self:ident, $field:ident, $rule:expr) => {
        $crate::validation::rule::RuleCheck::__check(
            &$rule,
            &$self.$field,
            &mut $errs,
            ::core::stringify!($field),
        );
    };
}

/// Internal dispatch macro used by [`validate!`] for optional-shape
/// rows. Runs against the borrowed inner value of an `Option`. Not
/// part of the public API.
#[macro_export]
#[doc(hidden)]
macro_rules! __validate_one_optional {
    ($errs:ident, $field:ident, $val:ident, $rule:expr => with $ctx:ident) => {
        $crate::validation::rule::ContextualRule::check_named(
            &$rule,
            $val,
            &mut $errs,
            ::core::stringify!($field),
            &$ctx,
        );
    };
    ($errs:ident, $field:ident, $val:ident, $rule:expr) => {
        $crate::validation::rule::RuleCheck::__check(
            &$rule,
            $val,
            &mut $errs,
            ::core::stringify!($field),
        );
    };
}
