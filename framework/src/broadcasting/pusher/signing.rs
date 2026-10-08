//! Pusher request signing.
//!
//! Pure functions over the Pusher signing contract: the channel and user
//! authorization strings, and the signed query of a REST call. They do
//! no I/O so the documented test vectors can pin them exactly.

use crate::FrameworkError;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use md5::{Digest, Md5};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Lowercase hex HMAC-SHA256 of `message` under `secret`.
///
/// HMAC accepts a key of any length, so the error branch cannot fire;
/// it exists so this path returns an error instead of panicking. Its
/// message names no input.
fn hmac_hex(secret: &str, message: &str) -> Result<String, FrameworkError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| FrameworkError::internal("Pusher signing: the HMAC key was refused"))?;
    mac.update(message.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// The signed query string of one `POST /apps/{app_id}/events` call.
///
/// The signature is kept apart from the rest of the query so the hub
/// can scrub it from an error excerpt before the excerpt reaches a log.
pub(crate) struct SignedQuery {
    /// `auth_key`, `auth_timestamp`, `auth_version` and `body_md5`,
    /// sorted by key and joined `k=v&...`. This is the exact string the
    /// signature covers.
    pub(crate) unsigned: String,
    /// Lowercase hex HMAC-SHA256 of the string to sign.
    pub(crate) signature: String,
}

impl SignedQuery {
    /// The query to send: the signed parameters with `auth_signature`
    /// appended last, as the Pusher REST API expects.
    pub(crate) fn to_query(&self) -> String {
        format!("{}&auth_signature={}", self.unsigned, self.signature)
    }
}

/// Sign a channel subscription: `{key}:{hex hmac}` over
/// `{socket_id}:{channel_name}`, or over
/// `{socket_id}:{channel_name}:{channel_data}` for a presence channel.
pub(crate) fn channel_auth(
    key: &str,
    secret: &str,
    socket_id: &str,
    channel_name: &str,
    channel_data: Option<&str>,
) -> Result<String, FrameworkError> {
    let message = match channel_data {
        Some(data) => format!("{socket_id}:{channel_name}:{data}"),
        None => format!("{socket_id}:{channel_name}"),
    };
    Ok(format!("{key}:{}", hmac_hex(secret, &message)?))
}

/// Sign a user authentication: `{key}:{hex hmac}` over
/// `{socket_id}::user::{user_data}`.
pub(crate) fn user_auth(
    key: &str,
    secret: &str,
    socket_id: &str,
    user_data: &str,
) -> Result<String, FrameworkError> {
    let message = format!("{socket_id}::user::{user_data}");
    Ok(format!("{key}:{}", hmac_hex(secret, &message)?))
}

/// Sign the query of `POST /apps/{app_id}/events` for this exact body.
///
/// `body_md5` binds the signature to the body bytes, so the caller must
/// send exactly the bytes it passes here.
pub(crate) fn events_query(
    key: &str,
    secret: &str,
    app_id: &str,
    timestamp: u64,
    body: &[u8],
) -> Result<SignedQuery, FrameworkError> {
    let body_md5 = hex::encode(Md5::digest(body));
    let timestamp = timestamp.to_string();
    let mut params = [
        ("auth_key", key),
        ("auth_timestamp", timestamp.as_str()),
        ("auth_version", "1.0"),
        ("body_md5", body_md5.as_str()),
    ];
    params.sort_by_key(|(name, _)| *name);
    let unsigned = params
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let signature = hmac_hex(secret, &format!("POST\n/apps/{app_id}/events\n{unsigned}"))?;
    Ok(SignedQuery {
        unsigned,
        signature,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "278d425bdf160c739803";
    const SECRET: &str = "7ad3773142a6692b25b8";

    #[test]
    fn pusher_signs_private_channel_with_documented_vector() {
        // Pusher's own documented example.
        let auth = channel_auth(KEY, SECRET, "1234.1234", "private-foobar", None).unwrap();
        assert_eq!(
            auth,
            "278d425bdf160c739803:58df8b0c36d6982b82c3ecf6b4662e34fe8c25bba48f5369f135bf843651c3a4"
        );
    }

    #[test]
    fn pusher_signs_presence_channel_over_channel_data() {
        let channel_data = r#"{"user_id":"10","user_info":{"name":"Mr. Channels"}}"#;
        let auth = channel_auth(
            KEY,
            SECRET,
            "1234.1234",
            "presence-foobar",
            Some(channel_data),
        )
        .unwrap();
        assert_eq!(
            auth,
            "278d425bdf160c739803:4c6d8fc42a207ba96a0779844171b0bb819d96ffceef9609f5cce596ab17a800"
        );
    }

    #[test]
    fn pusher_signs_user_auth() {
        let auth = user_auth(KEY, SECRET, "1234.1234", r#"{"id":"42"}"#).unwrap();
        assert_eq!(
            auth,
            "278d425bdf160c739803:23467c4156bacf702560be3fdf30b8bf0b44ca003cfca6223784b7bb763bd209"
        );
    }

    #[test]
    fn pusher_signs_rest_events_query() {
        let body =
            br#"{"name":"OrderShipped","channels":["private-orders.42"],"data":"{\"id\":42}"}"#;
        let signed = events_query(KEY, SECRET, "3", 1_700_000_000, body).unwrap();
        assert_eq!(
            signed.unsigned,
            "auth_key=278d425bdf160c739803&auth_timestamp=1700000000&auth_version=1.0\
             &body_md5=578fc8c8554786c7f71919898e5b9e79"
        );
        assert_eq!(
            signed.signature,
            "927d37a56401cbf139d27b5fbfea241b0b3edd53d143efb127106ba258b697c5"
        );
        assert_eq!(
            signed.to_query(),
            format!(
                "{}&auth_signature=927d37a56401cbf139d27b5fbfea241b0b3edd53d143efb127106ba258b697c5",
                signed.unsigned
            )
        );
    }
}
