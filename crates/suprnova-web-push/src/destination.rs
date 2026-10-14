//! Where a send may connect under the `Strict` endpoint policy.
//!
//! A subscription endpoint is user-derived data, so its host name can
//! resolve anywhere: a name an attacker controls can answer with a
//! loopback, private or cloud metadata address. The strict policy looks the
//! name up once, refuses it when any answer falls in a class that is not on
//! the public internet, and pins the answers it checked for the transport's
//! resolver. The connection then goes to a checked address while TLS and
//! the `Host` header keep the name, and a second answer for the same name,
//! as a DNS rebinding gives, never reaches a socket.

use crate::error::{AddressClass, WebPushError};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Mutex, PoisonError};

/// The class of an address the strict policy refuses, or `None` for an
/// address on the public internet.
pub(crate) fn classify(ip: IpAddr) -> Option<AddressClass> {
    match ip {
        IpAddr::V4(ip) => classify_v4(ip),
        IpAddr::V6(ip) => classify_v6(ip),
    }
}

fn classify_v4(ip: Ipv4Addr) -> Option<AddressClass> {
    // Metadata first: 169.254.169.254 is also link-local and
    // 100.100.100.200 is also shared address space, and the metadata class
    // is the one an operator needs to read.
    const METADATA: [Ipv4Addr; 2] = [
        Ipv4Addr::new(169, 254, 169, 254),
        Ipv4Addr::new(100, 100, 100, 200),
    ];
    if METADATA.contains(&ip) {
        return Some(AddressClass::Metadata);
    }
    let [a, b, c, _] = ip.octets();
    let class = if ip.is_unspecified() {
        AddressClass::Unspecified
    } else if ip.is_loopback() {
        AddressClass::Loopback
    } else if ip.is_private() || (a == 100 && (b & 0xc0) == 64) {
        // RFC 1918, and the RFC 6598 shared address space 100.64.0.0/10.
        AddressClass::Private
    } else if ip.is_link_local() {
        AddressClass::LinkLocal
    } else if ip.is_documentation() {
        AddressClass::Documentation
    } else if ip.is_multicast() {
        AddressClass::Multicast
    } else if a == 0
        || a >= 240
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b & 0xfe) == 18)
    {
        // 0.0.0.0/8, 240.0.0.0/4 with the broadcast address, the IETF
        // protocol assignments 192.0.0.0/24, the retired 6to4 relay
        // 192.88.99.0/24 and the benchmarking range 198.18.0.0/15.
        AddressClass::Reserved
    } else {
        return None;
    };
    Some(class)
}

fn classify_v6(ip: Ipv6Addr) -> Option<AddressClass> {
    const AWS_METADATA: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254);
    if ip == AWS_METADATA {
        return Some(AddressClass::Metadata);
    }
    // An IPv4-mapped address reaches the IPv4 address it carries.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return classify_v4(v4);
    }
    let s = ip.segments();
    // The NAT64 well-known prefix 64:ff9b::/96 (RFC 6052): a translator
    // connects to the IPv4 address in the low 32 bits.
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return classify_v4(Ipv4Addr::from((u32::from(s[6]) << 16) | u32::from(s[7])));
    }
    let class = if ip.is_unspecified() {
        AddressClass::Unspecified
    } else if ip.is_loopback() {
        AddressClass::Loopback
    } else if ip.is_multicast() {
        AddressClass::Multicast
    } else if (s[0] & 0xffc0) == 0xfe80 {
        AddressClass::LinkLocal
    } else if (s[0] & 0xfe00) == 0xfc00
        || (s[0] & 0xffc0) == 0xfec0
        || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1)
    {
        // Unique local fc00::/7, the deprecated site-local fec0::/10, and
        // the local-use NAT64 prefix 64:ff9b:1::/48 (RFC 8215).
        AddressClass::Private
    } else if (s[0] == 0x2001 && s[1] == 0x0db8) || (s[0] == 0x3fff && s[1] < 0x1000) {
        // 2001:db8::/32 and 3fff::/20 (RFC 9637).
        AddressClass::Documentation
    } else if (s[0] & 0xe000) != 0x2000 || (s[0] == 0x2001 && s[1] < 0x0200) || s[0] == 0x2002 {
        // Everything outside the global unicast space 2000::/3, the IETF
        // protocol assignments 2001::/23 (Teredo among them), and the
        // retired 6to4 prefix 2002::/16, which tunnels to any IPv4 address.
        AddressClass::Reserved
    } else {
        return None;
    };
    Some(class)
}

