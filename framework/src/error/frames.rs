//! The stack frames the development error page lists (PAR-013).
//!
//! # When frames are recorded
//!
//! Only while the server serves a request with debug mode on. The server
//! reads `Config::is_debug()` once per request and, when it is on, runs
//! the request inside [`record_frames`]. Inside that scope:
//!
//! - every `FrameworkError` and `AppError` constructor, and every `From`
//!   conversion into a `FrameworkError`, calls [`record_creation`], which
//!   captures the stack at that point;
//! - the panic hook calls [`capture_for_panic`], which captures the stack
//!   of the panic.
//!
//! Outside the scope - debug off, a CLI command, a queue worker, a task
//! the request spawned - both return at once and record nothing. With
//! debug off the server never opens the scope, so no request records a
//! frame; the unit tests below hold that rule, since no response can show
//! it.
//!
//! The constructors and conversions are `#[track_caller]`, so each
//! capture also keeps the site that called them: the line of a `?` that
//! converts the error, or of a call to a constructor. A call made from
//! inside a function that is not `#[track_caller]` reports that
//! function's line instead: `.map_err(FrameworkError::from)` reports a
//! line of `Result::map_err` in the toolchain, and the stack frames are
//! then what lead to the application's code. A panic keeps the location
//! the panic hook reports.
//!
//! # How a report finds its frames
//!
//! `FrameworkError` is a public enum with public variants, so it cannot
//! carry the frames itself without breaking every `match` an application
//! wrote against it. The recorder keeps them beside the error instead:
//! the frames of the last [`MAX_RECORDED`] errors created in the request,
//! each under the error's message. An `ErrorReport` built for an error
//! takes the most recent frames recorded under that error's message.
//! `FrameworkError::context` changes the message, so it moves the frames
//! to the new one with [`rename`].
//!
//! An error with no entry has no frames, and the page says so: an error
//! created on another task, before the request began, or built as a
//! struct literal rather than through a constructor. The recorder is a
//! task-local, so a request never sees another request's frames.
//!
//! # How a frame is classified
//!
//! A frame belongs to the crate that owns its function: the first path
//! segment of the function's name. Names of generic code carry the type
//! names it was instantiated with, so a frame of the standard library can
//! name an application type in its parameters; the owner is read from the
//! front of the name, never by searching inside it. For a trait method,
//! `<Type as Trait>::method`, the owner is the crate of `Type`, or of
//! `Trait` when `Type` is a bare generic parameter or a closure (a
//! closure has no impls of its own: an impl for one is a blanket impl in
//! the trait's crate, or the compiler's `Fn` glue in `core`). So
//! `<app::Auth as suprnova::Middleware>::handle` is the application's
//! frame, and `<core::pin::Pin<P> as core::future::Future>::poll` is the
//! standard library's.
//!
//! - **Standard library**: `std`, `core`, `alloc` and the other crates
//!   shipped with Rust, the compiler's own `__`-prefixed paths, any frame
//!   whose source is in the Rust toolchain, and frames with no crate path
//!   at all (the C runtime, thread start).
//! - **Async runtime**: `tokio` and the `tokio_*` crates.
//! - **Framework**: the library crates of this repository, by their
//!   `[lib]` names ([`FRAMEWORK_CRATES`]): `suprnova`, `suprnova_live`,
//!   `magnetar`, `suprnova_web_push`, the `suprnova_payments_*` adapters
//!   and their helpers.
//! - **Dependency**, by name, for the async and HTTP stack every request
//!   runs on ([`ASYNC_STACK_CRATES`]: `hyper`, `h2`, `http`, `tower`,
//!   `futures`, `tracing` and the like), wherever their source lives.
//! - **Application** or **dependency**, for every other crate. When the
//!   binary has debug info, the frame's source file decides: a file in
//!   Cargo's registry, a Cargo git checkout or a `vendor` directory is a
//!   dependency's, any other file is the application's. Without debug
//!   info, frames have no file, and the stack decides: the crates the
//!   framework's request dispatch polls, and the crate that called the
//!   constructor from local source, are the application's (see
//!   [`application_crates`]), and every frame of those crates is shown.
//!   Every other crate is a dependency.
//!
//! The page shows the application's frames and collapses every run of
//! the others behind a count.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::panic::Location;
use std::sync::Arc;

/// How many errors a request remembers frames for. A request that
/// creates and discards more errors than this keeps the most recent ones;
/// the one that fails the request is created last, or close to it.
pub(crate) const MAX_RECORDED: usize = 32;

/// The most frames one capture lists. Deep recursion can produce
/// thousands; the page names how many it left out.
pub(crate) const MAX_FRAMES: usize = 512;

tokio::task_local! {
    /// The frames recorded for the errors created in the current request.
    ///
    /// A task-local, so it exists only while this request's own future is
    /// polled: an error created on another task records nothing, and two
    /// requests never share a recorder.
    static RECORDER: RefCell<Recorder>;
}

#[derive(Default)]
struct Recorder {
    /// `(message, frames)` pairs, oldest first.
    recorded: VecDeque<(String, RecordedFrames)>,
}

/// The stack captured where an error was created or a panic was raised.
///
/// Capturing walks the stack and keeps raw addresses only. Function
/// names and source locations are looked up in [`Self::resolve`], which
/// only the development error page calls.
#[derive(Clone)]
pub(crate) struct RecordedFrames {
    backtrace: Arc<Backtrace>,
    /// `file:line:column` of the code that created the error or raised
    /// the panic.
    site: String,
}

impl RecordedFrames {
    fn capture(site: String) -> Self {
        // `force_capture`, not `capture`: the page must not depend on
        // `RUST_BACKTRACE`, which is unset in most development shells.
        Self {
            backtrace: Arc::new(Backtrace::force_capture()),
            site,
        }
    }

    /// `file:line:column` of the code that created the error or raised
    /// the panic.
    pub(crate) fn site(&self) -> &str {
        &self.site
    }

    /// Look up each frame's function and source location, and classify
    /// it.
    pub(crate) fn resolve(&self) -> ResolvedFrames {
        if self.backtrace.status() != BacktraceStatus::Captured {
            return ResolvedFrames::default();
        }
        resolve_text(&self.backtrace.to_string(), &self.site)
    }
}

