//! A client for the Pusher HTTP API.
//!
//! The endpoints, their parameters and answers, and the request signature
//! come from Pusher's HTTP API documentation ("HTTP API reference",
//! <https://pusher.com/docs/channels/library_auth_reference/rest-api/>).
//! They are not in the Laravel reference tree: Laravel reaches them through
//! the `pusher/pusher-php-server` package that `Broadcast::pusher($config)`
//! and `PusherBroadcaster::getPusher` return.

use super::config::PusherConfig;
use super::{encryption, names, signing};
use crate::FrameworkError;
use crate::broadcasting::BroadcastEnvelope;
use crate::broadcasting::ChannelRegistry;
use crate::broadcasting::hub::reject_reserved_channel;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A client for the HTTP API of a Pusher-protocol service: Pusher
/// Channels, Soketi or Laravel Reverb. Mirrors the configured Pusher
/// client Laravel's `Broadcast::pusher($config)` returns.
///
/// [`PusherBroadcastHub`](crate::PusherBroadcastHub) publishes through one,
/// and [`PusherBroadcastHub::client`](crate::PusherBroadcastHub::client)
/// hands it out, so an application can ask the service what the local hub
/// cannot know: which channels are occupied, and who is on a presence
/// channel across every process and every client connected to the service.
///
/// Channel names go through the [`ChannelRegistry`]'s wire mapping, the
/// same one publishing and authorization use, so `presence_users("room")`
/// asks about `presence-room` when the registry holds `room` as a presence
/// channel. Every request carries Pusher's signature, and no error quotes
/// the secret or a signature.
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use suprnova::broadcasting::ChannelRegistry;
/// use suprnova::{PusherClient, PusherConfig};
/// # async fn ex() -> Result<(), suprnova::FrameworkError> {
/// let client = PusherClient::new(PusherConfig::from_env()?, Arc::new(ChannelRegistry::new()))?;
/// let occupied = client.channels(Some("presence-"), &["user_count"]).await?;
/// # let _ = occupied;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct PusherClient {
    config: Arc<PusherConfig>,
    registry: Arc<ChannelRegistry>,
    http: reqwest::Client,
}

impl std::fmt::Debug for PusherClient {
    /// The app id and the base URL only: the config's own `Debug` already
    /// redacts the secret, and nothing else here helps a reader.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PusherClient")
            .field("app_id", &self.config.app_id)
            .field("base_url", &self.config.base_url())
            .finish_non_exhaustive()
    }
}

