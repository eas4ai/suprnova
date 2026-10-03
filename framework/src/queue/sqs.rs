//! Amazon SQS queue driver: standard queues over the AWS JSON 1.0 protocol
//! of SQS, signed with Signature Version 4.
//!
//! `QUEUE_DRIVER=sqs` builds it from the environment with
//! [`SqsQueueDriver::from_env`]; [`SqsQueueDriver::new`] builds one from an
//! [`SqsConfig`], for a second connection in another region or account. The
//! variables are Laravel's: `SQS_PREFIX`, `SQS_QUEUE` and `SQS_SUFFIX` build
//! the queue URL, `AWS_DEFAULT_REGION` (or `AWS_REGION`) names the region,
//! and `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and `AWS_SESSION_TOKEN`
//! are the keys. With no keys the driver uses the default credential chain
//! of AWS: the profile, web identity, the ECS task role and the instance
//! role. `SQS_ENDPOINT` points it at another endpoint: a service that is not
//! AWS, such as LocalStack or ElasticMQ, or a VPC endpoint.
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
//! SQS hides a message for at most 12 hours from the receive, so a `nack`
//! whose delay is longer than the time left also sends a copy, one that
//! counts the attempt.
//!
//! # Delays
//!
//! SQS takes at most 15 minutes of delay on one message. A job due later is
//! sent with 15 minutes, and a receive before its time sends it on with what
//! is left and deletes the copy that came too early. Waiting out a delay that
//! way is not an attempt: the new copy starts its receive count again.
//!
//! # The window a copy opens
//!
//! A release, a long `nack` and a delay sent on are each a send followed by
//! a delete, because SQS cannot change a message. If the delete fails after
//! the send, the job is on the queue twice until the original's visibility
//! runs out and it is received again. That is the at-least-once delivery the
//! queue documents; every other in-tree driver does these in place.
//!
//! # Queues
//!
//! A job goes to the queue its envelope names, or to `SQS_QUEUE`. A worker
//! receives from the queues `--queue` names, in order, and from `SQS_QUEUE`
//! when it names none, because SQS has no receive across queues. A name in
//! `--queue` is an SQS queue name: `default` is the queue
//! `SQS_PREFIX/default`, which holds the unrouted jobs only when `SQS_QUEUE`
//! is `default`.
//!
//! Each receive waits up to `SQS_WAIT_TIME_SECONDS` (default 1) for a
//! message, which keeps an idle worker to about one request a second for
//! each queue it names and makes SQS answer from all of its servers.
//!
//! # Overflow
//!
//! SQS takes at most 1 MiB in one message. With `SQS_OVERFLOW_ENABLED=true`,
//! a larger job is written to a disk (`SQS_OVERFLOW_DISK`, or else the
//! default disk) and SQS carries a pointer to it. Laravel keeps these
//! payloads in a cache store, which can evict one before its job runs.
//!
//! A payload is deleted only when the driver knows no message points at it
//! any more: after SQS refused the send that would have carried it, or after
//! a delete of its message on a reservation that had not expired. A payload
//! whose fate the driver cannot know, such as one whose send timed out, is
//! left on the disk.

use crate::error::FrameworkError;
use crate::filesystem::Storage;
use crate::queue::driver::{QueueDriver, QueueFilterCapability, Reservation, ReservationToken};
use crate::queue::envelope::Envelope;
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use reqsign_aws_v4::{
    Credential, DefaultCredentialProvider, RequestSigner, StaticCredentialProvider,
};
use reqsign_core::{Context, OsEnv, ProvideCredentialChain, Signer};
use reqsign_file_read_tokio::TokioFileRead;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;
use uuid::Uuid;

/// The most SQS takes in one message: 1 MiB.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// The longest delay SQS takes on one message: 15 minutes.
const MAX_DELAY_SECS: u64 = 900;

/// The most messages SQS takes in one `SendMessageBatch`.
const MAX_BATCH: usize = 10;

/// The longest SQS hides a message, counted from its receive: 12 hours.
const MAX_VISIBILITY_SECS: u64 = 43_200;

/// What a `nack` keeps in hand below the 12 hours, for the time between the
/// driver's reading of its clock and SQS's.
const VISIBILITY_MARGIN_SECS: u64 = 60;

/// How long before its deadline a reservation stops counting as current.
const DEADLINE_MARGIN_SECS: i64 = 5;

