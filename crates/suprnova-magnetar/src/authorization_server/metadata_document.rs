//! Fetching client metadata documents
//! (draft-ietf-oauth-client-id-metadata-document-00).
//!
//! A client that names an HTTPS URL as its `client_id` is identified by the
//! JSON document at that URL. Fetching it means the server makes a request
//! to an address an anonymous client chose, so every fetch is fenced: HTTPS
//! only, the host resolved here and every address it resolves to checked
//! against [`is_public_address`], the connection made to the checked
//! address (a second DNS answer cannot swap it), no redirect followed, and
//! the body and the whole exchange bounded. Documents are cached for a
//! bounded time and number.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use url::{Host, Url};

use super::client::ClientMetadata;
pub use super::client::is_public_address;
use crate::{Error, Result};

/// The longest `client_id` URL the server fetches.
const MAX_CLIENT_ID_LENGTH: usize = 2048;

/// Bounds on fetching and caching client metadata documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientMetadataPolicy {
    /// The largest document body read, in bytes.
    pub max_bytes: usize,
    /// The time allowed for the whole fetch: resolution, connection,
    /// response and body.
    pub timeout: Duration,
    /// How long a fetched document is reused.
    pub cache_ttl: Duration,
    /// How many documents the cache holds at most.
    pub cache_capacity: usize,
}

impl Default for ClientMetadataPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024,
            timeout: Duration::from_secs(5),
            cache_ttl: Duration::from_secs(300),
            cache_capacity: 512,
        }
    }
}

/// Name resolution for the fetch, kept separate so the server checks every
/// address before anything connects.
#[async_trait]
pub trait HostResolver: Send + Sync {
    /// Resolve `host` to every address it names.
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>>;
}

/// The operating system's resolver, through Tokio.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemResolver;

#[async_trait]
impl HostResolver for SystemResolver {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<IpAddr>> {
        // A lookup that outlives the fetch timeout keeps running on the
        // blocking pool; the per-address metadata-fetch budget bounds how
        // many can pile up.
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| Error::DependencyUnavailable {
                dependency: "dns".to_owned(),
                message: format!("resolve {host}: {error}"),
            })?;
        Ok(addresses.map(|address| address.ip()).collect())
    }
}

/// A response body read chunk by chunk, so the server can stop reading at
/// its size bound instead of buffering whatever the peer sends.
#[async_trait]
pub trait ResponseBody: Send {
    /// The next chunk, or `None` at the end of the body.
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>>;
}

/// The status and body of one pinned request.
pub struct PinnedResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The body, read lazily.
    pub body: Box<dyn ResponseBody>,
}

impl fmt::Debug for PinnedResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinnedResponse")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// An HTTPS client that sends one GET to `url` over a connection to
/// `address` and nothing else: no resolution of its own, no proxy and no
/// redirect.
#[async_trait]
pub trait PinnedHttpClient: Send + Sync {
    /// Send a GET for `url` to `address`, verifying TLS for the URL's host.
    async fn get(&self, url: &Url, address: SocketAddr) -> Result<PinnedResponse>;
}

/// The default [`PinnedHttpClient`], over `reqwest` with rustls.
///
/// Each fetch builds a client whose only name override is the checked
/// address, with proxies and redirects off. Documents are cached, so the
/// cost of building one client per fetch is paid rarely.
#[derive(Clone, Debug)]
pub struct ReqwestPinnedClient {
    timeout: Duration,
    /// An extra trust anchor, DER encoded. Only tests set it, to reach a
    /// local server with a certificate of their own.
    extra_root: Option<Vec<u8>>,
}

impl ReqwestPinnedClient {
    /// A client whose requests give up after `timeout`.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            extra_root: None,
        }
    }

    #[cfg(test)]
    fn trusting(timeout: Duration, root_der: Vec<u8>) -> Self {
        Self {
            timeout,
            extra_root: Some(root_der),
        }
    }
}

struct ReqwestBody(reqwest::Response);

#[async_trait]
impl ResponseBody for ReqwestBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        self.0
            .chunk()
            .await
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
            .map_err(|error| Error::DependencyUnavailable {
                dependency: "client metadata document".to_owned(),
                message: format!("read body: {error}"),
            })
    }
}

