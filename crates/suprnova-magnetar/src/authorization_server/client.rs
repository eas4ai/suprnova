//! Client identification: redirect URI rules, dynamic registration input
//! and the shape of a client metadata document.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::Deserialize;
use url::{Host, Url};

/// The longest redirect URI the server accepts.
const MAX_REDIRECT_URI_LENGTH: usize = 2048;
/// The most redirect URIs one client may register.
pub(crate) const MAX_REDIRECT_URIS: usize = 16;
/// The longest client name, home page or logo address the server keeps.
const MAX_CLIENT_TEXT_LENGTH: usize = 1024;

/// Where a client's registration came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientSource {
    /// A client metadata document fetched from the `client_id` URL.
    MetadataDocument,
    /// A dynamic client registration stored by this server.
    Registered,
}

/// A client the authorization endpoint can trust: its identifier and the
/// redirect URIs a request is compared against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedClient {
    /// The client identifier as the request named it.
    pub client_id: String,
    /// Where the registration came from.
    pub source: ClientSource,
    /// The name the client gave, if any. It is the client's own claim; the
    /// approval page shows the host beside it.
    pub client_name: Option<String>,
    /// The registered redirect URIs.
    pub redirect_uris: Vec<String>,
    /// The scopes the client limited itself to, when it named any.
    pub scope: Option<Vec<String>>,
    /// The client's home page, if it gave one.
    pub client_uri: Option<String>,
    /// The client's logo, if it gave one.
    pub logo_uri: Option<String>,
}

impl ResolvedClient {
    /// The host the approval page shows for this client and redirect URI:
    /// the host of a metadata document's URL, which the server fetched and
    /// so knows the client controls, or else the host of the redirect URI
    /// the code will go to.
    pub(crate) fn display_host(&self, redirect_uri: &str) -> String {
        let source = match self.source {
            ClientSource::MetadataDocument => self.client_id.as_str(),
            ClientSource::Registered => redirect_uri,
        };
        match Url::parse(source) {
            Ok(url) => url
                .host_str()
                .map_or_else(|| url.scheme().to_owned(), str::to_owned),
            Err(_) => source.to_owned(),
        }
    }

    /// The name to show: the client's own name, or its host when it gave
    /// none.
    pub(crate) fn display_name(&self, redirect_uri: &str) -> String {
        self.client_name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| self.display_host(redirect_uri))
    }

    /// Whether `requested` is one of the client's redirect URIs.
    pub(crate) fn allows_redirect(&self, requested: &str) -> bool {
        self.redirect_uris
            .iter()
            .any(|registered| redirect_uri_matches(registered, requested))
    }
}

/// Whether a redirect URI may be registered: an absolute URI without a
/// fragment or credentials, using HTTPS, plain HTTP on a loopback host for
/// native apps (RFC 8252 section 7.3), or a private-use scheme in reverse
/// domain form (RFC 8252 section 7.1). Schemes without a dot, such as
/// `javascript:` or `data:`, are never private-use schemes.
pub(crate) fn redirect_uri_is_acceptable(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_REDIRECT_URI_LENGTH {
        return false;
    }
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        "https" => url.host().is_some(),
        "http" => match url.host() {
            Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        },
        scheme => scheme.contains('.'),
    }
}

/// Compare a requested redirect URI with a registered one: exactly, except
/// that a loopback IP literal over HTTP may use any port, because a native
/// app listens on whatever port the system gives it (RFC 8252 section
/// 7.3). `localhost` gets no such allowance; it names whatever the resolver
/// says.
pub(crate) fn redirect_uri_matches(registered: &str, requested: &str) -> bool {
    if registered == requested {
        return true;
    }
    let (Ok(registered), Ok(requested)) = (Url::parse(registered), Url::parse(requested)) else {
        return false;
    };
    let loopback = |url: &Url| match url.host() {
        Some(Host::Ipv4(address)) => address == Ipv4Addr::LOCALHOST,
        Some(Host::Ipv6(address)) => address == Ipv6Addr::LOCALHOST,
        _ => false,
    };
    registered.scheme() == "http"
        && requested.scheme() == "http"
        && loopback(&registered)
        && registered.host() == requested.host()
        && registered.path() == requested.path()
        && registered.query() == requested.query()
        && requested.fragment().is_none()
        && requested.username().is_empty()
        && requested.password().is_none()
}

/// Whether `value` is a client identifier this server treats as a client
/// metadata document URL: anything with an `https` or `http` scheme. An
/// `http` one is refused later, rather than looked up as a registration.
pub(crate) fn names_metadata_document(value: &str) -> bool {
    let starts_with = |prefix: &str| {
        value
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    };
    starts_with("https://") || starts_with("http://")
}