/// The longest a receive waits for a message: 20 seconds.
const MAX_WAIT_SECS: u64 = 20;

/// How many times one request is made before its failure is returned.
const MAX_TRIES: u32 = 3;

/// The directory on the overflow disk that payloads are written under, one
/// subdirectory per queue.
const OVERFLOW_ROOT: &str = "sqs-payloads";

/// The key of the message body that points at an overflow payload, the one
/// Laravel's `SqsQueue` writes.
const POINTER_KEY: &str = "@pointer";

/// The keys the driver signs with, in place of the default credential chain
/// of AWS.
#[derive(Clone)]
pub struct SqsCredentials {
    /// `AWS_ACCESS_KEY_ID`.
    pub access_key_id: String,
    /// `AWS_SECRET_ACCESS_KEY`.
    pub secret_access_key: String,
    /// `AWS_SESSION_TOKEN`, for temporary keys.
    pub session_token: Option<String>,
}

impl fmt::Debug for SqsCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqsCredentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Where jobs too large for one SQS message go: the `SQS_OVERFLOW_*`
/// variables.
#[derive(Debug, Clone)]
pub struct SqsOverflow {
    /// The disk payloads are written to; `None` is the default disk.
    pub disk: Option<String>,
    /// Store every job on the disk, whatever its size.
    pub always: bool,
    /// Delete a job's payload when the job is acknowledged.
    pub delete_after_processing: bool,
    /// Delete the payloads of the driver's queue when `clear` purges it.
    pub flush_on_clear: bool,
}

impl Default for SqsOverflow {
    fn default() -> Self {
        Self {
            disk: None,
            always: false,
            delete_after_processing: true,
            flush_on_clear: false,
        }
    }
}

impl SqsOverflow {
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

/// The settings of an [`SqsQueueDriver`]. [`SqsConfig::from_env`] reads them
/// from the variables the module documentation lists.
#[derive(Debug, Clone)]
pub struct SqsConfig {
    /// The region the requests are signed for.
    pub region: String,
    /// The queue a job that names none goes to, and the one a worker with
    /// no `--queue` receives from: a name, or the URL of the queue.
    pub queue: String,
    /// The URL the queues are under, for queue names that are not URLs.
    pub prefix: Option<String>,
    /// Appended to every queue name that does not already end with it.
    pub suffix: String,
    /// The endpoint requests go to; `None` is the SQS endpoint of the
    /// region.
    pub endpoint: Option<String>,
    /// The keys to sign with; `None` is the default credential chain of AWS.
    pub credentials: Option<SqsCredentials>,
    /// How long a receive waits for a message, 0 to 20 seconds.
    pub wait_time_seconds: u64,
    /// Where jobs too large for one message go; `None` refuses them.
    pub overflow: Option<SqsOverflow>,
}

impl SqsConfig {
    /// The settings for `queue` in `region`, with no prefix, suffix,
    /// endpoint, keys or overflow, and a one-second wait.
    pub fn new(region: impl Into<String>, queue: impl Into<String>) -> Self {
        Self {
            region: region.into(),
            queue: queue.into(),
            prefix: None,
            suffix: String::new(),
            endpoint: None,
            credentials: None,
            wait_time_seconds: 1,
            overflow: None,
        }
    }

    /// Read the settings from the environment. See the module
    /// documentation for the variables.
    ///
    /// # Errors
    ///
    /// When no region is set, when only one of the two keys is set, and
    /// when `SQS_WAIT_TIME_SECONDS` is not a whole number. No error repeats
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
            .ok_or_else(missing_region)?;
        let credentials = match (var("AWS_ACCESS_KEY_ID"), var("AWS_SECRET_ACCESS_KEY")) {
            (Some(access_key_id), Some(secret_access_key)) => Some(SqsCredentials {
                access_key_id,
                secret_access_key,
                session_token: var("AWS_SESSION_TOKEN"),
            }),
            (None, None) => None,
            _ => {
                return Err(FrameworkError::internal(
                    "set both AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY for the sqs queue \
                     driver, or neither to use the default credential chain of AWS",
                ));
            }
        };
        let wait_time_seconds = match var("SQS_WAIT_TIME_SECONDS") {
            Some(value) => value.parse().map_err(|_| {
                FrameworkError::internal("SQS_WAIT_TIME_SECONDS must be a whole number, 0 to 20")
            })?,
            None => 1,
        };
        let enabled = |name: &str| matches!(var(name).as_deref(), Some("true") | Some("1"));
        let disabled = |name: &str| matches!(var(name).as_deref(), Some("false") | Some("0"));
        let overflow = enabled("SQS_OVERFLOW_ENABLED").then(|| SqsOverflow {
            disk: var("SQS_OVERFLOW_DISK"),
            always: enabled("SQS_OVERFLOW_ALWAYS"),
            delete_after_processing: !disabled("SQS_OVERFLOW_DELETE_AFTER_PROCESSING"),
            flush_on_clear: enabled("SQS_OVERFLOW_FLUSH_ON_CLEAR"),
        });
        Ok(Self {
            region,
            queue: var("SQS_QUEUE").unwrap_or_else(|| "default".to_owned()),
            prefix: var("SQS_PREFIX"),
            suffix: var("SQS_SUFFIX").unwrap_or_default(),
            endpoint: var("SQS_ENDPOINT"),
            credentials,
            wait_time_seconds,
            overflow,
        })
    }
}

