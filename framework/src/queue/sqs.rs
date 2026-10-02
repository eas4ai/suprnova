//! Amazon SQS queue driver: standard queues over the AWS JSON 1.0 protocol
//! of SQS, signed with Signature Version 4.
//!
//! `QUEUE_DRIVER=sqs` builds it from the environment with
//! [`SqsQueueDriver::from_env`]. The variables are Laravel's: `SQS_PREFIX`,
//! `SQS_QUEUE` and `SQS_SUFFIX` build the queue URL, `AWS_DEFAULT_REGION`
//! (or `AWS_REGION`) names the region, and `AWS_ACCESS_KEY_ID`,
//! `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN` are the keys. With no keys
//! the driver uses the default credential chain of AWS: the profile, web
//! identity, the ECS task role and the instance role. `SQS_ENDPOINT` points
//! it at a service that is not AWS, such as LocalStack or ElasticMQ.
//!
//! # Attempts
//!
//! An SQS message cannot be changed once it is sent, so the attempts a
//! delivery carries are the attempts in the body plus the receives SQS
//! counted before this one (`ApproximateReceiveCount`). A `nack` hides the
//! message again, and its next receive counts one more attempt, as does a
//! reservation that expires. A `release` must not count one, so it sends a
//! copy that carries the attempts of this delivery and deletes the original.
//! Laravel's SQS job counts every receive, so a release there costs an
//! attempt; here it matches the other drivers.
//!
//! # Delays
//!
//! SQS takes at most 15 minutes of delay on one message. A job due later is
//! sent with 15 minutes, and a receive before its time sends it on with what
//! is left and deletes the copy that came too early. Waiting out a delay that
//! way is not an attempt: the new copy starts its receive count again.
//!
//! # Queues
//!
//! A job goes to the queue its envelope names, or to `SQS_QUEUE`. A worker
//! receives from the queues `--queue` names, in order, and from `SQS_QUEUE`
//! when it names none, because SQS has no receive across queues.
//!
//! # Overflow
//!
//! SQS takes at most 1 MiB in one message. With `SQS_OVERFLOW_ENABLED=true`,
//! a larger job is written to a disk (`SQS_OVERFLOW_DISK`, or else the
//! default disk) and SQS carries a pointer to it. Laravel keeps these
//! payloads in a cache store, which can evict one before its job runs.

use crate::error::FrameworkError;
use crate::filesystem::Storage;
use crate::queue::driver::{QueueDriver, QueueFilterCapability, Reservation, ReservationToken};
use crate::queue::envelope::Envelope;
use async_trait::async_trait;
use bytes::Bytes;
use reqsign_aws_v4::{
    Credential, DefaultCredentialProvider, RequestSigner, StaticCredentialProvider,
};
use reqsign_core::{Context, OsEnv, ProvideCredentialChain, Signer};
use reqsign_file_read_tokio::TokioFileRead;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use uuid::Uuid;

/// The most SQS takes in one message: 1 MiB.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// The longest delay SQS takes on one message: 15 minutes.
const MAX_DELAY_SECS: u64 = 900;

/// The most messages SQS takes in one `SendMessageBatch`.
const MAX_BATCH: usize = 10;

/// The longest visibility timeout SQS takes: 12 hours.
const MAX_VISIBILITY_SECS: u64 = 43_200;

/// The directory on the overflow disk that payloads are written under, one
/// subdirectory per queue.
const OVERFLOW_ROOT: &str = "sqs-payloads";

/// The key of the message body that points at an overflow payload, the one
/// Laravel's `SqsQueue` writes.
const POINTER_KEY: &str = "@pointer";

/// Queue driver over Amazon SQS standard queues. See the module
/// documentation for how it maps the driver contract onto SQS.
pub struct SqsQueueDriver {
    client: reqwest::Client,
    /// The URL requests are posted to, with a trailing `/`.
    endpoint: String,
    signer: Signer<Credential>,
    prefix: Option<String>,
    /// `SQS_QUEUE`: a name, or a queue URL.
    queue: String,
    suffix: String,
    overflow: Option<Overflow>,
    held: Mutex<HashMap<ReservationToken, Held>>,
}

/// What a reservation needs to settle its message.
struct Held {
    queue_url: String,
    receipt: String,
    /// The envelope as it was delivered, with the attempts it carried
    /// before the worker's own count.
    envelope: Envelope,
    /// The overflow payload the message points at, if it does.
    pointer: Option<String>,
}