/// A short text field a client supplied, kept only when it is within the
/// length the server stores.
pub(crate) fn bounded_text(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty() && text.len() <= MAX_CLIENT_TEXT_LENGTH)
}

/// A link a client supplied, kept only when it is an `https` URL: an
/// approval page renders it as a link or an image, and a `javascript:` or
/// `data:` value there would run in the host's origin.
fn https_link(value: Option<String>) -> Option<String> {
    bounded_text(value).filter(|link| {
        Url::parse(link).is_ok_and(|url| url.scheme() == "https" && url.host().is_some())
    })
}

/// Client metadata (RFC 7591 section 2) as a client metadata document or a
/// registration request carries it: only the fields this server reads.
/// Unknown fields are ignored, as RFC 7591 requires.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct ClientMetadata {
    /// The client identifier. A metadata document must repeat its own URL
    /// here; a registration request leaves it out.
    pub client_id: Option<String>,
    /// The client's display name.
    pub client_name: Option<String>,
    /// The redirect URIs the client may use.
    pub redirect_uris: Option<Vec<String>>,
    /// How the client authenticates at the token endpoint; this server
    /// accepts only `none`.
    pub token_endpoint_auth_method: Option<String>,
    /// The grant types the client will use.
    pub grant_types: Option<Vec<String>>,
    /// The response types the client will use.
    pub response_types: Option<Vec<String>>,
    /// The space-separated scopes the client limits itself to.
    pub scope: Option<String>,
    /// The client's home page.
    pub client_uri: Option<String>,
    /// The client's logo.
    pub logo_uri: Option<String>,
    /// A shared secret; its presence refuses the metadata, since public
    /// clients have none.
    pub client_secret: Option<serde_json::Value>,
    /// A shared secret's expiry; refused like the secret itself.
    pub client_secret_expires_at: Option<serde_json::Value>,
}

/// Why client metadata was refused, in RFC 7591 terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MetadataProblem {
    /// A redirect URI is missing or not acceptable.
    RedirectUri(String),
    /// Another field is not acceptable.
    Metadata(String),
}

impl MetadataProblem {
    /// What is wrong, for an error description.
    pub(crate) fn description(&self) -> &str {
        match self {
            Self::RedirectUri(description) | Self::Metadata(description) => description,
        }
    }
}

/// The checked public-client metadata both identification paths share.
pub(crate) struct CheckedMetadata {
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub scope: Option<Vec<String>>,
    pub client_uri: Option<String>,
    pub logo_uri: Option<String>,
}

/// Check metadata for a public client of this server: redirect URIs that
/// pass [`redirect_uri_is_acceptable`], no shared secret, no client
/// authentication, and only the grants and response types this server
/// offers. `declared_scopes` limits a scope the client restricts itself to.
pub(crate) fn check_public_client(
    fields: ClientMetadata,
    declared_scopes: &[String],
) -> Result<CheckedMetadata, MetadataProblem> {
    let redirect_uris = fields.redirect_uris.unwrap_or_default();
    if redirect_uris.is_empty() {
        return Err(MetadataProblem::RedirectUri(
            "redirect_uris must name at least one redirect URI".to_owned(),
        ));
    }
    if redirect_uris.len() > MAX_REDIRECT_URIS {
        return Err(MetadataProblem::RedirectUri(format!(
            "at most {MAX_REDIRECT_URIS} redirect URIs are accepted"
        )));
    }
    if let Some(bad) = redirect_uris
        .iter()
        .find(|uri| !redirect_uri_is_acceptable(uri))
    {
        return Err(MetadataProblem::RedirectUri(format!(
            "redirect URI {bad:?} must be HTTPS, HTTP on a loopback host, or a private-use scheme, \
             without a fragment"
        )));
    }
    if fields.client_secret.is_some() || fields.client_secret_expires_at.is_some() {
        return Err(MetadataProblem::Metadata(
            "public clients carry no client_secret".to_owned(),
        ));
    }
    match fields.token_endpoint_auth_method.as_deref() {
        None | Some("none") => {}
        Some(other) => {
            return Err(MetadataProblem::Metadata(format!(
                "token_endpoint_auth_method {other:?} is not supported; only \"none\" is"
            )));
        }
    }
    let grant_types = fields
        .grant_types
        .unwrap_or_else(|| vec!["authorization_code".to_owned()]);
    if !grant_types
        .iter()
        .any(|grant| grant == "authorization_code")
        || grant_types
            .iter()
            .any(|grant| grant != "authorization_code" && grant != "refresh_token")
    {
        return Err(MetadataProblem::Metadata(
            "grant_types must include authorization_code and may add only refresh_token".to_owned(),
        ));
    }
    if let Some(response_types) = &fields.response_types
        && (response_types.is_empty() || response_types.iter().any(|kind| kind != "code"))
    {
        return Err(MetadataProblem::Metadata(
            "response_types may name only code".to_owned(),
        ));
    }
    let scope = match fields.scope.as_deref() {
        None => None,
        Some(scope) => {
            let scopes: Vec<String> = scope.split_whitespace().map(str::to_owned).collect();
            if let Some(unknown) = scopes.iter().find(|scope| !declared_scopes.contains(scope)) {
                return Err(MetadataProblem::Metadata(format!(
                    "scope {unknown:?} is not offered by this server"
                )));
            }
            Some(scopes)
        }
    };
    Ok(CheckedMetadata {
        client_name: bounded_text(fields.client_name),
        redirect_uris,
        grant_types,
        scope,
        client_uri: https_link(fields.client_uri),
        logo_uri: https_link(fields.logo_uri),
    })
}