#[async_trait]
impl PinnedHttpClient for ReqwestPinnedClient {
    async fn get(&self, url: &Url, address: SocketAddr) -> Result<PinnedResponse> {
        let transport = |message: String| Error::DependencyUnavailable {
            dependency: "client metadata document".to_owned(),
            message,
        };
        let host = url
            .host_str()
            .ok_or_else(|| transport("the URL has no host".to_owned()))?;
        let mut builder = reqwest::Client::builder()
            .resolve(host, address)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .https_only(true)
            .timeout(self.timeout);
        if let Some(root) = &self.extra_root {
            let certificate = reqwest::Certificate::from_der(root)
                .map_err(|error| transport(format!("extra trust anchor: {error}")))?;
            builder = builder.add_root_certificate(certificate);
        }
        let client = builder
            .build()
            .map_err(|error| transport(format!("build client: {error}")))?;
        let response = client
            .get(url.as_str())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| transport(format!("send: {error}")))?;
        Ok(PinnedResponse {
            status: response.status().as_u16(),
            body: Box::new(ReqwestBody(response)),
        })
    }
}

/// Why a client metadata document was not accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMetadataError {
    /// The `client_id` is not an `https` URL.
    NotHttps,
    /// The `client_id` URL breaks the draft's rules: credentials, a
    /// fragment, no path, dot segments, or a non-canonical spelling.
    InvalidUrl(String),
    /// The host did not resolve to any address.
    Unresolvable(String),
    /// The host resolved to an address the server will not connect to.
    ForbiddenAddress(IpAddr),
    /// The connection or the request failed.
    Transport(String),
    /// The document answered with a redirect, which is never followed.
    Redirected(u16),
    /// The document answered with a status other than 200.
    Status(u16),
    /// The body exceeded the size bound.
    TooLarge,
    /// The fetch did not finish within the time bound.
    TimedOut,
    /// The body is not a client metadata document.
    InvalidDocument(String),
    /// The document's `client_id` is not exactly the URL it was fetched
    /// from.
    ClientIdMismatch,
}

impl fmt::Display for ClientMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotHttps => write!(formatter, "the client_id URL must use https"),
            Self::InvalidUrl(reason) => write!(formatter, "invalid client_id URL: {reason}"),
            Self::Unresolvable(reason) => {
                write!(formatter, "the client_id host does not resolve: {reason}")
            }
            // The address stays out of the message, which can reach a
            // browser: it would report what an internal name resolves to.
            Self::ForbiddenAddress(_) => write!(
                formatter,
                "the client_id host resolves to a non-public address"
            ),
            Self::Transport(reason) => {
                write!(
                    formatter,
                    "the client metadata document fetch failed: {reason}"
                )
            }
            Self::Redirected(status) => write!(
                formatter,
                "the client metadata document answered with a redirect ({status})"
            ),
            Self::Status(status) => write!(
                formatter,
                "the client metadata document answered with status {status}"
            ),
            Self::TooLarge => write!(formatter, "the client metadata document is too large"),
            Self::TimedOut => write!(formatter, "the client metadata document fetch timed out"),
            Self::InvalidDocument(reason) => {
                write!(formatter, "invalid client metadata document: {reason}")
            }
            Self::ClientIdMismatch => write!(
                formatter,
                "the client metadata document's client_id differs from its URL"
            ),
        }
    }
}

impl std::error::Error for ClientMetadataError {}

struct CachedDocument {
    fetched_at: Instant,
    document: ClientMetadata,
}

/// Fetches, checks and caches client metadata documents.
pub struct ClientMetadataFetcher {
    resolver: Arc<dyn HostResolver>,
    client: Arc<dyn PinnedHttpClient>,
    policy: ClientMetadataPolicy,
    cache: Mutex<HashMap<String, CachedDocument>>,
    /// Which addresses a fetch may connect to: [`is_public_address`],
    /// except in tests that reach a server on the loopback address.
    address_allowed: fn(IpAddr) -> bool,
}

impl ClientMetadataFetcher {
    /// A fetcher over a resolver and a pinned client the host chose.
    #[must_use]
    pub fn new(
        resolver: Arc<dyn HostResolver>,
        client: Arc<dyn PinnedHttpClient>,
        policy: ClientMetadataPolicy,
    ) -> Self {
        Self {
            resolver,
            client,
            policy,
            cache: Mutex::new(HashMap::new()),
            address_allowed: is_public_address,
        }
    }

    #[cfg(test)]
    fn admitting(mut self, address_allowed: fn(IpAddr) -> bool) -> Self {
        self.address_allowed = address_allowed;
        self
    }

    /// A fetcher over the system resolver and [`ReqwestPinnedClient`].
    #[must_use]
    pub fn system(policy: ClientMetadataPolicy) -> Self {
        Self::new(
            Arc::new(SystemResolver),
            Arc::new(ReqwestPinnedClient::new(policy.timeout)),
            policy,
        )
    }

    /// Whether a fresh copy of the document is cached, so a lookup costs
    /// no request.
    pub fn is_cached(&self, client_id: &str) -> bool {
        self.cached(client_id).is_some()
    }