impl PusherClient {
    /// A client for `config`, naming channels through `registry`, as
    /// [`PusherBroadcastHub::new`](crate::PusherBroadcastHub::new) takes
    /// them: the wire name of a channel depends on what the registry says
    /// it is.
    ///
    /// # Errors
    ///
    /// Fails when a config value is invalid (see [`PusherConfig`]), with an
    /// error that names the field and never quotes the secret, or when the
    /// HTTP client cannot be built (for example, when the TLS backend
    /// cannot load).
    pub fn new(
        config: PusherConfig,
        registry: Arc<ChannelRegistry>,
    ) -> Result<Self, FrameworkError> {
        config.validate()?;
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "building the Pusher HTTP client failed: {}",
                    e.without_url()
                ))
            })?;
        Ok(Self {
            config: Arc::new(config),
            registry,
            http,
        })
    }

    /// The configuration, for the hub's authorization state.
    pub(crate) fn config(&self) -> &Arc<PusherConfig> {
        &self.config
    }

    /// The registry, for the hub's authorization state.
    pub(crate) fn registry(&self) -> &Arc<ChannelRegistry> {
        &self.registry
    }

    /// `GET /apps/{app_id}/channels`: the occupied channels, as the service
    /// answers them (`{"channels": {"presence-room": {"user_count": 2}}}`).
    ///
    /// `prefix` is sent as `filter_by_prefix` and is a wire prefix such as
    /// `presence-`, not a channel name, so it is sent as given. `info`
    /// names the attributes to include, joined with commas; Pusher accepts
    /// `user_count` there only together with the `presence-` prefix.
    ///
    /// # Errors
    ///
    /// The errors of [`PusherClient::get`].
    pub async fn channels(
        &self,
        prefix: Option<&str>,
        info: &[&str],
    ) -> Result<Value, FrameworkError> {
        let mut params: Vec<(String, String)> = Vec::new();
        if let Some(prefix) = prefix {
            params.push(("filter_by_prefix".into(), prefix.to_owned()));
        }
        if !info.is_empty() {
            params.push(("info".into(), info.join(",")));
        }
        self.get_json("/channels", params).await
    }

    /// `GET /apps/{app_id}/channels/{channel}`: one channel's state, as the
    /// service answers it (`{"occupied": true, "subscription_count": 3}`).
    /// `info` names the attributes to include: `user_count`,
    /// `subscription_count` or `cache`.
    ///
    /// # Errors
    ///
    /// Fails when the registry's wire mapping refuses `name`, when the
    /// wire name holds a character that would change the request path,
    /// and with the errors of [`PusherClient::get`].
    pub async fn channel(&self, name: &str, info: &[&str]) -> Result<Value, FrameworkError> {
        let wire = self.path_wire_name(name)?;
        let mut params: Vec<(String, String)> = Vec::new();
        if !info.is_empty() {
            params.push(("info".into(), info.join(",")));
        }
        self.get_json(&format!("/channels/{wire}"), params).await
    }

    /// `GET /apps/{app_id}/channels/{channel}/users`: the ids of the users
    /// on a presence channel, in the order the service lists them. This
    /// sees every member connected to the service, where the hub's
    /// [`list_members`](crate::broadcasting::BroadcastHub::list_members)
    /// sees only the members of this process.
    ///
    /// # Errors
    ///
    /// Fails when the registry does not hold `name` as a presence channel
    /// (Pusher answers this endpoint for presence channels only), when the
    /// answer has no `users` list of objects with an `id`, and with the
    /// errors of [`PusherClient::get`].
    pub async fn presence_users(&self, name: &str) -> Result<Vec<String>, FrameworkError> {
        let wire = self.path_wire_name(name)?;
        if !wire.starts_with(PRESENCE_PREFIX) {
            return Err(FrameworkError::internal(format!(
                "Pusher presence users of '{name}' refused: the channel registry does not hold \
                 it as a presence channel, so its wire name is '{wire}'"
            )));
        }
        let answer = self
            .get_json(&format!("/channels/{wire}/users"), Vec::new())
            .await?;
        let malformed = || {
            FrameworkError::internal(format!(
                "Pusher presence users of '{name}': the answer has no `users` list of objects \
                 with an `id`"
            ))
        };
        let users = answer
            .get("users")
            .and_then(Value::as_array)
            .ok_or_else(malformed)?;
        users
            .iter()
            .map(|user| match user.get("id") {
                Some(Value::String(id)) => Ok(id.clone()),
                Some(Value::Number(id)) => Ok(id.to_string()),
                _ => Err(malformed()),
            })
            .collect()
    }

    /// `POST /apps/{app_id}/batch_events`: publish every envelope in one
    /// request, each to its own channel, encrypted when its wire name is
    /// `private-encrypted-`, and without the connection its `except`
    /// names. An empty list sends nothing.
    ///
    /// This publishes to the service only. The in-process hub of a
    /// [`PusherBroadcastHub`](crate::PusherBroadcastHub) does not see these
    /// events; publish through the hub when a `ws!` endpoint of this
    /// process must receive them.
    ///
    /// Pusher Channels takes at most ten events per batch and answers a
    /// larger one with an error, which this returns; a self-hosted server
    /// sets its own limit, so the client does not split a batch.
    ///
    /// # Errors
    ///
    /// Fails before anything is sent when a channel name is reserved or
    /// refused by the wire mapping, or when an encrypted channel has no
    /// master key; then with the errors of the request.
    pub async fn trigger_batch(
        &self,
        envelopes: &[BroadcastEnvelope],
    ) -> Result<(), FrameworkError> {
        if envelopes.is_empty() {
            return Ok(());
        }
        let mut batch = Vec::with_capacity(envelopes.len());
        for envelope in envelopes {
            reject_reserved_channel(&envelope.channel)?;
            let channel = names::wire_name(&self.registry, &envelope.channel)?;
            let data = self.event_data(&channel, &envelope.data)?;
            batch.push(BatchEvent {
                name: &envelope.event,
                channel,
                data,
                socket_id: envelope.except.as_deref(),
            });
        }
        let body = serde_json::to_vec(&BatchRequest { batch }).map_err(|e| {
            FrameworkError::from_external_with("serializing a Pusher batch failed", e)
        })?;
        self.send(
            reqwest::Method::POST,
            "/batch_events",
            Vec::new(),
            Some(body),
            &format!("Pusher batch publish of {} events", envelopes.len()),
        )
        .await
        .map(|_| ())
    }

    /// `POST /apps/{app_id}/users/{user_id}/terminate_connections`: end
    /// every connection the user authenticated with user authentication
    /// holds, on every client.
    ///
    /// # Errors
    ///
    /// Fails when `user_id` is empty, `.` or `..`, or holds a character
    /// outside letters, digits and `-_=@,.;`, which would change the request
    /// path,
    /// and with the errors of the request.
    pub async fn terminate_user_connections(&self, user_id: &str) -> Result<(), FrameworkError> {
        check_path_segment("user id", user_id)?;
        self.send(
            reqwest::Method::POST,
            &format!("/users/{user_id}/terminate_connections"),
            Vec::new(),
            Some(b"{}".to_vec()),
            &format!("Pusher termination of the connections of user '{user_id}'"),
        )
        .await
        .map(|_| ())
    }

    /// A signed `GET /apps/{app_id}{path}` with `params` as its query, for
    /// an endpoint this client has no method for. Mirrors the `get` of
    /// Pusher's own libraries: `path` is relative to the app, so
    /// `get("/channels", &[])` asks `/apps/{app_id}/channels`. The answer is
    /// returned as JSON.
    ///
    /// # Errors
    ///
    /// Fails when `path` does not start with `/`, has a `.` or `..`
    /// segment, or holds `?`, `#`, `%`, whitespace or a control character, when a parameter is named like
    /// one of the signature's own (`auth_*`, `body_md5`), when the request
    /// cannot be sent or the service answers other than 2xx (the error
    /// names the status and quotes at most 200 bytes of the answer, with
    /// the secret and the signature removed), and when the answer is not
    /// JSON.
    pub async fn get(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, FrameworkError> {
        check_path(path)?;
        let params = params
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        self.get_json(path, params).await
    }

    /// `POST /apps/{app_id}/events` for one envelope: the hub's publish.
    pub(crate) async fn trigger(&self, envelope: &BroadcastEnvelope) -> Result<(), FrameworkError> {
        let channel = names::wire_name(&self.registry, &envelope.channel)?;
        let data = self.event_data(&channel, &envelope.data)?;
        let body = serde_json::to_vec(&EventRequest {
            name: &envelope.event,
            channels: [&channel],
            data: &data,
            socket_id: envelope.except.as_deref(),
        })
        .map_err(|e| FrameworkError::from_external_with("serializing a Pusher event failed", e))?;
        self.send(
            reqwest::Method::POST,
            "/events",
            Vec::new(),
            Some(body),
            &format!("Pusher publish to '{channel}'"),
        )
        .await
        .map(|_| ())
    }

    /// The wire name of `name`, checked for use as one path segment.
    fn path_wire_name(&self, name: &str) -> Result<String, FrameworkError> {
        let wire = names::wire_name(&self.registry, name)?;
        check_path_segment("channel name", &wire)?;
        Ok(wire)
    }

    /// A signed `GET`, its answer read as JSON.
    async fn get_json(
        &self,
        path: &str,
        params: Vec<(String, String)>,
    ) -> Result<Value, FrameworkError> {
        let what = format!("Pusher request GET {path}");
        let response = self
            .send(reqwest::Method::GET, path, params, None, &what)
            .await?;
        // The body read needs no scrubbing of the signature: an error here
        // carries no URL, and the secret is never sent.
        let answer = response.bytes().await.map_err(|e| {
            FrameworkError::internal(format!(
                "{what} failed reading the answer: {}",
                describe_transport_error(e, &[self.config.secret()])
            ))
        })?;
        serde_json::from_slice(&answer).map_err(|_| {
            FrameworkError::internal(format!("{what} answered with a body that is not JSON"))
        })
    }

    /// Sign and send one request to `/apps/{app_id}{path}`, and return the
    /// 2xx answer, its body unread, for the caller that needs it. `what`
    /// opens every error message.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        params: Vec<(String, String)>,
        body: Option<Vec<u8>>,
        what: &str,
    ) -> Result<reqwest::Response, FrameworkError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                FrameworkError::internal(
                    "the system clock reads before 1970, so a Pusher request cannot be timestamped",
                )
            })?
            .as_secs();
        let full_path = format!("/apps/{}{path}", self.config.app_id);
        let secret = self.config.secret();
        let signed = signing::rest_query(
            &self.config.key,
            secret,
            method.as_str(),
            &full_path,
            timestamp,
            &params,
            body.as_deref(),
        )?;
        let url = format!(
            "{}{full_path}?{}",
            self.config.base_url(),
            signed.to_query()
        );
        let scrub = [secret, signed.signature.as_str()];

        let mut request = self.http.request(method, url);
        if let Some(body) = body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let response = request.send().await.map_err(|e| {
            FrameworkError::internal(format!(
                "{what} failed: {}",
                describe_transport_error(e, &scrub)
            ))
        })?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let answer = match response.bytes().await {
            Ok(bytes) => excerpt(&String::from_utf8_lossy(&bytes), &scrub),
            Err(_) => "(the response body could not be read)".to_string(),
        };
        Err(FrameworkError::internal(format!(
            "{what} failed with HTTP {}: {answer}",
            status.as_u16()
        )))
    }

    /// The `data` field for `channel`: the event data as a JSON string,
    /// or that string encrypted when the wire name is
    /// `private-encrypted-`.
    ///
    /// The wire name decides, not the channel's visibility, because
    /// Pusher clients decide by the name too: whatever arrives on a
    /// `private-encrypted-` channel is decrypted, so it must never be
    /// plaintext.
    fn event_data(&self, channel: &str, data: &Value) -> Result<String, FrameworkError> {
        let serialize_failed =
            |e| FrameworkError::from_external_with("serializing Pusher event data failed", e);
        if !channel.starts_with(ENCRYPTED_PREFIX) {
            return serde_json::to_string(data).map_err(serialize_failed);
        }
        let master_key = self.config.encryption_master_key().ok_or_else(|| {
            FrameworkError::internal(format!(
                "Pusher publish to encrypted channel '{channel}' refused: no encryption \
                 master key is configured (set PUSHER_ENCRYPTION_MASTER_KEY_BASE64 or call \
                 PusherConfig::encryption_master_key_base64). The event is never sent as \
                 plaintext."
            ))
        })?;
        let plaintext = serde_json::to_vec(data).map_err(serialize_failed)?;
        encryption::encrypt(channel, master_key, &plaintext)
    }
}

