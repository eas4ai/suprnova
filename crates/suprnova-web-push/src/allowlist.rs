//! The push-service origins the `Allowlist` endpoint policy admits.
//!
//! The `Strict` policy refuses a destination that is not on the public
//! internet, but any public host passes it. A deployment that knows which
//! push services its users' browsers subscribe through can name their
//! origins, and the `Allowlist` policy then refuses every other host too.

use crate::error::InvalidAllowlist;
use url::{Host, Url};

/// The push-service origins that [`crate::EndpointPolicy::Allowlist`]
/// admits.
///
/// An origin is `https://host` or `https://host:port`, with no path, query,
/// fragment or user name. An endpoint matches it when its scheme is
/// `https`, its host is the same name without regard to case, and its port,
/// 443 when the URL names none, is the same. The host may start with the
/// label `*.`, as in `https://*.notify.windows.com`, which Microsoft's push
/// service needs because it spreads subscriptions over many hosts: the `*`
/// stands for exactly one label, so `wns2-par02p.notify.windows.com`
/// matches and `a.b.notify.windows.com` and `notify.windows.com` do not.
/// The name after `*.` must have two labels or more, so a wildcard cannot
/// cover a whole top-level domain.
///
/// ```
/// use suprnova_web_push::EndpointAllowlist;
///
/// let allowlist = EndpointAllowlist::new([
///     "https://updates.push.services.mozilla.com",
///     "https://fcm.googleapis.com",
///     "https://web.push.apple.com",
///     "https://*.notify.windows.com",
/// ])?;
/// assert!(allowlist.contains("https://fcm.googleapis.com/fcm/send/token"));
/// assert!(!allowlist.contains("https://push.example.net/token"));
/// # Ok::<(), suprnova_web_push::InvalidAllowlist>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointAllowlist {
    origins: Vec<AllowedOrigin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AllowedOrigin {
    host: AllowedHost,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AllowedHost {
    /// This name, lowercased and without a trailing dot.
    Exact(String),
    /// One label and a dot in front of this name: `*.notify.windows.com`
    /// holds `notify.windows.com`.
    OneLabelUnder(String),
}

impl EndpointAllowlist {
    /// Build an allowlist from origins such as `https://fcm.googleapis.com`.
    ///
    /// Each origin is parsed as written: surrounding spaces are not
    /// trimmed. An entry that is not `https://host` or `https://host:port`,
    /// with an optional leading `*.` label, is refused, as is an empty
    /// list, which would refuse every endpoint. A host that is an IP
    /// address is refused too: the strict checks the policy runs first
    /// refuse every endpoint that names an address.
    pub fn new<I>(origins: I) -> Result<Self, InvalidAllowlist>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let origins = origins
            .into_iter()
            .map(|origin| AllowedOrigin::parse(origin.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        if origins.is_empty() {
            return Err(InvalidAllowlist(
                "the endpoint allowlist names no origin".into(),
            ));
        }
        Ok(Self { origins })
    }

    /// Whether the origin of `endpoint` is one the list names. An endpoint
    /// that is no URL is not.
    ///
    /// This compares the origin alone. A send under
    /// [`crate::EndpointPolicy::Allowlist`] also runs every check of
    /// [`crate::EndpointPolicy::Strict`].
    pub fn contains(&self, endpoint: &str) -> bool {
        Url::parse(endpoint).is_ok_and(|url| self.admits(&url))
    }

    /// Whether the origin of `url` is one the list names.
    pub(crate) fn admits(&self, url: &Url) -> bool {
        if url.scheme() != "https" {
            return false;
        }
        let Some(Host::Domain(host)) = url.host() else {
            return false;
        };
        let Some(port) = url.port_or_known_default() else {
            return false;
        };
        let host = normalized(host);
        self.origins
            .iter()
            .any(|origin| origin.port == port && origin.host.matches(&host))
    }
}

impl AllowedOrigin {
    fn parse(entry: &str) -> Result<Self, InvalidAllowlist> {
        let refuse = |rule: &str| {
            InvalidAllowlist(format!("{entry:?} is not a push service origin: {rule}"))
        };
        // The URL parser drops tabs and line breaks inside a URL and trims
        // spaces around it, so a value with any of them would not be read
        // as it is written.
        if entry.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(refuse("an origin has no spaces or control characters"));
        }
        let Some(authority) = entry
            .get(..8)
            .filter(|scheme| scheme.eq_ignore_ascii_case("https://"))
            .map(|_| &entry[8..])
        else {
            return Err(refuse("an origin starts with https://"));
        };
        if authority.contains(['/', '\\', '?', '#']) {
            return Err(refuse("an origin has no path, query or fragment"));
        }
        if authority.contains('@') {
            return Err(refuse("an origin has no user name or password"));
        }
        if authority.ends_with(':') {
            return Err(refuse("the port after the colon is missing"));
        }
        let (wildcard, authority) = match authority.strip_prefix("*.") {
            Some(rest) => (true, rest),
            None => (false, authority),
        };
        if authority.contains('*') {
            return Err(refuse(
                "a wildcard is the whole first label, *., and appears once",
            ));
        }
        let url = Url::parse(&format!("https://{authority}"))
            .map_err(|error| refuse(&format!("the host or the port does not parse ({error})")))?;
        let host = match url.host() {
            Some(Host::Domain(host)) => normalized(host),
            Some(Host::Ipv4(_) | Host::Ipv6(_)) => {
                return Err(refuse("the host is a name, not an IP address"));
            }
            None => return Err(refuse("an origin has a host")),
        };
        if host.is_empty() {
            return Err(refuse("an origin has a host"));
        }
        let host = if wildcard {
            if !host.contains('.') {
                return Err(refuse(
                    "a wildcard covers one label under a name of two labels or more, \
                     such as *.notify.windows.com",
                ));
            }
            AllowedHost::OneLabelUnder(host)
        } else {
            AllowedHost::Exact(host)
        };
        let port = url.port_or_known_default().unwrap_or(443);
        Ok(Self { host, port })
    }
}

impl AllowedHost {
    fn matches(&self, host: &str) -> bool {
        match self {
            Self::Exact(name) => host == name,
            Self::OneLabelUnder(name) => host
                .strip_suffix(name.as_str())
                .and_then(|rest| rest.strip_suffix('.'))
                .is_some_and(|label| !label.is_empty() && !label.contains('.')),
        }
    }
}

/// A host name as the list compares it: lowercased, without the trailing
/// dot of a fully qualified name, which names the same host.
fn normalized(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    match host.strip_suffix('.') {
        Some(trimmed) => trimmed.to_owned(),
        None => host,
    }
}
