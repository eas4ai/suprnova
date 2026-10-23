//! A cap on the connections one client address holds open.
//!
//! [`RateLimitMiddleware`](super::RateLimitMiddleware) limits how often a
//! client may ask. It does not limit how much a client holds: a WebSocket
//! is asked for once and then stays open. The server's own connection cap
//! is one number for every client together, so one address that opens
//! sockets and keeps them can use all of it and lock every other client
//! out of HTTP and WebSocket alike. [`ConnectionsPerIp`] gives each address
//! a number of its own.

use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{Middleware, Next};
use async_trait::async_trait;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex};

/// Middleware that allows each client address a number of open connections
/// and refuses the next one with `429 Too Many Requests`. Build it with
/// [`RateLimitMiddleware::connections_per_ip`](super::RateLimitMiddleware::connections_per_ip).
///
/// ```rust,no_run
/// # use suprnova::{ws, async_trait, FrameworkError, Request};
/// # use suprnova::ws::{WebSocketHandler, WsSocket};
/// # struct Broadcast;
/// # #[async_trait]
/// # impl WebSocketHandler for Broadcast {
/// #     async fn handle(&self, _socket: WsSocket, _req: Request) -> Result<(), FrameworkError> { Ok(()) }
/// # }
/// use suprnova::rate_limit::RateLimitMiddleware;
///
/// ws!("/ws/broadcast", Broadcast).middleware(RateLimitMiddleware::connections_per_ip(100));
/// ```
///
/// # What is counted
///
/// On a WebSocket route, a socket from the moment this middleware lets
/// the upgrade pass until the session of the socket ends: the handler has
/// returned and the close handshake is done. The count rides on the
/// request ([`Request::hold_for_connection`]) into the task that runs the
/// socket. An upgrade that a later middleware refuses gives its place
/// back at once. A peer that goes away without closing the connection
/// holds its place until the handler notices, which is when a read or a
/// write of its socket fails.
///
/// On any other route, a request while it is handled. The place lives as
/// long as the request value, and it is given back when the handler drops
/// or consumes the request, which reading the body does. So the cap does
/// not count a connection that is kept alive and idle, and a streamed
/// response, server-sent events for one, outlives its place. This is a
/// cap for WebSocket routes.
///
/// # Whose count
///
/// The address is [`Request::ip`], which reads the forwarded headers of a
/// trusted proxy and of nobody else. An IPv4 address is counted by
/// itself. An IPv6 address is counted with its /64 network: a subscriber
/// line is given a /64 or more, and a count for each of its addresses
/// would be no cap at all.
///
/// A request with no address to resolve is not counted. The server gives
/// every connection it accepts the address of its peer. An embedder that
/// runs its own accept loop has to do the same, with
/// [`handle_request_with_peer`](crate::server::handle_request_with_peer):
/// through `handle_request` no request has an address, and nothing is
/// capped.
///
/// The counts are kept in this process. That is the right place for them:
/// the sockets an address holds in a process are what that process runs
/// out of. A clone shares the counts of the middleware it was cloned from,
/// so one cap can guard several routes, and an application can keep a
/// clone to read [`Self::open_for`].
#[derive(Clone)]
pub struct ConnectionsPerIp {
    max: usize,
    open: Arc<Mutex<HashMap<String, usize>>>,
}