fn missing_region() -> FrameworkError {
    FrameworkError::internal(
        "the sqs queue driver needs a region: set AWS_DEFAULT_REGION, or AWS_REGION",
    )
}

/// Queue driver over Amazon SQS standard queues. See the module
/// documentation for how it maps the driver contract onto SQS.
pub struct SqsQueueDriver {
    client: reqwest::Client,
    /// The URL requests are posted to, with a trailing `/`.
    endpoint: String,
    signer: Signer<Credential>,
    prefix: Option<String>,
    /// A queue name, or a queue URL.
    queue: String,
    suffix: String,
    wait_time_seconds: u64,
    overflow: Option<SqsOverflow>,
    held: Mutex<HashMap<ReservationToken, Held>>,
}

/// What a reservation needs to settle its message.
struct Held {
    queue_url: String,
    receipt: String,
    /// The message body as SQS delivered it, taken out of the reply rather
    /// than copied. The worker owns the decoded envelope; the rare path
    /// that sends the message on without it decodes this again.
    body: String,
    /// The attempts the delivered envelope carried, before the worker's
    /// own count.
    attempts: u32,
    /// The overflow payload the message points at, if it does.
    pointer: Option<String>,
    /// When the message was received, by the framework clock.
    received_at: DateTime<Utc>,
    /// When the reservation runs out and SQS may hand the message to
    /// someone else.
    deadline: DateTime<Utc>,
}

impl Held {
    /// Whether the reservation still holds the message, so a delete on its
    /// receipt handle deletes it.
    fn is_current(&self) -> bool {
        crate::clock::now() + chrono::Duration::seconds(DEADLINE_MARGIN_SECS) < self.deadline
    }
}

/// One message on its way to SQS.
struct Outgoing {
    body: String,
    /// `DelaySeconds`, at most 15 minutes.
    delay: u64,
    /// The overflow payload `body` points at, if it does.
    pointer: Option<String>,
}

/// An SQS error reply.
struct SqsError {
    status: u16,
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

    /// Whether the same request may succeed when it is made again: a
    /// throttled request or a fault of the service.
    fn is_retryable(&self) -> bool {
        self.status >= 500
            || matches!(
                self.code.as_str(),
                "ThrottlingException"
                    | "Throttling"
                    | "RequestThrottled"
                    | "TooManyRequestsException"
                    | "ServiceUnavailable"
                    | "InternalError"
                    | "InternalFailure"
                    | "RequestTimeout"
            )
    }
}

/// Why a request did not succeed. The difference that matters is whether
/// SQS may have acted on it: a payload written for a message SQS refused can
/// go, one for a message SQS may have accepted must stay.
enum Failure {
    /// Nothing reached SQS.
    NotSent(FrameworkError),
    /// SQS answered with an error.
    Refused(SqsError),
    /// No answer: SQS may have acted on it, and asking again may succeed.
    Unknown(FrameworkError),
    /// A success status with a reply that is not SQS's: whatever answered
    /// may not be SQS, so asking again will not help.
    Garbled(FrameworkError),
}

impl Failure {
    /// Whether SQS did not act on the request.
    fn is_definite(&self) -> bool {
        matches!(self, Failure::NotSent(_) | Failure::Refused(_))
    }