impl fmt::Debug for RecordedFrames {
    /// Never resolves the frames: that is slow, and a report's `Debug`
    /// output is read in test failures, not on the page.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordedFrames")
            .field("site", &self.site)
            .finish_non_exhaustive()
    }
}

/// The frames of one capture, innermost first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedFrames {
    /// At most [`MAX_FRAMES`] frames.
    pub(crate) frames: Vec<Frame>,
    /// How many frames beyond [`MAX_FRAMES`] were left out.
    pub(crate) omitted: usize,
}

/// One stack frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    /// The function, as the standard library prints it: without the
    /// symbol hash.
    pub(crate) function: String,
    /// `file:line:column`, when the binary has the debug info for it.
    pub(crate) location: Option<String>,
    /// Whose code the frame runs.
    pub(crate) origin: Origin,
}

/// Whose code a frame runs. See the module docs for the rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The application's own code: shown on the page.
    App,
    /// Suprnova's crates.
    Framework,
    /// The async runtime.
    Runtime,
    /// Any other dependency.
    Dependency,
    /// The standard library, the C runtime, or a frame with no name.
    Std,
}

impl Origin {
    /// What the page calls the group.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::App => "application",
            Self::Framework => "framework",
            Self::Runtime => "async runtime",
            Self::Dependency => "dependencies",
            Self::Std => "standard library",
        }
    }
}

/// Run `future` with frame recording on when `enabled` is true, and as it
/// is when it is false.
///
/// The server passes `Config::is_debug()`, read once for the request.
/// Taking the decision as an argument keeps the rule testable without
/// changing the process's debug mode.
pub(crate) async fn record_frames<F: Future>(enabled: bool, future: F) -> F::Output {
    if enabled {
        RECORDER
            .scope(RefCell::new(Recorder::default()), future)
            .await
    } else {
        future.await
    }
}

/// Whether the current task records frames.
pub(crate) fn is_recording() -> bool {
    RECORDER.try_with(|_| ()).is_ok()
}

/// Record the stack for `error`, just created at `site`, under its
/// message.
///
/// Returns at once, recording nothing, outside [`record_frames`].
pub(crate) fn record_creation(error: &dyn fmt::Display, site: &'static Location<'static>) {
    if !is_recording() {
        return;
    }
    // Both run before the recorder is borrowed: formatting an error is
    // application code, and may create errors of its own.
    let message = error.to_string();
    let frames = RecordedFrames::capture(site.to_string());
    let _ = RECORDER.try_with(|recorder| {
        if let Ok(mut recorder) = recorder.try_borrow_mut() {
            if recorder.recorded.len() >= MAX_RECORDED {
                recorder.recorded.pop_front();
            }
            recorder.recorded.push_back((message, frames));
        }
    });
}

/// Move the frames recorded under `before` to `after`: the error was
/// rewrapped with a new message.
pub(crate) fn rename(before: &str, after: String) {
    let _ = RECORDER.try_with(|recorder| {
        if let Ok(mut recorder) = recorder.try_borrow_mut()
            && let Some(entry) = recorder
                .recorded
                .iter_mut()
                .rev()
                .find(|(message, _)| message == before)
        {
            entry.0 = after;
        }
    });
}

/// The frames most recently recorded under `message` in this request.
pub(crate) fn recorded_for(message: &str) -> Option<RecordedFrames> {
    RECORDER
        .try_with(|recorder| {
            recorder.try_borrow().ok().and_then(|recorder| {
                recorder
                    .recorded
                    .iter()
                    .rev()
                    .find(|(recorded, _)| recorded == message)
                    .map(|(_, frames)| frames.clone())
            })
        })
        .ok()
        .flatten()
}

/// The stack of a panic being raised now at `location`, when the current
/// task records frames. Called from the panic hook.
pub(crate) fn capture_for_panic(location: &Location<'_>) -> Option<RecordedFrames> {
    is_recording().then(|| RecordedFrames::capture(location.to_string()))
}

/// Parse the standard library's `Display` of a backtrace, then classify
/// each frame. `site` is where the error was created or the panic raised.
fn resolve_text(text: &str, site: &str) -> ResolvedFrames {
    let mut resolved = parse_frames(text);
    let app_crates = application_crates(&resolved.frames, site);
    for frame in &mut resolved.frames {
        frame.origin = classify(&frame.function, frame.location.as_deref(), &app_crates);
    }
    resolved
}

/// The crates a stack without debug info names as the application's.
///
/// Two signals, either enough:
///
/// - **The dispatch.** Every crate whose frame the framework's request
///   dispatch polls: the middleware chain, the router and the
///   framework's own middleware poll the application's middleware and
///   handlers (see [`is_dispatch`]). Walking from the outermost frame
///   inward, a frame of a crate that is not the standard library's, the
///   async stack's or the framework's, whose nearest outer frame of any
///   such crate or of the framework is a dispatch frame, is the
///   application's. A dependency the framework calls elsewhere, such as
///   `sea_orm` under the Eloquent builder, is not.
/// - **The creation site.** When the site that created the error or
///   raised the panic is local source, not downloaded or in the
///   toolchain, the crate of the frame that called the constructor, or
///   that panicked.
fn application_crates(frames: &[Frame], site: &str) -> Vec<String> {
    let mut crates: Vec<String> = Vec::new();
    let mut called_by_dispatch = false;
    for frame in frames.iter().rev() {
        let Some(owner) = owner_crate(&frame.function) else {
            continue;
        };
        if is_dispatch(&frame.function) {
            called_by_dispatch = true;
            continue;
        }
        if is_std_crate(owner) || is_async_stack_crate(owner) {
            continue;
        }
        if FRAMEWORK_CRATES.contains(&owner) {
            called_by_dispatch = false;
            continue;
        }
        if called_by_dispatch && !crates.iter().any(|known| known == owner) {
            crates.push(owner.to_string());
        }
        called_by_dispatch = false;
    }
    if is_local_source(&site_file(site).replace('\\', "/"))
        && let Some(owner) = creation_crate(frames)
        && !crates.iter().any(|known| known == owner)
    {
        crates.push(owner.to_string());
    }
    crates
}

