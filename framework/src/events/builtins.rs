//! Framework-emitted events. Consumers can listen to these the same
//! way they listen to their own events.

use super::Event;

/// Dispatched **best-effort** when a `FrameworkError` converts to a 5xx
/// response, so listeners can ship to Sentry, Datadog, Slack, etc.
///
/// Delivery is not guaranteed. The dispatch is spawned (not awaited) so it
/// never blocks response conversion, and if no Tokio runtime is active at
/// conversion time - e.g. a 5xx path exercised by a non-async unit test - it
/// is dropped. Inside a running server every 5xx triggers it. Treat it as a
/// high-signal error feed, not a complete audit log: a listener that must
/// observe every error should also consume the structured `tracing` 5xx logs,
/// which are emitted unconditionally on the same path.
#[derive(Debug, Clone)]
pub struct ErrorOccurred {
    /// The full internal error message, including its `source` chain
    /// (see [`crate::render_error_chain`]). This is deliberately **not**
    /// sanitized - the client received a generic message instead.
    /// A listener forwarding this value to an external or user-visible
    /// sink must sanitize it itself.
    pub error_message: String,
    /// HTTP status code of the response (always 5xx in current usage).
    pub status_code: u16,
    /// Request id of the failing request, when one was installed.
    pub request_id: Option<String>,
}

impl Event for ErrorOccurred {
    fn event_name() -> &'static str {
        "ErrorOccurred"
    }
}

/// Dispatched when the SSR worker failed to render a first visit (PAR-059):
/// it answered with an error, or could not be reached. Laravel's
/// `Inertia\Ssr\SsrRenderFailed`.
///
/// The worker's error JSON says why a page failed on the server, which the
/// client-rendered fallback hides: a component that touched `window`, one
/// the worker could not resolve, or a render that threw. A listener sends it
/// to the application's error tracker. The event is dispatched inline,
/// before the visit falls back to the client or, under
/// `InertiaConfig::ssr_throw_on_error`, fails, and only when something
/// listens for it or a fake records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsrRenderFailed {
    /// The page component the worker was asked to render (`Unknown` when
    /// the page carried none).
    pub component: String,
    /// The page's URL (`/` when the page carried none).
    pub url: String,
    /// The worker's error message, or the transport failure's for a
    /// worker that could not be reached.
    pub error: String,
    /// What kind of failure it was; [`SsrErrorType::Connection`] for a
    /// transport failure.
    pub error_type: SsrErrorType,
    /// The worker's hint on how to fix the error, when it gave one.
    pub hint: Option<String>,
    /// The browser API the component used on the server (`window`,
    /// `document`, ...), for a [`SsrErrorType::BrowserApi`] failure.
    pub browser_api: Option<String>,
    /// The worker's stack trace, when it sent one.
    pub stack: Option<String>,
    /// Where the error occurred (`file:line:column`), when the worker
    /// knows.
    pub source_location: Option<String>,
}

impl SsrRenderFailed {
    /// The message a visit fails with under `ssr_throw_on_error`, Laravel's
    /// `SsrException::fromEvent`: the component, the error, and the source
    /// location when there is one.
    pub fn message(&self) -> String {
        let mut message = format!(
            "SSR render failed for component [{}]: {}",
            self.component, self.error
        );
        if let Some(location) = self.source_location.as_deref().filter(|l| !l.is_empty()) {
            message.push_str(" at ");
            message.push_str(location);
        }
        message
    }
}

impl Event for SsrRenderFailed {
    fn event_name() -> &'static str {
        "SsrRenderFailed"
    }
}

/// The kind of an SSR render failure, Laravel's `Inertia\Ssr\SsrErrorType`.
/// The worker names it in its error JSON's `type`; the framework uses
/// [`Connection`](Self::Connection) when the worker could not be reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SsrErrorType {
    /// The component used a browser API (`window`, `document`, ...) while
    /// rendering on the server. `browser-api`.
    BrowserApi,
    /// The worker could not resolve the page component.
    /// `component-resolution`.
    ComponentResolution,
    /// The component threw while rendering. `render`.
    Render,
    /// The worker could not be reached, or its answer could not be read.
    /// `connection`.
    Connection,
    /// Any other failure, and a type the worker named that is none of the
    /// above. `unknown`.
    Unknown,
}

impl SsrErrorType {
    /// The type the worker's `type` names, [`Unknown`](Self::Unknown) for a
    /// name it does not know, as Laravel's `SsrErrorType::fromString`.
    pub fn from_name(name: &str) -> Self {
        match name {
            "browser-api" => Self::BrowserApi,
            "component-resolution" => Self::ComponentResolution,
            "render" => Self::Render,
            "connection" => Self::Connection,
            _ => Self::Unknown,
        }
    }

    /// The type's name, the string the worker sends and Laravel's enum
    /// carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BrowserApi => "browser-api",
            Self::ComponentResolution => "component-resolution",
            Self::Render => "render",
            Self::Connection => "connection",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for SsrErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