/// The wire-name prefix of an end-to-end encrypted channel.
const ENCRYPTED_PREFIX: &str = "private-encrypted-";

/// The wire-name prefix of a presence channel.
const PRESENCE_PREFIX: &str = "presence-";

/// The JSON body of one `POST /apps/{app_id}/events` call. A struct
/// rather than `json!` so the body has one fixed shape, and `socket_id`
/// is left out entirely when no connection is excluded.
#[derive(Serialize)]
struct EventRequest<'a> {
    name: &'a str,
    channels: [&'a str; 1],
    data: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    socket_id: Option<&'a str>,
}

/// The JSON body of one `POST /apps/{app_id}/batch_events` call.
#[derive(Serialize)]
struct BatchRequest<'a> {
    batch: Vec<BatchEvent<'a>>,
}

/// One event of a batch: unlike `/events`, each names one channel.
#[derive(Serialize)]
struct BatchEvent<'a> {
    name: &'a str,
    channel: String,
    data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    socket_id: Option<&'a str>,
}

/// Refuse a path segment that is empty, `.` or `..`, or holds a character
/// outside the set Pusher's own libraries accept in a channel name, so the
/// segment cannot add or remove a path level, or add a query or a fragment
/// to the signed request.
fn check_path_segment(what: &str, segment: &str) -> Result<(), FrameworkError> {
    let valid = !segment.is_empty()
        && !is_dot_segment(segment)
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_=@,.;".contains(&b));
    if valid {
        Ok(())
    } else {
        Err(FrameworkError::internal(format!(
            "the Pusher {what} '{segment}' is refused: it must be one or more letters, digits \
             or `-_=@,.;`, so it cannot change the request path"
        )))
    }
}