/// Whether `function` is part of the framework's request dispatch, the
/// frames that poll the application's middleware and handlers:
///
/// - the middleware chain and the router (`suprnova::middleware` and
///   `suprnova::routing`);
/// - the body of one of the framework's own middleware
///   (`<suprnova::.. as suprnova::middleware::Middleware>::handle`), which
///   polls what `next` returned: the next middleware, or the handler;
/// - the poll of a boxed `Response` future, when the name spells out its
///   instantiation, as v0 symbol names do.
fn is_dispatch(function: &str) -> bool {
    let in_module = ["suprnova::middleware::", "suprnova::routing::"]
        .iter()
        .any(|module| {
            function.starts_with(module)
                || function
                    .strip_prefix('<')
                    .is_some_and(|qualified| qualified.starts_with(module))
        });
    let framework_middleware = function.contains(" as suprnova::middleware::Middleware>::")
        && owner_crate(function).is_some_and(|owner| FRAMEWORK_CRATES.contains(&owner));
    let response_future = function.contains(
        "Future<Output = core::result::Result<suprnova::http::response::HttpResponse, \
         suprnova::http::response::HttpResponse>>",
    );
    in_module || framework_middleware || response_future
}

/// Split the standard library's `Display` of a backtrace into frames: one
/// `  N: function` line per frame, each followed by an `at file:line:col`
/// line when the binary has the debug info for it. Every frame comes back
/// classified as the standard library's; [`resolve_text`] classifies them.
fn parse_frames(text: &str) -> ResolvedFrames {
    let mut resolved = ResolvedFrames::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(location) = line.strip_prefix("at ") {
            if resolved.omitted == 0
                && let Some(frame) = resolved.frames.last_mut()
                && frame.location.is_none()
            {
                frame.location = Some(location.trim().to_string());
            }
            continue;
        }
        if resolved.frames.len() >= MAX_FRAMES {
            resolved.omitted += 1;
            continue;
        }
        let function = match line.split_once(": ") {
            Some((index, function))
                if !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) =>
            {
                function
            }
            _ => line,
        };
        resolved.frames.push(Frame {
            function: function.to_string(),
            location: None,
            origin: Origin::Std,
        });
    }
    resolved
}

/// The crates that ship with Rust. Crate paths that start with `__` are
/// the compiler's own, such as `__rustc::rust_begin_unwind`; see
/// [`is_std_crate`].
const STD_CRATES: &[&str] = &[
    "std",
    "core",
    "alloc",
    "proc_macro",
    "test",
    "panic_unwind",
    "panic_abort",
    "unwind",
    "std_detect",
    "compiler_builtins",
];

/// The library crates of this repository, by the names their frames
/// carry: each package's `[lib]` name, which for `suprnova-magnetar` is
/// `magnetar`.
const FRAMEWORK_CRATES: &[&str] = &[
    "suprnova",
    "suprnova_macros",
    "suprnova_live",
    "suprnova_live_test_support",
    "suprnova_live_macro_fixture",
    "magnetar",
    "suprnova_web_push",
    "suprnova_payments_stripe",
    "suprnova_payments_paddle",
    "suprnova_payments_nowpayments",
];

/// The async and HTTP stack every request runs on, besides `tokio`.
/// Their frames are dependencies by name, wherever their source lives: a
/// vendored, Nix or Bazel build gives them paths no rule can recognize,
/// and every request has them on its stack. A name ending in `_` covers
/// the crate and every crate it prefixes (`tower_` covers `tower_http`).
const ASYNC_STACK_CRATES: &[&str] = &[
    "hyper",
    "hyper_",
    "h2",
    "http",
    "http_body",
    "http_body_",
    "tower",
    "tower_",
    "futures",
    "futures_",
    "async_trait",
    "tracing",
    "tracing_",
    "mio",
    "pin_project",
    "pin_project_",
];

/// Whose code the frame running `function`, at `location`, belongs to.
/// `app_crates` are the crates the stack names as the application's; see
/// [`application_crates`].
fn classify(function: &str, location: Option<&str>, app_crates: &[String]) -> Origin {
    let file = location.map(|location| location.replace('\\', "/"));
    if file.as_deref().is_some_and(is_toolchain_source) {
        return Origin::Std;
    }
    let Some(owner) = owner_crate(function) else {
        return Origin::Std;
    };
    if is_std_crate(owner) {
        Origin::Std
    } else if is_runtime_crate(owner) {
        Origin::Runtime
    } else if FRAMEWORK_CRATES.contains(&owner) {
        Origin::Framework
    } else if is_async_stack_crate(owner) {
        Origin::Dependency
    } else {
        match file {
            Some(file) if is_downloaded_source(&file) => Origin::Dependency,
            Some(_) => Origin::App,
            None if app_crates.iter().any(|app| app == owner) => Origin::App,
            None => Origin::Dependency,
        }
    }
}

fn is_std_crate(name: &str) -> bool {
    STD_CRATES.contains(&name) || name.starts_with("__")
}

fn is_runtime_crate(name: &str) -> bool {
    name == "tokio" || name.starts_with("tokio_")
}

/// Whether `name` is `tokio` or one of [`ASYNC_STACK_CRATES`].
fn is_async_stack_crate(name: &str) -> bool {
    is_runtime_crate(name)
        || ASYNC_STACK_CRATES
            .iter()
            .any(|family| match family.strip_suffix('_') {
                Some(_) => name.starts_with(family),
                None => name == *family,
            })
}

/// The crate of the frame that created the error or raised the panic:
/// the innermost frame that is not the standard library's, the
/// runtime's, or the error module's own machinery. `None` when that frame
/// is the framework's.
fn creation_crate(frames: &[Frame]) -> Option<&str> {
    frames
        .iter()
        .filter(|frame| !is_error_machinery(&frame.function))
        .find_map(|frame| {
            owner_crate(&frame.function)
                .filter(|owner| !is_std_crate(owner) && !is_runtime_crate(owner))
        })
        .filter(|owner| !FRAMEWORK_CRATES.contains(owner))
}

/// Whether `function` is part of how an error or a panic gets recorded:
/// the constructors, the conversions, the recorder and the panic hook,
/// which all live in this module tree.
fn is_error_machinery(function: &str) -> bool {
    function.starts_with("suprnova::error::") || function.starts_with("<suprnova::error::")
}