    /// The document at `client_id`, from the cache or fetched under the
    /// policy, after checking that it names its own URL as its
    /// `client_id`.
    pub async fn fetch(
        &self,
        client_id: &str,
    ) -> std::result::Result<ClientMetadata, ClientMetadataError> {
        if let Some(document) = self.cached(client_id) {
            return Ok(document);
        }
        let url = document_url(client_id)?;
        let document = tokio::time::timeout(self.policy.timeout, self.fetch_uncached(&url))
            .await
            .map_err(|_| ClientMetadataError::TimedOut)??;
        if document.client_id.as_deref() != Some(client_id) {
            return Err(ClientMetadataError::ClientIdMismatch);
        }
        self.remember(client_id, document.clone());
        Ok(document)
    }

    async fn fetch_uncached(
        &self,
        url: &Url,
    ) -> std::result::Result<ClientMetadata, ClientMetadataError> {
        let port = url.port_or_known_default().unwrap_or(443);
        let addresses: Vec<IpAddr> = match url.host() {
            Some(Host::Ipv4(address)) => vec![IpAddr::V4(address)],
            Some(Host::Ipv6(address)) => vec![IpAddr::V6(address)],
            Some(Host::Domain(domain)) => self
                .resolver
                .resolve(domain, port)
                .await
                .map_err(|error| ClientMetadataError::Unresolvable(error.to_string()))?,
            None => return Err(ClientMetadataError::InvalidUrl("no host".to_owned())),
        };
        let Some(first) = addresses.first().copied() else {
            return Err(ClientMetadataError::Unresolvable(
                "no address returned".to_owned(),
            ));
        };
        // Every address is checked, not only the one used: a resolver that
        // mixes a public and a private answer is refused outright.
        if let Some(forbidden) = addresses
            .iter()
            .copied()
            .find(|address| !(self.address_allowed)(*address))
        {
            return Err(ClientMetadataError::ForbiddenAddress(forbidden));
        }
        let mut response = self
            .client
            .get(url, SocketAddr::new(first, port))
            .await
            .map_err(|error| ClientMetadataError::Transport(error.to_string()))?;
        if (300..400).contains(&response.status) {
            return Err(ClientMetadataError::Redirected(response.status));
        }
        if response.status != 200 {
            return Err(ClientMetadataError::Status(response.status));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .body
            .next_chunk()
            .await
            .map_err(|error| ClientMetadataError::Transport(error.to_string()))?
        {
            if body.len().saturating_add(chunk.len()) > self.policy.max_bytes {
                return Err(ClientMetadataError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|error| {
            ClientMetadataError::InvalidDocument(crate::crypto::decode_failure(&error))
        })
    }

    fn cached(&self, client_id: &str) -> Option<ClientMetadata> {
        let cache = match self.cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        cache
            .get(client_id)
            .filter(|entry| entry.fetched_at.elapsed() < self.policy.cache_ttl)
            .map(|entry| entry.document.clone())
    }

    fn remember(&self, client_id: &str, document: ClientMetadata) {
        if self.policy.cache_capacity == 0 || self.policy.cache_ttl.is_zero() {
            return;
        }
        let mut cache = match self.cache.lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        };
        if cache.len() >= self.policy.cache_capacity && !cache.contains_key(client_id) {
            let ttl = self.policy.cache_ttl;
            cache.retain(|_, entry| entry.fetched_at.elapsed() < ttl);
            if cache.len() >= self.policy.cache_capacity
                && let Some(oldest) = cache
                    .iter()
                    .min_by_key(|(_, entry)| entry.fetched_at)
                    .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(
            client_id.to_owned(),
            CachedDocument {
                fetched_at: Instant::now(),
                document,
            },
        );
    }
}

/// Parse and check a `client_id` URL against the draft's rules.
fn document_url(client_id: &str) -> std::result::Result<Url, ClientMetadataError> {
    if client_id.len() > MAX_CLIENT_ID_LENGTH {
        return Err(ClientMetadataError::InvalidUrl("too long".to_owned()));
    }
    let url = Url::parse(client_id)
        .map_err(|error| ClientMetadataError::InvalidUrl(error.to_string()))?;
    if url.scheme() != "https" {
        return Err(ClientMetadataError::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ClientMetadataError::InvalidUrl(
            "credentials are not allowed".to_owned(),
        ));
    }
    if url.fragment().is_some() {
        return Err(ClientMetadataError::InvalidUrl(
            "a fragment is not allowed".to_owned(),
        ));
    }
    if url.path() == "/" || url.path().is_empty() {
        return Err(ClientMetadataError::InvalidUrl(
            "a path is required".to_owned(),
        ));
    }
    // The parser removes dot segments and lowercases the host, so a URL
    // that does not survive a round trip carried one of those, or another
    // spelling the document's exact `client_id` comparison would trip on.
    if url.as_str() != client_id {
        return Err(ClientMetadataError::InvalidUrl(
            "the URL must be in canonical form, without dot segments".to_owned(),
        ));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::rustls;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    use super::*;

    /// A name that never resolves in public DNS (RFC 2606), so a fetch
    /// that reaches the server proves it used the checked address.
    const HOST: &str = "metadata.test";

    struct Loopback;

    #[async_trait]
    impl HostResolver for Loopback {
        async fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>> {
            Ok(vec![IpAddr::from([127, 0, 0, 1])])
        }
    }

    type Route = fn(&str, u16) -> (u16, Vec<(&'static str, String)>, String);

    struct Server {
        port: u16,
        root: Vec<u8>,
        requests: Arc<AtomicUsize>,
    }

    /// An HTTPS server on the loopback address with a certificate for
    /// [`HOST`], answering every request through `route`.
    async fn https_server(route: Route) -> Server {
        let certified =
            rcgen::generate_simple_self_signed(vec![HOST.to_owned()]).expect("a certificate");
        let root = certified.cert.der().to_vec();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            certified.signing_key.serialize_der(),
        ));
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(root.clone())], key)
        .expect("a server configuration");
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let port = listener.local_addr().expect("a local address").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (acceptor, counter) = (acceptor.clone(), counter.clone());
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    let mut request = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        match tls.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&chunk[..read]),
                        }
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    let text = String::from_utf8_lossy(&request);
                    let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let (status, headers, body) = route(&path, port);
                    let mut response = format!(
                        "HTTP/1.1 {status} Test\r\ncontent-length: {}\r\nconnection: close\r\n",
                        body.len()
                    );
                    for (name, value) in headers {
                        response.push_str(&format!("{name}: {value}\r\n"));
                    }
                    response.push_str("\r\n");
                    response.push_str(&body);
                    let _ = tls.write_all(response.as_bytes()).await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        Server {
            port,
            root,
            requests,
        }
    }