impl ConnectionsPerIp {
    pub(super) fn new(max: usize) -> Self {
        Self {
            max,
            open: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The connections `address` holds open through this cap right now.
    /// For an IPv6 address that is the count of its /64 network.
    pub fn open_for(&self, address: &str) -> usize {
        counts(&self.open)
            .get(&counted_as(address))
            .copied()
            .unwrap_or(0)
    }

    /// Count one more connection for `address`, unless it is at the cap.
    fn admit(&self, address: String) -> Option<Held> {
        let address = counted_as(&address);
        let mut open = counts(&self.open);
        let held = open.entry(address.clone()).or_insert(0);
        if *held >= self.max {
            // An address at a cap of zero leaves an entry of zero behind.
            if *held == 0 {
                open.remove(&address);
            }
            return None;
        }
        *held += 1;
        Some(Held {
            open: Arc::clone(&self.open),
            address,
        })
    }
}

/// What `address` is counted as: itself when it is an IPv4 address, and
/// its /64 network when it is an IPv6 address. An IPv4 address that is
/// written as an IPv6 one, `::ffff:203.0.113.1`, is the IPv4 address.
fn counted_as(address: &str) -> String {
    match address.parse::<IpAddr>() {
        Ok(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let [a, b, c, d, ..] = v6.segments();
                format!("{}/64", Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
            }
        },
        Ok(IpAddr::V4(v4)) => v4.to_string(),
        // `Request::ip` returns addresses alone. A text that is none is
        // what a caller of `open_for` typed, and it is counted as typed.
        Err(_) => address.to_owned(),
    }
}

/// The counts, whatever happened to the last holder of the lock. A count
/// is a number that is incremented and decremented under the lock, so a
/// panic elsewhere cannot leave it half written, and refusing to count
/// after one would turn the cap off for good.
fn counts(
    open: &Mutex<HashMap<String, usize>>,
) -> std::sync::MutexGuard<'_, HashMap<String, usize>> {
    open.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One counted connection. Dropping it gives the place back.
struct Held {
    open: Arc<Mutex<HashMap<String, usize>>>,
    address: String,
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut open = counts(&self.open);
        if let Some(held) = open.get_mut(&self.address) {
            *held = held.saturating_sub(1);
            // An address with nothing open leaves no entry, so the map is
            // as large as the number of addresses connected right now.
            if *held == 0 {
                open.remove(&self.address);
            }
        }
    }
}

#[async_trait]
impl Middleware for ConnectionsPerIp {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        let Some(address) = request.ip() else {
            return next(request).await;
        };
        match self.admit(address) {
            Some(held) => {
                request.hold_for_connection(held);
                next(request).await
            }
            None => Err(HttpResponse::text("429 Too Many Requests").status(429)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_admitted_up_to_the_cap_and_again_when_a_place_is_given_back() {
        let cap = ConnectionsPerIp::new(2);
        let first = cap.admit("203.0.113.1".into()).expect("the first place");
        let second = cap.admit("203.0.113.1".into()).expect("the second place");

        assert!(cap.admit("203.0.113.1".into()).is_none(), "the cap is 2");
        assert_eq!(cap.open_for("203.0.113.1"), 2);
        assert!(
            cap.admit("203.0.113.2".into()).is_some(),
            "another address has places of its own"
        );

        drop(first);
        assert_eq!(cap.open_for("203.0.113.1"), 1);
        let third = cap
            .admit("203.0.113.1".into())
            .expect("a place was given back");

        drop(second);
        drop(third);
        assert_eq!(cap.open_for("203.0.113.1"), 0);
        assert!(
            !counts(&cap.open).contains_key("203.0.113.1"),
            "an address with nothing open must leave no entry behind"
        );
    }

    #[test]
    fn a_clone_shares_the_counts() {
        let cap = ConnectionsPerIp::new(1);
        let watching = cap.clone();

        let held = cap.admit("203.0.113.1".into()).expect("the one place");
        assert_eq!(watching.open_for("203.0.113.1"), 1);
        assert!(watching.admit("203.0.113.1".into()).is_none());

        drop(held);
        assert_eq!(watching.open_for("203.0.113.1"), 0);
    }

    #[test]
    fn the_addresses_of_one_ipv6_network_share_a_count() {
        let cap = ConnectionsPerIp::new(2);
        let _first = cap.admit("2001:db8:1:2::1".into()).expect("a place");
        let _second = cap
            .admit("2001:db8:1:2:ffff:ffff:ffff:ffff".into())
            .expect("a place");

        assert!(
            cap.admit("2001:db8:1:2::3".into()).is_none(),
            "a client that has a /64 has 2^64 addresses, and one cap"
        );
        assert_eq!(cap.open_for("2001:db8:1:2::99"), 2);
        assert!(
            cap.admit("2001:db8:1:3::1".into()).is_some(),
            "the next network has places of its own"
        );
    }

    #[test]
    fn an_ipv4_address_written_as_ipv6_is_the_ipv4_address() {
        let cap = ConnectionsPerIp::new(1);
        let _held = cap.admit("203.0.113.1".into()).expect("the one place");

        assert!(cap.admit("::ffff:203.0.113.1".into()).is_none());
        assert_eq!(cap.open_for("::ffff:203.0.113.1"), 1);
        assert!(
            cap.admit("203.0.113.2".into()).is_some(),
            "an IPv4 address is counted by itself"
        );
    }

    #[test]
    fn a_cap_of_zero_admits_nobody_and_keeps_no_entry() {
        let cap = ConnectionsPerIp::new(0);
        assert!(cap.admit("203.0.113.1".into()).is_none());
        assert!(counts(&cap.open).is_empty());
    }
}
