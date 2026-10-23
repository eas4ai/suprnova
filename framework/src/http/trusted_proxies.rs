//! Trusted-proxy gating for header-derived request accessors.
//!
//! Several `Request` accessors - `ip()`, `ips()`, `secure()`, `host()`,
//! `http_host()`, `port()` - read the `X-Forwarded-*` and `X-Real-IP`
//! headers ahead of the actual TCP peer. That is right behind a
//! terminating proxy (nginx, ALB, Cloudflare) that writes these headers
//! itself, and it is a security hole anywhere else, because any client
//! can send `X-Forwarded-For: 1.2.3.4`.
//!
//! [`TrustedProxiesConfig`] is the explicit allowlist that gates this
//! trust. The default - empty allowlist - means proxy headers are
//! **ignored**: `Request::ip()` falls back to the TCP peer, `secure()`
//! to the URI scheme, `host()` to the `Host` header, and so on. The
//! operator opts in by listing proxy IPs (the addresses of the
//! terminating edge they actually run); only when the TCP peer
//! matches one of those is the request's `X-Forwarded-*` chain
//! honoured.
//!
//! ## Resolution
//!
//! [`crate::server::handle_request_with_peer`] resolves the
//! configured allowlist once per request (via `Config::get`) and
//! threads it into the [`Request`](crate::Request) builder. This
//! keeps the accessor methods pure `&self` (no global lookups) and
//! makes parallel tests trivial - bind a `TrustedProxiesConfig`
//! directly into the test's `Request` via
//! [`Request::with_trusted_proxies`](crate::Request::with_trusted_proxies)
//! without touching the global container.
//!
//! ## Configuration
//!
//! `APP_TRUSTED_PROXIES` is the usual way: a list of addresses and of
//! ranges in CIDR form, separated by commas. In code the allowlist goes
//! into the [`AppConfig`](crate::config::AppConfig) the application
//! registers:
//!
//! ```rust
//! use std::net::IpAddr;
//! use suprnova::AppConfig;
//! use suprnova::http::TrustedProxiesConfig;
//!
//! // Trust the loopback edge running our terminating nginx.
//! let trusted = TrustedProxiesConfig::with_ips([IpAddr::from([127, 0, 0, 1])]);
//! let config = AppConfig::builder().trusted_proxies(trusted).build();
//! # assert!(config.trusted_proxies.trusts(Some(IpAddr::from([127, 0, 0, 1]))));
//! ```
//!
//! ## Deployment guidance
//!
//! Deployments *not* behind a terminating proxy must leave the
//! allowlist empty - any inbound `X-Forwarded-*` from a direct client
//! is hostile. Deployments behind a real proxy must list every
//! address from which the proxy hops can reach the framework, NOT
//! every client IP; the proxy itself terminates the TCP connection.
//!
//! List every proxy of the chain, not the last one alone.
//! `X-Forwarded-For` is read from the right, and the client is the
//! first address that is no proxy of the list (see
//! [`Request::ip`](crate::Request::ip)). Behind a content delivery
//! network in front of a proxy of your own, the address of the
//! network's edge is what stands there, and with the edge not listed
//! every client of that edge is one client. An edge has no single
//! address, so an entry may be a range in CIDR form:
//! `APP_TRUSTED_PROXIES=10.0.0.5,173.245.48.0/20,2400:cb00::/32`.
//!
//! A load balancer may add an address of its own behind the address of
//! the client, as the external Application Load Balancer of Google
//! Cloud does with the address of its forwarding rule. That address is
//! a proxy of the chain, and it has to be listed like the others.
//!
//! **A range must hold proxies and nothing else.** Whoever connects
//! from an address of the allowlist is believed: a client inside a
//! trusted range writes its own `X-Forwarded-For`, and with it the
//! address [`Request::ip`](crate::Request::ip) returns, and the
//! forwarded host and scheme as well. The network of the pods of a
//! cluster and the range of a VPN are ranges with clients in them. A
//! range that holds every address, `0.0.0.0/0`, is refused.
//!
//! The proxy has to write `X-Forwarded-For` itself, by adding the
//! address it saw to the header or by replacing the header. A proxy
//! that passes the header of the client on as it came is not a proxy
//! to list here. A proxy that writes `X-Real-IP` and nothing else has
//! to remove `X-Forwarded-For` from the request: `X-Real-IP` is read
//! only when the request has no `X-Forwarded-For`. The `Forwarded`
//! header of RFC 7239 is not read.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::sync::Arc;

