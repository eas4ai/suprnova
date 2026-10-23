//! Request extraction traits for handler parameter injection
//!
//! This module provides the `FromRequest` trait which enables the `#[handler]`
//! macro to automatically extract typed parameters from incoming requests.

use super::Request;
use crate::error::FrameworkError;
use async_trait::async_trait;

/// Trait for types that can be extracted from an HTTP request
///
/// This trait is used by the `#[handler]` macro to automatically
/// extract and inject typed parameters into controller handlers.
///
/// # Implementations
///
/// - `Request` - passes the request through unchanged
/// - Any type implementing `FormRequest` - automatically parses and validates
///
/// # Example
///
/// The `#[handler]` macro uses this trait to transform:
///
/// ```rust,ignore
/// #[handler]
/// pub async fn store(form: CreateUserRequest) -> Response {
///     // ...
/// }
/// ```
///
/// Into:
///
/// ```rust,ignore
/// pub async fn store(req: Request) -> Response {
///     let form = <CreateUserRequest as FromRequest>::from_request(req).await?;
///     // ...
/// }
/// ```
#[async_trait]
pub trait FromRequest: Sized + Send {
    /// Extract Self from the incoming request
    ///
    /// Returns `Err(FrameworkError)` if extraction fails, which will be
    /// converted to an appropriate HTTP error response.
    async fn from_request(req: Request) -> Result<Self, FrameworkError>;
}

/// Request passes through unchanged
#[async_trait]
impl FromRequest for Request {
    async fn from_request(req: Request) -> Result<Self, FrameworkError> {
        Ok(req)
    }
}

/// Trait for types that can be extracted from a single path parameter
///
/// This trait enables automatic extraction of typed values from route parameters
/// like `/users/{id}` where `{id}` can be extracted as an `i32`.
///
/// # Example
///
/// The `#[handler]` macro uses this trait to transform:
///
/// ```rust,ignore
/// #[handler]
/// pub async fn show(id: i32, slug: String) -> Response {
///     // ...
/// }
/// ```
///
/// Into code that extracts `id` and `slug` from the route parameters.
pub trait FromParam: Sized {
    /// Extract Self from a string parameter value
    ///
    /// Returns `Err(FrameworkError)` if extraction fails, which will be
    /// converted to an appropriate HTTP error response (400 Bad Request).
    fn from_param(value: &str) -> Result<Self, FrameworkError>;
}

impl FromParam for String {
    fn from_param(value: &str) -> Result<Self, FrameworkError> {
        Ok(value.to_string())
    }
}

impl FromParam for i32 {
    fn from_param(value: &str) -> Result<Self, FrameworkError> {
        value
            .parse()
            .map_err(|_| FrameworkError::param_parse(value, "i32"))
    }
}

impl FromParam for i64 {
    fn from_param(value: &str) -> Result<Self, FrameworkError> {
        value
            .parse()
            .map_err(|_| FrameworkError::param_parse(value, "i64"))
    }
}

impl FromParam for u32 {
    fn from_param(value: &str) -> Result<Self, FrameworkError> {
        value
            .parse()
            .map_err(|_| FrameworkError::param_parse(value, "u32"))
    }
}

impl FromParam for u64 {
    fn from_param(value: &str) -> Result<Self, FrameworkError> {
        value
            .parse()
            .map_err(|_| FrameworkError::param_parse(value, "u64"))
    }
}

impl FromParam for usize {
    fn from_param(value: &str) -> Result<Self, FrameworkError> {
        value
            .parse()
            .map_err(|_| FrameworkError::param_parse(value, "usize"))
    }
}
