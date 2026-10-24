//! Application-chosen login budgets, separate from the authentication guard.

use std::sync::Arc;
use std::time::Duration;

use crate::events::EventFacade;
use crate::rate_limit::RateLimiter;
use crate::{
    Authenticatable, Credentials, FrameworkError, Middleware, Next, Request, Response,
    ValidationErrors,
};

/// Request metadata carried by a login lockout. Bodies, headers and query
/// strings are excluded so credentials never enter the event payload.
#[derive(Clone, Debug)]
pub struct LoginThrottleRequest {
    /// Trusted client address, or `unknown` when no address is available.
    pub ip: String,
    /// HTTP method, empty when constructed with only an email and IP.
    pub method: String,
    /// Request path without its query, empty when constructed with only an IP.
    pub path: String,
}

impl LoginThrottleRequest {
    fn from_request(request: &Request) -> Self {
        Self {
            ip: request.ip().unwrap_or_else(|| "unknown".into()),
            method: request.method().to_string(),
            path: request.path().to_owned(),
        }
    }
}

tokio::task_local! {
    static LOGIN_REQUEST: LoginThrottleRequest;
}

/// Password failure budget chosen by the application, keyed by lowercase(email)|ip.
///
/// This is opt-in for existing applications. `Auth::attempt` stays unchanged.
#[derive(Clone, Debug)]
pub struct LoginThrottle {
    key: String,
    request: LoginThrottleRequest,
    max_attempts: i64,
    decay_seconds: u64,
}

impl LoginThrottle {
    /// Build a login budget with the attempts and window the application chooses.
    /// Both must be positive, and the window must be a supported whole-second duration.
    /// Use the trusted client address from `Request::ip`, never a raw proxy header.
    pub fn new(
        email: &str,
        client_ip: &str,
        attempts: u32,
        window: Duration,
    ) -> Result<Self, FrameworkError> {
        if attempts == 0 {
            return Err(FrameworkError::param("login max_attempts must be positive"));
        }
        let decay_seconds = window.as_secs();
        let supported_deadline = i64::try_from(decay_seconds)
            .ok()
            .and_then(|seconds| chrono::Utc::now().timestamp().checked_add(seconds));
        if decay_seconds == 0
            || window.subsec_nanos() != 0
            || supported_deadline.is_none()
            || std::time::Instant::now().checked_add(window).is_none()
        {
            return Err(FrameworkError::param(
                "login window must be a positive supported whole-second duration",
            ));
        }
        Ok(Self {
            key: format!("{}|{client_ip}", email.to_lowercase()),
            request: LoginThrottleRequest {
                ip: client_ip.to_owned(),
                method: String::new(),
                path: String::new(),
            },
            max_attempts: i64::from(attempts),
            decay_seconds,
        })
    }

    /// Build the application's chosen budget and trusted HTTP lockout metadata.
    pub fn for_request(
        email: &str,
        request: &Request,
        attempts: u32,
        window: Duration,
    ) -> Result<Self, FrameworkError> {
        Self::from_context(
            email,
            LoginThrottleRequest::from_request(request),
            attempts,
            window,
        )
    }

    fn from_context(
        email: &str,
        request: LoginThrottleRequest,
        attempts: u32,
        window: Duration,
    ) -> Result<Self, FrameworkError> {
        let mut throttle = Self::new(email, &request.ip, attempts, window)?;
        throttle.request = request;
        Ok(throttle)
    }

    /// Use the application's chosen budget for Live actions behind [`Self::request_context`].
    /// Missing middleware returns an error rather than using client-controlled action arguments.
    pub fn for_current_request(
        email: &str,
        attempts: u32,
        window: Duration,
    ) -> Result<Self, FrameworkError> {
        LOGIN_REQUEST
            .try_with(|request| Self::from_context(email, request.clone(), attempts, window))
            .map_err(|_| {
                FrameworkError::internal("LoginThrottle requires its request_context middleware")
            })?
    }

    /// Capture trusted request metadata for Live actions in this request only.
    pub fn request_context() -> impl Middleware {
        LoginRequestContext
    }

    /// Seconds remaining in the current window, or zero when it has expired.
    pub async fn available_in(&self) -> Result<u64, FrameworkError> {
        RateLimiter::available_in(&self.key).await
    }

    /// Refuse an exhausted budget before any password verification, including
    /// a correct password. Cache errors fail the attempt closed.
    pub async fn ensure_not_rate_limited(&self) -> Result<(), FrameworkError> {
        if !RateLimiter::too_many_attempts(&self.key, self.max_attempts).await? {
            return Ok(());
        }
        EventFacade::dispatch(super::events::Lockout {
            request: self.request.clone(),
        })
        .await?;
        let seconds = self.available_in().await?;
        let mut errors = ValidationErrors::new();
        errors.add(
            "email",
            format!("Too many login attempts. Please try again in {seconds} seconds."),
        );
        Err(FrameworkError::Validation(errors))
    }

    /// Count a failed credential check, whether or not the account exists.
    pub async fn hit(&self) -> Result<(), FrameworkError> {
        RateLimiter::hit(&self.key, self.decay_seconds).await?;
        Ok(())
    }

    /// Clear the failed-attempt budget after a successful sign-in.
    pub async fn clear(&self) -> Result<(), FrameworkError> {
        RateLimiter::clear(&self.key).await
    }

    /// Check the budget, call `Auth::attempt`, then count failure or clear
    /// success. Existing applications can replace their `Auth::attempt` call.
    pub async fn attempt(
        &self,
        credentials: &Credentials,
        remember: bool,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        self.ensure_not_rate_limited().await?;
        let user = super::Auth::attempt(credentials, remember).await?;
        if user.is_some() {
            self.clear().await?;
        } else {
            self.hit().await?;
        }
        Ok(user)
    }
}

struct LoginRequestContext;

#[async_trait::async_trait]
impl Middleware for LoginRequestContext {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let context = LoginThrottleRequest::from_request(&request);
        LOGIN_REQUEST.scope(context, next(request)).await
    }
}