/// Refuse the destination when any address the name resolved to is in a
/// refused class. One bad answer is enough: the connector may try any of
/// them.
pub(crate) fn refuse_unsafe(addrs: &[SocketAddr]) -> Result<(), WebPushError> {
    match addrs.iter().find_map(|addr| classify(addr.ip())) {
        Some(class) => Err(WebPushError::RefusedDestination(class)),
        None => Ok(()),
    }
}

/// Where the strict policy looks a host name up.
#[derive(Debug, Clone, Default)]
pub(crate) enum Lookup {
    /// The system resolver, through tokio.
    #[default]
    System,
    /// A fixed answer, so a test can make a name resolve to any address.
    #[cfg(test)]
    Fixed(Vec<IpAddr>),
}

impl Lookup {
    /// Every address `host` resolves to, with `port`. A name that resolves
    /// to no address is a failed lookup, so nothing is sent to it.
    pub(crate) async fn resolve(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Vec<SocketAddr>, WebPushError> {
        let addrs: Vec<SocketAddr> = match self {
            Self::System => tokio::net::lookup_host((host, port))
                .await
                .map_err(WebPushError::Lookup)?
                .collect(),
            #[cfg(test)]
            Self::Fixed(ips) => ips.iter().map(|ip| SocketAddr::new(*ip, port)).collect(),
        };
        if addrs.is_empty() {
            return Err(WebPushError::Lookup(io::Error::new(
                io::ErrorKind::NotFound,
                "the host name resolved to no address",
            )));
        }
        Ok(addrs)
    }
}

/// The name resolver of a transport this crate builds.
///
/// A host name a strict send checked and pinned answers with the addresses
/// that were checked, and is never looked up again while the send holds
/// its pin. Any other name, such as an endpoint's under the `AllowAny`
/// policy, is looked up as reqwest's own resolver would. The transport
/// sends through no proxy, so every name it connects to is resolved here,
/// and none by a proxy.
#[derive(Debug, Default)]
pub(crate) struct PinnedResolver {
    pins: Mutex<HashMap<String, PinEntry>>,
}

#[derive(Debug)]
struct PinEntry {
    addrs: Vec<SocketAddr>,
    holders: usize,
}

impl PinnedResolver {
    /// Pin `addrs` for `host` until the returned guard drops.
    ///
    /// Two sends to one host share its entry. The later one's answers
    /// replace the earlier one's, and both were checked, so a connection
    /// either send opens goes to a checked address.
    pub(crate) fn pin(&self, host: &str, addrs: Vec<SocketAddr>) -> PinGuard<'_> {
        let host = host.to_ascii_lowercase();
        let mut pins = self.lock();
        let entry = pins.entry(host.clone()).or_insert(PinEntry {
            addrs: Vec::new(),
            holders: 0,
        });
        entry.addrs = addrs;
        entry.holders += 1;
        drop(pins);
        PinGuard {
            resolver: self,
            host,
        }
    }

    /// The addresses pinned for `host`, if a send holds a pin for it.
    pub(crate) fn pinned(&self, host: &str) -> Option<Vec<SocketAddr>> {
        self.lock()
            .get(&host.to_ascii_lowercase())
            .map(|entry| entry.addrs.clone())
    }