use crate::error::FrameworkError;

/// A range of addresses in CIDR form: `10.0.0.0/8`, `2001:db8::/32`.
///
/// A proxy that is one machine is listed by its address. A content
/// delivery network or a load balancer of a cloud reaches the
/// application from any address of a range, and the range is what its
/// operator publishes.
///
/// A range must hold proxies and nothing else: a client that connects
/// from an address of a trusted range is believed like a proxy, and
/// chooses the address and the host the application sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProxyNetwork {
    network: IpAddr,
    prefix: u8,
}

impl ProxyNetwork {
    /// The range of the addresses that share the first `prefix` bits of
    /// `address`. The bits behind the prefix are dropped, so
    /// `10.1.2.3/8` is `10.0.0.0/8`.
    ///
    /// An IPv4 range that is written in the IPv6 form,
    /// `::ffff:10.0.0.0/104`, is the IPv4 range `10.0.0.0/8`: the first
    /// 96 bits of that form are the same for every IPv4 address.
    ///
    /// # Errors
    ///
    /// When `prefix` is longer than the address: more than 32 for IPv4,
    /// more than 128 for IPv6. When `prefix` is 0, which is the range of
    /// every address: with it every client is a trusted proxy, and each
    /// of them writes the address the application sees. When the address
    /// is in the IPv6 form of an IPv4 address and `prefix` is 96 or
    /// shorter: 96 is every IPv4 address, and a shorter one is no range
    /// of IPv4 addresses.
    pub fn new(address: IpAddr, prefix: u8) -> Result<Self, FrameworkError> {
        let (address, prefix, written) = match address {
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) if prefix > 96 && prefix <= 128 => (IpAddr::V4(v4), prefix - 96, address),
                Some(_) => {
                    return Err(FrameworkError::internal(format!(
                        "`{address}/{prefix}` is no range of proxies: an IPv4 range in the \
                         IPv6 form has a prefix from 97 to 128, and 96 is every IPv4 \
                         address. Write the IPv4 form, `10.0.0.0/8`"
                    )));
                }
                None => (address, prefix, address),
            },
            IpAddr::V4(_) => (address, prefix, address),
        };
        let longest = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > longest {
            return Err(FrameworkError::internal(format!(
                "`{written}/{prefix}` is no range of addresses: the prefix of this kind of \
                 address is {longest} bits at most"
            )));
        }
        if prefix == 0 {
            return Err(FrameworkError::internal(format!(
                "`{written}/0` is every address, and no range of proxies: with it every \
                 client is trusted and writes the address the application sees. List the \
                 ranges the proxies connect from"
            )));
        }
        Ok(Self {
            network: masked(address, prefix),
            prefix,
        })
    }

    /// Whether `address` is in the range. An IPv4 address that is
    /// written as an IPv6 one, `::ffff:10.0.0.5`, is the IPv4 address.
    pub fn contains(&self, address: IpAddr) -> bool {
        let address = address.to_canonical();
        match (self.network, address) {
            (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_)) => {
                masked(address, self.prefix) == self.network
            }
            _ => false,
        }
    }

    /// The first address of the range.
    pub fn network(&self) -> IpAddr {
        self.network
    }

    /// The number of bits every address of the range shares.
    pub fn prefix(&self) -> u8 {
        self.prefix
    }
}