/// One message on its way to SQS.
struct Outgoing {
    body: String,
    /// `DelaySeconds`, at most 15 minutes.
    delay: u64,
    /// The overflow payload `body` points at, if it does.
    pointer: Option<String>,
}

/// The overflow settings, `SQS_OVERFLOW_*`.
struct Overflow {
    /// `SQS_OVERFLOW_DISK`; `None` is the default disk.
    disk: Option<String>,
    always: bool,
    delete_after_processing: bool,
    flush_on_clear: bool,
}

impl Overflow {
    fn operator(&self) -> Result<opendal::Operator, FrameworkError> {
        match &self.disk {
            Some(name) => Storage::disk(name).map_err(|_| {
                FrameworkError::internal(format!(
                    "SQS_OVERFLOW_DISK names the disk '{name}', which is not registered: \
                     register a disk named '{name}' in the bootstrap, or name a registered disk"
                ))
            }),
            None => Storage::default_disk().map_err(|error| {
                FrameworkError::internal(format!(
                    "SQS_OVERFLOW_ENABLED is on and SQS_OVERFLOW_DISK names no disk, so \
                     overflow uses the default disk: {error}"
                ))
            }),
        }
    }
}

/// An SQS error reply: the error code and its message.
struct SqsError {
    code: String,
    message: String,
}

impl SqsError {
    /// Whether SQS refused a receipt handle that is no longer the current
    /// one: the reservation expired and the message was received again, or
    /// it is gone. SQS reports an expired handle as `InvalidParameterValue`
    /// naming the receipt handle.
    fn is_stale_receipt(&self) -> bool {
        let message = self.message.to_ascii_lowercase();
        matches!(
            self.code.as_str(),
            "ReceiptHandleIsInvalid" | "MessageNotInflight"
        ) || (self.code == "InvalidParameterValue"
            && (message.contains("receipt handle") || message.contains("receipthandle")))
    }
}

/// Sends the credential requests of the AWS chain (instance metadata, the
/// ECS task role, STS) over the driver's own client.
#[derive(Debug, Clone)]
struct ReqwestSend(reqwest::Client);

impl reqsign_core::HttpSend for ReqwestSend {
    async fn http_send(
        &self,
        request: http::Request<Bytes>,
    ) -> reqsign_core::Result<http::Response<Bytes>> {
        let (parts, body) = request.into_parts();
        let response = self
            .0
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body)
            .send()
            .await
            .map_err(|error| {
                reqsign_core::Error::unexpected("send an AWS credential request").with_source(error)
            })?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.map_err(|error| {
            reqsign_core::Error::unexpected("read an AWS credential response").with_source(error)
        })?;
        let mut out = http::Response::new(bytes);
        *out.status_mut() = status;
        *out.headers_mut() = headers;
        Ok(out)
    }
}