/// Whether an outbound fetch may connect to `address`: only a public
/// unicast address. Loopback, private, link-local, unspecified, shared,
/// documentation, benchmarking, multicast and reserved ranges are refused,
/// and an IPv6 address that embeds an IPv4 one (mapped, compatible,
/// NAT64 or 6to4) is judged by the address it embeds.
pub fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (18..=19).contains(&b))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    if let Some(v4) = address.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    // IPv4-compatible (::a.b.c.d), deprecated but still routable on some
    // stacks; `::` and `::1` fall in here too and are refused below.
    if segments[..6] == [0, 0, 0, 0, 0, 0] {
        return false;
    }
    // NAT64 well-known prefix 64:ff9b::/96 reaches the embedded IPv4.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return is_public_v4(embedded_v4(segments[6], segments[7]));
    }
    // 6to4 2002::/16 embeds an IPv4 in the next 32 bits.
    if segments[0] == 0x2002 {
        return is_public_v4(embedded_v4(segments[1], segments[2]));
    }
    let first = segments[0];
    !((first & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (first & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (first & 0xffc0) == 0xfec0 // site-local fec0::/10
        || (first & 0xff00) == 0xff00 // multicast
        || (first == 0x2001 && segments[1] == 0x0db8) // documentation
        || (first == 0x2001 && segments[1] == 0) // Teredo
        || (first == 0x0100 && segments[1..4] == [0, 0, 0])) // discard 100::/64
}

fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    let [a, b] = high.to_be_bytes();
    let [c, d] = low.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_005_loopback_redirects_ignore_only_the_port() {
        assert!(redirect_uri_matches(
            "http://127.0.0.1:3000/cb",
            "http://127.0.0.1:61000/cb"
        ));
        assert!(redirect_uri_matches(
            "http://[::1]:3000/cb",
            "http://[::1]/cb"
        ));
        assert!(!redirect_uri_matches(
            "http://127.0.0.1:3000/cb",
            "http://127.0.0.1:61000/other"
        ));
        assert!(!redirect_uri_matches(
            "http://localhost:3000/cb",
            "http://localhost:4000/cb"
        ));
        assert!(!redirect_uri_matches(
            "https://127.0.0.1:3000/cb",
            "https://127.0.0.1:4000/cb"
        ));
        assert!(!redirect_uri_matches(
            "https://client.test/cb",
            "https://client.test/cb/"
        ));
    }

    #[test]
    fn agent_005_redirect_uri_registration_rules() {
        for good in [
            "https://client.test/cb",
            "http://127.0.0.1:3000/cb",
            "http://[::1]/cb",
            "http://localhost:8080/cb",
            "com.example.app:/oauth/cb",
        ] {
            assert!(redirect_uri_is_acceptable(good), "{good} is acceptable");
        }
        for bad in [
            "http://client.test/cb",
            "https://client.test/cb#fragment",
            "https://user:pass@client.test/cb",
            "javascript:alert(1)",
            "data:text/html,hi",
            "file:///etc/passwd",
            "/relative",
            "",
        ] {
            assert!(!redirect_uri_is_acceptable(bad), "{bad} is refused");
        }
    }

    #[test]
    fn agent_005_only_public_addresses_are_fetchable() {
        for public in ["93.184.215.14", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(is_public_address(public.parse().unwrap()), "{public}");
        }
        for refused in [
            "0.0.0.0",
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "64:ff9b::a00:1",
            "2002:7f00:1::",
            "2001:db8::1",
            "2001::1",
        ] {
            assert!(!is_public_address(refused.parse().unwrap()), "{refused}");
        }
    }
}
