//! Wire names for the Pusher-protocol driver.
//!
//! One function decides the wire name so publishing and authorization
//! agree. The registry is the source of truth for what a channel is.

use crate::FrameworkError;
use crate::broadcasting::ChannelRegistry;
use crate::broadcasting::ChannelVisibility;

/// The kind of channel a wire name denotes, read from its prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WireKind {
    /// `private-encrypted-`
    Encrypted,
    /// `presence-`
    Presence,
    /// `private-`
    Private,
}

/// Split a wire name into its kind and the application channel name,
/// the way Pusher clients and the authorization endpoint read it. The
/// order matters: `private-encrypted-` must be tried before `private-`,
/// which is its prefix. A bare name is a public channel, so it yields
/// `None`.
pub(crate) fn strip_prefix(wire: &str) -> Option<(WireKind, &str)> {
    if let Some(rest) = wire.strip_prefix("private-encrypted-") {
        return Some((WireKind::Encrypted, rest));
    }
    if let Some(rest) = wire.strip_prefix("presence-") {
        return Some((WireKind::Presence, rest));
    }
    wire.strip_prefix("private-")
        .map(|rest| (WireKind::Private, rest))
}

/// Decide the Pusher wire name for an application channel name.
///
/// Presence channels always use `presence-` whatever `visibility`
/// returns. Otherwise the visibility decides: public stays bare,
/// private gains `private-`, encrypted gains `private-encrypted-`.
/// Unknown channels fail closed to `private-` since authorization
/// refuses them before publishing.
///
/// # Errors
///
/// Refuses any name whose wire name would not read back, through
/// [`strip_prefix`], as this same name and kind:
///
/// - a public name starting with `private-` or `presence-`;
/// - a private or unregistered name starting with `encrypted-`, which
///   would read as the encrypted channel without that prefix;
/// - a presence channel whose visibility is `Encrypted`, because Pusher
///   has no encrypted presence channels and `presence-` would carry
///   plaintext.
///
/// Because `strip_prefix` inverts every name this returns, no two
/// channels can share a wire name. Without the check, authorizing the
/// encrypted `x` would hand out the key that decrypts a private
/// `encrypted-x`, and a public `private-x` would publish into the
/// private channel `x`.
pub(crate) fn wire_name(
    registry: &ChannelRegistry,
    channel: &str,
) -> Result<String, FrameworkError> {
    let (kind, wire) = match registry.resolve(channel) {
        None => (Some(WireKind::Private), format!("private-{channel}")),
        Some((resolved, _params)) if resolved.presence_info().is_some() => {
            if resolved.visibility() == ChannelVisibility::Encrypted {
                return Err(FrameworkError::internal(format!(
                    "Pusher channel '{channel}' is a presence channel with Encrypted \
                     visibility; Pusher has no encrypted presence channels, so it is refused \
                     rather than sent as plaintext"
                )));
            }
            (Some(WireKind::Presence), format!("presence-{channel}"))
        }
        Some((resolved, _params)) => match resolved.visibility() {
            ChannelVisibility::Public => (None, channel.to_string()),
            ChannelVisibility::Private => (Some(WireKind::Private), format!("private-{channel}")),
            ChannelVisibility::Encrypted => (
                Some(WireKind::Encrypted),
                format!("private-encrypted-{channel}"),
            ),
        },
    };
    if strip_prefix(&wire) != kind.map(|kind| (kind, channel)) {
        return Err(FrameworkError::internal(format!(
            "Pusher channel '{channel}' is refused: its wire name '{wire}' reads back as a \
             different channel. A public channel may not start with `private-` or \
             `presence-`, and a private channel may not start with `encrypted-`"
        )));
    }
    Ok(wire)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcasting::Channel;
    use async_trait::async_trait;
    use serde_json::Value;

    struct PublicChan;
    #[async_trait]
    impl Channel for PublicChan {
        fn name(&self) -> &'static str {
            "orders"
        }
        fn visibility(&self) -> ChannelVisibility {
            ChannelVisibility::Public
        }
    }

    struct PrivateChan;
    #[async_trait]
    impl Channel for PrivateChan {
        fn name(&self) -> &'static str {
            "orders"
        }
    }

    struct EncryptedChan;
    #[async_trait]
    impl Channel for EncryptedChan {
        fn name(&self) -> &'static str {
            "orders.42"
        }
        fn visibility(&self) -> ChannelVisibility {
            ChannelVisibility::Encrypted
        }
    }

    struct PresenceChan;
    #[async_trait]
    impl Channel for PresenceChan {
        fn name(&self) -> &'static str {
            "chat"
        }
        fn presence_info(&self) -> Option<&dyn crate::broadcasting::PresenceChannel> {
            Some(self)
        }
    }

    #[async_trait]
    impl crate::broadcasting::PresenceChannel for PresenceChan {
        async fn member_info(
            &self,
            _req: &crate::http::Request,
            _params: &crate::broadcasting::ChannelParams,
        ) -> Result<Value, crate::FrameworkError> {
            Ok(serde_json::json!({}))
        }
    }

    struct PresencePublicChan;
    #[async_trait]
    impl Channel for PresencePublicChan {
        fn name(&self) -> &'static str {
            "lobby"
        }
        fn visibility(&self) -> ChannelVisibility {
            ChannelVisibility::Public
        }
        fn presence_info(&self) -> Option<&dyn crate::broadcasting::PresenceChannel> {
            Some(self)
        }
    }

    #[async_trait]
    impl crate::broadcasting::PresenceChannel for PresencePublicChan {
        async fn member_info(
            &self,
            _req: &crate::http::Request,
            _params: &crate::broadcasting::ChannelParams,
        ) -> Result<Value, crate::FrameworkError> {
            Ok(serde_json::json!({}))
        }
    }

    /// A channel with a chosen name and visibility.
    struct Named(&'static str, ChannelVisibility);
    #[async_trait]
    impl Channel for Named {
        fn name(&self) -> &'static str {
            self.0
        }
        fn visibility(&self) -> ChannelVisibility {
            self.1
        }
    }

    /// A presence channel with a chosen name and visibility.
    struct NamedPresence(&'static str, ChannelVisibility);
    #[async_trait]
    impl Channel for NamedPresence {
        fn name(&self) -> &'static str {
            self.0
        }
        fn visibility(&self) -> ChannelVisibility {
            self.1
        }
        fn presence_info(&self) -> Option<&dyn crate::broadcasting::PresenceChannel> {
            Some(self)
        }
    }
    #[async_trait]
    impl crate::broadcasting::PresenceChannel for NamedPresence {
        async fn member_info(
            &self,
            _req: &crate::http::Request,
            _params: &crate::broadcasting::ChannelParams,
        ) -> Result<Value, crate::FrameworkError> {
            Ok(serde_json::json!({}))
        }
    }

    fn registry_with<C: Channel + 'static>(channel: C) -> ChannelRegistry {
        let mut registry = ChannelRegistry::new();
        registry.register(channel);
        registry
    }

    #[test]
    fn pusher_public_channel_maps_bare() {
        let registry = registry_with(PublicChan);
        assert_eq!(wire_name(&registry, "orders").unwrap(), "orders");
    }

    #[test]
    fn pusher_private_channel_maps_with_prefix() {
        let registry = registry_with(PrivateChan);
        assert_eq!(wire_name(&registry, "orders").unwrap(), "private-orders");
    }

    #[test]
    fn pusher_encrypted_channel_maps_with_encrypted_prefix() {
        let registry = registry_with(EncryptedChan);
        assert_eq!(
            wire_name(&registry, "orders.42").unwrap(),
            "private-encrypted-orders.42"
        );
    }

    #[test]
    fn pusher_presence_channel_maps_with_presence_prefix() {
        let registry = registry_with(PresenceChan);
        assert_eq!(wire_name(&registry, "chat").unwrap(), "presence-chat");
    }

    #[test]
    fn pusher_presence_wins_over_public_visibility() {
        let registry = registry_with(PresencePublicChan);
        assert_eq!(wire_name(&registry, "lobby").unwrap(), "presence-lobby");
    }

    #[test]
    fn pusher_unknown_channel_fails_closed_to_private() {
        let registry = ChannelRegistry::new();
        assert_eq!(wire_name(&registry, "nope").unwrap(), "private-nope");
    }

    #[test]
    fn pusher_strip_prefix_tries_encrypted_before_private() {
        assert_eq!(
            strip_prefix("private-encrypted-vault.1"),
            Some((WireKind::Encrypted, "vault.1"))
        );
        assert_eq!(
            strip_prefix("presence-chat"),
            Some((WireKind::Presence, "chat"))
        );
        assert_eq!(
            strip_prefix("private-orders.42"),
            Some((WireKind::Private, "orders.42"))
        );
        assert_eq!(
            strip_prefix("presence-private-x"),
            Some((WireKind::Presence, "private-x"))
        );
        assert_eq!(strip_prefix("news"), None);
        assert_eq!(strip_prefix("encrypted-x"), None);
    }

    #[test]
    fn pusher_private_name_that_would_read_as_encrypted_is_refused() {
        // `encrypted-x` (private) and `x` (encrypted) would both be
        // `private-encrypted-x`, and authorizing `x` would hand out the
        // key to `encrypted-x`'s events.
        let mut registry = ChannelRegistry::new();
        registry.register(Named("encrypted-x", ChannelVisibility::Private));
        registry.register(Named("x", ChannelVisibility::Encrypted));
        assert!(wire_name(&registry, "encrypted-x").is_err());
        assert_eq!(
            wire_name(&registry, "x").unwrap(),
            "private-encrypted-x",
            "the encrypted channel keeps its name"
        );
    }

    #[test]
    fn pusher_public_name_with_a_reserved_prefix_is_refused() {
        for name in ["private-x", "presence-x", "private-encrypted-x"] {
            let registry = registry_with(Named(name, ChannelVisibility::Public));
            let err = wire_name(&registry, name).expect_err("would read as a private name");
            assert!(err.to_string().contains(name), "{err}");
        }
    }

    #[test]
    fn pusher_unknown_name_that_would_read_as_encrypted_is_refused() {
        let registry = ChannelRegistry::new();
        assert!(wire_name(&registry, "encrypted-x").is_err());
    }

    #[test]
    fn pusher_pattern_parameter_cannot_forge_a_prefix() {
        let registry = registry_with(Named("{slug}.orders", ChannelVisibility::Private));
        assert!(wire_name(&registry, "encrypted-foo.orders").is_err());
        assert_eq!(
            wire_name(&registry, "foo.orders").unwrap(),
            "private-foo.orders"
        );

        let registry = registry_with(Named("{slug}.news", ChannelVisibility::Public));
        assert!(wire_name(&registry, "private-foo.news").is_err());
        assert!(wire_name(&registry, "presence-foo.news").is_err());
        assert_eq!(wire_name(&registry, "foo.news").unwrap(), "foo.news");
    }

    #[test]
    fn pusher_encrypted_presence_channel_is_refused() {
        // Pusher has no encrypted presence channels: publishing it as
        // `presence-` would send plaintext.
        let registry = registry_with(NamedPresence("room", ChannelVisibility::Encrypted));
        let err = wire_name(&registry, "room").expect_err("no encrypted presence");
        assert!(err.to_string().contains("presence"), "{err}");
    }

    #[test]
    fn pusher_safe_prefixes_still_round_trip() {
        let cases = [
            (
                registry_with(Named("private-x", ChannelVisibility::Private)),
                "private-x",
                "private-private-x",
            ),
            (
                registry_with(Named("presence-x", ChannelVisibility::Private)),
                "presence-x",
                "private-presence-x",
            ),
            (
                registry_with(Named("encrypted-x", ChannelVisibility::Encrypted)),
                "encrypted-x",
                "private-encrypted-encrypted-x",
            ),
            (
                registry_with(NamedPresence("private-x", ChannelVisibility::Private)),
                "private-x",
                "presence-private-x",
            ),
            (
                registry_with(Named("encrypted", ChannelVisibility::Public)),
                "encrypted",
                "encrypted",
            ),
        ];
        for (registry, name, wire) in cases {
            assert_eq!(wire_name(&registry, name).unwrap(), wire, "{name}");
        }
    }

    #[test]
    fn pusher_no_two_channels_share_a_wire_name() {
        // Every prefix combination, under every kind of channel: a wire
        // name the function accepts must belong to exactly one name.
        let names = [
            "x",
            "private-x",
            "presence-x",
            "encrypted-x",
            "private-encrypted-x",
            "presence-private-x",
            "private-presence-x",
            "encrypted-private-x",
            "private-private-x",
            "presence-presence-x",
            "private-encrypted-encrypted-x",
        ];
        let mut owners: std::collections::HashMap<String, (&str, &str)> =
            std::collections::HashMap::new();
        for name in names {
            let kinds: [(&str, ChannelRegistry); 6] = [
                (
                    "public",
                    registry_with(Named(name, ChannelVisibility::Public)),
                ),
                (
                    "private",
                    registry_with(Named(name, ChannelVisibility::Private)),
                ),
                (
                    "encrypted",
                    registry_with(Named(name, ChannelVisibility::Encrypted)),
                ),
                (
                    "presence",
                    registry_with(NamedPresence(name, ChannelVisibility::Private)),
                ),
                (
                    "public presence",
                    registry_with(NamedPresence(name, ChannelVisibility::Public)),
                ),
                ("unregistered", ChannelRegistry::new()),
            ];
            for (kind, registry) in kinds {
                let Ok(wire) = wire_name(&registry, name) else {
                    continue;
                };
                // Presence wins over visibility, so the two presence
                // kinds share a wire name for the same channel name.
                let family = match kind {
                    "public presence" => "presence",
                    "unregistered" => "private",
                    other => other,
                };
                if let Some((owner, owner_family)) = owners.get(&wire) {
                    assert!(
                        *owner == name && *owner_family == family,
                        "{wire} is claimed by {owner} ({owner_family}) and {name} ({family})"
                    );
                } else {
                    owners.insert(wire, (name, family));
                }
            }
        }
    }
}