    /// The pin table. It holds plain data that no change leaves half
    /// written, so a lock poisoned by a panic elsewhere is taken over
    /// rather than spread as another panic.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PinEntry>> {
        self.pins.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Resolve for PinnedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        if let Some(addrs) = self.pinned(name.as_str()) {
            let addrs: Addrs = Box::new(addrs.into_iter());
            return Box::pin(std::future::ready(Ok(addrs)));
        }
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let addrs: Addrs = Box::new(addrs.into_iter());
            Ok(addrs)
        })
    }
}

/// Holds a pin for the length of one send. The pin goes when the last
/// send holding it ends, so the table never outgrows the sends in flight.
#[must_use = "the pin is released when the guard drops"]
pub(crate) struct PinGuard<'a> {
    resolver: &'a PinnedResolver,
    host: String,
}

impl Drop for PinGuard<'_> {
    fn drop(&mut self) {
        let mut pins = self.resolver.lock();
        if let Some(pin) = pins.get_mut(&self.host) {
            pin.holders = pin.holders.saturating_sub(1);
            if pin.holders == 0 {
                pins.remove(&self.host);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;
    use std::sync::Arc;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("test address parses")
    }

    fn assert_class(addresses: &[&str], expected: AddressClass) {
        for address in addresses {
            assert_eq!(
                classify(ip(address)),
                Some(expected),
                "{address} must be refused as {expected}"
            );
        }
    }

    #[test]
    fn hardening_loopback_addresses_are_refused() {
        assert_class(
            &[
                "127.0.0.1",
                "127.255.255.254",
                "::1",
                "::ffff:127.0.0.1",
                "64:ff9b::7f00:1",
            ],
            AddressClass::Loopback,
        );
    }

    #[test]
    fn hardening_private_addresses_are_refused() {
        assert_class(
            &[
                "10.0.0.1",
                "10.255.255.255",
                "172.16.0.1",
                "172.31.255.255",
                "192.168.1.1",
                "100.64.0.1",
                "100.127.255.255",
                "fc00::1",
                "fd12:3456::1",
                "fec0::1",
                "64:ff9b:1::a00:1",
                "::ffff:10.0.0.1",
            ],
            AddressClass::Private,
        );
    }

    #[test]
    fn hardening_link_local_multicast_unspecified_documentation_and_reserved_are_refused() {
        assert_class(&["169.254.0.1", "fe80::1"], AddressClass::LinkLocal);
        assert_class(
            &["224.0.0.1", "239.255.255.255", "ff02::1"],
            AddressClass::Multicast,
        );
        assert_class(&["0.0.0.0", "::"], AddressClass::Unspecified);
        assert_class(
            &[
                "192.0.2.1",
                "198.51.100.7",
                "203.0.113.9",
                "2001:db8::1",
                "3fff:fff::1",
            ],
            AddressClass::Documentation,
        );
        assert_class(
            &[
                "0.1.2.3",
                "192.0.0.8",
                "192.88.99.1",
                "198.18.0.1",
                "198.19.255.255",
                "240.0.0.1",
                "255.255.255.255",
                "::7f00:1",
                "100::1",
                "2001::1",
                "2001:1ff::1",
                "2002:7f00:1::",
                "5f00::1",
            ],
            AddressClass::Reserved,
        );
    }

    #[test]
    fn hardening_metadata_addresses_are_refused_as_metadata() {
        assert_class(
            &[
                "169.254.169.254",
                "fd00:ec2::254",
                "100.100.100.200",
                "::ffff:169.254.169.254",
            ],
            AddressClass::Metadata,
        );
    }

    #[test]
    fn hardening_public_addresses_pass() {
        for address in [
            "8.8.8.8",
            "142.250.72.14",
            "172.15.255.255",
            "172.32.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "198.17.255.255",
            "198.20.0.1",
            "223.255.255.255",
            "2607:f8b0:4004:c1b::64",
            "2a00:1450:4001:82a::200e",
            "2001:200::1",
            "64:ff9b::808:808",
            "::ffff:8.8.8.8",
        ] {
            assert_eq!(classify(ip(address)), None, "{address} is public");
        }
    }

    #[test]
    fn hardening_one_refused_answer_refuses_the_host() {
        let public = SocketAddr::new(ip("8.8.8.8"), 443);
        let private = SocketAddr::new(ip("10.0.0.7"), 443);
        assert!(refuse_unsafe(&[public]).is_ok());
        let err = refuse_unsafe(&[public, private]).unwrap_err();
        assert!(
            matches!(err, WebPushError::RefusedDestination(AddressClass::Private)),
            "{err:?}"
        );
        assert!(!err.is_retryable(), "a retry changes nothing");
        let text = err.to_string();
        assert_eq!(
            text,
            "subscription cannot be used: its host name resolves to a private address"
        );
        assert!(
            !text.contains("10.0.0.7"),
            "the class is named, not the address"
        );
        assert!(!format!("{err:?}").contains("10.0.0.7"));
    }

    #[tokio::test]
    async fn hardening_a_name_that_resolves_to_no_address_is_a_failed_lookup() {
        let err = Lookup::Fixed(Vec::new())
            .resolve("push.example.com", 443)
            .await
            .unwrap_err();
        assert!(matches!(err, WebPushError::Lookup(_)), "{err:?}");
        assert!(err.is_retryable(), "a lookup can fail for a moment");
    }

    /// A transport this crate builds connects a pinned name to the pinned
    /// address, with the name in the `Host` header. `.test` never resolves
    /// in DNS, so the request arriving proves the pin was used and no
    /// lookup was made; the trailing dot and the capitals prove the pin
    /// matches the name reqwest hands its resolver.
    #[tokio::test]
    async fn hardening_a_pinned_name_connects_to_the_pinned_address_under_its_name() {
        let server = MockServer::start().await;
        let port = server.address().port();
        Mock::given(method("POST"))
            .and(path("/push"))
            .and(header("host", format!("push.pinned.test.:{port}").as_str()))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;

        let pins = Arc::new(PinnedResolver::default());
        let http = Client::builder()
            .dns_resolver(Arc::clone(&pins))
            .no_proxy()
            .build()
            .expect("client builds");
        let url = url::Url::parse(&format!("http://Push.Pinned.Test.:{port}/push"))
            .expect("test url parses");
        let host = url.host_str().expect("test url has a host").to_owned();
        let _pin = pins.pin(&host, vec![*server.address()]);

        let response = http
            .post(url)
            .send()
            .await
            .expect("the pinned name connects");
        assert_eq!(response.status(), 201);
    }

    /// A name nothing pinned, such as an endpoint's under `AllowAny`, is
    /// looked up as reqwest would, so a send that checks nothing still
    /// reaches its endpoint.
    #[tokio::test]
    async fn hardening_an_unpinned_name_is_looked_up() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/push"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;
        let http = Client::builder()
            .dns_resolver(Arc::new(PinnedResolver::default()))
            .no_proxy()
            .build()
            .expect("client builds");
        let port = server.address().port();
        let response = http
            .post(format!("http://localhost:{port}/push"))
            .send()
            .await
            .expect("localhost resolves");
        assert_eq!(response.status(), 201);
    }

    #[test]
    fn hardening_a_pin_lasts_as_long_as_the_sends_that_hold_it() {
        let pins = PinnedResolver::default();
        let first = SocketAddr::new(ip("8.8.8.8"), 443);
        let second = SocketAddr::new(ip("8.8.4.4"), 443);
        let one = pins.pin("push.example.com", vec![first]);
        let two = pins.pin("PUSH.example.com", vec![second]);
        assert_eq!(pins.pinned("push.example.com"), Some(vec![second]));
        drop(one);
        assert_eq!(
            pins.pinned("push.example.com"),
            Some(vec![second]),
            "a pin stays while a send holds it"
        );
        drop(two);
        assert_eq!(
            pins.pinned("push.example.com"),
            None,
            "the last send releases it"
        );
    }
}