/// `address` with every bit behind the first `prefix` bits set to zero.
/// The caller has checked that `prefix` is no longer than the address.
fn masked(address: IpAddr, prefix: u8) -> IpAddr {
    match address {
        IpAddr::V4(v4) => {
            let mask = u32::MAX
                .checked_shl(32 - u32::from(prefix.min(32)))
                .unwrap_or(0);
            IpAddr::V4(Ipv4Addr::from(u32::from(v4) & mask))
        }
        IpAddr::V6(v6) => {
            let mask = u128::MAX
                .checked_shl(128 - u32::from(prefix.min(128)))
                .unwrap_or(0);
            IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask))
        }
    }
}

impl FromStr for ProxyNetwork {
    type Err = FrameworkError;

    /// Read `address/prefix`. An address alone is the range of that one
    /// address.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let not_a_range = || {
            FrameworkError::internal(format!(
                "`{text}` is no address and no range of addresses. Write `10.0.0.5` or \
                 `10.0.0.0/8`"
            ))
        };
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address = address
            .trim()
            .parse::<IpAddr>()
            .map_err(|_| not_a_range())?;
        let prefix = match prefix {
            Some(prefix) => prefix.trim().parse::<u8>().map_err(|_| not_a_range())?,
            None => match address {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            },
        };
        Self::new(address, prefix)
    }
}

impl fmt::Display for ProxyNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

/// An entry of a proxy list that was refused, and the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefusedEntry {
    /// The entry as it was written.
    pub(crate) entry: String,
    /// Why it was refused, as a sentence.
    pub(crate) reason: String,
}

/// Allowlist of TCP peer addresses whose `X-Forwarded-*` / `X-Real-IP`
/// headers may be trusted.
///
/// The default constructor returns an **empty** allowlist - proxy
/// headers are ignored on every request. This is fail-safe: a
/// deployment that forgets to configure trusted proxies cannot have
/// its `Request::ip()` spoofed.
///
/// An entry is one address or a range of addresses ([`ProxyNetwork`]).
/// Both lists are behind an `Arc`, so the config is cheap to clone into
/// every `Request` for the lifetime of the request builder.
#[derive(Debug, Clone, Default)]
pub struct TrustedProxiesConfig {
    proxies: Arc<[IpAddr]>,
    networks: Arc<[ProxyNetwork]>,
}

impl TrustedProxiesConfig {
    /// Construct an empty allowlist - proxy headers ignored on every
    /// request. Equivalent to [`TrustedProxiesConfig::default()`].
    pub fn empty() -> Self {
        Self::default()
    }

    /// Construct an allowlist from an iterator of trusted proxy IPs.
    ///
    /// Duplicate addresses are silently deduplicated (peer-match is
    /// O(N) over the slice; trimming duplicates is free).
    pub fn with_ips<I>(ips: I) -> Self
    where
        I: IntoIterator<Item = IpAddr>,
    {
        let mut v: Vec<IpAddr> = ips.into_iter().collect();
        v.sort();
        v.dedup();
        Self {
            proxies: v.into_boxed_slice().into(),
            networks: Arc::default(),
        }
    }

    /// Trust the addresses of these ranges as well. This is how the edge
    /// of a content delivery network is listed, which has no single
    /// address.
    ///
    /// ```rust
    /// use suprnova::http::{ProxyNetwork, TrustedProxiesConfig};
    ///
    /// # fn main() -> Result<(), suprnova::FrameworkError> {
    /// let trusted = TrustedProxiesConfig::with_ips(["10.0.0.5".parse().unwrap()])
    ///     .and_networks(["173.245.48.0/20".parse::<ProxyNetwork>()?]);
    ///
    /// assert!(trusted.trusts(Some("173.245.49.7".parse().unwrap())));
    /// # Ok(())
    /// # }
    /// ```
    pub fn and_networks<I>(mut self, networks: I) -> Self
    where
        I: IntoIterator<Item = ProxyNetwork>,
    {
        let mut all: Vec<ProxyNetwork> = self.networks.iter().copied().collect();
        all.extend(networks);
        all.sort();
        all.dedup();
        self.networks = all.into_boxed_slice().into();
        self
    }