/// The file of a `file:line:column` site.
fn site_file(site: &str) -> &str {
    let mut parts = site.rsplitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(_column), Some(_line), Some(file)) => file,
        _ => site,
    }
}

/// Whether `file` is source the application builds itself: not part of
/// the Rust toolchain, and not a downloaded or vendored dependency.
fn is_local_source(file: &str) -> bool {
    !file.is_empty() && !is_toolchain_source(file) && !is_downloaded_source(file)
}

/// Whether `file` is part of the Rust toolchain's own sources.
fn is_toolchain_source(file: &str) -> bool {
    file.starts_with("/rustc/") || file.contains("/lib/rustlib/")
}

/// Whether `file` is a dependency's source: a Cargo registry crate, a
/// Cargo git checkout, or a crate copied into a `vendor` directory, as
/// `cargo vendor` lays them out.
fn is_downloaded_source(file: &str) -> bool {
    file.contains("/registry/src/")
        || file.contains("/git/checkouts/")
        || file.starts_with("vendor/")
        || file.contains("/vendor/")
}

/// The crate that owns `function`: the first segment of its path, or of
/// its self type for a `<Type as Trait>::method` name.
fn owner_crate(function: &str) -> Option<&str> {
    let function = function.trim();
    match function.strip_prefix('<') {
        Some(qualified) => {
            let (self_type, trait_path) = split_qualified(qualified);
            let trait_crate = trait_path.and_then(type_crate);
            // A closure, or another item the compiler names `{...}`, has
            // no impls of its own: the impl is the trait crate's.
            let self_type = self_type.trim();
            let anonymous = self_type.ends_with('}')
                && self_type
                    .rsplit("::")
                    .next()
                    .is_some_and(|last| last.starts_with('{'));
            if anonymous && trait_crate.is_some() {
                return trait_crate;
            }
            type_crate(self_type).or(trait_crate)
        }
        None => path_crate(function),
    }
}

/// Split the text after the `<` of `<Type as Trait>::method` into `Type`
/// and `Trait`. A `<Type>::method` name has no trait.
fn split_qualified(text: &str) -> (&str, Option<&str>) {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut as_at = None;
    for (at, &byte) in bytes.iter().enumerate() {
        match byte {
            b'<' => depth += 1,
            // The arrow of a function type, `fn() -> T`, is not a bracket.
            b'>' if at > 0 && bytes[at - 1] == b'-' => {}
            b'>' if depth == 0 => {
                return match as_at {
                    Some(start) => (&text[..start], text.get(start + 4..at)),
                    None => (&text[..at], None),
                };
            }
            b'>' => depth -= 1,
            b' ' if depth == 0 && as_at.is_none() && text[at..].starts_with(" as ") => {
                as_at = Some(at);
            }
            _ => {}
        }
    }
    (text, None)
}

/// The crate of a type as a symbol name spells it, past any reference,
/// pointer, slice or `dyn` prefix. A bare generic parameter has none.
fn type_crate(ty: &str) -> Option<&str> {
    let mut ty = ty.trim();
    loop {
        let rest = if let Some(rest) = ty.strip_prefix('&') {
            // A lifetime after `&`: `&'a T`.
            match rest.strip_prefix('\'') {
                Some(lifetime) => lifetime
                    .find(' ')
                    .map_or("", |space| &lifetime[space + 1..]),
                None => rest,
            }
        } else if let Some(rest) = ["mut ", "*const ", "*mut ", "dyn ", "impl ", "[", "("]
            .iter()
            .find_map(|prefix| ty.strip_prefix(prefix))
        {
            rest
        } else {
            break;
        };
        ty = rest.trim_start();
    }
    if ty.starts_with('<') {
        owner_crate(ty)
    } else {
        path_crate(ty)
    }
}

