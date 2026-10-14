//! Pusher request signing.
//!
//! Pure functions over the Pusher signing contract: the channel and user
//! authorization strings, and the signed query of a REST call. They do
//! no I/O so the documented test vectors can pin them exactly.
//!
//! The REST signature follows Pusher's HTTP API documentation, which is not
//! in the Laravel reference tree: the string to sign is the method, the
//! path and the query, one per line, where the query is every parameter
//! but `auth_signature`, keys in lowercase, sorted by key and joined
//! `key=value&...` without URL escaping. `body_md5` joins them only for a
//! request with a body.

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

/// The signed query string of one REST call.
///
/// The signature is kept apart from the rest of the query so the client
/// can scrub it from an error excerpt before the excerpt reaches a log.
pub(crate) struct SignedQuery {
    /// Lowercase hex HMAC-SHA256 of the string to sign.
    pub(crate) signature: String,
    /// Every signed parameter, keys in lowercase and sorted by key.
    params: Vec<(String, String)>,
}

impl SignedQuery {
    /// The signed parameters joined `k=v&...` without escaping: the query
    /// line of the string the signature covers. The tests pin it.
    #[cfg(test)]
    fn unsigned(&self) -> String {
        join_unescaped(&self.params)
    }

    /// The query to send: the signed parameters, URL-encoded, with
    /// `auth_signature` appended last, as the Pusher REST API expects. The
    /// service decodes the values before it checks the signature, so the
    /// encoding does not change what is signed.
    pub(crate) fn to_query(&self) -> String {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in &self.params {
            query.append_pair(name, value);
        }
        query.append_pair("auth_signature", &self.signature);
        query.finish()
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

/// Sign one REST call: `method` (`GET`, `POST`) on `path` (the whole path,
/// `/apps/{app_id}/...`) with `params` as its query.
///
/// `body_md5` joins the signed parameters when `body` is a non-empty body,
/// binding the signature to those bytes, so the caller must send exactly
/// the bytes it passes here. A request without a body signs without it, as
/// Pusher's documentation asks.
///
/// # Errors
///
/// Refuses a parameter named like one the signature adds itself
/// (`auth_key`, `auth_timestamp`, `auth_version`, `auth_signature`, any
/// other `auth_` name, or `body_md5`), since the service would read two of
/// them.
pub(crate) fn rest_query(
    key: &str,
    secret: &str,
    method: &str,
    path: &str,
    timestamp: u64,
    params: &[(String, String)],
    body: Option<&[u8]>,
) -> Result<SignedQuery, FrameworkError> {
    let mut signed: Vec<(String, String)> = Vec::with_capacity(params.len() + 4);
    for (name, value) in params {
        let name = name.to_ascii_lowercase();
        if name.starts_with("auth_") || name == "body_md5" {
            return Err(FrameworkError::internal(format!(
                "the Pusher query parameter `{name}` is reserved for the request signature"
            )));
        }
        signed.push((name, value.clone()));
    }
    signed.push(("auth_key".into(), key.to_owned()));
    signed.push(("auth_timestamp".into(), timestamp.to_string()));
    signed.push(("auth_version".into(), "1.0".into()));
    if let Some(body) = body.filter(|body| !body.is_empty()) {
        signed.push(("body_md5".into(), hex::encode(Md5::digest(body))));
    }
    signed.sort_by(|a, b| a.0.cmp(&b.0));
    let unsigned = join_unescaped(&signed);
    let signature = hmac_hex(secret, &format!("{method}\n{path}\n{unsigned}"))?;
    Ok(SignedQuery {
        signature,
        params: signed,
    })
}

/// `params` joined `k=v&...` without URL escaping, as the string to sign
/// takes them.
fn join_unescaped(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&")
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
        let signed = rest_query(
            KEY,
            SECRET,
            "POST",
            "/apps/3/events",
            1_700_000_000,
            &[],
            Some(body),
        )
        .unwrap();
        assert_eq!(
            signed.unsigned(),
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
                signed.unsigned()
            )
        );
    }

    #[test]
    fn pusher_signs_a_get_without_body_md5_and_with_its_parameters_sorted() {
        let params = vec![
            (
                "info".to_string(),
                "user_count,subscription_count".to_string(),
            ),
            ("filter_by_prefix".to_string(), "presence-".to_string()),
        ];
        let signed = rest_query(
            KEY,
            SECRET,
            "GET",
            "/apps/3/channels",
            1_700_000_000,
            &params,
            None,
        )
        .unwrap();
        assert_eq!(
            signed.unsigned(),
            "auth_key=278d425bdf160c739803&auth_timestamp=1700000000&auth_version=1.0\
             &filter_by_prefix=presence-&info=user_count,subscription_count",
            "no body_md5 without a body, values unescaped"
        );
        let expected = hmac_hex(
            SECRET,
            &format!("GET\n/apps/3/channels\n{}", signed.unsigned()),
        )
        .unwrap();
        assert_eq!(signed.signature, expected);
        assert!(
            signed
                .to_query()
                .contains("info=user_count%2Csubscription_count"),
            "the URL carries the values encoded: {}",
            signed.to_query()
        );
        assert!(
            signed
                .to_query()
                .ends_with(&format!("&auth_signature={expected}"))
        );
    }

    #[test]
    fn pusher_refuses_a_parameter_the_signature_owns() {
        for name in ["auth_key", "AUTH_SIGNATURE", "auth_other", "body_md5"] {
            let params = vec![(name.to_string(), "x".to_string())];
            let err = rest_query(KEY, SECRET, "GET", "/apps/3/channels", 1, &params, None)
                .err()
                .map(|e| e.to_string())
                .expect("refused");
            assert!(err.contains("reserved"), "{err}");
            assert!(!err.contains(SECRET), "{err}");
        }
    }
}