    /// Read a list of addresses and ranges that are separated by commas,
    /// the form of `APP_TRUSTED_PROXIES`. An empty entry is skipped.
    ///
    /// The error names the entry that was refused and says why, so the
    /// caller can put the variable it came from in front. A range can be
    /// refused for what it is, as `0.0.0.0/0` is, and an error that said
    /// "unparseable" would send the operator looking for a typing error.
    pub(crate) fn from_list(list: &str) -> Result<Self, RefusedEntry> {
        let mut addresses = Vec::new();
        let mut networks = Vec::new();
        for entry in list.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let refused = |reason: String| RefusedEntry {
                entry: entry.to_owned(),
                reason,
            };
            if entry.contains('/') {
                networks.push(
                    entry
                        .parse::<ProxyNetwork>()
                        .map_err(|e| refused(e.message().to_owned()))?,
                );
            } else {
                addresses.push(entry.parse::<IpAddr>().map_err(|_| {
                    refused(format!(
                        "`{entry}` is no IP address. Write an address (10.0.0.5) or a range \
                         in CIDR form (10.0.0.0/8)"
                    ))
                })?);
            }
        }
        Ok(Self::with_ips(addresses).and_networks(networks))
    }

    /// Whether the configured allowlist is empty. Useful for short-
    /// circuiting before the per-request peer match.
    pub fn is_empty(&self) -> bool {
        self.proxies.is_empty() && self.networks.is_empty()
    }

    /// Whether the supplied TCP peer is in the allowlist.
    ///
    /// Used by the [`Request`](crate::Request) accessors to gate
    /// proxy-header trust on each call. A request with no recorded
    /// peer (i.e. `peer_addr == None`, common in unit tests that build
    /// a `Request` directly) is treated as untrusted.
    ///
    /// An IPv4 address that is written as an IPv6 one,
    /// `::ffff:10.0.0.5`, is the IPv4 address: that is what a peer looks
    /// like on a socket that listens on both.
    pub fn trusts(&self, peer: Option<IpAddr>) -> bool {
        let Some(peer) = peer else { return false };
        let canonical = peer.to_canonical();
        self.proxies
            .iter()
            .any(|proxy| *proxy == peer || proxy.to_canonical() == canonical)
            || self.networks.iter().any(|network| network.contains(peer))
    }

    /// Read-only view of the single addresses of the allowlist, sorted
    /// and without duplicates. The ranges are in
    /// [`networks`](Self::networks).
    pub fn proxies(&self) -> &[IpAddr] {
        &self.proxies
    }

    /// Read-only view of the ranges, sorted and without duplicates. The
    /// single addresses are in [`proxies`](Self::proxies).
    pub fn networks(&self) -> &[ProxyNetwork] {
        &self.networks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowlist_trusts_no_peer() {
        let cfg = TrustedProxiesConfig::empty();
        assert!(!cfg.trusts(Some("127.0.0.1".parse().unwrap())));
        assert!(!cfg.trusts(None));
    }

    #[test]
    fn with_ips_trusts_listed_peers() {
        let cfg = TrustedProxiesConfig::with_ips([
            "127.0.0.1".parse().unwrap(),
            "10.0.0.1".parse().unwrap(),
        ]);
        assert!(cfg.trusts(Some("127.0.0.1".parse().unwrap())));
        assert!(cfg.trusts(Some("10.0.0.1".parse().unwrap())));
        assert!(!cfg.trusts(Some("8.8.8.8".parse().unwrap())));
    }

    #[test]
    fn no_peer_is_never_trusted() {
        let cfg = TrustedProxiesConfig::with_ips(["127.0.0.1".parse().unwrap()]);
        assert!(!cfg.trusts(None));
    }

    #[test]
    fn dedups_and_sorts_inputs() {
        let cfg = TrustedProxiesConfig::with_ips([
            "127.0.0.1".parse().unwrap(),
            "10.0.0.1".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
        ]);
        assert_eq!(cfg.proxies().len(), 2);
        assert!(cfg.trusts(Some("127.0.0.1".parse().unwrap())));
        assert!(cfg.trusts(Some("10.0.0.1".parse().unwrap())));
    }

    #[test]
    fn is_empty_reflects_allowlist_state() {
        assert!(TrustedProxiesConfig::empty().is_empty());
        assert!(!TrustedProxiesConfig::with_ips(["127.0.0.1".parse().unwrap()]).is_empty());
        assert!(
            !TrustedProxiesConfig::empty()
                .and_networks(["10.0.0.0/8".parse().unwrap()])
                .is_empty()
        );
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn a_range_holds_its_addresses_and_no_others() {
        let range: ProxyNetwork = "173.245.48.0/20".parse().unwrap();

        assert!(range.contains(ip("173.245.48.0")));
        assert!(range.contains(ip("173.245.63.255")));
        assert!(!range.contains(ip("173.245.64.0")));
        assert!(!range.contains(ip("173.245.47.255")));
        assert!(!range.contains(ip("2001:db8::1")));

        let v6: ProxyNetwork = "2400:cb00::/32".parse().unwrap();
        assert!(v6.contains(ip("2400:cb00:1234::1")));
        assert!(!v6.contains(ip("2400:cb01::1")));
        assert!(!v6.contains(ip("36.0.203.0")));
    }

    #[test]
    fn the_bits_behind_the_prefix_are_dropped() {
        let range: ProxyNetwork = "10.1.2.3/8".parse().unwrap();
        assert_eq!(range.to_string(), "10.0.0.0/8");
        assert_eq!(range.network(), ip("10.0.0.0"));
        assert_eq!(range.prefix(), 8);
        assert_eq!(range, "10.200.0.0/8".parse().unwrap());
    }

    #[test]
    fn the_shortest_and_the_longest_prefix() {
        let half: ProxyNetwork = "128.0.0.0/1".parse().unwrap();
        assert!(half.contains(ip("203.0.113.9")));
        assert!(!half.contains(ip("127.255.255.255")));
        assert!(!half.contains(ip("2001:db8::1")));

        let one: ProxyNetwork = "203.0.113.9/32".parse().unwrap();
        assert!(one.contains(ip("203.0.113.9")));
        assert!(!one.contains(ip("203.0.113.8")));

        let one_v6: ProxyNetwork = "2001:db8::1/128".parse().unwrap();
        assert!(one_v6.contains(ip("2001:db8::1")));
        assert!(!one_v6.contains(ip("2001:db8::2")));

        let alone: ProxyNetwork = "203.0.113.9".parse().unwrap();
        assert_eq!(alone, one, "an address alone is the range of that address");
    }

    #[test]
    fn the_range_of_every_address_is_refused() {
        // With it every client is a trusted proxy, and a client that
        // connects with no proxy in front writes the address it is
        // taken for.
        for text in ["0.0.0.0/0", "::/0", "10.0.0.0/0"] {
            let error = text.parse::<ProxyNetwork>().expect_err("every address");
            assert!(
                error.message().contains("every address"),
                "`{text}`: {}",
                error.message()
            );
        }
        assert!(TrustedProxiesConfig::from_list("10.0.0.5, 0.0.0.0/0").is_err());
    }

    #[test]
    fn an_ipv4_range_in_the_ipv6_form_is_the_ipv4_range() {
        let mapped: ProxyNetwork = "::ffff:10.0.0.0/104".parse().unwrap();
        assert_eq!(mapped, "10.0.0.0/8".parse().unwrap());
        assert_eq!(mapped.to_string(), "10.0.0.0/8");
        assert!(mapped.contains(ip("10.9.9.9")));
        assert!(mapped.contains(ip("::ffff:10.9.9.9")));
        assert!(!mapped.contains(ip("11.0.0.1")));

        let one: ProxyNetwork = "::ffff:10.0.0.5".parse().unwrap();
        assert_eq!(one, "10.0.0.5/32".parse().unwrap());

        // The first and the last prefix of the form.
        let half: ProxyNetwork = "::ffff:0.0.0.0/97".parse().unwrap();
        assert_eq!(half.to_string(), "0.0.0.0/1");
        let last: ProxyNetwork = "::ffff:10.0.0.5/128".parse().unwrap();
        assert_eq!(last.to_string(), "10.0.0.5/32");
        assert!(
            "::ffff:0:0/96".parse::<ProxyNetwork>().is_err(),
            "the 96 bits in front are the same for every IPv4 address"
        );

        // `/8` of the IPv6 form is no range of IPv4 addresses, and
        // reading it as the IPv4 `/8` would be a guess.
        let error = "::ffff:10.0.0.0/8"
            .parse::<ProxyNetwork>()
            .expect_err("no IPv4 range");
        assert!(
            error.message().contains("::ffff:10.0.0.0/8"),
            "the error shows what was typed: {}",
            error.message()
        );
    }

    #[test]
    fn what_is_no_range_is_refused() {
        for text in [
            "10.0.0.0/33",
            "2001:db8::/129",
            "10.0.0.0/",
            "10.0.0/8",
            "/8",
            "ten/8",
            "10.0.0.0/-1",
            "10.0.0.0/8/8",
        ] {
            assert!(
                text.parse::<ProxyNetwork>().is_err(),
                "`{text}` must be refused"
            );
        }
    }

    #[test]
    fn an_ipv4_peer_written_as_ipv6_is_the_ipv4_peer() {
        let cfg = TrustedProxiesConfig::with_ips([ip("10.0.0.5")])
            .and_networks(["192.168.0.0/16".parse().unwrap()]);

        assert!(cfg.trusts(Some(ip("::ffff:10.0.0.5"))));
        assert!(cfg.trusts(Some(ip("::ffff:192.168.7.7"))));
        assert!(!cfg.trusts(Some(ip("::ffff:10.0.0.6"))));
    }

    #[test]
    fn a_list_holds_addresses_and_ranges() {
        let cfg = TrustedProxiesConfig::from_list(" 10.0.0.5, 173.245.48.0/20 ,,2400:cb00::/32 ")
            .expect("the list reads");

        assert_eq!(cfg.proxies(), [ip("10.0.0.5")]);
        assert_eq!(cfg.networks().len(), 2);
        assert!(cfg.trusts(Some(ip("10.0.0.5"))));
        assert!(cfg.trusts(Some(ip("173.245.50.1"))));
        assert!(cfg.trusts(Some(ip("2400:cb00::7"))));
        assert!(!cfg.trusts(Some(ip("10.0.0.6"))));

        let too_long = TrustedProxiesConfig::from_list("10.0.0.5, 10.0.0.0/40").unwrap_err();
        assert_eq!(too_long.entry, "10.0.0.0/40");
        assert!(too_long.reason.contains("32 bits"), "{}", too_long.reason);

        let a_name = TrustedProxiesConfig::from_list("10.0.0.5, proxy.internal").unwrap_err();
        assert_eq!(a_name.entry, "proxy.internal");
        assert!(a_name.reason.contains("no IP address"), "{}", a_name.reason);

        let everything = TrustedProxiesConfig::from_list("10.0.0.5, 0.0.0.0/0").unwrap_err();
        assert_eq!(everything.entry, "0.0.0.0/0");
        assert!(
            everything.reason.contains("every address"),
            "the reason is the one the range was refused for: {}",
            everything.reason
        );
        assert!(
            TrustedProxiesConfig::from_list("")
                .expect("an empty list")
                .is_empty()
        );
    }
}
