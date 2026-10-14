//! [`ValidateSignature`] - the route middleware that admits a request only
//! with a valid signed URL. Mirrors
//! `Illuminate\Routing\Middleware\ValidateSignature`.
//!
//! A route that serves a signed link, such as an email verification or a
//! one-time download, checks the signature before its handler runs. A
//! handler that called [`crate::url::has_valid_signature`] itself could
//! forget to; the middleware puts the check on the route.

use std::sync::RwLock;

use async_trait::async_trait;

use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{Middleware, Next};

/// The message of the `403` a request without a valid signature gets,
/// Laravel's `InvalidSignatureException` message.
const INVALID_SIGNATURE: &str = "Invalid signature.";

/// The alias argument that asks for a relative signature, read first.
const RELATIVE_ARGUMENT: &str = "relative";

/// Parameters every [`ValidateSignature`] leaves out, set with
/// [`ValidateSignature::except`]. Laravel's static `$neverValidate`.
static NEVER_VALIDATE: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Route middleware that lets a request through only when its URL carries
/// a valid signature.
///
/// A request whose signature is missing, wrong or expired gets `403` with
/// the message `Invalid signature.` in the framework's usual error body,
/// as Laravel's `InvalidSignatureException` answers. One answer for all
/// three keeps the response from telling a prober which part failed. A
/// handler that needs to tell an expired link from a forged one, to offer
/// a fresh link, matches on [`crate::url::signature_verdict`] instead.
///
/// Suprnova signs the public root, the path and the key-sorted query of
/// every URL, never the scheme or the host. [`Self::new`] and
/// [`Self::relative`] therefore verify the same text: `relative` exists so
/// a route written for Laravel's `signed:relative` keeps its meaning. A
/// server without an encryption key answers `500`, because it cannot check
/// any signature.
///
/// ```rust,no_run
/// use suprnova::middleware::register_middleware_alias_with_args;
/// use suprnova::routing::ValidateSignature;
///
/// // `.middleware_named("signed")` or `"signed:relative,utm_source"`.
/// register_middleware_alias_with_args("signed", ValidateSignature::from_alias_args);
///
/// // Or on one route, ignoring a parameter a mail client appends.
/// let middleware = ValidateSignature::new().ignore(["utm_source"]);
/// # let _ = middleware;
/// ```
#[derive(Debug, Clone, Default)]
pub struct ValidateSignature {
    /// Parameters this instance leaves out of the verified text.
    ignore: Vec<String>,
}

impl ValidateSignature {
    /// A middleware that verifies every query parameter the URL carries.
    pub fn new() -> Self {
        Self::default()
    }

    /// The middleware Laravel's `ValidateSignature::relative($ignore)`
    /// names, leaving `ignore` out of the verified text.
    ///
    /// Suprnova's signatures never cover the scheme or the host, so this
    /// verifies the same text as [`Self::new`] followed by
    /// [`Self::ignore`].
    pub fn relative<I, S>(ignore: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new().ignore(ignore)
    }

    /// Leave the query parameters `names` out of the verified text, for a
    /// parameter added to a signed link after it was minted, such as the
    /// `utm_source` a mail client appends. The handler reads that
    /// parameter's value unsigned, so it must not trust it.
    ///
    /// `signature` and `expires` cannot be ignored. Names add to the ones
    /// given before.
    pub fn ignore<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for name in names {
            let name = name.into();
            if !self.ignore.contains(&name) {
                self.ignore.push(name);
            }
        }
        self
    }

    /// Leave the query parameters `names` out of the verified text of
    /// every [`ValidateSignature`] in the application, as Laravel's
    /// `ValidateSignature::except($parameters)` does. Call it at boot.
    ///
    /// The list only grows: a name given twice is kept once. It applies to
    /// the middleware, not to [`crate::url::has_valid_signature`], as
    /// Laravel's `$neverValidate` does.
    pub fn except<I, S>(names: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut never = NEVER_VALIDATE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for name in names {
            let name = name.into();
            if !never.contains(&name) {
                never.push(name);
            }
        }
    }

    /// Build the middleware from the arguments of a `signed` alias, the
    /// way Laravel reads them. It is the factory to register the alias
    /// with:
    ///
    /// ```rust,no_run
    /// use suprnova::middleware::register_middleware_alias_with_args;
    /// use suprnova::routing::ValidateSignature;
    ///
    /// register_middleware_alias_with_args("signed", ValidateSignature::from_alias_args);
    /// ```
    ///
    /// | A route writes | It gets |
    /// |---|---|
    /// | `signed` | [`Self::new`] |
    /// | `signed:relative` | [`Self::relative`] with nothing ignored |
    /// | `signed:relative,utm_source` | [`Self::relative`] ignoring `utm_source` |
    /// | `signed:utm_source,ref` | [`Self::new`] ignoring `utm_source` and `ref` |
    ///
    /// # Errors
    ///
    /// An empty parameter name, as in `signed:relative,`, which is a typo
    /// rather than a parameter. The route that names the alias then fails
    /// to register, at boot.
    pub fn from_alias_args(arguments: &[&str]) -> Result<Self, FrameworkError> {
        let names = match arguments.split_first() {
            Some((&RELATIVE_ARGUMENT, rest)) => rest,
            _ => arguments,
        };
        if names.iter().any(|name| name.is_empty()) {
            return Err(FrameworkError::internal(format!(
                "signed:{} names an empty parameter to ignore",
                arguments.join(",")
            )));
        }
        Ok(Self::new().ignore(names.iter().copied()))
    }

    /// The names this instance leaves out, followed by the ones
    /// [`Self::except`] set for the application.
    fn ignored(&self) -> Vec<String> {
        let never = NEVER_VALIDATE
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut names = self.ignore.clone();
        names.extend(
            never
                .iter()
                .filter(|name| !self.ignore.contains(name))
                .cloned(),
        );
        names
    }
}

#[async_trait]
impl Middleware for ValidateSignature {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let ignored = self.ignored();
        let ignored: Vec<&str> = ignored.iter().map(String::as_str).collect();
        match crate::routing::url::has_valid_signature_ignoring(&request, &ignored) {
            Ok(true) => next(request).await,
            Ok(false) => Err(HttpResponse::from(FrameworkError::domain(
                INVALID_SIGNATURE,
                403,
            ))),
            // No key to check with: a misconfigured server, not a client
            // error, and never a pass.
            Err(error) => Err(HttpResponse::from(error)),
        }
    }
}