impl SqsQueueDriver {
    /// Build the driver from the environment. See the module documentation
    /// for the variables.
    ///
    /// # Errors
    ///
    /// When no region is set; when `SQS_QUEUE` is not a URL and `SQS_PREFIX`
    /// is not set; when the queue is a FIFO queue; when only one of the two
    /// keys is set; when `SQS_ENDPOINT` is not an `http` or `https` URL; and
    /// when overflow is on and its disk is not registered. No error repeats
    /// a key.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_variables(|name| std::env::var(name).ok())
    }

    fn from_variables(variable: impl Fn(&str) -> Option<String>) -> Result<Self, FrameworkError> {
        let var = |name: &str| {
            variable(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let region = var("AWS_DEFAULT_REGION")
            .or_else(|| var("AWS_REGION"))
            .ok_or_else(|| {
                FrameworkError::internal(
                    "the sqs queue driver needs a region: set AWS_DEFAULT_REGION, or AWS_REGION",
                )
            })?;
        let endpoint = match var("SQS_ENDPOINT") {
            Some(endpoint) => {
                if !is_url(&endpoint) {
                    return Err(FrameworkError::internal(
                        "SQS_ENDPOINT must be an http or https URL",
                    ));
                }
                endpoint
            }
            None => format!("https://sqs.{region}.amazonaws.com"),
        };
        let endpoint = format!("{}/", endpoint.trim_end_matches('/'));

        let keys = match (var("AWS_ACCESS_KEY_ID"), var("AWS_SECRET_ACCESS_KEY")) {
            (Some(key), Some(secret)) => Some((key, secret, var("AWS_SESSION_TOKEN"))),
            (None, None) => None,
            _ => {
                return Err(FrameworkError::internal(
                    "set both AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY for the sqs queue \
                     driver, or neither to use the default credential chain of AWS",
                ));
            }
        };

        let enabled = |name: &str| matches!(var(name).as_deref(), Some("true") | Some("1"));
        let disabled = |name: &str| matches!(var(name).as_deref(), Some("false") | Some("0"));
        let overflow = enabled("SQS_OVERFLOW_ENABLED").then(|| Overflow {
            disk: var("SQS_OVERFLOW_DISK"),
            always: enabled("SQS_OVERFLOW_ALWAYS"),
            delete_after_processing: !disabled("SQS_OVERFLOW_DELETE_AFTER_PROCESSING"),
            flush_on_clear: enabled("SQS_OVERFLOW_FLUSH_ON_CLEAR"),
        });
        if let Some(overflow) = &overflow {
            overflow.operator()?;
        }

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| {
                FrameworkError::internal(format!("could not build the SQS HTTP client: {error}"))
            })?;
        let context = Context::new()
            .with_file_read(TokioFileRead)
            .with_http_send(ReqwestSend(client.clone()))
            .with_env(OsEnv);
        let mut credentials =
            ProvideCredentialChain::new().push(DefaultCredentialProvider::builder().build());
        if let Some((key, secret, token)) = keys {
            let provider = match token {
                Some(token) => {
                    StaticCredentialProvider::new(&key, &secret).with_session_token(&token)
                }
                None => StaticCredentialProvider::new(&key, &secret),
            };
            credentials = credentials.push_front(provider);
        }
        let signer = Signer::new(context, credentials, RequestSigner::new("sqs", &region));

        let driver = Self {
            client,
            endpoint,
            signer,
            prefix: var("SQS_PREFIX"),
            queue: var("SQS_QUEUE").unwrap_or_else(|| "default".to_owned()),
            suffix: var("SQS_SUFFIX").unwrap_or_default(),
            overflow,
            held: Mutex::new(HashMap::new()),
        };
        driver.queue_url(None)?;
        Ok(driver)
    }

    /// The URL of the queue `name` names, or of `SQS_QUEUE`, built as
    /// Laravel's `SqsQueue::getQueue` builds it: a URL as it is, and
    /// otherwise the prefix, the name, and the suffix unless the name
    /// already ends with it.
    fn queue_url(&self, name: Option<&str>) -> Result<String, FrameworkError> {
        let name = name.filter(|name| !name.is_empty()).unwrap_or(&self.queue);
        let url = if is_url(name) {
            name.to_owned()
        } else {
            let prefix = self.prefix.as_deref().ok_or_else(|| {
                FrameworkError::internal(format!(
                    "the sqs queue '{name}' is not a URL and SQS_PREFIX is not set: set \
                     SQS_PREFIX to the URL your queues are under \
                     (https://sqs.<region>.amazonaws.com/<account id>), or set SQS_QUEUE to \
                     the URL of the queue"
                ))
            })?;
            let (base, fifo) = match name.strip_suffix(".fifo") {
                Some(base) => (base, ".fifo"),
                None => (name, ""),
            };
            let base = if base.ends_with(&self.suffix) {
                base.to_owned()
            } else {
                format!("{base}{}", self.suffix)
            };
            format!("{}/{base}{fifo}", prefix.trim_end_matches('/'))
        };
        if url.ends_with(".fifo") {
            return Err(FrameworkError::internal(format!(
                "the sqs queue '{url}' is a FIFO queue, and the sqs driver sends to standard \
                 queues only: a FIFO queue needs a message group and a deduplication ID on \
                 each job. Name a standard queue in SQS_QUEUE or on the job"
            )));
        }
        Ok(url)
    }

    /// Post one action and return SQS's reply, or its error reply.
    async fn request(
        &self,
        action: &str,
        body: &Value,
    ) -> Result<Result<Value, SqsError>, FrameworkError> {
        let payload = serde_json::to_vec(body)
            .map_err(|error| FrameworkError::internal(format!("SQS {action}: encode: {error}")))?;
        let hash = hex::encode(Sha256::digest(&payload));
        let (mut parts, ()) = http::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .header(http::header::CONTENT_TYPE, "application/x-amz-json-1.0")
            .header("x-amz-target", format!("AmazonSQS.{action}"))
            .header("x-amz-content-sha256", hash)
            .body(())
            .map_err(|error| FrameworkError::internal(format!("SQS {action}: request: {error}")))?
            .into_parts();
        self.signer.sign(&mut parts, None).await.map_err(|error| {
            FrameworkError::internal(format!("SQS {action}: could not sign the request: {error}"))
        })?;

        let response = self
            .client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(payload)
            .send()
            .await
            .map_err(|error| FrameworkError::internal(format!("SQS {action}: {error}")))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| FrameworkError::internal(format!("SQS {action}: {error}")))?;
        let reply: Value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        if status.is_success() {
            return Ok(Ok(reply));
        }
        let code = reply["__type"]
            .as_str()
            .map(|kind| kind.rsplit('#').next().unwrap_or(kind).to_owned())
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        let message = reply["message"]
            .as_str()
            .or_else(|| reply["Message"].as_str())
            .unwrap_or_default()
            .to_owned();
        Ok(Err(SqsError { code, message }))
    }

    /// [`request`](Self::request), with an error reply as an error.
    async fn call(&self, action: &str, body: Value) -> Result<Value, FrameworkError> {
        self.request(action, &body).await?.map_err(|error| {
            FrameworkError::internal(format!(
                "SQS {action} failed: {}: {}",
                error.code, error.message
            ))
        })
    }

    /// A call on a receipt handle, where a handle that is no longer current
    /// is not an error: the reservation expired, so the message is someone
    /// else's now, or it is gone.
    async fn call_on_receipt(&self, action: &str, body: Value) -> Result<(), FrameworkError> {
        match self.request(action, &body).await? {
            Ok(_) => Ok(()),
            Err(error) if error.is_stale_receipt() => {
                tracing::warn!(
                    action,
                    code = %error.code,
                    message = %error.message,
                    "SQS receipt handle is no longer current; the reservation expired"
                );
                Ok(())
            }
            Err(error) => Err(FrameworkError::internal(format!(
                "SQS {action} failed: {}: {}",
                error.code, error.message
            ))),
        }
    }

    /// Send `envelope` to `queue_url`, delayed until its `available_at` or
    /// by at most 15 minutes, writing it to the overflow disk when it is too
    /// large for one message.
    async fn send(&self, queue_url: &str, envelope: &Envelope) -> Result<(), FrameworkError> {
        let message = self.encode(queue_url, envelope).await?;
        let mut request = json!({ "QueueUrl": queue_url, "MessageBody": message.body });
        if message.delay > 0 {
            request["DelaySeconds"] = json!(message.delay);
        }
        if let Err(error) = self.call("SendMessage", request).await {
            if let Some(path) = &message.pointer {
                self.delete_payload(path).await;
            }
            return Err(error);
        }
        Ok(())
    }

    /// The message for `envelope`: its body, or a pointer to the overflow
    /// payload it was written to, and its `DelaySeconds`.
    async fn encode(
        &self,
        queue_url: &str,
        envelope: &Envelope,
    ) -> Result<Outgoing, FrameworkError> {
        let body = envelope
            .to_json()
            .map_err(|error| FrameworkError::internal(format!("SQS: encode the job: {error}")))?;
        let (message, pointer) = match &self.overflow {
            Some(overflow) if overflow.always || body.len() >= MAX_MESSAGE_BYTES => {
                let path = format!(
                    "{OVERFLOW_ROOT}/{}/{}.json",
                    queue_key(queue_url),
                    Uuid::new_v4()
                );
                overflow
                    .operator()?
                    .write(&path, body.into_bytes())
                    .await
                    .map_err(|error| {
                        FrameworkError::internal(format!(
                            "SQS: could not write the job to the overflow disk: {error}"
                        ))
                    })?;
                (json!({ POINTER_KEY: path }).to_string(), Some(path))
            }
            None if body.len() > MAX_MESSAGE_BYTES => {
                return Err(FrameworkError::internal(format!(
                    "the job '{}' is {} bytes, over the 1 MiB SQS takes in one message: set \
                     SQS_OVERFLOW_ENABLED=true to store large jobs on a disk",
                    envelope.job_name,
                    body.len()
                )));
            }
            _ => (body, None),
        };
        Ok(Outgoing {
            body: message,
            delay: delay_secs(envelope.available_at),
            pointer,
        })
    }

    /// Send `messages` to `queue_url` with `SendMessageBatch`, ten at a
    /// time and at most 1 MiB a batch, stopping at the first batch SQS
    /// rejects any of, so a later job cannot arrive ahead of one that was
    /// not sent. The overflow payloads of the messages not sent are deleted.
    async fn send_batches(
        &self,
        queue_url: &str,
        messages: Vec<Outgoing>,
    ) -> Result<(), FrameworkError> {
        let mut chunks: Vec<Vec<Outgoing>> = Vec::new();
        let mut bytes = 0;
        for message in messages {
            let full = chunks.last().is_none_or(|chunk| {
                chunk.len() >= MAX_BATCH || bytes + message.body.len() > MAX_MESSAGE_BYTES
            });
            if full {
                chunks.push(Vec::new());
                bytes = 0;
            }
            bytes += message.body.len();
            if let Some(chunk) = chunks.last_mut() {
                chunk.push(message);
            }
        }

        for (number, chunk) in chunks.iter().enumerate() {
            let entries: Vec<Value> = chunk
                .iter()
                .enumerate()
                .map(|(index, message)| {
                    let mut entry = json!({ "Id": index.to_string(), "MessageBody": message.body });
                    if message.delay > 0 {
                        entry["DelaySeconds"] = json!(message.delay);
                    }
                    entry
                })
                .collect();
            let request = json!({ "QueueUrl": queue_url, "Entries": entries });
            let (unsent, error): (Vec<&Outgoing>, FrameworkError) =
                match self.call("SendMessageBatch", request).await {
                    Ok(reply) => {
                        let failed = reply["Failed"].as_array().cloned().unwrap_or_default();
                        let Some(first) = failed.first() else {
                            continue;
                        };
                        let rejected: Vec<usize> = failed
                            .iter()
                            .filter_map(|entry| entry["Id"].as_str()?.parse().ok())
                            .collect();
                        let error = FrameworkError::internal(format!(
                            "SQS SendMessageBatch rejected {} of {} messages. First failure {}: {}",
                            failed.len(),
                            chunk.len(),
                            first["Code"].as_str().unwrap_or("Unknown"),
                            first["Message"].as_str().unwrap_or_default()
                        ));
                        let unsent = chunk
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| rejected.contains(index))
                            .map(|(_, message)| message)
                            .collect();
                        (unsent, error)
                    }
                    Err(error) => (chunk.iter().collect(), error),
                };
            let later = chunks.iter().skip(number + 1).flatten();
            for message in unsent.into_iter().chain(later) {
                if let Some(path) = &message.pointer {
                    self.delete_payload(path).await;
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// The envelope a message body carries, reading it from the overflow
    /// disk when the body points there.
    async fn decode(&self, body: &str) -> Result<(Envelope, Option<String>), FrameworkError> {
        let value: Value = serde_json::from_str(body).map_err(|error| {
            FrameworkError::internal(format!("SQS: the message is not a job: {error}"))
        })?;
        let Some(path) = value.get(POINTER_KEY).and_then(Value::as_str) else {
            let envelope = Envelope::from_json(body)
                .map_err(|error| FrameworkError::internal(format!("SQS: {error}")))?;
            return Ok((envelope, None));
        };
        let overflow = self.overflow.as_ref().ok_or_else(|| {
            FrameworkError::internal(format!(
                "SQS: the message points at the overflow payload '{path}', and \
                 SQS_OVERFLOW_ENABLED is off"
            ))
        })?;
        let bytes = overflow.operator()?.read(path).await.map_err(|error| {
            FrameworkError::internal(format!(
                "SQS: could not read the overflow payload '{path}': {error}"
            ))
        })?;
        let text = String::from_utf8(bytes.to_vec()).map_err(|error| {
            FrameworkError::internal(format!(
                "SQS: the overflow payload '{path}' is not text: {error}"
            ))
        })?;
        let envelope = Envelope::from_json(&text)
            .map_err(|error| FrameworkError::internal(format!("SQS: {error}")))?;
        Ok((envelope, Some(path.to_owned())))
    }

    /// Delete an overflow payload, logging a failure: the message that
    /// pointed at it is settled either way.
    async fn delete_payload(&self, path: &str) {
        let Some(overflow) = &self.overflow else {
            return;
        };
        let result = match overflow.operator() {
            Ok(operator) => operator
                .delete(path)
                .await
                .map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = result {
            tracing::warn!(path, %error, "could not delete an SQS overflow payload");
        }
    }

    /// Delete the payload of a settled message when the settings say so.
    async fn drop_payload(&self, pointer: Option<&str>) {
        if let (Some(path), Some(overflow)) = (pointer, &self.overflow)
            && overflow.delete_after_processing
        {
            self.delete_payload(path).await;
        }
    }

    /// Receive one message from `queue_url`.
    async fn receive(
        &self,
        queue_url: &str,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        let reply = self
            .call(
                "ReceiveMessage",
                json!({
                    "QueueUrl": queue_url,
                    "MaxNumberOfMessages": 1,
                    "VisibilityTimeout": seconds(visibility_timeout, MAX_VISIBILITY_SECS),
                    "AttributeNames": ["ApproximateReceiveCount"],
                    "MessageSystemAttributeNames": ["ApproximateReceiveCount"],
                }),
            )
            .await?;
        let Some(message) = reply["Messages"]
            .as_array()
            .and_then(|messages| messages.first())
        else {
            return Ok(None);
        };
        let receipt = message["ReceiptHandle"]
            .as_str()
            .ok_or_else(|| {
                FrameworkError::internal("SQS ReceiveMessage: a message has no receipt handle")
            })?
            .to_owned();
        let body = message["Body"]
            .as_str()
            .ok_or_else(|| FrameworkError::internal("SQS ReceiveMessage: a message has no body"))?;
        let receives: u32 = message["Attributes"]["ApproximateReceiveCount"]
            .as_str()
            .and_then(|count| count.parse().ok())
            .unwrap_or(1);
        let (mut envelope, pointer) = self.decode(body).await?;
        envelope.attempts = envelope.attempts.saturating_add(receives.saturating_sub(1));

        if envelope.available_at > crate::clock::now() {
            // Received before its time, which only a delay over 15 minutes
            // allows: send it on with what is left and drop this copy.
            self.send(queue_url, &envelope).await?;
            self.call_on_receipt(
                "DeleteMessage",
                json!({ "QueueUrl": queue_url, "ReceiptHandle": receipt }),
            )
            .await?;
            self.drop_payload(pointer.as_deref()).await;
            return Ok(None);
        }

        let token = ReservationToken(Uuid::new_v4());
        self.held_map()?.insert(
            token.clone(),
            Held {
                queue_url: queue_url.to_owned(),
                receipt,
                envelope: envelope.clone(),
                pointer,
            },
        );
        Ok(Some(Reservation { envelope, token }))
    }

    fn held_map(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<ReservationToken, Held>>, FrameworkError> {
        self.held
            .lock()
            .map_err(|_| FrameworkError::internal("the SQS reservation map is poisoned"))
    }

    fn take(&self, token: &ReservationToken) -> Result<Option<Held>, FrameworkError> {
        Ok(self.held_map()?.remove(token))
    }

    /// The approximate counts SQS keeps for `SQS_QUEUE`: visible, in
    /// flight, and delayed.
    async fn counts(&self) -> Result<(u64, u64, u64), FrameworkError> {
        let reply = self
            .call(
                "GetQueueAttributes",
                json!({
                    "QueueUrl": self.queue_url(None)?,
                    "AttributeNames": [
                        "ApproximateNumberOfMessages",
                        "ApproximateNumberOfMessagesNotVisible",
                        "ApproximateNumberOfMessagesDelayed",
                    ],
                }),
            )
            .await?;
        let count = |name: &str| {
            reply["Attributes"][name]
                .as_str()
                .and_then(|count| count.parse::<u64>().ok())
                .unwrap_or(0)
        };
        Ok((
            count("ApproximateNumberOfMessages"),
            count("ApproximateNumberOfMessagesNotVisible"),
            count("ApproximateNumberOfMessagesDelayed"),
        ))
    }
}

#[async_trait]
impl QueueDriver for SqsQueueDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        let url = self.queue_url(env.queue.as_deref())?;
        self.send(&url, &env).await
    }

    /// Each queue's envelopes go in `SendMessageBatch` requests, in the
    /// order given. An envelope that cannot be sent, too large with
    /// overflow off or bound for a FIFO queue, fails the call before
    /// anything for its queue is sent.
    async fn bulk_push(&self, envs: Vec<Envelope>) -> Result<(), FrameworkError> {
        let mut queues: Vec<(String, Vec<Envelope>)> = Vec::new();
        for env in envs {
            let url = self.queue_url(env.queue.as_deref())?;
            match queues.iter_mut().find(|(queue, _)| *queue == url) {
                Some((_, group)) => group.push(env),
                None => queues.push((url, vec![env])),
            }
        }
        for (url, group) in queues {
            let mut messages = Vec::with_capacity(group.len());
            for env in &group {
                match self.encode(&url, env).await {
                    Ok(message) => messages.push(message),
                    Err(error) => {
                        for message in &messages {
                            if let Some(path) = &message.pointer {
                                self.delete_payload(path).await;
                            }
                        }
                        return Err(error);
                    }
                }
            }
            self.send_batches(&url, messages).await?;
        }
        Ok(())
    }

    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        let url = self.queue_url(None)?;
        self.receive(&url, visibility_timeout).await
    }

    fn queue_filter_capability(&self) -> QueueFilterCapability {
        QueueFilterCapability::Supported
    }

    async fn pop_from(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        if queues.is_empty() {
            return self.pop(visibility_timeout).await;
        }
        for queue in queues {
            let url = self.queue_url(Some(queue))?;
            if let Some(reservation) = self.receive(&url, visibility_timeout).await? {
                return Ok(Some(reservation));
            }
        }
        Ok(None)
    }

    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        let Some(held) = self.take(token)? else {
            return Ok(());
        };
        let deleted = self
            .call_on_receipt(
                "DeleteMessage",
                json!({ "QueueUrl": held.queue_url, "ReceiptHandle": held.receipt }),
            )
            .await;
        if let Err(error) = deleted {
            self.held_map()?.insert(token.clone(), held);
            return Err(error);
        }
        self.drop_payload(held.pointer.as_deref()).await;
        Ok(())
    }

    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        let Some(held) = self.take(token)? else {
            return Ok(());
        };
        if requeue_delay > Duration::from_secs(MAX_VISIBILITY_SECS) {
            // Longer than SQS can hide a message: send a copy that counts
            // the attempt and waits out the delay, and drop this one.
            let mut copy = held.envelope.clone();
            copy.attempts = copy.attempts.saturating_add(1);
            copy.available_at = crate::clock::now()
                + chrono::Duration::from_std(requeue_delay)
                    .unwrap_or_else(|_| chrono::Duration::zero());
            self.send(&held.queue_url, &copy).await?;
            self.call_on_receipt(
                "DeleteMessage",
                json!({ "QueueUrl": held.queue_url, "ReceiptHandle": held.receipt }),
            )
            .await?;
            self.drop_payload(held.pointer.as_deref()).await;
            return Ok(());
        }
        self.call_on_receipt(
            "ChangeMessageVisibility",
            json!({
                "QueueUrl": held.queue_url,
                "ReceiptHandle": held.receipt,
                "VisibilityTimeout": seconds(requeue_delay, MAX_VISIBILITY_SECS),
            }),
        )
        .await
    }

    async fn release(
        &self,
        token: &ReservationToken,
        env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        let Some(held) = self.take(token)? else {
            return Ok(());
        };
        let mut copy = env.clone();
        copy.attempts = held.envelope.attempts;
        copy.available_at = crate::clock::now()
            + chrono::Duration::from_std(delay).unwrap_or_else(|_| chrono::Duration::zero());
        self.send(&held.queue_url, &copy).await?;
        self.call_on_receipt(
            "DeleteMessage",
            json!({ "QueueUrl": held.queue_url, "ReceiptHandle": held.receipt }),
        )
        .await?;
        self.drop_payload(held.pointer.as_deref()).await;
        Ok(())
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        let (visible, in_flight, delayed) = self.counts().await?;
        Ok(visible + in_flight + delayed)
    }

    async fn pending_size(&self) -> Result<u64, FrameworkError> {
        Ok(self.counts().await?.0)
    }

    async fn reserved_size(&self) -> Result<u64, FrameworkError> {
        Ok(self.counts().await?.1)
    }

    async fn delayed_size(&self) -> Result<u64, FrameworkError> {
        Ok(self.counts().await?.2)
    }

    async fn clear(&self) -> Result<u64, FrameworkError> {
        let held = self.size().await?;
        let url = self.queue_url(None)?;
        self.call("PurgeQueue", json!({ "QueueUrl": url })).await?;
        if let Some(overflow) = self
            .overflow
            .as_ref()
            .filter(|overflow| overflow.flush_on_clear)
        {
            let directory = format!("{OVERFLOW_ROOT}/{}/", queue_key(&url));
            overflow
                .operator()?
                .delete_with(&directory)
                .recursive(true)
                .await
                .map_err(|error| {
                    FrameworkError::internal(format!(
                        "SQS: purged the queue but could not delete its overflow payloads: {error}"
                    ))
                })?;
        }
        Ok(held)
    }

    fn name(&self) -> &'static str {
        "sqs"
    }
}