    fn document_url(port: u16) -> String {
        format!("https://{HOST}:{port}/client.json")
    }

    fn document(port: u16) -> String {
        serde_json::json!({
            "client_id": document_url(port),
            "client_name": "Loopback Client",
            "redirect_uris": ["http://127.0.0.1:3000/callback"],
        })
        .to_string()
    }

    fn fetcher(server: &Server) -> ClientMetadataFetcher {
        let policy = ClientMetadataPolicy::default();
        ClientMetadataFetcher::new(
            Arc::new(Loopback),
            Arc::new(ReqwestPinnedClient::trusting(
                policy.timeout,
                server.root.clone(),
            )),
            policy,
        )
        .admitting(|address| address.is_loopback())
    }

    #[tokio::test]
    async fn agent_005_the_real_client_connects_to_the_checked_address() {
        let server = https_server(|path, port| match path {
            "/client.json" => (
                200,
                vec![("content-type", "application/json".to_owned())],
                document(port),
            ),
            _ => (404, Vec::new(), String::new()),
        })
        .await;
        let fetched = fetcher(&server)
            .fetch(&document_url(server.port))
            .await
            .expect("the document is fetched from the checked address");
        assert_eq!(fetched.client_name.as_deref(), Some("Loopback Client"));
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn agent_005_the_real_client_follows_no_redirect() {
        let server = https_server(|path, port| match path {
            "/client.json" => (
                302,
                vec![("location", format!("https://{HOST}:{port}/moved.json"))],
                String::new(),
            ),
            // A followed redirect would find a valid document here.
            "/moved.json" => (
                200,
                vec![("content-type", "application/json".to_owned())],
                document(port),
            ),
            _ => (404, Vec::new(), String::new()),
        })
        .await;
        assert_eq!(
            fetcher(&server).fetch(&document_url(server.port)).await,
            Err(ClientMetadataError::Redirected(302))
        );
        assert_eq!(
            server.requests.load(Ordering::SeqCst),
            1,
            "the redirect was not followed"
        );
    }

    #[tokio::test]
    async fn agent_005_the_loopback_server_is_refused_without_the_test_policy() {
        let server = https_server(|path, port| match path {
            "/client.json" => (200, Vec::new(), document(port)),
            _ => (404, Vec::new(), String::new()),
        })
        .await;
        let policy = ClientMetadataPolicy::default();
        let production = ClientMetadataFetcher::new(
            Arc::new(Loopback),
            Arc::new(ReqwestPinnedClient::trusting(
                policy.timeout,
                server.root.clone(),
            )),
            policy,
        );
        assert!(matches!(
            production.fetch(&document_url(server.port)).await,
            Err(ClientMetadataError::ForbiddenAddress(_))
        ));
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    }
}