    fn into_error(self, action: &str) -> FrameworkError {
        match self {
            Failure::NotSent(error) | Failure::Unknown(error) | Failure::Garbled(error) => error,
            Failure::Refused(error) => FrameworkError::internal(format!(
                "SQS {action} failed: {}: {}",
                error.code, error.message
            )),
        }
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
    /// Build the driver from the environment: [`SqsConfig::from_env`], then
    /// [`SqsQueueDriver::new`].
    ///
    /// # Errors
    ///
    /// The errors of both.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::new(SqsConfig::from_env()?)
    }

    /// Build the driver from `config`. Nothing is sent to SQS here.
    ///
    /// # Errors
    ///
    /// When the region is empty; when the queue is not a URL and there is
    /// no prefix; when the queue is a FIFO queue; when the endpoint is not
    /// an `http` or `https` URL; when the wait is over 20 seconds; and when
    /// overflow is on and its disk is not registered.
    pub fn new(config: SqsConfig) -> Result<Self, FrameworkError> {
        let region = config.region.trim().to_owned();
        if region.is_empty() {
            return Err(missing_region());
        }
        let endpoint = match config.endpoint {
            Some(endpoint) => {
                if !is_url(&endpoint) {
                    return Err(FrameworkError::internal(
                        "SQS_ENDPOINT must be an http or https URL",
                    ));
                }
                endpoint
            }
            None => default_endpoint(&region),
        };
        let endpoint = format!("{}/", endpoint.trim_end_matches('/'));
        if config.wait_time_seconds > MAX_WAIT_SECS {
            return Err(FrameworkError::internal(
                "SQS_WAIT_TIME_SECONDS must be a whole number, 0 to 20",
            ));
        }
        if let Some(overflow) = &config.overflow {
            overflow.operator()?;
        }

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60 + config.wait_time_seconds))
            .build()
            .map_err(|error| {
                FrameworkError::internal(format!("could not build the SQS HTTP client: {error}"))
            })?;
        let context = Context::new()
            .with_file_read(TokioFileRead)
            .with_http_send(ReqwestSend(client.clone()))
            .with_env(OsEnv);
        let mut chain =
            ProvideCredentialChain::new().push(DefaultCredentialProvider::builder().build());
        if let Some(keys) = &config.credentials {
            let provider =
                StaticCredentialProvider::new(&keys.access_key_id, &keys.secret_access_key);
            let provider = match &keys.session_token {
                Some(token) => provider.with_session_token(token),
                None => provider,
            };
            chain = chain.push_front(provider);
        }
        let signer = Signer::new(context, chain, RequestSigner::new("sqs", &region));

        let driver = Self {
            client,
            endpoint,
            signer,
            prefix: config.prefix.filter(|prefix| !prefix.trim().is_empty()),
            queue: config.queue,
            suffix: config.suffix,
            wait_time_seconds: config.wait_time_seconds,
            overflow: config.overflow,
            held: Mutex::new(HashMap::new()),
        };
        driver.queue_url(None)?;
        Ok(driver)
    }

    /// Send any SQS action, signed as the driver's own requests are, and
    /// return SQS's reply: the escape hatch Laravel's `getSqs()` gives
    /// through the AWS client. `body` is the action's request in the AWS JSON
    /// protocol, `QueueUrl` included.
    ///
    /// ```rust,ignore
    /// let reply = driver
    ///     .call("ListQueues", serde_json::json!({ "QueueNamePrefix": "orders" }))
    ///     .await?;
    /// ```
    ///
    /// # Errors
    ///
    /// When SQS answers with an error, which the error names, or when no
    /// answer comes after three tries.
    pub async fn call(&self, action: &str, body: Value) -> Result<Value, FrameworkError> {
        self.request(action, &body)
            .await
            .map_err(|failure| failure.into_error(action))
    }

    /// The URL of the queue `name` names, or of the driver's own queue,
    /// built as Laravel's `SqsQueue::getQueue` builds it: a URL as it is, and
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

    /// Post one action, trying again after a throttled request, a fault of
    /// the service, or no answer, up to three tries in all.
    async fn request(&self, action: &str, body: &Value) -> Result<Value, Failure> {
        // Serialized once; each try sends the same shared bytes rather than
        // a copy of them.
        let payload = bytes::Bytes::from(serde_json::to_vec(body).map_err(|error| {
            Failure::NotSent(FrameworkError::internal(format!(
                "SQS {action}: encode: {error}"
            )))
        })?);
        let mut tries = 0;
        loop {
            tries += 1;
            let outcome = self.request_once(action, &payload).await;
            let again = match &outcome {
                Err(Failure::Refused(error)) => error.is_retryable(),
                Err(Failure::Unknown(_)) => true,
                _ => false,
            };
            if !again || tries >= MAX_TRIES {
                return outcome;
            }
            // 100 ms, then 200 ms, each with up to 50 ms of jitter so workers
            // throttled together do not retry together.
            let jitter = (Uuid::new_v4().as_u128() % 50) as u64;
            tokio::time::sleep(Duration::from_millis(100 * (1 << (tries - 1)) + jitter)).await;
        }
    }

    async fn request_once(&self, action: &str, payload: &bytes::Bytes) -> Result<Value, Failure> {
        let hash = hex::encode(Sha256::digest(payload));
        let (mut parts, ()) = http::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .header(http::header::CONTENT_TYPE, "application/x-amz-json-1.0")
            .header("x-amz-target", format!("AmazonSQS.{action}"))
            .header("x-amz-content-sha256", hash)
            .body(())
            .map_err(|error| {
                Failure::NotSent(FrameworkError::internal(format!(
                    "SQS {action}: request: {error}"
                )))
            })?
            .into_parts();
        self.signer.sign(&mut parts, None).await.map_err(|error| {
            Failure::NotSent(FrameworkError::internal(format!(
                "SQS {action}: could not sign the request: {error}"
            )))
        })?;

        let response = self
            .client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(payload.clone())
            .send()
            .await
            .map_err(|error| {
                // A connection that was never made carried nothing to SQS.
                let not_sent = error.is_connect() || error.is_builder();
                let error = FrameworkError::internal(format!("SQS {action}: {error}"));
                if not_sent {
                    Failure::NotSent(error)
                } else {
                    Failure::Unknown(error)
                }
            })?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| {
            Failure::Unknown(FrameworkError::internal(format!("SQS {action}: {error}")))
        })?;
        let reply: Option<Value> = if bytes.is_empty() {
            Some(Value::Null)
        } else {
            serde_json::from_slice(&bytes).ok()
        };
        if status.is_success() {
            return reply.ok_or_else(|| {
                Failure::Garbled(FrameworkError::internal(format!(
                    "SQS {action}: the reply (HTTP {}) is not SQS JSON; check that \
                     SQS_ENDPOINT is an SQS endpoint",
                    status.as_u16()
                )))
            });
        }
        let reply = reply.unwrap_or(Value::Null);
        let code = reply["__type"]
            .as_str()
            .map(|kind| kind.rsplit('#').next().unwrap_or(kind).to_owned())
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        let message = reply["message"]
            .as_str()
            .or_else(|| reply["Message"].as_str())
            .unwrap_or_default()
            .to_owned();
        Err(Failure::Refused(SqsError {
            status: status.as_u16(),
            code,
            message,
        }))
    }

    /// A call on a receipt handle. `Ok(true)` when SQS acted on it,
    /// `Ok(false)` when SQS says the handle is no longer current: the
    /// reservation expired, so the message is someone else's now, or it is
    /// gone.
    async fn call_on_receipt(&self, action: &str, body: Value) -> Result<bool, FrameworkError> {
        match self.request(action, &body).await {
            Ok(_) => Ok(true),
            Err(Failure::Refused(error)) if error.is_stale_receipt() => {
                tracing::warn!(
                    action,
                    code = %error.code,
                    message = %error.message,
                    "SQS receipt handle is no longer current; the reservation expired"
                );
                Ok(false)
            }
            Err(failure) => Err(failure.into_error(action)),
        }
    }

    /// Delete `held`'s message, then the payload it pointed at when this
    /// delete is known to have taken the message: SQS acted on it and the
    /// reservation had not expired. `superseded` says a copy now carries the
    /// job, so the payload goes whatever `delete_after_processing` says.
    async fn delete_held(&self, held: &Held, superseded: bool) -> Result<(), FrameworkError> {
        let current = held.is_current();
        let deleted = self
            .call_on_receipt(
                "DeleteMessage",
                json!({ "QueueUrl": held.queue_url, "ReceiptHandle": held.receipt }),
            )
            .await?;
        let (Some(path), Some(overflow)) = (&held.pointer, &self.overflow) else {
            return Ok(());
        };
        if !(superseded || overflow.delete_after_processing) {
            return Ok(());
        }
        if deleted && current {
            self.delete_payload(path).await;
        } else {
            tracing::warn!(
                path,
                "kept an SQS overflow payload: its reservation expired, so another delivery \
                 of the message may still point at it"
            );
        }
        Ok(())
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
        let outcome = self
            .request("SendMessage", &request)
            .await
            .and_then(|reply| match reply["MessageId"].as_str() {
                Some(_) => Ok(()),
                None => Err(Failure::Garbled(FrameworkError::internal(
                    "SQS SendMessage: the reply has no MessageId; check that SQS_ENDPOINT is \
                     an SQS endpoint",
                ))),
            });
        match outcome {
            Ok(()) => Ok(()),
            Err(failure) => {
                self.discard(&message, &failure).await;
                Err(failure.into_error("SendMessage"))
            }
        }
    }

    /// Delete the payload of a message whose send failed, when SQS is known
    /// not to have taken it.
    async fn discard(&self, message: &Outgoing, failure: &Failure) {
        let Some(path) = &message.pointer else {
            return;
        };
        if failure.is_definite() {
            self.delete_payload(path).await;
        } else {
            tracing::warn!(
                path,
                "kept an SQS overflow payload: the send failed without an answer, so SQS may \
                 hold a message that points at it"
            );
        }
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
        let (body, pointer) = match &self.overflow {
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
            body,
            delay: delay_secs(envelope.available_at),
            pointer,
        })
    }

    /// Send `messages` to `queue_url` with `SendMessageBatch`, ten at a
    /// time and at most 1 MiB a batch, stopping at the first batch SQS
    /// rejects any of, so a later job cannot arrive ahead of one that was
    /// not sent.
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
            // The messages of this batch SQS did not take, and the failure.
            let (unsent, failure): (Vec<&Outgoing>, Failure) =
                match self.request("SendMessageBatch", &request).await {
                    Ok(reply) => {
                        if !reply["Successful"].is_array() && !reply["Failed"].is_array() {
                            let failure = Failure::Garbled(FrameworkError::internal(
                                "SQS SendMessageBatch: the reply lists no messages; check that \
                                 SQS_ENDPOINT is an SQS endpoint",
                            ));
                            (Vec::new(), failure)
                        } else {
                            let failed = reply["Failed"].as_array().cloned().unwrap_or_default();
                            let Some(first) = failed.first() else {
                                continue;
                            };
                            let rejected: Vec<usize> = failed
                                .iter()
                                .filter_map(|entry| entry["Id"].as_str()?.parse().ok())
                                .collect();
                            let failure = Failure::NotSent(FrameworkError::internal(format!(
                                "SQS SendMessageBatch rejected {} of {} messages. First \
                                 failure {}: {}",
                                failed.len(),
                                chunk.len(),
                                first["Code"].as_str().unwrap_or("Unknown"),
                                first["Message"].as_str().unwrap_or_default()
                            )));
                            let unsent = chunk
                                .iter()
                                .enumerate()
                                .filter(|(index, _)| rejected.contains(index))
                                .map(|(_, message)| message)
                                .collect();
                            (unsent, failure)
                        }
                    }
                    Err(failure) if failure.is_definite() => (chunk.iter().collect(), failure),
                    Err(failure) => (Vec::new(), failure),
                };
            // The later batches were never sent.
            let later = chunks.iter().skip(number + 1).flatten();
            for message in unsent.into_iter().chain(later) {
                if let Some(path) = &message.pointer {
                    self.delete_payload(path).await;
                }
            }
            return Err(failure.into_error("SendMessageBatch"));
        }
        Ok(())
    }

    /// The envelope a message body carries, reading it from the overflow
    /// disk when the body points there.
    ///
    /// A body that is an envelope, which every inline message is, is parsed
    /// once, straight into the envelope. Only a body that is not one is
    /// parsed again to look for an overflow pointer, which a pointer body
    /// cannot be mistaken for: it has none of an envelope's required
    /// fields. Every error reads as it always has.
    async fn decode(&self, body: &str) -> Result<(Envelope, Option<String>), FrameworkError> {
        let envelope_error = match Envelope::from_json(body) {
            Ok(envelope) => return Ok((envelope, None)),
            Err(error) => error,
        };
        let value: Value = serde_json::from_str(body).map_err(|error| {
            FrameworkError::internal(format!("SQS: the message is not a job: {error}"))
        })?;
        let Some(path) = value.get(POINTER_KEY).and_then(Value::as_str) else {
            return Err(FrameworkError::internal(format!("SQS: {envelope_error}")));
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
        let bytes = bytes.to_bytes();
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            FrameworkError::internal(format!(
                "SQS: the overflow payload '{path}' is not text: {error}"
            ))
        })?;
        let envelope = Envelope::from_json(text)
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

    /// Receive one message from `queue_url`.
    async fn receive(
        &self,
        queue_url: &str,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        let visibility = seconds(visibility_timeout, MAX_VISIBILITY_SECS);
        let mut reply = self
            .call(
                "ReceiveMessage",
                json!({
                    "QueueUrl": queue_url,
                    "MaxNumberOfMessages": 1,
                    "VisibilityTimeout": visibility,
                    "WaitTimeSeconds": self.wait_time_seconds,
                    "AttributeNames": ["ApproximateReceiveCount"],
                    "MessageSystemAttributeNames": ["ApproximateReceiveCount"],
                }),
            )
            .await?;
        let received_at = crate::clock::now();
        let Some(message) = reply["Messages"]
            .as_array_mut()
            .and_then(|messages| messages.first_mut())
        else {
            return Ok(None);
        };
        let receipt = message["ReceiptHandle"]
            .as_str()
            .ok_or_else(|| {
                FrameworkError::internal("SQS ReceiveMessage: a message has no receipt handle")
            })?
            .to_owned();
        let Value::String(body) = message["Body"].take() else {
            return Err(FrameworkError::internal(
                "SQS ReceiveMessage: a message has no body",
            ));
        };
        let receives: u32 = message["Attributes"]["ApproximateReceiveCount"]
            .as_str()
            .and_then(|count| count.parse().ok())
            .unwrap_or(1);
        let (mut envelope, pointer) = self.decode(&body).await?;
        envelope.attempts = envelope.attempts.saturating_add(receives.saturating_sub(1));
        let held = Held {
            queue_url: queue_url.to_owned(),
            receipt,
            body,
            attempts: envelope.attempts,
            pointer,
            received_at,
            deadline: received_at + chrono::Duration::seconds(visibility as i64),
        };

        if envelope.available_at > crate::clock::now() {
            // Received before its time, which only a delay over 15 minutes
            // allows: send it on with what is left and drop this copy.
            self.send(queue_url, &envelope).await?;
            self.delete_held(&held, true).await?;
            return Ok(None);
        }

        let token = ReservationToken(Uuid::new_v4());
        self.held_map()?.insert(token.clone(), held);
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

    /// The approximate counts SQS keeps for the driver's queue: visible, in
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
                .ok_or_else(|| {
                    FrameworkError::internal(format!(
                        "SQS GetQueueAttributes: the reply has no {name}"
                    ))
                })
        };
        Ok((
            count("ApproximateNumberOfMessages")?,
            count("ApproximateNumberOfMessagesNotVisible")?,
            count("ApproximateNumberOfMessagesDelayed")?,
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
        if let Err(error) = self.delete_held(&held, false).await {
            self.held_map()?.insert(token.clone(), held);
            return Err(error);
        }
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
        let held_for = (crate::clock::now() - held.received_at)
            .num_seconds()
            .max(0) as u64;
        let left = MAX_VISIBILITY_SECS
            .saturating_sub(held_for)
            .saturating_sub(VISIBILITY_MARGIN_SECS);
        let wait = seconds(requeue_delay, u64::MAX);
        if wait > left {
            // Longer than SQS can still hide this message: send a copy that
            // counts the attempt and waits out the delay, and drop this one.
            let (mut copy, _) = self.decode(&held.body).await?;
            copy.attempts = held.attempts.saturating_add(1);
            copy.available_at = crate::clock::now()
                + chrono::Duration::from_std(requeue_delay)
                    .unwrap_or_else(|_| chrono::Duration::zero());
            self.send(&held.queue_url, &copy).await?;
            return self.delete_held(&held, true).await;
        }
        self.call_on_receipt(
            "ChangeMessageVisibility",
            json!({
                "QueueUrl": held.queue_url,
                "ReceiptHandle": held.receipt,
                "VisibilityTimeout": wait,
            }),
        )
        .await
        .map(|_| ())
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
        copy.attempts = held.attempts;
        copy.available_at = crate::clock::now()
            + chrono::Duration::from_std(delay).unwrap_or_else(|_| chrono::Duration::zero());
        self.send(&held.queue_url, &copy).await?;
        self.delete_held(&held, true).await
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

/// The SQS endpoint of `region`, in the China partition for a `cn-` region.
fn default_endpoint(region: &str) -> String {
    if region.starts_with("cn-") {
        format!("https://sqs.{region}.amazonaws.com.cn")
    } else {
        format!("https://sqs.{region}.amazonaws.com")
    }
}

/// The seconds of `duration`, rounded up so nothing comes back early, and
/// capped at `max`.
fn seconds(duration: Duration, max: u64) -> u64 {
    let whole = duration.as_secs() + u64::from(duration.subsec_nanos() > 0);
    whole.min(max)
}

/// The `DelaySeconds` that holds a job until `available_at`, or as close to
/// it as SQS goes.
fn delay_secs(available_at: DateTime<Utc>) -> u64 {
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
    fn a_china_region_gets_the_china_endpoint() {
        assert_eq!(
            default_endpoint("cn-north-1"),
            "https://sqs.cn-north-1.amazonaws.com.cn"
        );
        assert_eq!(
            default_endpoint("eu-west-2"),
            "https://sqs.eu-west-2.amazonaws.com"
        );
    }

    #[test]
    fn one_key_without_the_other_is_refused_without_repeating_it() {
        let error = SqsConfig::from_variables(variables(&[
            ("AWS_DEFAULT_REGION", "us-east-1"),
            ("AWS_ACCESS_KEY_ID", "AKIDONLYHALF"),
        ]))
        .expect_err("one key without the other must be refused");
        let text = error.to_string();
        assert!(text.contains("AWS_SECRET_ACCESS_KEY"), "{text}");
        assert!(!text.contains("AKIDONLYHALF"), "{text}");
    }

    #[test]
    fn the_credentials_debug_output_hides_the_secret() {
        let keys = SqsCredentials {
            access_key_id: "AKIDVISIBLE".into(),
            secret_access_key: "very-secret".into(),
            session_token: Some("also-secret".into()),
        };
        let text = format!("{keys:?}");
        assert!(text.contains("AKIDVISIBLE"));
        assert!(
            !text.contains("very-secret") && !text.contains("also-secret"),
            "{text}"
        );
    }

    #[test]
    fn a_wait_that_is_not_a_number_or_over_twenty_seconds_is_refused() {
        assert!(
            SqsConfig::from_variables(variables(&[
                ("AWS_DEFAULT_REGION", "us-east-1"),
                ("SQS_WAIT_TIME_SECONDS", "soon"),
            ]))
            .is_err()
        );
        let mut config = SqsConfig::new("us-east-1", "https://sqs.us-east-1.amazonaws.com/1/q");
        config.wait_time_seconds = 21;
        let Err(error) = SqsQueueDriver::new(config) else {
            panic!("a wait over 20 seconds must be refused");
        };
        assert!(error.to_string().contains("SQS_WAIT_TIME_SECONDS"));
    }

    #[test]
    fn an_endpoint_that_is_not_a_url_is_refused() {
        let mut config = SqsConfig::new("us-east-1", "https://sqs.us-east-1.amazonaws.com/1/q");
        config.endpoint = Some("localhost:9324".into());
        assert!(SqsQueueDriver::new(config).is_err());
    }

    #[test]
    fn a_fifo_queue_url_is_refused() {
        let config = SqsConfig::new(
            "us-east-1",
            "https://sqs.us-east-1.amazonaws.com/1/orders.fifo",
        );
        let Err(error) = SqsQueueDriver::new(config) else {
            panic!("a FIFO queue URL must be refused");
        };
        assert!(error.to_string().contains("FIFO"));
    }

    #[test]
    fn stale_receipt_and_retryable_errors_are_told_apart() {
        let error = |status, code: &str, message: &str| SqsError {
            status,
            code: code.into(),
            message: message.into(),
        };
        assert!(error(400, "ReceiptHandleIsInvalid", "").is_stale_receipt());
        assert!(
            error(
                400,
                "InvalidParameterValue",
                "Value x for parameter ReceiptHandle is invalid. Reason: The receipt handle has expired."
            )
            .is_stale_receipt()
        );
        assert!(
            !error(
                400,
                "InvalidParameterValue",
                "Total VisibilityTimeout for the message is beyond the limit"
            )
            .is_stale_receipt()
        );
        assert!(error(400, "RequestThrottled", "").is_retryable());
        assert!(error(503, "HTTP 503", "").is_retryable());
        assert!(!error(400, "QueueDoesNotExist", "").is_retryable());
    }
}