fn is_url(value: &str) -> bool {
    value.starts_with("https://") || value.starts_with("http://")
}

/// The seconds of `duration`, rounded up so nothing comes back early, and
/// capped at `max`.
fn seconds(duration: Duration, max: u64) -> u64 {
    let whole = duration.as_secs() + u64::from(duration.subsec_nanos() > 0);
    whole.min(max)
}

/// The `DelaySeconds` that holds a job until `available_at`, or as close to
/// it as SQS goes.
fn delay_secs(available_at: chrono::DateTime<chrono::Utc>) -> u64 {
    match (available_at - crate::clock::now()).to_std() {
        Ok(wait) => seconds(wait, MAX_DELAY_SECS),
        Err(_) => 0,
    }
}

/// A directory name for the queue at `queue_url`: its last path segment,
/// with anything but letters, digits, `-` and `_` replaced.
fn queue_key(queue_url: &str) -> String {
    queue_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variables(set: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            set.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn seconds_round_up_and_cap() {
        assert_eq!(seconds(Duration::from_millis(1500), 900), 2);
        assert_eq!(seconds(Duration::from_secs(3600), 900), 900);
        assert_eq!(seconds(Duration::ZERO, 900), 0);
    }

    #[test]
    fn the_queue_key_is_the_last_segment() {
        assert_eq!(
            queue_key("https://sqs.us-east-1.amazonaws.com/1/jobs-prod"),
            "jobs-prod"
        );
        assert_eq!(queue_key("http://localhost:9324/queue/a.b"), "a_b");
    }

    #[test]
    fn one_key_without_the_other_is_refused_without_repeating_it() {
        let Err(error) = SqsQueueDriver::from_variables(variables(&[
            ("AWS_DEFAULT_REGION", "us-east-1"),
            ("SQS_PREFIX", "https://sqs.us-east-1.amazonaws.com/1"),
            ("AWS_ACCESS_KEY_ID", "AKIDONLYHALF"),
        ])) else {
            panic!("one key without the other must be refused");
        };
        let text = error.to_string();
        assert!(text.contains("AWS_SECRET_ACCESS_KEY"), "{text}");
        assert!(!text.contains("AKIDONLYHALF"), "{text}");
    }

    #[test]
    fn an_endpoint_that_is_not_a_url_is_refused() {
        assert!(
            SqsQueueDriver::from_variables(variables(&[
                ("AWS_DEFAULT_REGION", "us-east-1"),
                ("SQS_PREFIX", "https://sqs.us-east-1.amazonaws.com/1"),
                ("SQS_ENDPOINT", "localhost:9324"),
            ]))
            .is_err()
        );
    }

    #[test]
    fn a_fifo_suffix_is_kept_after_the_suffix_check() {
        let Err(error) = SqsQueueDriver::from_variables(variables(&[
            ("AWS_DEFAULT_REGION", "us-east-1"),
            ("SQS_PREFIX", "https://sqs.us-east-1.amazonaws.com/1"),
            (
                "SQS_QUEUE",
                "https://sqs.us-east-1.amazonaws.com/1/orders.fifo",
            ),
        ])) else {
            panic!("a FIFO queue URL must be refused");
        };
        assert!(error.to_string().contains("FIFO"));
    }
}
