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
//! capture also keeps the site that created the error: the line of the
//! application's `?`, or of its call to the constructor. A panic keeps
//! the location the panic hook reports.
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
//! - **Framework**: the crates of this repository that an application
//!   links: `suprnova`, `suprnova_macros`, `suprnova_live`,
//!   `suprnova_magnetar`, `suprnova_web_push` and the
//!   `suprnova_payments_*` adapters.
//! - **Application** or **dependency**, for every other crate. When the
//!   binary has debug info, the frame's source file decides: a file in
//!   Cargo's registry or a Cargo git checkout is a dependency's, any
//!   other file is the application's. Without debug info, frames have no
//!   file, and the creation site decides: when it is not in Cargo's
//!   registry, a git checkout or the toolchain, the crate of the frame
//!   that called the constructor, or that panicked, is the application's,
//!   and so is every frame of that crate. Every other crate is a
//!   dependency.
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
    let app_crate = if is_local_source(&site_file(site).replace('\\', "/")) {
        creation_crate(&resolved.frames).map(str::to_string)
    } else {
        None
    };
    for frame in &mut resolved.frames {
        frame.origin = classify(
            &frame.function,
            frame.location.as_deref(),
            app_crate.as_deref(),
        );
    }
    resolved
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

/// Suprnova's crates that an application links.
const FRAMEWORK_CRATES: &[&str] = &[
    "suprnova",
    "suprnova_macros",
    "suprnova_live",
    "suprnova_magnetar",
    "suprnova_web_push",
    "suprnova_payments_stripe",
    "suprnova_payments_paddle",
    "suprnova_payments_nowpayments",
];

/// Whose code the frame running `function`, at `location`, belongs to.
/// `app_crate` is the crate the creation site names as the application's.
fn classify(function: &str, location: Option<&str>, app_crate: Option<&str>) -> Origin {
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
    } else {
        match file {
            Some(file) if is_cargo_source(&file) => Origin::Dependency,
            Some(_) => Origin::App,
            None if app_crate == Some(owner) => Origin::App,
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
/// the Rust toolchain, and not downloaded by Cargo.
fn is_local_source(file: &str) -> bool {
    !file.is_empty() && !is_toolchain_source(file) && !is_cargo_source(file)
}

/// Whether `file` is part of the Rust toolchain's own sources.
fn is_toolchain_source(file: &str) -> bool {
    file.starts_with("/rustc/") || file.contains("/lib/rustlib/")
}

/// Whether `file` was downloaded by Cargo: a registry crate or a git
/// dependency.
fn is_cargo_source(file: &str) -> bool {
    file.contains("/registry/src/") || file.contains("/git/checkouts/")
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
                classify(function, location, None),
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
    fn without_debug_info_a_downloaded_creation_site_names_no_application_crate() {
        for site in [
            "/home/dev/.cargo/registry/src/index/app-1.0.0/src/ledger.rs:41:13",
            "/rustc/48a229cea/library/core/src/convert/mod.rs:767:9",
        ] {
            let resolved = resolve_text(NO_DEBUG_INFO, site);

            assert!(
                resolved
                    .frames
                    .iter()
                    .all(|frame| frame.origin != Origin::App),
                "a site at {site} is not the application's: {resolved:#?}"
            );
        }
    }

    #[test]
    fn an_error_the_framework_creates_names_no_application_crate() {
        let text = "   0: suprnova::error::frames::record_creation
   1: <suprnova::error::FrameworkError>::internal::<&str>
   2: suprnova::session::middleware::persist::{closure#0}
   3: app::main::{closure#0}
";
        let resolved = resolve_text(text, "framework/src/session/middleware.rs:90:20");

        assert_eq!(resolved.frames[3].origin, Origin::Dependency);
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