/// The first segment of `path`, when it has more than one: a crate name.
fn path_crate(path: &str) -> Option<&str> {
    let first = &path[..path.find("::")?];
    // A v0 symbol may name a crate with its disambiguator: `core[1a2b]`.
    let name = first.find('[').map_or(first, |at| &first[..at]);
    (!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .then_some(name)
}

#[cfg(test)]
mod tests {
    //! Recording is checked here with the decision passed in, not read
    //! from the environment, so these tests never change the debug mode
    //! that other tests in this binary read.

    use super::*;
    use crate::error::{AppError, ErrorReport, FrameworkError, catch_panic};

    /// Whether any frame of `frames` names `function`.
    fn names(frames: &RecordedFrames, function: &str) -> bool {
        frames
            .resolve()
            .frames
            .iter()
            .any(|frame| frame.function.contains(function))
    }

    #[tokio::test]
    async fn with_debug_off_no_frames_are_recorded() {
        let (recording, recorded, report_frames, panic_frames) = record_frames(false, async {
            let error = FrameworkError::internal("the ledger closed");
            let app_error = AppError::new("the ledger is read-only");
            let panic = catch_panic(async {
                panic!("the ledger index is unreadable");
            })
            .await
            .expect_err("the panic must be caught");
            (
                is_recording(),
                recorded_for(&error.to_string()).is_some()
                    || recorded_for(&app_error.to_string()).is_some(),
                ErrorReport::from_error(&error).frames().is_some(),
                panic.frames.is_some(),
            )
        })
        .await;

        assert!(!recording, "debug off must not open a recorder");
        assert!(!recorded, "debug off must record no frames for an error");
        assert!(
            !report_frames,
            "a report built with debug off has no frames"
        );
        assert!(!panic_frames, "debug off must record no frames for a panic");
    }

    #[tokio::test]
    async fn with_debug_on_an_error_records_the_frames_and_the_site_that_created_it() {
        let line = line!() + 2;
        let report = record_frames(true, async {
            let error = FrameworkError::internal("the ledger closed");
            ErrorReport::from_error(&error)
        })
        .await;

        let frames = report.frames().expect("debug on records the frames");
        assert!(
            names(
                frames,
                "with_debug_on_an_error_records_the_frames_and_the_site_that_created_it"
            ),
            "the frames must include the function that created the error: {:#?}",
            frames.resolve()
        );
        assert!(
            frames.site().starts_with(&format!("{}:{line}:", file!())),
            "the site is the constructor's caller, {}:{line}; got {}",
            file!(),
            frames.site()
        );
    }

    #[tokio::test]
    async fn with_debug_on_a_conversion_and_an_app_error_record_their_callers() {
        fn convert() -> Result<(), FrameworkError> {
            Err::<(), _>(sea_orm::DbErr::Custom("disk full".into()))?;
            Ok(())
        }
        let convert_line = line!() - 3;
        let app_error_line = line!() + 3;
        let (converted, app_error) = record_frames(true, async {
            let converted = convert().expect_err("the conversion fails");
            let app_error: FrameworkError = AppError::not_found("no ledger 7").into();
            (
                ErrorReport::from_error(&converted),
                ErrorReport::from_error(&app_error),
            )
        })
        .await;

        let converted = converted.frames().expect("a `?` conversion records frames");
        assert!(
            converted
                .site()
                .starts_with(&format!("{}:{convert_line}:", file!())),
            "the site is the `?`; got {}",
            converted.site()
        );
        let app_error = app_error.frames().expect(
            "an `AppError` records where it was created, and keeps those frames once it \
             becomes a `FrameworkError`",
        );
        assert!(
            app_error
                .site()
                .starts_with(&format!("{}:{app_error_line}:", file!())),
            "the site is the call to `AppError::not_found`; got {}",
            app_error.site()
        );
    }

    #[tokio::test]
    async fn with_debug_on_a_panic_records_the_frames_where_it_was_raised() {
        fn read_ledger_index() {
            panic!("the ledger index is unreadable");
        }

        let panic = record_frames(true, async {
            catch_panic(async { read_ledger_index() })
                .await
                .expect_err("the panic must be caught")
        })
        .await;

        let frames = panic.frames.expect("debug on records a panic's frames");
        assert!(
            names(&frames, "read_ledger_index"),
            "the frames must include the panicking function: {:#?}",
            frames.resolve()
        );
        assert_eq!(Some(frames.site()), panic.location.as_deref());
    }

    #[tokio::test]
    async fn context_keeps_the_frames_of_the_error_it_wraps() {
        let report = record_frames(true, async {
            let error =
                FrameworkError::internal("the ledger closed").context("posting the invoice");
            ErrorReport::from_error(&error)
        })
        .await;

        assert!(report.frames().is_some());
    }

    #[tokio::test]
    async fn an_error_created_on_another_task_has_no_frames() {
        let report = record_frames(true, async {
            let error = tokio::spawn(async { FrameworkError::internal("the ledger closed") })
                .await
                .expect("the task must finish");
            ErrorReport::from_error(&error)
        })
        .await;

        assert!(
            report.frames().is_none(),
            "a task-local recorder must not reach a spawned task"
        );
    }

    #[tokio::test]
    async fn a_request_remembers_the_most_recent_errors_only() {
        let (oldest, newest) = record_frames(true, async {
            for n in 0..=MAX_RECORDED {
                let _ = FrameworkError::internal(format!("error {n}"));
            }
            (
                recorded_for("Internal server error: error 0").is_some(),
                recorded_for(&format!("Internal server error: error {MAX_RECORDED}")).is_some(),
            )
        })
        .await;

        assert!(!oldest, "the oldest entry is dropped past the bound");
        assert!(newest);
    }

    #[test]
    fn the_owner_is_read_from_the_front_of_the_name() {
        let cases = [
            ("app::ledger::post_invoice::{{closure}}", Some("app")),
            ("std::panicking::begin_panic", Some("std")),
            (
                "<app::Auth as suprnova::middleware::Middleware>::handle::{{closure}}",
                Some("app"),
            ),
            (
                "<core::pin::Pin<P> as core::future::future::Future>::poll",
                Some("core"),
            ),
            ("<F as core::future::future::Future>::poll", Some("core")),
            // v0 names carry the instantiation: the app type is a
            // parameter, and the owner is still `core`.
            (
                "<core::pin::Pin<&mut app::ledger::Posting> as core::future::future::Future>::poll",
                Some("core"),
            ),
            (
                "<&mut hyper::proto::h1::Conn<I, B, T> as core::fmt::Debug>::fmt",
                Some("hyper"),
            ),
            // A closure's impls are the trait crate's: `core`'s `Fn` glue,
            // or a blanket impl.
            (
                "<app::ledger::routes::{closure#0} as core::ops::function::FnOnce<()>>::call_once",
                Some("core"),
            ),
            (
                "<app::ledger::routes::{{closure}} as suprnova::routing::Handler>::call",
                Some("suprnova"),
            ),
            // A function pointer has no crate: the trait's crate owns it.
            (
                "<fn() -> app::Ledger as core::ops::function::FnOnce<()>>::call_once",
                Some("core"),
            ),
            // Only the self type's own last segment marks a closure, not
            // one inside its generic arguments.
            (
                "<hyper::service::util::ServiceFn<app::routes::{closure#0}, B> as tokio::Service>::call",
                Some("hyper"),
            ),
            (
                "<suprnova::error::FrameworkError>::internal::<&str>",
                Some("suprnova"),
            ),
            ("core[1a2b3c]::panicking::panic_fmt", Some("core")),
            ("__rustc::rust_begin_unwind", Some("__rustc")),
            ("__rust_try", None),
            ("<unknown>", None),
        ];
        for (function, owner) in cases {
            assert_eq!(owner_crate(function), owner, "owner of {function}");
        }
    }

    #[test]
    fn with_debug_info_the_source_file_tells_the_application_from_a_dependency() {
        let registry = "/home/dev/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.6.0/src/proto/h1/dispatch.rs:130:23";
        let cases = [
            (
                "app::ledger::post_invoice",
                Some("./src/ledger.rs:12:5"),
                Origin::App,
            ),
            (
                "<app::Auth as suprnova::Middleware>::handle",
                Some("/srv/app/src/auth.rs:40:9"),
                Origin::App,
            ),
            (
                "suprnova::server::serve_request::{{closure}}",
                Some("/srv/suprnova/framework/src/server.rs:1:1"),
                Origin::Framework,
            ),
            (
                "tokio::runtime::task::harness::poll_future",
                Some(registry),
                Origin::Runtime,
            ),
            (
                "hyper::proto::h1::dispatch::Dispatcher<D,Bs,I,T>::poll_inner",
                Some(registry),
                Origin::Dependency,
            ),
            (
                "sea_orm::executor::execute::{{closure}}",
                Some("/cargo/git/checkouts/sea-orm-1/abc/src/executor/execute.rs:9:1"),
                Origin::Dependency,
            ),
            (
                "std::panicking::begin_panic::{{closure}}",
                Some("/rustc/48a229cea/library/std/src/panicking.rs:700:12"),
                Origin::Std,
            ),
            (
                "hashbrown::raw::RawTable<T,A>::find",
                Some("/rustc/48a229cea/library/std/src/../../hashbrown/src/raw.rs:1:1"),
                Origin::Std,
            ),
            ("__libc_start_main", None, Origin::Std),
            ("__rustc::rust_begin_unwind", None, Origin::Std),
            (
                "app::ledger::post_invoice",
                Some("C:\\Users\\dev\\.cargo\\registry\\src\\index\\app-1.0.0\\src\\ledger.rs:1:1"),
                Origin::Dependency,
            ),
        ];
        for (function, location, origin) in cases {
            assert_eq!(
                classify(function, location, &[]),
                origin,
                "{function} at {location:?}"
            );
        }
    }

    /// A backtrace with no debug info, as a release build or this
    /// repository's test profile prints it: names only.
    const NO_DEBUG_INFO: &str = "   0: <std::backtrace::Backtrace>::force_capture
   1: <suprnova::error::frames::RecordedFrames>::capture
   2: suprnova::error::frames::record_creation
   3: <suprnova::error::FrameworkError as core::convert::From<sea_orm::error::DbErr>>::from
   4: <core::result::Result<(), suprnova::error::FrameworkError> as core::ops::try_trait::FromResidual<core::result::Result<core::convert::Infallible, sea_orm::error::DbErr>>>::from_residual
   5: app::ledger::show_rows::{closure#0}
   6: <core::pin::Pin<&mut app::ledger::show_rows::{closure#0}> as core::future::future::Future>::poll
   7: suprnova::routing::router::dispatch::{closure#0}
   8: hyper::proto::h1::dispatch::Dispatcher::poll_loop
   9: app::main::{closure#0}
  10: tokio::runtime::park::CachedParkThread::block_on
  11: __libc_start_main
";

    #[test]
    fn without_debug_info_a_local_creation_site_names_the_application_crate() {
        let resolved = resolve_text(NO_DEBUG_INFO, "src/ledger.rs:41:13");

        let origins: Vec<Origin> = resolved.frames.iter().map(|frame| frame.origin).collect();
        assert_eq!(
            origins,
            [
                Origin::Std,
                Origin::Framework,
                Origin::Framework,
                Origin::Framework,
                Origin::Std,
                Origin::App,
                Origin::Std,
                Origin::Framework,
                Origin::Dependency,
                Origin::App,
                Origin::Runtime,
                Origin::Std,
            ]
        );
    }

    #[test]
    fn without_debug_info_or_dispatch_a_downloaded_creation_site_names_no_application_crate() {
        // No frame of the framework's dispatch: nothing on this stack was
        // called by the middleware chain or the router.
        let text = "   0: suprnova::error::frames::record_creation
   1: <suprnova::error::FrameworkError as core::convert::From<sea_orm::error::DbErr>>::from
   2: app::ledger::show_rows::{closure#0}
   3: app::main::{closure#0}
   4: tokio::runtime::park::CachedParkThread::block_on
";
        for site in [
            "/home/dev/.cargo/registry/src/index/app-1.0.0/src/ledger.rs:41:13",
            "/rustc/48a229cea/library/core/src/convert/mod.rs:767:9",
        ] {
            let resolved = resolve_text(text, site);

            assert!(
                resolved
                    .frames
                    .iter()
                    .all(|frame| frame.origin != Origin::App),
                "a site at {site} is not the application's: {resolved:#?}"
            );
        }
    }

    /// The request stack of a handler and an app middleware, as a build
    /// without debug info prints it, under the frames of whatever created
    /// the error. `{inner}` is replaced with those frames.
    const DISPATCHED: &str = "{inner}
  20: app::ledger::list_entries::{closure#0}
  21: <core::pin::Pin<alloc::boxed::Box<dyn core::future::future::Future<Output = core::result::Result<suprnova::http::response::HttpResponse, suprnova::http::response::HttpResponse>> + core::marker::Send>> as core::future::future::Future>::poll
  22: <app::middleware::Audit as suprnova::middleware::Middleware>::handle::{closure#0}
  23: <core::pin::Pin<alloc::boxed::Box<dyn core::future::future::Future<Output = ()>>> as core::future::future::Future>::poll
  24: suprnova::middleware::into_boxed::<app::middleware::Audit>::{closure#0}::{closure#0}
  25: <suprnova::middleware::chain::MiddlewareChain>::execute::{closure#0}::{closure#1}::{closure#0}
  26: <suprnova::logging::request_id::RequestIdMiddleware as suprnova::middleware::Middleware>::handle::{closure#0}
  27: <tracing::instrument::Instrumented<F> as core::future::future::Future>::poll
  28: <suprnova::middleware::chain::MiddlewareChain>::execute::{closure#0}
  29: <futures_util::future::future::catch_unwind::CatchUnwind<F> as core::future::future::Future>::poll
  30: suprnova::error::report::catch_panic::<F>::{closure#0}
  31: suprnova::server::execute_chain_safely::{closure#0}
  32: suprnova::server::handle_request_inner::{closure#0}
  33: <hyper::proto::h1::dispatch::Dispatcher<D, Bs, I, T>>::poll_loop
  34: app::main::{closure#0}
  35: tokio::runtime::park::CachedParkThread::block_on
  36: __libc_start_main
";

    /// `DISPATCHED` under `inner`, resolved without debug info, as
    /// `(function, origin)` pairs.
    fn dispatched(inner: &str, site: &str) -> Vec<(String, Origin)> {
        resolve_text(&DISPATCHED.replace("{inner}", inner), site)
            .frames
            .into_iter()
            .map(|frame| (frame.function, frame.origin))
            .collect()
    }

    fn origin_of(frames: &[(String, Origin)], function: &str) -> Origin {
        frames
            .iter()
            .find(|(name, _)| name.starts_with(function))
            .map(|(_, origin)| *origin)
            .unwrap_or_else(|| panic!("no frame {function} in {frames:#?}"))
    }

    #[test]
    fn without_debug_info_an_error_the_framework_creates_shows_the_handler() {
        let frames = dispatched(
            "   0: suprnova::error::frames::record_creation
   1: <suprnova::error::FrameworkError>::database::<alloc::string::String>
   2: <suprnova::eloquent::builder::Builder<app::models::Entry>>::get::{closure#0}::{closure#0}
   3: <core::result::Result<T, E>>::map_err::<suprnova::error::FrameworkError, F>
   4: <suprnova::eloquent::builder::Builder<app::models::Entry>>::get::{closure#0}",
            "framework/src/eloquent/builder.rs:4627:26",
        );

        assert_eq!(origin_of(&frames, "app::ledger::list_entries"), Origin::App);
        assert_eq!(
            origin_of(&frames, "<app::middleware::Audit as"),
            Origin::App
        );
        assert_eq!(
            origin_of(&frames, "<suprnova::eloquent::builder::Builder"),
            Origin::Framework
        );
        assert_eq!(origin_of(&frames, "app::main"), Origin::App);
        assert_eq!(origin_of(&frames, "<hyper::proto"), Origin::Dependency);
        assert_eq!(origin_of(&frames, "<tracing::"), Origin::Dependency);
        assert_eq!(origin_of(&frames, "<futures_util::"), Origin::Dependency);
    }

    #[test]
    fn without_debug_info_a_handler_the_frameworks_own_middleware_polls_is_shown() {
        // Legacy symbol names, which do not spell out the boxed future's
        // type: the handler's nearest outer frame of any crate is the
        // framework's own middleware, which polls what `next` returned.
        let text = "   0: suprnova::error::frames::record_creation
   1: suprnova::error::FrameworkError::domain
   2: suprnova::http::abort::abort
   3: app::ledger::close::{{closure}}
   4: <core::pin::Pin<P> as core::future::future::Future>::poll
   5: <suprnova::logging::request_id::RequestIdMiddleware as suprnova::middleware::Middleware>::handle::{{closure}}
   6: <core::pin::Pin<P> as core::future::future::Future>::poll
   7: suprnova::middleware::chain::MiddlewareChain::execute::{{closure}}
   8: suprnova::server::execute_chain_safely::{{closure}}
   9: <hyper::proto::h1::dispatch::Dispatcher<D,Bs,I,T> as core::future::future::Future>::poll
";
        let resolved = resolve_text(text, "src/ledger.rs:12:5");

        assert_eq!(resolved.frames[3].origin, Origin::App);
        assert_eq!(resolved.frames[5].origin, Origin::Framework);
        assert_eq!(resolved.frames[9].origin, Origin::Dependency);
    }

    #[test]
    fn without_debug_info_a_handler_polled_through_a_boxed_response_future_is_shown() {
        // v0 names spell out the boxed `Response` future the framework
        // polls; the frame inward of it is a handler or a middleware.
        let text = "   0: suprnova::error::frames::record_creation
   1: <suprnova::error::FrameworkError>::internal::<&str>
   2: suprnova::session::store::persist::{closure#0}
   3: app::ledger::close::{closure#0}
   4: <core::pin::Pin<alloc::boxed::Box<dyn core::future::future::Future<Output = core::result::Result<suprnova::http::response::HttpResponse, suprnova::http::response::HttpResponse>> + core::marker::Send>> as core::future::future::Future>::poll
   5: <suprnova::session::middleware::SessionMiddleware as suprnova::middleware::Middleware>::handle::{closure#0}
   6: suprnova::server::execute_chain_safely::{closure#0}
";
        let resolved = resolve_text(text, "framework/src/session/store.rs:90:20");

        assert_eq!(resolved.frames[3].origin, Origin::App);
        assert_eq!(resolved.frames[2].origin, Origin::Framework);
    }

    #[test]
    fn without_debug_info_a_handler_that_calls_abort_if_is_shown() {
        let frames = dispatched(
            "   0: suprnova::error::frames::record_creation
   1: <suprnova::error::FrameworkError>::domain::<&str>
   2: suprnova::http::abort::abort::<&str>
   3: suprnova::http::abort::abort_if::<&str>",
            "src/ledger.rs:12:5",
        );

        assert_eq!(origin_of(&frames, "app::ledger::list_entries"), Origin::App);
        assert_eq!(
            origin_of(&frames, "suprnova::http::abort::abort_if"),
            Origin::Framework
        );
    }

    #[test]
    fn without_debug_info_a_conversion_through_map_err_shows_the_handler() {
        // `Result::map_err` is not `#[track_caller]`: the site is in the
        // toolchain, and only the stack names the handler.
        let frames = dispatched(
            "   0: suprnova::error::frames::record_creation
   1: <suprnova::error::FrameworkError as core::convert::From<sea_orm::error::DbErr>>::from
   2: <core::result::Result<T, E>>::map_err::<suprnova::error::FrameworkError, F>",
            "/rustc/48a229cea/library/core/src/result.rs:860:27",
        );

        assert_eq!(origin_of(&frames, "app::ledger::list_entries"), Origin::App);
    }

    #[test]
    fn without_debug_info_a_panic_inside_a_dependency_shows_the_handler_and_collapses_the_dependency()
     {
        let frames = dispatched(
            "   0: suprnova::error::report::install_location_hook::{closure#0}::{closure#0}
   1: std::panicking::panic_with_hook
   2: core::panicking::panic_fmt
   3: <str as serde_json::value::index::Index>::index_or_insert
   4: <serde_json::value::Value as core::ops::index::IndexMut<&str>>::index_mut",
            "/home/dev/.cargo/registry/src/index/serde_json-1.0.0/src/value/index.rs:102:18",
        );

        assert_eq!(origin_of(&frames, "app::ledger::list_entries"), Origin::App);
        assert_eq!(
            origin_of(&frames, "<serde_json::value::Value"),
            Origin::Dependency
        );
        assert_eq!(origin_of(&frames, "<str as serde_json"), Origin::Dependency);
    }

    #[test]
    fn without_debug_info_a_dependency_the_framework_calls_is_not_the_application() {
        // `sea_orm`, called by the framework's Eloquent builder, is not
        // called by the dispatch, so it stays a dependency.
        let frames = dispatched(
            "   0: suprnova::error::report::install_location_hook::{closure#0}::{closure#0}
   1: core::panicking::panic_fmt
   2: sea_orm::executor::query::QueryResult::try_get::{closure#0}
   3: <suprnova::eloquent::builder::Builder<app::models::Entry>>::get::{closure#0}",
            "/home/dev/.cargo/registry/src/index/sea-orm-2.0.0/src/executor/query.rs:1:1",
        );

        assert_eq!(origin_of(&frames, "sea_orm::executor"), Origin::Dependency);
        assert_eq!(origin_of(&frames, "app::ledger::list_entries"), Origin::App);
    }

    #[test]
    fn the_async_stack_crates_are_dependencies_whatever_their_path() {
        for (function, location) in [
            (
                "<hyper::proto::h1::dispatch::Dispatcher<D, Bs, I, T>>::poll_loop",
                "/nix/store/5x1-hyper-1.6.0/src/proto/h1/dispatch.rs:130:23",
            ),
            (
                "h2::proto::connection::Connection::poll",
                "/src/third_party/h2/src/proto/connection.rs:1:1",
            ),
            (
                "http::header::map::HeaderMap::get",
                "./vendor-free/http/src/header/map.rs:1:1",
            ),
            (
                "<futures_util::future::future::catch_unwind::CatchUnwind<F> as core::future::future::Future>::poll",
                "/work/external/futures-util/src/future/future/catch_unwind.rs:1:1",
            ),
            (
                "tower::util::oneshot::Oneshot::poll",
                "/opt/src/tower/src/util/oneshot.rs:1:1",
            ),
            (
                "tracing::instrument::Instrumented::poll",
                "/opt/src/tracing/src/instrument.rs:1:1",
            ),
            (
                "async_trait::__private::Box::pin",
                "/opt/src/async-trait/src/lib.rs:1:1",
            ),
            (
                "http_body_util::combinators::BoxBody::poll_frame",
                "/opt/src/http-body-util/src/lib.rs:1:1",
            ),
        ] {
            assert_eq!(
                classify(function, Some(location), &[]),
                Origin::Dependency,
                "{function} at {location}"
            );
        }
        assert_eq!(
            classify(
                "tokio::runtime::task::harness::poll_future",
                Some("/nix/store/5x1-tokio-1.48.0/src/runtime/task/harness.rs:1:1"),
                &[]
            ),
            Origin::Runtime
        );
    }

    #[test]
    fn a_vendored_dependency_is_a_dependency() {
        assert_eq!(
            classify(
                "serde_json::de::from_str",
                Some("/build/app/vendor/serde_json/src/de.rs:2676:5"),
                &[]
            ),
            Origin::Dependency
        );
        assert_eq!(
            classify(
                "app::ledger::post",
                Some("/build/app/src/ledger.rs:12:5"),
                &[]
            ),
            Origin::App
        );
    }

    #[test]
    fn every_library_crate_of_this_repository_is_the_framework() {
        for function in [
            "suprnova::server::serve_request",
            "suprnova_live::render_cache::key::Key::new",
            "magnetar::sessions::WebSessionBinding::bind",
            "suprnova_web_push::send",
            "suprnova_payments_stripe::webhook::verify",
            "suprnova_payments_paddle::webhook::verify",
            "suprnova_payments_nowpayments::webhook::verify",
        ] {
            for location in [None, Some("/srv/suprnova/crates/x/src/lib.rs:1:1")] {
                assert_eq!(
                    classify(function, location, &[]),
                    Origin::Framework,
                    "{function} at {location:?}"
                );
            }
        }
    }

    #[test]
    fn the_standard_librarys_backtrace_text_parses_into_frames() {
        let text = "   0: std::backtrace::Backtrace::force_capture
             at /rustc/48a229cea/library/std/src/backtrace.rs:312:9
   1: app::ledger::post_invoice::{{closure}}
             at ./src/ledger.rs:12:5
   2: <unknown>
   3: tokio::runtime::park::CachedParkThread::block_on
             at /home/dev/.cargo/registry/src/index/tokio-1.0.0/src/runtime/park.rs:285:60
";
        let resolved = resolve_text(text, "src/ledger.rs:12:5");

        assert_eq!(resolved.omitted, 0);
        assert_eq!(
            resolved.frames,
            [
                Frame {
                    function: "std::backtrace::Backtrace::force_capture".into(),
                    location: Some("/rustc/48a229cea/library/std/src/backtrace.rs:312:9".into()),
                    origin: Origin::Std,
                },
                Frame {
                    function: "app::ledger::post_invoice::{{closure}}".into(),
                    location: Some("./src/ledger.rs:12:5".into()),
                    origin: Origin::App,
                },
                Frame {
                    function: "<unknown>".into(),
                    location: None,
                    origin: Origin::Std,
                },
                Frame {
                    function: "tokio::runtime::park::CachedParkThread::block_on".into(),
                    location: Some(
                        "/home/dev/.cargo/registry/src/index/tokio-1.0.0/src/runtime/park.rs:285:60"
                            .into()
                    ),
                    origin: Origin::Runtime,
                },
            ]
        );
    }

    #[test]
    fn a_capture_past_the_frame_limit_counts_what_it_left_out() {
        let text: String = (0..MAX_FRAMES + 3)
            .map(|n| format!("{n:4}: app::recurse\n             at ./src/lib.rs:1:1\n"))
            .collect();

        let resolved = parse_frames(&text);

        assert_eq!(resolved.frames.len(), MAX_FRAMES);
        assert_eq!(resolved.omitted, 3);
    }

    #[test]
    fn a_site_splits_into_its_file() {
        assert_eq!(site_file("src/ledger.rs:41:13"), "src/ledger.rs");
        assert_eq!(
            site_file("C:\\app\\src\\ledger.rs:41:13"),
            "C:\\app\\src\\ledger.rs"
        );
        assert_eq!(site_file("no-location"), "no-location");
    }
}