/// `.` and `..`, which a URL resolves away, so the path sent would not be
/// the path signed.
fn is_dot_segment(segment: &str) -> bool {
    segment == "." || segment == ".."
}

/// Refuse a [`PusherClient::get`] path that is not an absolute path under
/// the app, or that would carry its own query or fragment.
fn check_path(path: &str) -> Result<(), FrameworkError> {
    let valid = path.starts_with('/')
        && !path.split('/').any(is_dot_segment)
        && !path
            .chars()
            .any(|c| matches!(c, '?' | '#' | '%') || c.is_whitespace() || c.is_control());
    if valid {
        Ok(())
    } else {
        Err(FrameworkError::internal(
            "a Pusher request path must start with `/` and hold no `?`, `#`, `%`, whitespace \
             or control character; pass query parameters as params",
        ))
    }
}

/// At most this many bytes of an error answer go into the error, so a
/// large error page cannot flood the log.
const ERROR_EXCERPT_BYTES: usize = 200;

/// At most [`ERROR_EXCERPT_BYTES`] of `text`, with each of `secrets`
/// replaced by `"[redacted]"` first.
///
/// Scrubbing comes before truncating so a cut can never leave a whole
/// secret behind. A server that echoes the request back (some error
/// pages quote the string they expected to be signed) would otherwise
/// put the signature into a log line.
fn excerpt(text: &str, secrets: &[&str]) -> String {
    let mut text = text.to_string();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        text = text.replace(secret, "[redacted]");
    }
    let mut end = text.len().min(ERROR_EXCERPT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// Describe a transport error without the request URL, whose query
/// carries the signature: reqwest's own message quotes the URL.
fn describe_transport_error(error: reqwest::Error, secrets: &[&str]) -> String {
    let error = error.without_url();
    let mut text = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    excerpt(&text, secrets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pusher_error_excerpt_is_capped_and_scrubbed() {
        let signature = "927d37a56401cbf139d27b5fbfea241b0b3edd53d143efb127106ba258b697c5";
        let body = format!("bad signature {signature} for secret app-secret");
        let text = excerpt(&body, &["app-secret", signature]);
        assert!(!text.contains(signature), "{text}");
        assert!(!text.contains("app-secret"), "{text}");
        assert!(text.contains("[redacted]"), "{text}");

        // Capped at 200 bytes, on a character boundary.
        let long = "é".repeat(150); // 300 bytes
        let capped = excerpt(&long, &[]);
        assert!(capped.len() <= 200, "{} bytes", capped.len());
        assert_eq!(capped, "é".repeat(100));

        // An empty secret never redacts every gap between characters.
        assert_eq!(excerpt("plain", &[""]), "plain");
    }

    #[test]
    fn pusher_path_segments_cannot_change_the_path() {
        assert!(check_path_segment("user id", "42").is_ok());
        assert!(check_path_segment("channel name", "presence-room.1").is_ok());
        for bad in ["", ".", "..", "a/b", "a?b", "a#b", "a b", "a%2Fb", "a\u{0}"] {
            assert!(check_path_segment("user id", bad).is_err(), "{bad:?}");
        }
        assert!(check_path("/channels").is_ok());
        for bad in [
            "channels",
            "/channels?x=1",
            "/a#b",
            "/a b",
            "/a%2F",
            "/a/../b",
            "/./a",
        ] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
    }
}
