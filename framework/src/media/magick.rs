//! The opt-in ImageMagick driver: `IMAGE_DRIVER=magick`.
//!
//! Laravel's image surface has always been two drivers - GD by default,
//! Imagick when the host provides it. This is the same shape. The framework
//! ships no codec here, links nothing, and compiles nothing native: it runs
//! a binary the host operator installed, and whatever formats that binary's
//! delegates support come along for free. HEIC is the motivating case.
//!
//! # Execution safety
//!
//! Arguments are **always** a fixed array handed straight to
//! [`std::process::Command`]. There is no shell, no `sh -c`, and no string
//! interpolation into a command line anywhere in this module. That matters
//! because the input to an image pipeline is attacker-controlled by
//! definition - it arrives as an upload. Every numeric argument is formatted
//! from an already-validated `u32`/`f32`/`u8` field of a
//! [`Transformation`], never from caller-supplied text, so there is no
//! argument position an attacker can reach. A colour is a typed
//! [`Color`](super::Color) the driver formats itself. The arguments that
//! choose which metadata ImageMagick keeps are fixed strings, and so is the
//! transform that applies an orientation the framework read. A custom
//! transformation contributes no argument at all: it runs on the pixels,
//! in Rust, between two ImageMagick runs. The image bytes go over stdin and
//! come back over stdout; the driver writes the image to no file, so there
//! is no path an input names and nothing of it to clean up. (ImageMagick
//! itself copies stdin into its own temporary directory before it decodes,
//! and removes the copy when it exits.)
//!
//! The input argument always names the coder (`png:-`, `heic:-`), so
//! ImageMagick never chooses one from the bytes of an upload. Bytes the
//! framework cannot name are refused before ImageMagick starts, unless the
//! application opts into autodetection with `IMAGE_MAGICK_AUTODETECT`; see
//! [`input_spec`].
//!
//! # Policy
//!
//! Every run reads the framework's own ImageMagick security policy,
//! [`MagickCliDriver::POLICY`], whether or not autodetection is on (HRD-010).
//! The driver writes it as `policy.xml` into a directory of its own, one a
//! process under the system's temporary directory, checks the file before
//! each run and writes it again when it differs, and starts ImageMagick
//! with `MAGICK_CONFIGURE_PATH` naming that directory first and last. It is
//! the one file the driver writes. Because ImageMagick applies the last
//! entry that matches, and reads its own configuration directory and the
//! user's after that path, the driver also checks the policy in effect with
//! `magick -list policy`, once a process. A run whose policy cannot be put
//! in place, or whose policy in effect reopens a denial, is refused before
//! ImageMagick starts for it.
//!
//! # Two-tier limits
//!
//! Decode limits are enforced twice, because this driver's whole purpose is
//! reading formats the framework cannot parse:
//!
//! 1. **Framework tier.** For the five formats the framework reads
//!    itself, the header is parsed and the caps applied before the process
//!    is even spawned - identical to the pure-Rust driver.
//! 2. **ImageMagick tier.** For the formats [`sniff`](super::sniff) names
//!    but only ImageMagick reads (HEIC, AVIF, TIFF, PSD), and for any input
//!    under the autodetection opt-in, a pre-parse is impossible, so every
//!    invocation carries ImageMagick's own resource limits derived from the
//!    same [`ImageConfig`]. `-limit disk 0` is deliberate: without it, IM
//!    spills the pixel cache to disk and the memory cap stops being a cap.

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use crate::config::env_optional;
use crate::error::FrameworkError;

use super::ImageConfig;
use super::color::Color;
use super::custom::{CustomTransformation, TransformationSettings};
use super::driver::{ImageDriver, ImagePipeline, OutputFormat, Transformation};
use super::metadata::{self, ColourClass, IccData, Kept, SrgbConversion};
use super::orientation::Orientation;
use super::oxideav::{OxideAvImageDriver, encode_png};
use super::sniff;

/// Default binary name. ImageMagick 7 only.
///
/// Not `convert`: that is the ImageMagick 6 name, and IM6's argument
/// handling differs enough that silently accepting it would produce subtly
/// wrong output rather than an honest failure.
const DEFAULT_BINARY: &str = "magick";

/// How long the hard deadline waits past ImageMagick's own `-limit time`.
///
/// IM's limit should be what fires; this is only the backstop for a child
/// wedged before its monitor engages. A couple of seconds keeps the two from
/// racing without making a genuine IM timeout wait noticeably longer.
const DEADLINE_GRACE: Duration = Duration::from_secs(2);

/// How often the deadline loop checks whether the child has exited. Short
/// enough to be responsive, long enough that a 30-second wait costs nothing.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// How long to wait for the pipe readers once the child has already exited.
///
/// Its pipes are closed at that point, so the readers drain whatever the
/// kernel buffered and finish essentially at once. The bound only exists so a
/// process that somehow outlived a successful run cannot extend a call that
/// already met its deadline.
const READER_DRAIN_GRACE: Duration = Duration::from_secs(2);

/// The least the output buffer grows by when it has no room left to read
/// into: `read_to_end`'s own default read size.
const MIN_OUTPUT_READ: usize = 8 * 1024;

/// An [`ImageDriver`] that shells out to a host-installed ImageMagick 7.
///
/// Selected with `IMAGE_DRIVER=magick`. The binary name comes from
/// `IMAGE_MAGICK_BINARY` and defaults to `magick`; an absent or
/// non-executing binary is an error at first use, naming the env var.
///
/// Input is read with a named coder: `png:-` and its kin for the five
/// formats the framework reads, and `heic:-`, `avif:-`, `tiff:-`, `psd:-`
/// or `psb:-` for the formats only ImageMagick reads. Other bytes are
/// refused before the binary starts, unless `IMAGE_MAGICK_AUTODETECT` lets
/// ImageMagick choose the coder.
///
/// Every run reads the framework's ImageMagick policy, [`Self::POLICY`],
/// which denies the network, script, text and Ghostscript coders, every
/// delegate, and indirect `@file` reads. The host's own policy still
/// applies beside it, and the driver refuses to run when the policy in
/// effect reopens any of those denials.
///
/// Output formats are whatever the host's delegates can write. WebP goes
/// through libwebp, and [`OutputFormat::WebP`] hands it the pipeline's
/// quality for a lossy encode, with one difference from the built-in
/// `oxideav` driver: an image with transparency is still written lossy,
/// because libwebp stores the alpha channel beside the lossy picture. The
/// built-in driver's lossy encoder has no alpha channel, so it writes such
/// an image lossless.
///
/// The output is lossy at every quality. ImageMagick switches its WebP
/// coder to lossless by itself at a quality of 100, so the driver passes
/// `-define webp:lossless=false` with the quality.
///
/// [`OutputFormat::WebPLossless`] is lossless under both drivers.
pub struct MagickCliDriver {
    binary: String,
}

impl MagickCliDriver {
    /// Build a driver around a specific binary name or path.
    pub fn new(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Build a driver from `IMAGE_MAGICK_BINARY`, defaulting to `magick`.
    pub fn from_env() -> Self {
        Self::new(
            env_optional::<String>("IMAGE_MAGICK_BINARY")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_BINARY.to_string()),
        )
    }

    /// The binary this driver invokes.
    pub fn binary(&self) -> &str {
        &self.binary
    }

    /// The ImageMagick security policy every run of this driver reads
    /// (HRD-010).
    ///
    /// Before each run the driver writes this text as `policy.xml` into a
    /// directory of its own and starts ImageMagick with
    /// `MAGICK_CONFIGURE_PATH` naming that directory first and last,
    /// whether or not `IMAGE_MAGICK_AUTODETECT` is on. A named coder
    /// (`png:-`) already keeps an upload away from the coders below; this
    /// policy is what keeps them closed to input under the autodetection
    /// opt-in, which ImageMagick reads as whatever its bytes say.
    ///
    /// Every entry denies, with `rights="none"`:
    ///
    /// - `URL`, `HTTP`, `HTTPS`, `FTP`: the coders that fetch from the
    ///   network. Input that names one, such as `url:http://...`, would make
    ///   the server send a request the uploader chose.
    /// - `MVG`, `MSL`: ImageMagick's drawing language and its scripting
    ///   language. Both read and write other files and URLs by name, which
    ///   turns an upload into a program (the ImageTragick class of bug).
    /// - `TEXT`, `LABEL`, `CAPTION`, `PANGO`: the coders that draw text.
    ///   They read a file their input names (`text:/etc/passwd`,
    ///   `label:@file`) and draw it into the image the server sends back.
    /// - `SCREENSHOT`, `X`: the coders that read the server's display.
    /// - `EPHEMERAL`: reads a file and then deletes it.
    /// - `PS`, `PS2`, `PS3`, `EPS`, `PDF`, `XPS`: the formats ImageMagick
    ///   hands to Ghostscript, an interpreter with its own record of sandbox
    ///   escapes. They are denied by coder as well as by delegate, so the
    ///   refusal comes before ImageMagick chooses how to run Ghostscript,
    ///   as an external program or as a library it links.
    /// - `domain="delegate" pattern="*"`: every external program ImageMagick
    ///   would start for a format, such as Ghostscript or ffmpeg. A format
    ///   the host's ImageMagick reads through a library it links (libheif
    ///   for HEIC, libtiff, libwebp) uses no delegate and still reads.
    /// - `domain="path" pattern="@*"`: indirect reads, where an argument that
    ///   starts with `@` names a file for ImageMagick to read in its place.
    ///
    /// A pattern without a wildcard names one coder: `PS` leaves `PSD`
    /// alone, and `TEXT` leaves `TXT`, the pixel listing `dominant_color`
    /// writes, alone.
    ///
    /// ImageMagick reads every `policy.xml` it finds: the directories of
    /// `MAGICK_CONFIGURE_PATH` in order, then its own configuration
    /// directory, then the user's (`$XDG_CONFIG_HOME/ImageMagick` or
    /// `~/.config/ImageMagick`). Where two entries match, it applies the one
    /// it read last. The driver names its directory both first and last on
    /// `MAGICK_CONFIGURE_PATH`, so no entry on the operator's path outranks
    /// this policy, while the host's denials still apply. ImageMagick's own
    /// directory and the user's are read after it, though, and a grant
    /// there, such as `rights="read|write" pattern="MVG"` or a grant on
    /// `pattern="*"`, would outrank it. So the driver checks the result:
    /// before the first run of a process it reads the policy in effect with
    /// `magick -list policy`, and refuses every run when any of these
    /// denials is reopened or missing. The listing leaves out an entry
    /// marked `stealth="true"`, so the check also runs one MVG probe that
    /// the policy in effect must refuse.
    pub const POLICY: &'static str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!--
  Suprnova's magick image driver writes this file and checks it before
  every ImageMagick run, so an edit here does not last. Put your own policy
  in ImageMagick's configuration directory: ImageMagick reads both.
-->
<policymap>
  <policy domain="coder" rights="none" pattern="URL" />
  <policy domain="coder" rights="none" pattern="HTTP" />
  <policy domain="coder" rights="none" pattern="HTTPS" />
  <policy domain="coder" rights="none" pattern="FTP" />
  <policy domain="coder" rights="none" pattern="MVG" />
  <policy domain="coder" rights="none" pattern="MSL" />
  <policy domain="coder" rights="none" pattern="TEXT" />
  <policy domain="coder" rights="none" pattern="LABEL" />
  <policy domain="coder" rights="none" pattern="CAPTION" />
  <policy domain="coder" rights="none" pattern="PANGO" />
  <policy domain="coder" rights="none" pattern="SCREENSHOT" />
  <policy domain="coder" rights="none" pattern="X" />
  <policy domain="coder" rights="none" pattern="EPHEMERAL" />
  <policy domain="coder" rights="none" pattern="PS" />
  <policy domain="coder" rights="none" pattern="PS2" />
  <policy domain="coder" rights="none" pattern="PS3" />
  <policy domain="coder" rights="none" pattern="EPS" />
  <policy domain="coder" rights="none" pattern="PDF" />
  <policy domain="coder" rights="none" pattern="XPS" />
  <policy domain="delegate" rights="none" pattern="*" />
  <policy domain="path" rights="none" pattern="@*" />
</policymap>
"#;

    /// Name the input, and apply the framework-tier limits to input we can
    /// parse ourselves.
    ///
    /// Returns the format the framework reads, or the one it names but only
    /// ImageMagick reads (HEIC, TIFF), which is not an error here, unlike in
    /// the pure-Rust driver. Bytes the framework cannot name are refused
    /// before ImageMagick starts, unless `config` opts into autodetection.
    fn guard(contents: &[u8], config: &ImageConfig) -> Result<Source, FrameworkError> {
        if let Some(format) = sniff::guard(contents, config)? {
            return Ok(Source::Read(format));
        }
        if let Some(format) = sniff::detect_delegated(contents) {
            return Ok(Source::Delegated(format));
        }
        if config.magick_autodetect {
            return Ok(Source::Autodetected);
        }
        Err(FrameworkError::param(
            "image format is not one Suprnova names, so the magick driver does not hand it \
             to ImageMagick; set IMAGE_MAGICK_AUTODETECT=true to let ImageMagick choose the \
             coder for such input",
        ))
    }

    /// Spawn the binary, stream `input` in, and collect stdout.
    fn run(
        &self,
        args: &[String],
        input: &[u8],
        config: &ImageConfig,
    ) -> Result<Vec<u8>, FrameworkError> {
        self.run_leaving_room(args, input, config, 0)
    }

    /// [`Self::run`], with the stdout buffer keeping `room` bytes of
    /// capacity past the output, for the metadata [`Self::settle`] adds.
    fn run_leaving_room(
        &self,
        args: &[String],
        input: &[u8],
        config: &ImageConfig,
        room: usize,
    ) -> Result<Vec<u8>, FrameworkError> {
        // HRD-010: the policy goes in place and the policy in effect is
        // checked first, so a run that cannot have it is refused before
        // ImageMagick is started for it. Every ImageMagick run passes
        // through here, the probes included.
        let operator = operator_configure_path();
        let configure_path = prepare_policy(operator.as_deref())?;
        self.verify_policy(&configure_path, operator, config)?;
        self.execute(args, input, config, room, &configure_path)
    }

    /// Start the binary with `args` under `configure_path`, stream `input`
    /// in, and collect stdout, keeping `room` bytes of capacity past it.
    /// A run that exits with a failure, or writes nothing, is an error.
    ///
    /// The caller has put the framework's policy in place on
    /// `configure_path` and, for a run on an image, checked the policy in
    /// effect.
    fn execute(
        &self,
        args: &[String],
        input: &[u8],
        config: &ImageConfig,
        room: usize,
        configure_path: &OsStr,
    ) -> Result<Vec<u8>, FrameworkError> {
        let Finished {
            status,
            stdout,
            stderr,
        } = self.finish(args, input, config, room, configure_path)?;
        if !status.success() {
            let stderr = String::from_utf8_lossy(&stderr);
            let detail = match sanitise_stderr(&stderr) {
                Some(detail) => detail,
                None => format!("exited with {status}"),
            };
            // A failure here is usually a bad or unsupported input, so this
            // is caller-facing (4xx) rather than an internal fault.
            return Err(FrameworkError::param(format!(
                "image processing failed in ImageMagick: {detail}"
            )));
        }
        if stdout.is_empty() {
            return Err(FrameworkError::param(
                "image processing failed in ImageMagick: it produced no output",
            ));
        }
        Ok(stdout)
    }

    /// [`Self::execute`] up to the child's exit: the exit status and both
    /// streams, whatever the status. An error here means the child could not
    /// be started, waited for, or finished within the deadline, so there is
    /// no exit status to judge.
    fn finish(
        &self,
        args: &[String],
        input: &[u8],
        config: &ImageConfig,
        room: usize,
        configure_path: &OsStr,
    ) -> Result<Finished, FrameworkError> {
        // The driver makes the stdin pipe itself, rather than through
        // `Stdio::piped()`, to keep a read end of its own: see the writer
        // below.
        let pipe_error = |e: std::io::Error| {
            FrameworkError::internal(format!(
                "image: could not open a pipe to `{}`: {e}",
                self.binary
            ))
        };
        let (stdin_reader, mut stdin) = std::io::pipe().map_err(pipe_error)?;
        let mut unread = stdin_reader.try_clone().map_err(pipe_error)?;
        let mut command = Command::new(&self.binary);
        command
            .args(args)
            .env("MAGICK_CONFIGURE_PATH", configure_path)
            .stdin(stdin_reader)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Give the child its own process group, so a delegate it spawns
            // can be signalled as a unit. Without this, killing the child on
            // timeout leaves any delegate running - and a delegate inherits
            // the pipe write ends, so it keeps our readers blocked even after
            // the process we started is gone.
            command.process_group(0);
        }

        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FrameworkError::internal(format!(
                    "image: the `{}` binary was not found. IMAGE_DRIVER=magick requires \
                     ImageMagick 7 installed on the host; install it, or set \
                     IMAGE_MAGICK_BINARY to its path, or use the default IMAGE_DRIVER=oxideav.",
                    self.binary
                ))
            } else {
                FrameworkError::internal(format!("image: could not run `{}`: {e}", self.binary))
            }
        })?;

        // Every pipe gets its own thread. Writing the whole input before
        // reading stdout deadlocks the moment the output outgrows the pipe
        // buffer, and polling for the child's exit without draining stdout
        // deadlocks the same way - so all three have to move concurrently.
        //
        // The two readers are detached, each reporting through a channel
        // rather than a join handle, because a thread blocked in
        // `read_to_end` cannot be joined: the read only returns when every
        // write end of the pipe is closed, and an orphaned delegate holds one.
        // `recv_timeout` lets this function walk away from a worker that is
        // never coming back, which is what bounds the call.
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| FrameworkError::internal("image: could not open the magick stdout"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| FrameworkError::internal("image: could not open the magick stderr"))?;

        let (out_tx, out_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = out_tx.send(read_leaving_room(&mut stdout, room));
        });
        let (err_tx, err_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = stderr.read_to_end(&mut buffer);
            let _ = err_tx.send(buffer);
        });

        // The writer borrows the input rather than a copy of it (MEM-003),
        // so it is scoped and has to end before this function returns. A
        // write blocks while the pipe is full and nothing reads it: a child
        // that exits, or is killed, before it has read all of its input, or
        // a delegate that inherited stdin and outlives it. So once the child
        // is gone, the driver reads whatever is left through its own read
        // end, which lets the write finish; that is at most the unread part
        // of the input. Dropping the write end when the write finishes is
        // what tells the child its input has ended.
        let waited = std::thread::scope(|scope| {
            scope.spawn(move || {
                // An error here means the child stopped reading early; its
                // stderr is the useful diagnostic, not this.
                let _ = stdin.write_all(input);
            });
            let waited = self.wait_with_deadline(&mut child, config);
            let _ = std::io::copy(&mut unread, &mut std::io::sink());
            waited
        });

        let Some(status) = waited? else {
            // Timed out. Do NOT wait on the readers here: their buffers are
            // discarded on this path anyway, and if something survived the
            // signal they will never report. Walking away is what makes the
            // deadline a real bound rather than an aspiration.
            return Err(self.timeout_error(config));
        };

        // The child exited on its own, so its pipes should close immediately.
        // Still bounded: a delegate that outlived a *successful* run would
        // otherwise extend the call past the deadline it just met.
        let stdout = out_rx.recv_timeout(READER_DRAIN_GRACE).unwrap_or_default();
        let stderr = err_rx.recv_timeout(READER_DRAIN_GRACE).unwrap_or_default();
        Ok(Finished {
            status,
            stdout,
            stderr,
        })
    }

    /// The error a wall-clock kill produces.
    ///
    /// Deliberately `internal` (5xx) rather than `param` (4xx), even though a
    /// request triggered it. A timeout means the image path was wedged hard
    /// enough that the framework had to kill a process - an operational fault,
    /// and precisely the signal an operator needs in their server-error
    /// monitoring. Classifying it 4xx would file it as "client sent something
    /// odd" and hide the one condition worth paging on. Yes, that means a
    /// client can provoke a 5xx; being able to see that happening is the
    /// point. Please do not "fix" this back to `param`.
    fn timeout_error(&self, config: &ImageConfig) -> FrameworkError {
        FrameworkError::internal(format!(
            "image processing timed out: `{}` did not finish within {} seconds and was \
             terminated. Raise IMAGE_MAGICK_TIMEOUT_SECS if this is a legitimately slow \
             conversion.",
            self.binary, config.magick_timeout_secs
        ))
    }

    /// Wait for the child, terminating it once the deadline passes.
    ///
    /// Returns `Some(status)` when the child exited on its own and `None` when
    /// it had to be killed.
    ///
    /// # Why this exists on top of `-limit time`
    ///
    /// ImageMagick's `-limit time` is enforced by its own resource monitor,
    /// which only runs once the monitor is engaged. A child wedged *inside* a
    /// delegate before that point never trips it, and waiting on the child
    /// would then block forever - holding the `spawn_blocking` worker this
    /// driver runs on, which is exactly the pool exhaustion the time limit
    /// exists to prevent. So IM's limit is the polite bound and this is the
    /// hard one.
    fn wait_with_deadline(
        &self,
        child: &mut Child,
        config: &ImageConfig,
    ) -> Result<Option<ExitStatus>, FrameworkError> {
        let deadline = Instant::now()
            + Duration::from_secs(u64::from(config.magick_timeout_secs))
            + DEADLINE_GRACE;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(Some(status)),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        self.terminate(child);
                        return Ok(None);
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => {
                    // Reap on the way out too. Returning straight from here
                    // would leave a running child behind with nothing left
                    // holding a handle to it.
                    self.terminate(child);
                    return Err(FrameworkError::internal(format!(
                        "image: `{}` failed: {e}",
                        self.binary
                    )));
                }
            }
        }
    }

    /// Kill the child and, on unix, everything it spawned; then reap it.
    ///
    /// The process group is the part that matters. `Child::kill` signals only
    /// the process we started, so an ImageMagick delegate that outlives it
    /// keeps running - still holding the inherited pipe write ends, still
    /// burning whatever resource made us give up. Because the child was
    /// spawned with `process_group(0)`, its group id equals its pid, and
    /// signalling the negated pid reaches the whole tree.
    ///
    /// Signalling a group needs `killpg`, which means either `libc` (not a
    /// direct dependency here) or `unsafe` (which this crate forbids), so it
    /// goes through the `kill` utility with a fixed, fully numeric argv. No
    /// shell, and nothing an input can influence. If the utility is missing
    /// the direct `kill` below still runs, so the worst case is the delegate
    /// leaking - which the abandoned readers already tolerate.
    fn terminate(&self, child: &mut Child) {
        #[cfg(unix)]
        {
            let group = child.id();
            let _ = Command::new("kill")
                .args(["-KILL", &format!("-{group}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = child.kill();
        // Reap, or the zombie outlives the request.
        let _ = child.wait();
    }
}

/// The file ImageMagick looks for in each directory of its configure path.
const POLICY_FILE: &str = "policy.xml";

/// The start of the name of a process's policy directory. The process id
/// and a random suffix follow: the id so an operator can tell which process
/// a directory belongs to, and the suffix so nobody can guess the name and
/// make the directory first.
const POLICY_DIR_PREFIX: &str = "suprnova-magick-policy-";

/// ImageMagick's separator between the directories of
/// `MAGICK_CONFIGURE_PATH`, its `DirectoryListSeparator`: `;` on Windows
/// and `:` everywhere else.
const CONFIGURE_PATH_SEPARATOR: &str = if cfg!(windows) { ";" } else { ":" };

/// This process's policy directory, made by the first run that needs one.
///
/// A mutex rather than a `OnceLock`, because the slot changes after it is
/// first filled: a directory that cannot be made at one run may be made at
/// the next, and one removed under the process is replaced. Holding the lock
/// while the policy is checked and written also keeps two runs of this
/// process from writing the file at the same time.
static POLICY_DIR: Mutex<Option<PolicyDir>> = Mutex::new(None);

/// The directory a process keeps its ImageMagick policy in (HRD-010).
///
/// One a process, under [`std::env::temp_dir`]: the framework has no
/// temporary directory of its own, and the system's is the one place every
/// process can expect to write. The directory is made in one `mkdir` that
/// fails if the name exists, with a name nobody can guess and, on unix, mode
/// `0700`. So no other user can make it first, put a file in it, or replace
/// the policy in it; the only users who can are this process's own and root,
/// which can change what ImageMagick runs anyway.
///
/// It stays after the process exits, because Rust runs no destructor for a
/// static. It holds one small file, and the system's temporary-file cleanup
/// removes it in time.
struct PolicyDir {
    path: PathBuf,
    /// The user that owned the directory when this process made it: this
    /// process's user, read back from the directory, because std has no
    /// portable call that names it.
    #[cfg(unix)]
    owner: u32,
}

impl PolicyDir {
    /// Make a new policy directory under the system's temporary directory.
    fn create() -> Result<Self, FrameworkError> {
        let parent = std::env::temp_dir();
        // ImageMagick splits `MAGICK_CONFIGURE_PATH` at the separator and
        // has no way to quote one, so a directory whose path holds it would
        // be read as two other directories.
        if parent
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .any(|byte| CONFIGURE_PATH_SEPARATOR.as_bytes().contains(byte))
        {
            return Err(policy_error(
                &parent,
                format!(
                    "the path holds `{CONFIGURE_PATH_SEPARATOR}`, which ImageMagick reads as the \
                     end of a directory in MAGICK_CONFIGURE_PATH; point TMPDIR at a path \
                     without it"
                ),
            ));
        }
        let prefix = format!("{POLICY_DIR_PREFIX}{}-", std::process::id());
        let mut builder = tempfile::Builder::new();
        builder.prefix(&prefix);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // tempfile makes a directory with mode 0777 less the umask
            // unless told otherwise.
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let path = builder
            .tempdir_in(&parent)
            .map_err(|e| policy_error(&parent, format!("could not make a directory in it: {e}")))?
            .keep();
        #[cfg(unix)]
        let owner = {
            use std::os::unix::fs::MetadataExt;
            std::fs::symlink_metadata(&path)
                .map_err(|e| policy_error(&path, format!("could not read the directory: {e}")))?
                .uid()
        };
        Ok(Self {
            path,
            #[cfg(unix)]
            owner,
        })
    }

    /// Whether the directory is still the one this process made: a
    /// directory, not a link to one, owned by the user that made it, and
    /// closed to everyone else. One that a temporary-file cleaner removed,
    /// or that was replaced or opened up, is not, and the caller makes a new
    /// one rather than trust it.
    fn is_intact(&self) -> bool {
        let Ok(meta) = std::fs::symlink_metadata(&self.path) else {
            return false;
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if meta.uid() != self.owner || meta.permissions().mode() & 0o077 != 0 {
                return false;
            }
        }
        meta.is_dir()
    }

    /// Leave the framework's policy in the directory: check the file, write
    /// it again when it is missing or differs, and check it again after the
    /// write, so a run starts only on a file known to hold the policy.
    fn ensure_policy(&self) -> Result<(), FrameworkError> {
        let file = self.path.join(POLICY_FILE);
        match check_policy(&file) {
            Ok(PolicyFile::Same) => return Ok(()),
            // A new directory, or one whose file a cleaner removed.
            Ok(PolicyFile::Missing) => {}
            Ok(PolicyFile::Different) | Err(_) => {
                tracing::warn!(
                    path = %file.display(),
                    "image: the magick driver's ImageMagick policy is not the framework's; \
                     writing it again"
                );
            }
        }
        write_policy(&self.path, &file)
            .map_err(|e| policy_error(&file, format!("could not write it: {e}")))?;
        match check_policy(&file) {
            Ok(PolicyFile::Same) => Ok(()),
            Ok(PolicyFile::Missing | PolicyFile::Different) => Err(policy_error(
                &file,
                "it does not hold the framework's policy after the driver wrote it",
            )),
            Err(e) => Err(policy_error(&file, format!("could not read it back: {e}"))),
        }
    }
}

/// What a policy file holds, against [`MagickCliDriver::POLICY`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyFile {
    /// No file by that name.
    Missing,
    /// A file holding exactly the framework's policy.
    Same,
    /// Anything else: other bytes, or a link or a directory by that name.
    Different,
}

/// Check the file at `path` against the framework's policy.
///
/// A link counts as different whatever it points to, because ImageMagick
/// would follow it and its target can change after the check. The read stops
/// one byte past the policy's length, which is enough to tell a longer file
/// apart, so a large file costs no more to check than the policy.
fn check_policy(path: &Path) -> std::io::Result<PolicyFile> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(PolicyFile::Missing),
        Err(e) => return Err(e),
    };
    if !meta.is_file() {
        return Ok(PolicyFile::Different);
    }
    let policy = MagickCliDriver::POLICY.as_bytes();
    let mut held = Vec::with_capacity(policy.len() + 1);
    std::fs::File::open(path)?
        .take(policy.len() as u64 + 1)
        .read_to_end(&mut held)?;
    Ok(if held == policy {
        PolicyFile::Same
    } else {
        PolicyFile::Different
    })
}

/// Write the framework's policy to `file` in `dir`, whole or not at all.
///
/// The bytes go to a new file in the same directory, which then replaces
/// `file` in one rename, a link by that name included. An ImageMagick run
/// this process started earlier can be reading the policy at that moment,
/// and it reads the old file or the new one, never a half-written one that
/// ends before the entries that matter. There is no `sync_all`: ImageMagick
/// reads the file through the page cache, so the rename is enough for it to
/// see every byte, and after a crash the next run checks the file again.
fn write_policy(dir: &Path, file: &Path) -> std::io::Result<()> {
    let mut temporary = tempfile::Builder::new()
        .prefix(".policy-")
        .tempfile_in(dir)?;
    temporary.write_all(MagickCliDriver::POLICY.as_bytes())?;
    // A failed rename drops the temporary file, which removes it.
    temporary.persist(file).map_err(|e| e.error)?;
    Ok(())
}

/// The error that refuses a run whose policy cannot be put in place.
///
/// An `internal` (5xx) error: the cause is the host, such as a temporary
/// directory the process cannot write, not the request, and an operator
/// watches server errors. It names the path and the cause, and never the
/// policy's bytes.
fn policy_error(path: &Path, cause: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::internal(format!(
        "image: the magick driver could not put its ImageMagick policy in place at `{}`: \
         {cause}. ImageMagick was not started.",
        path.display()
    ))
}

/// The operator's own `MAGICK_CONFIGURE_PATH`, when it names anything. An
/// empty value adds no directory, so it counts as unset.
fn operator_configure_path() -> Option<OsString> {
    std::env::var_os("MAGICK_CONFIGURE_PATH").filter(|value| !value.is_empty())
}

/// Put the framework's policy in place for one run, and return the
/// `MAGICK_CONFIGURE_PATH` the run starts with.
///
/// A poisoned lock is taken as it stands: the slot holds a path, and
/// nothing a panic could leave half done is trusted, since every run checks
/// the directory and the file again.
fn prepare_policy(operator: Option<&OsStr>) -> Result<OsString, FrameworkError> {
    let mut slot = POLICY_DIR.lock().unwrap_or_else(PoisonError::into_inner);
    let dir = match slot.take() {
        Some(dir) if dir.is_intact() => dir,
        previous => {
            if let Some(previous) = previous {
                tracing::warn!(
                    path = %previous.path.display(),
                    "image: the magick driver's ImageMagick policy directory was removed or \
                     changed; making a new one"
                );
            }
            PolicyDir::create()?
        }
    };
    let dir = slot.insert(dir);
    dir.ensure_policy()?;
    Ok(configure_path(&dir.path, operator))
}

/// The `MAGICK_CONFIGURE_PATH` a run starts with: the policy directory,
/// the operator's own value unchanged, and the policy directory again.
///
/// ImageMagick reads the `policy.xml` of every directory on the path, in
/// order, and where two entries match it applies the one it read last. With
/// the framework's directory last as well as first, no entry on the
/// operator's path can outrank the framework's, while the operator's
/// denials, and the other configuration files the operator keeps there,
/// still apply. The operator's value is kept byte for byte rather than split
/// and joined again, so a directory list ImageMagick accepted before reads
/// the same after. With no operator value the one directory is both first
/// and last.
fn configure_path(dir: &Path, operator: Option<&OsStr>) -> OsString {
    let mut value = dir.as_os_str().to_owned();
    if let Some(operator) = operator {
        value.push(CONFIGURE_PATH_SEPARATOR);
        value.push(operator);
        value.push(CONFIGURE_PATH_SEPARATOR);
        value.push(dir);
    }
    value
}

/// The verdict on the policy in effect, for each binary and operator
/// configure path this process has run ImageMagick with (HRD-010).
///
/// ImageMagick reads its own configuration directory and the user's after
/// every directory on `MAGICK_CONFIGURE_PATH`, so a grant there outranks the
/// framework's policy wherever the driver puts it. The driver therefore
/// asks ImageMagick for the policy it applies, once a process for each pair,
/// and keeps the verdict: the host's policy files do not change under a
/// running process in any way the driver is meant to follow. A vector, not
/// a map, because a process runs one binary under one configure path in
/// practice. The lock is held while ImageMagick lists its policy, so two
/// first runs ask once.
static POLICY_VERDICTS: Mutex<Vec<PolicyVerdict>> = Mutex::new(Vec::new());

/// What the check of the policy in effect found for one binary and one
/// operator configure path.
struct PolicyVerdict {
    binary: String,
    operator: Option<OsString>,
    /// Why the policy in effect fails the check, or `None` when it passes.
    refusal: Option<String>,
}

impl MagickCliDriver {
    /// Check that the policy ImageMagick applies under `configure_path`
    /// denies everything the framework's policy denies, asking it once a
    /// process for this binary and `operator` value.
    ///
    /// The check reads `<binary> -list policy`, started under the same
    /// environment as the runs, and when the listing passes, runs one MVG
    /// probe ([`Self::probe_mvg`]) under it too. A listing that shows any of
    /// the framework's denials reopened, or missing, or that the driver
    /// cannot read, and a probe that ImageMagick does not refuse by its
    /// policy, refuse the run and every later one with this binary and
    /// operator value. A listing or probe that could not be had at all (the
    /// binary is missing, or the listing failed or either timed out)
    /// refuses the run but is not kept: that is a fault of the moment, such
    /// as ImageMagick installed after the process started, and the next run
    /// asks again.
    fn verify_policy(
        &self,
        configure_path: &OsStr,
        operator: Option<OsString>,
        config: &ImageConfig,
    ) -> Result<(), FrameworkError> {
        let mut verdicts = POLICY_VERDICTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(found) = verdicts
            .iter()
            .find(|verdict| verdict.binary == self.binary && verdict.operator == operator)
        {
            return match &found.refusal {
                None => Ok(()),
                Some(cause) => Err(policy_refusal(cause)),
            };
        }
        let args = ["-list".to_string(), "policy".to_string()];
        let listing = self
            .execute(&args, b"", config, 0, configure_path)
            .map_err(|e| {
                policy_refusal(&format!(
                    "`{} -list policy` failed, so the policy in effect cannot be verified: {}",
                    self.binary,
                    e.message()
                ))
            })?;
        let mut refusal = parse_listing(&String::from_utf8_lossy(&listing))
            .and_then(|entries| judge_listing(&entries))
            .err();
        if refusal.is_none() {
            refusal = self.probe_mvg(configure_path, config)?;
        }
        verdicts.push(PolicyVerdict {
            binary: self.binary.clone(),
            operator,
            refusal: refusal.clone(),
        });
        match refusal {
            None => Ok(()),
            Some(cause) => Err(policy_refusal(&cause)),
        }
    }
}

/// The MVG script the probe hands ImageMagick: one line, a one-pixel
/// canvas with nothing drawn on it, so a policy that lets it through costs
/// nothing and reads no file.
const MVG_PROBE: &[u8] = b"viewbox 0 0 1 1\n";

impl MagickCliDriver {
    /// Ask ImageMagick to read an MVG script under `configure_path`, and
    /// check that its security policy refuses it. Returns why the policy in
    /// effect fails the check, or `None` when the refusal came.
    ///
    /// `magick -list policy` leaves out an entry marked `stealth="true"`, so
    /// a stealth grant in the host's policy passes [`judge_listing`]. The
    /// probe sees what ImageMagick does rather than what it lists, for MVG,
    /// the script coder an upload most often abuses. Its arguments are
    /// fixed: the limits of every run, `-format %w` with `info:`, which
    /// writes the width to stdout and no file, and `mvg:-`, which names the
    /// coder. The refusal counts only as a failed exit whose stderr names the
    /// security policy and `MVG`: ImageMagick 7.1 writes "not authorized by
    /// the security policy", and earlier releases "not allowed". A probe
    /// that cannot be run at all is an error, which the caller does not
    /// keep. The error never repeats what the probe wrote.
    fn probe_mvg(
        &self,
        configure_path: &OsStr,
        config: &ImageConfig,
    ) -> Result<Option<String>, FrameworkError> {
        let mut args = limit_args(config);
        args.extend(["-format", "%w", "mvg:-", "info:"].map(String::from));
        let finished = self
            .finish(&args, MVG_PROBE, config, 0, configure_path)
            .map_err(|e| {
                policy_refusal(&format!(
                    "the MVG probe of `{}` could not be run, so the policy in effect cannot be \
                     verified: {}",
                    self.binary,
                    e.message()
                ))
            })?;
        let stderr = String::from_utf8_lossy(&finished.stderr);
        let refused = !finished.status.success()
            && stderr.contains("security policy")
            && (stderr.contains("not authorized") || stderr.contains("not allowed"))
            && stderr.contains("`MVG'");
        if refused {
            return Ok(None);
        }
        let outcome = if finished.status.success() {
            "ImageMagick read an MVG probe script"
        } else {
            "ImageMagick failed an MVG probe script for a reason other than its security policy"
        };
        Ok(Some(format!(
            "the policy in effect cannot be shown to deny the MVG coder: {outcome}, so either a \
             policy entry that `magick -list policy` does not show (one marked \
             `stealth=\"true\"`) reopened it, or ImageMagick refused it in words the driver \
             does not recognise"
        )))
    }
}

/// The error that refuses a run because the policy in effect fails the
/// check.
///
/// The same `internal` (5xx) class as [`policy_error`]: the cause is the
/// host's ImageMagick configuration, not the request. It names the entry and
/// the file it came from, and never the bytes of a policy.
fn policy_refusal(cause: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "image: the magick driver refused to run ImageMagick: {cause}"
    ))
}

/// One entry of `magick -list policy`, in the order ImageMagick applies
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListedEntry {
    /// The `Path:` line above the entry: the file it came from, or
    /// `[built-in]`.
    source: String,
    /// The `Policy:` line: `Coder`, `Delegate`, `Path`, `Resource`, and so on.
    domain: String,
    /// The rights, for a domain that has them: `None`, or any of `Read`,
    /// `Write` and `Execute`.
    rights: Option<String>,
    pattern: Option<String>,
}

impl ListedEntry {
    fn is(&self, domain: &str) -> bool {
        self.domain.eq_ignore_ascii_case(domain)
    }

    /// Whether the entry grants any right. An entry without rights grants
    /// none: [`parse_listing`] refuses a coder, delegate or path entry
    /// without them.
    fn grants(&self) -> bool {
        self.rights
            .as_deref()
            .is_some_and(|rights| rights != "None")
    }

    /// The entry as an error names it.
    fn describe(&self) -> String {
        format!(
            "the entry with pattern `{}` and rights {} in {}",
            self.pattern.as_deref().unwrap_or(""),
            self.rights.as_deref().unwrap_or("None"),
            self.source
        )
    }
}

/// Read a `magick -list policy` listing into its entries.
///
/// ImageMagick prints a `Path:` line for each file, then each entry as a
/// `  Policy:` line followed by indented `name: value` fields; for the
/// domains that carry rights, a `rights:` field (`None`, or any of `Read`,
/// `Write` and `Execute`) and a `pattern:` field. The reading is strict, so
/// a listing in any other shape is refused rather than guessed at: an
/// unverifiable policy is not a verified one. A field this function does not
/// know is skipped, so a later ImageMagick that prints one more still reads.
/// An error names the line, never its text, which is the host's.
fn parse_listing(listing: &str) -> Result<Vec<ListedEntry>, String> {
    let unreadable = |line: usize, what: &str| {
        format!(
            "line {line} of `magick -list policy` {what}, so the policy in effect cannot be \
             verified"
        )
    };
    let mut source: Option<String> = None;
    let mut entries: Vec<ListedEntry> = Vec::new();
    // Whether the last entry belongs to the current `Path:` line, so a
    // field line can only ever fill in the entry above it.
    let mut open = false;
    for (index, line) in listing.lines().enumerate() {
        let number = index + 1;
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(path) = line.strip_prefix("Path: ") {
            source = Some(path.to_string());
            open = false;
            continue;
        }
        if let Some(domain) = line.strip_prefix("  Policy: ") {
            let Some(source) = source.clone() else {
                return Err(unreadable(number, "is an entry before any `Path:` line"));
            };
            entries.push(ListedEntry {
                source,
                domain: domain.trim().to_string(),
                rights: None,
                pattern: None,
            });
            open = true;
            continue;
        }
        let field = line
            .strip_prefix("    ")
            .and_then(|field| field.split_once(':'));
        let (Some((name, value)), Some(entry), true) = (field, entries.last_mut(), open) else {
            return Err(unreadable(number, "is not a line ImageMagick writes"));
        };
        let value = value.trim();
        match name {
            "rights" => {
                let known = value
                    .split_whitespace()
                    .all(|right| matches!(right, "None" | "Read" | "Write" | "Execute"));
                if value.is_empty() || !known {
                    return Err(unreadable(
                        number,
                        "names a right ImageMagick does not have",
                    ));
                }
                entry.rights = Some(value.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            "pattern" => entry.pattern = Some(value.to_string()),
            _ => {}
        }
    }
    if entries.is_empty() {
        return Err(
            "`magick -list policy` listed no entries, so the policy in effect cannot be verified"
                .to_string(),
        );
    }
    if let Some(entry) = entries.iter().find(|entry| {
        ["coder", "delegate", "path"]
            .iter()
            .any(|domain| entry.is(domain))
            && (entry.rights.is_none() || entry.pattern.is_none())
    }) {
        return Err(format!(
            "`magick -list policy` lists a {} entry in {} without its rights or pattern, so the \
             policy in effect cannot be verified",
            entry.domain, entry.source
        ));
    }
    Ok(entries)
}

/// The coders the framework's policy denies, read from
/// [`MagickCliDriver::POLICY`] itself, so the check can never fall out of
/// step with the file it checks.
fn denied_coders() -> impl Iterator<Item = &'static str> {
    MagickCliDriver::POLICY.lines().filter_map(|line| {
        line.trim()
            .strip_prefix(r#"<policy domain="coder" rights="none" pattern=""#)?
            .strip_suffix(r#"" />"#)
    })
}

/// Check the policy in effect, as listed, against the framework's denials.
///
/// ImageMagick applies the last entry that matches, so for each coder the
/// framework denies, the last entry whose pattern matches it must deny it.
/// ImageMagick 7.1.2 matches patterns with case, but the check does not rely
/// on that: an entry that grants and matches ignoring case reopens the
/// coder, and only a denial that matches with case closes it. A coder no
/// entry names is open, since ImageMagick then did not read the framework's
/// policy at all. Every delegate stays denied when no delegate entry after
/// the last `*` denial grants anything, and indirect reads stay denied when
/// no path entry after the last `@*` denial grants a pattern that could
/// match a path starting with `@`.
fn judge_listing(entries: &[ListedEntry]) -> Result<(), String> {
    for coder in denied_coders() {
        let mut denied = false;
        for entry in entries.iter().rev().filter(|entry| entry.is("coder")) {
            let pattern = entry.pattern.as_deref().unwrap_or("").as_bytes();
            if !glob_matches(pattern, coder.as_bytes(), true) {
                continue;
            }
            if entry.grants() {
                return Err(format!(
                    "the policy in effect grants the {coder} coder, which the framework's \
                     policy denies: {} outranks the framework's, because ImageMagick applies \
                     the last entry that matches. Remove that grant from the host's policy",
                    entry.describe()
                ));
            }
            if glob_matches(pattern, coder.as_bytes(), false) {
                denied = true;
                break;
            }
        }
        if !denied {
            return Err(format!(
                "the policy in effect does not deny the {coder} coder: no entry of `magick \
                 -list policy` denies it, so ImageMagick did not apply the framework's policy"
            ));
        }
    }
    let later_grant = |domain: &str, denial: &str, reaches: &dyn Fn(&str) -> bool| {
        let Some(last) = entries.iter().rposition(|entry| {
            entry.is(domain) && entry.pattern.as_deref() == Some(denial) && !entry.grants()
        }) else {
            return Err(format!(
                "the policy in effect has no {domain} entry that denies `{denial}`, so \
                 ImageMagick did not apply the framework's policy"
            ));
        };
        match entries[last + 1..].iter().find(|entry| {
            entry.is(domain) && entry.grants() && reaches(entry.pattern.as_deref().unwrap_or(""))
        }) {
            Some(entry) => Err(format!(
                "the policy in effect reopens what the framework's `{domain}` entry `{denial}` \
                 denies: {} outranks the framework's, because ImageMagick applies the last \
                 entry that matches. Remove that grant from the host's policy",
                entry.describe()
            )),
            None => Ok(()),
        }
    };
    later_grant("delegate", "*", &|_| true)?;
    later_grant("path", "@*", &may_match_indirect)?;
    Ok(())
}

/// Whether a path pattern could match a path that starts with `@`, an
/// indirect read. Only a pattern that starts with a plain character other
/// than `@` is known not to; any other start, a wildcard, a class, a set of
/// alternatives or an escape, counts as reaching it.
fn may_match_indirect(pattern: &str) -> bool {
    match pattern.as_bytes().first() {
        None | Some(b'@' | b'*' | b'?' | b'[' | b'{' | b'\\') => true,
        Some(_) => false,
    }
}

/// Whether `pattern` matches all of `text`, as ImageMagick's glob reads a
/// policy pattern: `*` for any run of characters, `?` for one, `[...]` for
/// one of a set (with ranges, and `!` or `^` to negate), `{a,b}` for one of
/// several alternatives, and `\` to take the next character as it is. With
/// `fold`, letters match regardless of case. A `[` or `{` that is never
/// closed is a plain character. The patterns come from the host's
/// configuration and the texts are coder names, so the recursion stays
/// shallow.
fn glob_matches(pattern: &[u8], text: &[u8], fold: bool) -> bool {
    let same = |a: u8, b: u8| {
        if fold {
            a.eq_ignore_ascii_case(&b)
        } else {
            a == b
        }
    };
    let Some((&first, rest)) = pattern.split_first() else {
        return text.is_empty();
    };
    match first {
        b'*' => (0..=text.len()).any(|skip| glob_matches(rest, &text[skip..], fold)),
        b'?' => !text.is_empty() && glob_matches(rest, &text[1..], fold),
        b'\\' if !rest.is_empty() => {
            text.first().is_some_and(|&c| same(rest[0], c))
                && glob_matches(&rest[1..], &text[1..], fold)
        }
        b'[' if rest.contains(&b']') => {
            let Some(close) = rest
                .iter()
                .skip(1)
                .position(|&c| c == b']')
                .map(|at| at + 1)
            else {
                return false;
            };
            let (negated, set) = match rest[..close].split_first() {
                Some((b'!' | b'^', set)) => (true, set),
                _ => (false, &rest[..close]),
            };
            let Some(&c) = text.first() else {
                return false;
            };
            let mut inside = false;
            let mut at = 0;
            while at < set.len() {
                if at + 2 < set.len() && set[at + 1] == b'-' {
                    let (low, high) = (set[at], set[at + 2]);
                    inside |= (low..=high).any(|member| same(member, c));
                    at += 3;
                } else {
                    inside |= same(set[at], c);
                    at += 1;
                }
            }
            inside != negated && glob_matches(&rest[close + 1..], &text[1..], fold)
        }
        b'{' if rest.contains(&b'}') => {
            let Some(close) = rest.iter().position(|&c| c == b'}') else {
                return false;
            };
            rest[..close].split(|&c| c == b',').any(|alternative| {
                let mut joined = alternative.to_vec();
                joined.extend_from_slice(&rest[close + 1..]);
                glob_matches(&joined, text, fold)
            })
        }
        c => text.first().is_some_and(|&t| same(c, t)) && glob_matches(rest, &text[1..], fold),
    }
}

/// A child that exited on its own: its status and what it wrote.
struct Finished {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// What the driver tells ImageMagick to read the input as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// One of the five formats the framework reads and measures itself.
    Read(sniff::InputFormat),
    /// A format the framework names but only ImageMagick reads.
    Delegated(sniff::DelegatedFormat),
    /// Bytes the framework cannot name, handed over for ImageMagick to
    /// choose the coder: only under `ImageConfig::magick_autodetect`.
    Autodetected,
}

impl Source {
    /// The format, when the framework reads it itself.
    fn read(self) -> Option<sniff::InputFormat> {
        match self {
            Self::Read(format) => Some(format),
            Self::Delegated(_) | Self::Autodetected => None,
        }
    }
}

/// How the `magick` driver orients an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Orienting {
    /// Whether the framework reads the format itself (one of its five).
    /// When it does, the orientation comes from the reader the
    /// built-in driver uses, so both drivers turn a file alike; when it does
    /// not (HEIC, TIFF), ImageMagick's own reading is the only one there is.
    known: bool,
    /// The orientation that reader found, if any.
    tag: Option<Orientation>,
}

impl Orienting {
    /// The arguments that apply the orientation: the fixed transform for
    /// the tag the framework read, or ImageMagick's `-auto-orient` for a
    /// format the framework cannot read.
    fn args(self) -> Vec<String> {
        if !self.known {
            return vec!["-auto-orient".into()];
        }
        let turn: &[&str] = match self.tag.map(Orientation::tag) {
            Some(2) => &["-flop"],
            Some(3) => &["-rotate", "180"],
            Some(4) => &["-flip"],
            Some(5) => &["-transpose"],
            Some(6) => &["-rotate", "90"],
            Some(7) => &["-transverse"],
            Some(8) => &["-rotate", "270"],
            _ => return Vec::new(),
        };
        turn.iter()
            .chain(&["+repage"])
            .map(|arg| (*arg).to_string())
            .collect()
    }

    /// Whether ImageMagick keeps the source's EXIF while orientation is
    /// unapplied: only for a format the framework cannot read, so the PNG
    /// between two runs carries ImageMagick's reading of the tag to an
    /// `orient()` after a custom step (see
    /// [`metadata::magick_png_orientation`]). The tag the output keeps
    /// comes from the orientation probe instead, and
    /// [`MagickCliDriver::settle`] strips the EXIF ImageMagick wrote.
    fn keeps_exif(self, applied: bool) -> bool {
        !self.known && !applied
    }
}

/// The image once every custom step has run, and what is left to do.
struct Tail<'p> {
    /// The PNG the last custom step left, or `None` when the pipeline has
    /// no custom step and the last run reads the source itself.
    input: Option<Vec<u8>>,
    /// The steps after the last custom step.
    steps: &'p [Transformation],
    /// Whether orientation is applied by then.
    applied: bool,
}

/// What one ImageMagick run needs to know about the whole pipeline.
struct Run<'a> {
    config: &'a ImageConfig,
    source: Source,
    target: OutputFormat,
    orienting: Orienting,
    /// Drop the source's ICC profile as the image is read: its length is
    /// not the one its header gives, so it is no profile, and libpng
    /// refuses to write it into a PNG.
    drops_profile: bool,
}

impl Run<'_> {
    /// The arguments of one run up to its output settings: the limits, the
    /// input (the source, or the PNG between two runs), the orientation on
    /// decode in the first run, and the steps, an `orient()` among them.
    fn args(&self, first: bool, steps: &[Transformation], applied: &mut bool) -> Vec<String> {
        let mut args = limit_args(self.config);
        if first {
            args.extend(first_frame_input(self.source));
            if self.drops_profile {
                args.push("+profile".into());
                args.push("icc".into());
            }
            if self.config.auto_orient {
                *applied = true;
                args.extend(self.orienting.args());
            }
        } else {
            args.push("png:-[0]".into());
        }
        for step in steps {
            if *step == Transformation::Orient {
                if !*applied {
                    *applied = true;
                    args.extend(self.orienting.args());
                }
                continue;
            }
            args.extend(transformation_args(*step, self.target));
        }
        args
    }
}

impl MagickCliDriver {
    /// Run every custom step of the pipeline, each between two ImageMagick
    /// runs that pass the image over stdout and stdin as a PNG, and return
    /// what is left.
    ///
    /// The runs happen once: when the end of the pipeline has to run again
    /// to keep a profile consistent with its pixels, it starts from here,
    /// so no custom step runs twice.
    fn run_custom_steps<'p>(
        &self,
        contents: &[u8],
        pipeline: &'p ImagePipeline,
        run: &Run<'_>,
    ) -> Result<Tail<'p>, FrameworkError> {
        let (stages, rest) = custom_stages(&pipeline.transformations);
        let mut applied = false;
        let mut input: Option<Vec<u8>> = None;
        for (steps, custom) in stages {
            let mut args = run.args(input.is_none(), steps, &mut applied);
            args.extend(intermediate_args(run.orienting.keeps_exif(applied)));
            let intermediate = self.run(&args, input.as_deref().unwrap_or(contents), run.config)?;
            input = Some(rust_stage(
                &intermediate,
                AfterStage::Custom(custom),
                &pipeline.settings,
                run.orienting.keeps_exif(applied),
                run.config,
            )?);
        }
        Ok(Tail {
            input,
            steps: rest,
            applied,
        })
    }

    /// Run the end of the pipeline under `plan`: one ImageMagick run, or
    /// two around a conversion to sRGB in Rust. Returns the output and
    /// whether orientation was applied. The output keeps `room` bytes of
    /// capacity past its end.
    fn run_tail(
        &self,
        contents: &[u8],
        tail: &Tail<'_>,
        pipeline: &ImagePipeline,
        run: &Run<'_>,
        plan: ColourPlan,
        room: usize,
    ) -> Result<(Vec<u8>, bool), FrameworkError> {
        let mut applied = tail.applied;
        let first = tail.input.is_none();
        let source = tail.input.as_deref().unwrap_or(contents);
        let background = flatten_background(pipeline, run.target);
        let mut args = run.args(first, tail.steps, &mut applied);
        let keeps_exif = run.orienting.keeps_exif(applied);
        if !plan.srgb {
            args.extend(output_args(
                pipeline, run.target, plan, keeps_exif, background,
            ));
            return Ok((
                self.run_leaving_room(&args, source, run.config, room)?,
                applied,
            ));
        }
        args.extend(intermediate_args(keeps_exif));
        let intermediate = self.run(&args, source, run.config)?;
        let converted = rust_stage(
            &intermediate,
            AfterStage::Srgb,
            &pipeline.settings,
            keeps_exif,
            run.config,
        )?;
        let mut args = run.args(false, &[], &mut applied);
        args.extend(output_args(
            pipeline, run.target, plan, keeps_exif, background,
        ));
        Ok((
            self.run_leaving_room(&args, &converted, run.config, room)?,
            applied,
        ))
    }

    /// Keep, of what ImageMagick wrote, what IMG-002 keeps, in place.
    ///
    /// The profile ImageMagick carried is kept as it wrote it, when its
    /// length is the one its header gives and its colour space is the
    /// output pixels' own; it is read to check, without being held. When
    /// the colour spaces differ, the caller runs the end of the pipeline
    /// again with a plan that keeps the two consistent, which
    /// `Settled::Mismatch` reports. A PNG source's colour chunks go with
    /// its profile: they are kept when the output carries the profile, or
    /// when the source had none.
    fn settle(
        &self,
        mut output: Vec<u8>,
        target: OutputFormat,
        orientation: Option<Orientation>,
        png_colour: &[([u8; 4], Vec<u8>)],
        source_had_profile: bool,
        config: &ImageConfig,
    ) -> Result<Settled, FrameworkError> {
        let (class, gif_profile) = {
            let found = metadata::find_output_profile(target, &output);
            // A GIF's palette converts from an RGB or grey profile only, so
            // a CMYK or Lab one is never joined (MEM-003). Any other
            // output's profile is checked by its length and kept or
            // stripped where it stands, so it holds no copy.
            let converts = target == OutputFormat::Gif
                && found
                    .as_ref()
                    .is_some_and(|found| found.header.class != ColourClass::Other);
            if let Some(found) = &found {
                let (bytes, what) = if converts {
                    found.read_cost()
                } else {
                    found.check_cost()
                };
                let room = config.max_alloc_bytes.saturating_sub(output.len() as u64);
                if bytes > room {
                    return Err(FrameworkError::param(format!(
                        "image exceeds configured decode limits: ImageMagick wrote a {}-byte ICC \
                         profile that needs {bytes} bytes {what}, over what the \
                         IMAGE_MAX_ALLOC_BYTES limit of {} leaves beside the output",
                        found.header.size, config.max_alloc_bytes
                    )));
                }
            }
            let found = found.filter(|found| found.is_whole());
            let class = found.as_ref().map(|found| found.header.class);
            // A GIF's profile is joined, so it is owned: `into_owned` moves
            // it, and leaves the output free to convert in place.
            let gif_profile = found
                .filter(|_| converts)
                .and_then(|found| found.read())
                .map(Cow::into_owned);
            (class, gif_profile)
        };
        if target == OutputFormat::Gif {
            // A GIF's palette converts to sRGB in place, so no class
            // mismatches; the profile goes.
            if let Some(profile) = &gif_profile {
                metadata::gif_to_srgb(&mut output, profile)?;
            }
            metadata::strip(&mut output, target, false)?;
            return Ok(Settled::Done(output));
        }
        let pixels = metadata::pixel_class(target, &output);
        let keep_icc = match class {
            Some(class) if class == pixels => true,
            Some(class @ (ColourClass::Rgb | ColourClass::Gray)) => {
                return Ok(Settled::Mismatch {
                    class,
                    png_colour_type: png_colour_type(&output),
                });
            }
            // A CMYK or Lab profile on pixels ImageMagick converted out of
            // that space no longer describes them, and one whose length is
            // not its header's is no profile.
            Some(ColourClass::Other) | None => false,
        };
        metadata::strip(&mut output, target, keep_icc)?;
        let kept = Kept {
            icc: None,
            orientation,
            png_colour: if keep_icc || (class.is_none() && !source_had_profile) {
                png_colour
            } else {
                &[]
            },
        };
        metadata::add(&mut output, &kept.prepare(target)?)?;
        Ok(Settled::Done(output))
    }
}

/// How the driver keeps a profile and the pixels consistent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ColourPlan {
    /// Write a JPEG with three components even when every pixel is grey,
    /// so an RGB profile still describes it.
    jpeg_true_colour: bool,
    /// The PNG colour type to write: 2 or 6 when every pixel is grey and an
    /// RGB profile has to keep describing them.
    png_colour_type: Option<u8>,
    /// Convert the pixels to sRGB in Rust before the last run, and drop the
    /// profile: for a grey profile on pixels the output stores as RGB.
    srgb: bool,
}

impl ColourPlan {
    /// The plan for a profile of `class`, before anything has run.
    fn before(class: Option<ColourClass>, target: OutputFormat) -> Self {
        match class {
            Some(ColourClass::Rgb) => Self {
                jpeg_true_colour: target == OutputFormat::Jpeg,
                ..Self::default()
            },
            // WebP and BMP store grey as RGB, so a grey profile cannot stay.
            Some(ColourClass::Gray) => Self {
                srgb: matches!(
                    target,
                    OutputFormat::WebP | OutputFormat::WebPLossless | OutputFormat::Bmp
                ),
                ..Self::default()
            },
            Some(ColourClass::Other) | None => Self::default(),
        }
    }

    /// The plan after output came back with a profile of `class` that does
    /// not match its pixels.
    fn after_mismatch(
        class: ColourClass,
        target: OutputFormat,
        png_colour_type: Option<u8>,
    ) -> Self {
        match class {
            ColourClass::Rgb => Self {
                jpeg_true_colour: target == OutputFormat::Jpeg,
                // A grey PNG, with alpha (4) or without (0), written as RGB
                // with the same alpha.
                png_colour_type: match png_colour_type {
                    Some(4) => Some(6),
                    _ if target == OutputFormat::Png => Some(2),
                    _ => None,
                },
                srgb: false,
            },
            ColourClass::Gray | ColourClass::Other => Self {
                srgb: true,
                ..Self::default()
            },
        }
    }
}

/// What [`MagickCliDriver::settle`] found.
enum Settled {
    /// The finished output.
    Done(Vec<u8>),
    /// ImageMagick wrote a profile whose colour space is not its pixels'.
    Mismatch {
        class: ColourClass,
        png_colour_type: Option<u8>,
    },
}

/// The colour type in a PNG's header.
fn png_colour_type(bytes: &[u8]) -> Option<u8> {
    (bytes.get(12..16)? == b"IHDR").then(|| bytes.get(25).copied())?
}

/// What runs in Rust between two ImageMagick runs.
#[derive(Debug, Clone, Copy, PartialEq)]
enum AfterStage {
    /// A registered custom transformation.
    Custom(CustomTransformation),
    /// The conversion of the pixels from their profile to sRGB.
    Srgb,
}

/// Split a pipeline at its custom steps: the steps before each custom step,
/// with the step, and the steps after the last.
fn custom_stages(
    transformations: &[Transformation],
) -> (
    Vec<(&[Transformation], CustomTransformation)>,
    &[Transformation],
) {
    let mut stages = Vec::new();
    let mut start = 0;
    for (index, step) in transformations.iter().enumerate() {
        if let Transformation::Custom(custom) = step {
            stages.push((&transformations[start..index], *custom));
            start = index + 1;
        }
    }
    (stages, &transformations[start..])
}

/// The settings that end a run whose image goes on to Rust: keep the ICC
/// profile, and the EXIF when `keeps_exif`, and write an 8-bit PNG to
/// stdout.
fn intermediate_args(keeps_exif: bool) -> Vec<String> {
    let keep = if keeps_exif { "!icc,!exif,*" } else { "!icc,*" };
    vec![
        "+profile".into(),
        keep.into(),
        "-depth".into(),
        "8".into(),
        "png:-".into(),
    ]
}

/// Run a Rust stage on the PNG one ImageMagick run wrote, and encode the
/// PNG the next run reads.
///
/// The next PNG carries the profile when it is an RGB one, and, when
/// `keeps_exif`, the orientation ImageMagick read, as EXIF the next run
/// reads back. A grey profile, or any profile at the sRGB stage, is
/// converted away here, since the PNG between runs is RGBA.
///
/// The RGB profile goes on as the `iCCP` chunk ImageMagick wrote, checked
/// by its length: it is not inflated into a buffer or compressed again,
/// and it is copied once, into the next PNG (MEM-003). The encoder writes
/// the pixels alone, and the chunk and the EXIF are added after, as
/// [`metadata::add`] adds them to any output, into the room the encoder's
/// buffer was given for them: the encoder would compress the profile again
/// rather than carry the chunk as it stands.
///
/// A custom step finds the settings it was recorded with in `settings`,
/// the pipeline's table; they reach only its Rust function, never an
/// ImageMagick argument.
fn rust_stage(
    intermediate: &[u8],
    after: AfterStage,
    settings: &TransformationSettings,
    keeps_exif: bool,
    config: &ImageConfig,
) -> Result<Vec<u8>, FrameworkError> {
    let decoder = OxideAvImageDriver::new();
    let pixels = decoder.decode_unoriented(intermediate, config)?;
    let orientation = if keeps_exif {
        metadata::magick_png_orientation(intermediate)
    } else {
        None
    };
    let found = metadata::find_output_profile(OutputFormat::Png, intermediate);
    let held = pixels.pixels().len() as u64;
    let class = found.as_ref().map(|found| found.header.class);
    // The profile is inflated only where this stage converts from it: a
    // grey one, or an RGB one at the sRGB stage. An RGB one after a custom
    // step is carried as its chunk, and a CMYK or Lab one is dropped as it
    // stands (MEM-003).
    let converts = class == Some(ColourClass::Gray)
        || (class == Some(ColourClass::Rgb) && after == AfterStage::Srgb);
    let carries = class == Some(ColourClass::Rgb) && after != AfterStage::Srgb;
    if let Some(found) = &found {
        let chunk = found.png_chunk().map_or(0, |chunk| chunk.len() as u64);
        let charges: metadata::ProfileCharges = [
            if converts {
                found.read_cost()
            } else if carries {
                found.check_cost()
            } else {
                (0, "")
            },
            if carries {
                (chunk, "for the copy the next PNG carries")
            } else {
                (0, "")
            },
            (0, ""),
        ];
        let charged = metadata::charged(&charges);
        let needed = held.saturating_add(charged);
        if charged > 0 && needed > config.max_alloc_bytes {
            return Err(FrameworkError::param(format!(
                "image exceeds configured decode limits: its {}-byte ICC profile needs {}, which \
                 with the {held} bytes of pixels between two ImageMagick runs is about {needed} \
                 bytes, over the IMAGE_MAX_ALLOC_BYTES limit of {}",
                found.header.size,
                metadata::charges_named(&charges),
                config.max_alloc_bytes
            )));
        }
    }
    let profile = found
        .as_ref()
        .filter(|_| converts)
        .and_then(|found| found.read());
    let carried = found
        .as_ref()
        .filter(|found| carries && found.is_whole())
        .and_then(|found| found.png_chunk());
    let mut pixels = match after {
        AfterStage::Custom(custom) => {
            let out = custom.apply_with(pixels, settings)?;
            sniff::enforce_limits(out.width(), out.height(), config)?;
            out
        }
        AfterStage::Srgb => pixels,
    };
    if let Some(conversion) = profile.as_deref().and_then(SrgbConversion::from_profile) {
        conversion.convert_rgba(pixels.pixels_mut())?;
    }
    let additions = Kept {
        icc: carried.map(IccData::PngChunk),
        orientation,
        png_colour: &[],
    }
    .prepare(OutputFormat::Png)?;
    let (width, height) = (pixels.width(), pixels.height());
    let mut png =
        encode_png(width, height, pixels.into_pixels(), additions.len()).map_err(|e| {
            FrameworkError::internal(format!("image encode failed between ImageMagick runs: {e}"))
        })?;
    if additions.len() > 0 {
        metadata::add(&mut png, &additions)?;
    }
    Ok(png)
}

/// The colour JPEG and GIF output flattens transparency onto: the last
/// rotation's background put over white, or white when nothing rotates.
fn flatten_background(pipeline: &ImagePipeline, target: OutputFormat) -> Color {
    pipeline
        .transformations
        .iter()
        .rev()
        .find_map(|step| match step {
            Transformation::Rotate { background, .. } => {
                Some(background.unwrap_or(metadata::default_background(target)))
            }
            _ => None,
        })
        .unwrap_or(Color::WHITE)
        .over_white()
}

impl ImageDriver for MagickCliDriver {
    fn process(
        &self,
        contents: &[u8],
        pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, FrameworkError> {
        let config = super::config();
        let source = Self::guard(contents, &config)?;
        let detected = source.read();
        let target = match pipeline.format.or_else(|| detected.map(same_format)) {
            Some(format) => format,
            None => {
                return Err(FrameworkError::param(
                    "image format is not one Suprnova writes, so the output format is \
                     ambiguous; call to_format() to choose one",
                ));
            }
        };
        // For the five formats the framework reads, the profile's colour
        // space is known before ImageMagick runs, from its header alone;
        // for the rest, from what ImageMagick writes. A profile stored
        // whole (all but a PNG's, which is compressed and is not inflated
        // here) is measured too, and one that is not as long as its header
        // says is dropped as ImageMagick reads it.
        let found = detected.and_then(|format| metadata::find_profile(format, contents));
        let drops_profile = found
            .as_ref()
            .is_some_and(|found| found.png_chunk().is_none() && !found.is_whole());
        let source_class = found
            .filter(|_| !drops_profile)
            .map(|found| found.header.class);
        // The tag the output keeps when orientation stays unapplied. For a
        // format the framework cannot read it is ImageMagick's own reading,
        // the one `-auto-orient` applies: ImageMagick keeps a TIFF's tag in
        // none of the EXIF it writes, so it cannot be read back from the
        // output. It is asked for only when the output keeps it.
        let unapplied =
            !config.auto_orient && !pipeline.transformations.contains(&Transformation::Orient);
        let tag = match detected {
            Some(format) => metadata::source_orientation(format, contents),
            None if unapplied => parse_orientation(&String::from_utf8_lossy(&self.run(
                &orientation_args(&config, source),
                contents,
                &config,
            )?)),
            None => None,
        };
        let run = Run {
            config: &config,
            source,
            target,
            orienting: Orienting {
                known: detected.is_some(),
                tag,
            },
            drops_profile,
        };
        let png_colour = if detected == Some(sniff::InputFormat::Png) && target == OutputFormat::Png
        {
            metadata::png_colour_chunks(contents)
        } else {
            Vec::new()
        };
        // The most `settle` can add: every colour chunk and the tag. The
        // last run's output keeps that much room, so adding them moves bytes
        // within it rather than copying the output into a larger buffer
        // (MEM-003).
        let room = Kept {
            icc: None,
            orientation: run.orienting.tag,
            png_colour: &png_colour,
        }
        .prepare(target)?
        .len();
        let tail = self.run_custom_steps(contents, pipeline, &run)?;
        let mut plan = ColourPlan::before(source_class, target);
        let mut retried = false;
        loop {
            let (output, applied) = self.run_tail(contents, &tail, pipeline, &run, plan, room)?;
            let orientation = if applied { None } else { run.orienting.tag };
            let colour = if plan.srgb { &[][..] } else { &png_colour[..] };
            match self.settle(
                output,
                target,
                orientation,
                colour,
                source_class.is_some(),
                &config,
            )? {
                Settled::Done(bytes) => return Ok(bytes),
                Settled::Mismatch {
                    class,
                    png_colour_type,
                } if !retried => {
                    retried = true;
                    plan = ColourPlan::after_mismatch(class, target, png_colour_type);
                }
                Settled::Mismatch { .. } => {
                    return Err(FrameworkError::internal(
                        "image: ImageMagick wrote an ICC profile that does not describe its \
                         pixels, twice",
                    ));
                }
            }
        }
    }

    /// The size of the image as decoding presents it: a tag that turns it
    /// a quarter swaps the sides `identify` reports, as the built-in
    /// driver's answer does. The tag is the one decoding applies: the
    /// framework's reading for a format it reads, and ImageMagick's own,
    /// from a second probe, for one it cannot (HEIC, TIFF).
    fn dimensions(&self, contents: &[u8]) -> Result<(u32, u32), FrameworkError> {
        let config = super::config();
        let source = Self::guard(contents, &config)?;
        let raw = self.run(&dimensions_args(&config, source), contents, &config)?;
        let (width, height) = parse_dimensions(&String::from_utf8_lossy(&raw))?;
        let tag = match source.read() {
            _ if !config.auto_orient => None,
            Some(format) => metadata::source_orientation(format, contents),
            None => parse_orientation(&String::from_utf8_lossy(&self.run(
                &orientation_args(&config, source),
                contents,
                &config,
            )?)),
        };
        let swaps = tag.is_some_and(Orientation::swaps_axes);
        Ok(if swaps {
            (height, width)
        } else {
            (width, height)
        })
    }

    fn dominant_color(&self, contents: &[u8]) -> Result<String, FrameworkError> {
        let config = super::config();
        let source = Self::guard(contents, &config)?;
        let raw = self.run(&dominant_color_args(&config, source), contents, &config)?;
        parse_hex_pixel(&String::from_utf8_lossy(&raw))
    }

    fn name(&self) -> &'static str {
        "magick"
    }
}

/// The `OutputFormat` that re-encodes a recognised input unchanged.
fn same_format(format: sniff::InputFormat) -> OutputFormat {
    match format {
        sniff::InputFormat::Png => OutputFormat::Png,
        sniff::InputFormat::Jpeg => OutputFormat::Jpeg,
        sniff::InputFormat::WebP => OutputFormat::WebP,
        sniff::InputFormat::Gif => OutputFormat::Gif,
        sniff::InputFormat::Bmp => OutputFormat::Bmp,
    }
}

/// ImageMagick's own resource caps, derived from [`ImageConfig`].
///
/// These are settings, so they precede the input on the command line.
fn limit_args(config: &ImageConfig) -> Vec<String> {
    let dimension = config.max_dimension.to_string();
    let bytes = config.max_alloc_bytes.to_string();
    let seconds = config.magick_timeout_secs.to_string();
    vec![
        // Wall-clock bound. Without it a delegate that stalls - a malformed
        // HEIC that sends libheif into a long loop, a network-backed coder -
        // holds a `spawn_blocking` worker for the life of the process. The
        // pipe plumbing alone cannot rescue that: the child is simply never
        // going to write, so nothing here would ever return.
        "-limit".into(),
        "time".into(),
        seconds,
        "-limit".into(),
        "width".into(),
        dimension.clone(),
        "-limit".into(),
        "height".into(),
        dimension,
        "-limit".into(),
        "area".into(),
        bytes.clone(),
        "-limit".into(),
        "memory".into(),
        bytes.clone(),
        "-limit".into(),
        "map".into(),
        bytes,
        // Without this, ImageMagick spills the pixel cache to disk when the
        // memory cap is hit and carries on - which would make every cap
        // above advisory rather than enforced.
        "-limit".into(),
        "disk".into(),
        "0".into(),
    ]
}

/// The IM7 operator sequence for one recorded transformation.
///
/// Geometry suffixes carry the semantics: `!` forces exact dimensions,
/// `>` shrinks only, `^` fills the box, and a bare `WxH` fits inside it.
fn transformation_args(step: Transformation, target: OutputFormat) -> Vec<String> {
    let arg = |value: &str| value.to_string();
    match step {
        Transformation::Resize { width, height } => {
            vec![arg("-resize"), format!("{width}x{height}!")]
        }
        Transformation::ResizeWidth(width) => vec![arg("-resize"), format!("{width}x")],
        Transformation::ResizeHeight(height) => vec![arg("-resize"), format!("x{height}")],
        // The kept side is the image's own at this point of the run:
        // ImageMagick 7 expands `%[h]` and `%[w]` in the geometry against
        // the image the operator receives, after every step before it.
        Transformation::ResizeWidthOnly(width) => {
            vec![arg("-resize"), format!("{width}x%[h]!")]
        }
        Transformation::ResizeHeightOnly(height) => {
            vec![arg("-resize"), format!("%[w]x{height}!")]
        }
        Transformation::Scale { width, height } => {
            vec![arg("-resize"), format!("{width}x{height}>")]
        }
        Transformation::ScaleWidth(width) => vec![arg("-resize"), format!("{width}x>")],
        Transformation::ScaleHeight(height) => vec![arg("-resize"), format!("x{height}>")],
        Transformation::Contain { width, height } => {
            vec![arg("-resize"), format!("{width}x{height}")]
        }
        Transformation::Cover { width, height } => vec![
            arg("-resize"),
            format!("{width}x{height}^"),
            arg("-gravity"),
            arg("center"),
            arg("-extent"),
            format!("{width}x{height}"),
            // Reset the virtual canvas the crop leaves behind, or the offset
            // is baked into the output file's geometry.
            arg("+repage"),
        ],
        Transformation::Crop {
            width,
            height,
            x,
            y,
        } => vec![
            arg("-crop"),
            format!("{width}x{height}+{x}+{y}"),
            arg("+repage"),
        ],
        // The colour is the driver's own formatting of a typed value.
        Transformation::Rotate {
            degrees,
            background,
        } => vec![
            arg("-background"),
            background
                .unwrap_or(metadata::default_background(target))
                .to_hex(),
            arg("-rotate"),
            format!("{degrees}"),
            arg("+repage"),
        ],
        Transformation::FlipVertically => vec![arg("-flip")],
        Transformation::FlipHorizontally => vec![arg("-flop")],
        Transformation::Blur(amount) => match blur_sigma(amount) {
            Some(sigma) => vec![arg("-blur"), format!("0x{sigma}")],
            None => Vec::new(),
        },
        Transformation::Sharpen(amount) => match sharpen_amount(amount) {
            Some(strength) => vec![arg("-unsharp"), format!("0x1+{strength}+0")],
            None => Vec::new(),
        },
        Transformation::Grayscale => vec![arg("-colorspace"), arg("Gray")],
        // Neither reaches ImageMagick as an argument of its own: the
        // pipeline places the orientation's fixed transform itself, and a
        // custom step runs in Rust between two runs.
        Transformation::Orient | Transformation::Custom(_) => Vec::new(),
    }
}

/// Blur strength `0..=100` to a Gaussian sigma.
///
/// Matches the pure-Rust driver's radius mapping (`amount/100 * 15`, rounded
/// up, sigma = radius/2) so switching drivers does not visibly change the
/// strength of the same pipeline.
fn blur_sigma(amount: u32) -> Option<f32> {
    let amount = amount.min(100);
    if amount == 0 {
        return None;
    }
    let radius = ((f64::from(amount) / 100.0) * 15.0).ceil().max(1.0);
    Some((radius / 2.0) as f32)
}

/// Sharpen strength `0..=100` to an unsharp amount, matching the pure-Rust
/// driver's scale where 50 is the classic `1.0`.
fn sharpen_amount(amount: u32) -> Option<f32> {
    let amount = amount.min(100);
    if amount == 0 {
        return None;
    }
    Some(amount as f32 / 50.0)
}

/// How stdin is named on the command line.
///
/// A bare `-` lets ImageMagick choose the decoder from the bytes it is handed,
/// which is the ImageTragick shape: a file whose magic says MVG or MSL is read
/// as a *script*, no matter what the application believed it was accepting,
/// with only a `policy.xml` standing in the way. So the
/// driver names the coder for every format the sniffer names, the five the
/// framework reads and the ones only ImageMagick reads - `png:-` decodes as
/// PNG or fails, and `heic:-` as HEIC, and neither can be talked into
/// anything else.
///
/// Bytes the sniffer cannot name never get here by default: the driver
/// refuses them before ImageMagick starts. Only an application that opts
/// into autodetection hands them over on a bare `-`, and then the
/// framework's policy ([`MagickCliDriver::POLICY`]), with the host's beside
/// it, is what keeps them from the script and network coders.
fn input_spec(source: Source) -> String {
    match source {
        Source::Read(format) => format!("{}:-", format.magick_coder()),
        Source::Delegated(format) => format!("{}:-", format.magick_coder()),
        Source::Autodetected => "-".to_string(),
    }
}

/// The input argument and the settings that make ImageMagick work on the
/// image the built-in driver decodes: the first frame (`[0]`), and for a GIF
/// that frame composed onto its logical screen (`-coalesce`).
///
/// The pipeline uses only the first frame. Without `[0]` ImageMagick keeps
/// every frame of an animation and transforms each on its own, so a resize
/// of a GIF whose frames have different sizes scales each one differently.
/// A GIF frame can sit at an offset on a larger screen; the built-in driver
/// composes it onto that screen, and `-coalesce` does the same. Other formats
/// keep their own pixels, as the built-in driver reads them.
fn first_frame_input(source: Source) -> Vec<String> {
    let mut args = vec![format!("{}[0]", input_spec(source))];
    if source == Source::Read(sniff::InputFormat::Gif) {
        args.push("-coalesce".into());
    }
    args
}

/// Full argv (after the binary) for a pipeline with no custom step.
#[cfg(test)]
fn process_args(
    pipeline: &ImagePipeline,
    config: &ImageConfig,
    source: Source,
    target: OutputFormat,
    orienting: Orienting,
) -> Vec<String> {
    let run = Run {
        config,
        source,
        target,
        orienting,
        drops_profile: false,
    };
    let mut applied = false;
    let mut args = run.args(true, &pipeline.transformations, &mut applied);
    args.extend(output_args(
        pipeline,
        target,
        ColourPlan::default(),
        orienting.keeps_exif(applied),
        flatten_background(pipeline, target),
    ));
    args
}

/// The settings that end the last run: flatten for a format without
/// alpha, keep only the metadata IMG-002 keeps, hold the pixels in the
/// profile's colour space, set the quality, and write the output to stdout.
///
/// Every one is a fixed string or the driver's formatting of a value.
fn output_args(
    pipeline: &ImagePipeline,
    target: OutputFormat,
    plan: ColourPlan,
    keeps_exif: bool,
    background: Color,
) -> Vec<String> {
    let mut args = Vec::new();
    if metadata::flattens(target) {
        args.push("-background".into());
        args.push(background.to_hex());
        args.push("-alpha".into());
        args.push("remove".into());
        args.push("-alpha".into());
        args.push("off".into());
    }
    // ImageMagick keeps every profile it read unless told otherwise. The
    // EXIF stays only while orientation is unapplied in a format the
    // framework cannot read (see `Orienting::keeps_exif`); the driver
    // strips it and everything else it writes, and adds the tag alone.
    args.push("+profile".into());
    args.push(if keeps_exif { "!icc,!exif,*" } else { "!icc,*" }.into());
    if plan.jpeg_true_colour && target == OutputFormat::Jpeg {
        args.push("-type".into());
        args.push("TrueColor".into());
    }
    if let (Some(colour_type), OutputFormat::Png) = (plan.png_colour_type, target) {
        args.push("-define".into());
        args.push(format!("png:color-type={colour_type}"));
    }
    args.push("-quality".into());
    args.push(pipeline.quality.to_string());
    // Both WebP variants write through the one `webp` coder, and this
    // coder option chooses between them. It is given for the lossy variant
    // too, because ImageMagick switches to lossless by itself at a quality
    // of 100.
    match target {
        OutputFormat::WebP => {
            args.push("-define".into());
            args.push("webp:lossless=false".into());
        }
        OutputFormat::WebPLossless => {
            args.push("-define".into());
            args.push("webp:lossless=true".into());
        }
        OutputFormat::Jpeg | OutputFormat::Png | OutputFormat::Gif | OutputFormat::Bmp => {}
    }
    // `format:-` forces the output encoder and writes to stdout.
    args.push(format!("{}:-", target.extension()));
    args
}

/// Full argv for a dimensions probe.
///
/// The probe reads the first frame only (`[0]`). `-format` writes no
/// separator between frames, so probing a whole animation prints
/// `100 100100 100` for two 100x100 frames, and the second number reads as a
/// height of 100100.
///
/// A GIF reports its logical screen (`%W %H`), not its first frame's own
/// size (`%w %h`): a GIF frame can be smaller than the screen it is placed
/// on, and the built-in driver composes the frame onto that screen, so both
/// drivers answer with the screen. Every other format reports its pixels,
/// because a page offset stored in, say, a PNG is not part of the image the
/// built-in driver decodes.
fn dimensions_args(config: &ImageConfig, source: Source) -> Vec<String> {
    let mut args = vec!["identify".to_string()];
    args.extend(limit_args(config));
    args.push("-format".into());
    args.push(match source {
        Source::Read(sniff::InputFormat::Gif) => "%W %H".into(),
        _ => "%w %h".into(),
    });
    args.push(format!("{}[0]", input_spec(source)));
    args
}

/// Full argv for an orientation probe of a format the framework cannot
/// read: ImageMagick's own reading of the first frame's tag, which is what
/// `-auto-orient` applies.
fn orientation_args(config: &ImageConfig, source: Source) -> Vec<String> {
    let mut args = vec!["identify".to_string()];
    args.extend(limit_args(config));
    args.push("-format".into());
    args.push("%[orientation]".into());
    args.push(format!("{}[0]", input_spec(source)));
    args
}

/// The orientation an orientation probe printed, by ImageMagick's names
/// for the EXIF values 1 to 8. `Undefined`, or anything else, is none.
fn parse_orientation(raw: &str) -> Option<Orientation> {
    let tag = match raw.trim() {
        "TopLeft" => 1,
        "TopRight" => 2,
        "BottomRight" => 3,
        "BottomLeft" => 4,
        "LeftTop" => 5,
        "RightTop" => 6,
        "RightBottom" => 7,
        "LeftBottom" => 8,
        _ => return None,
    };
    Orientation::from_tag(tag)
}

/// Full argv for an average-colour probe.
///
/// Alpha is switched off *before* the downscale so a transparent image's
/// colour is not blended toward the background, matching the pure-Rust
/// driver and Laravel, both of which drop alpha rather than weighting by it.
fn dominant_color_args(config: &ImageConfig, source: Source) -> Vec<String> {
    let mut args = limit_args(config);
    args.extend(first_frame_input(source));
    args.push("-alpha".into());
    args.push("off".into());
    args.push("-resize".into());
    args.push("1x1!".into());
    args.push("-depth".into());
    args.push("8".into());
    // The `txt:` encoder's pixel enumeration is stable across IM versions
    // and always includes a `#RRGGBB` token.
    args.push("txt:-".into());
    args
}

/// Read `reader` to its end into a buffer that keeps `room` bytes of
/// capacity past what it read (MEM-003).
///
/// `read_to_end` can end with no capacity to spare, and then the metadata
/// the driver inserts afterwards grows the buffer, which copies the whole
/// output. Here the buffer grows by doubling as `read_to_end` grows one, but
/// every read stops `room` bytes short of its capacity, so the insert moves
/// bytes within it. A read error ends the output where it stopped, as it
/// does in `read_to_end`; the exit status is what judges the run.
fn read_leaving_room(reader: &mut impl Read, room: usize) -> Vec<u8> {
    let mut buffer = Vec::new();
    loop {
        if buffer.capacity() - buffer.len() <= room {
            buffer.reserve(room + MIN_OUTPUT_READ);
        }
        let limit = buffer.capacity() - buffer.len() - room;
        // `Take` reads into the capacity already there and never grows the
        // buffer: only a read that filled all of it can have more after it.
        match reader.by_ref().take(limit as u64).read_to_end(&mut buffer) {
            Ok(read) if read == limit => {}
            _ => return buffer,
        }
    }
}

/// Reduce ImageMagick's stderr to the part a caller should see.
///
/// IM appends its own build detail to every message - the source file and line
/// that raised it (`... @ error/constitute.c/ReadImage/741`), and for a policy
/// rejection the path of the `policy.xml` that did it. That is host
/// configuration leaking into a 4xx body, so keep the first line and cut at
/// the `@ error/` marker.
fn sanitise_stderr(stderr: &str) -> Option<String> {
    let first = stderr.lines().find(|line| !line.trim().is_empty())?;
    let trimmed = first.split(" @ error/").next().unwrap_or(first).trim();
    let trimmed = trimmed
        .split(" @ warning/")
        .next()
        .unwrap_or(trimmed)
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    // A defensive ceiling: a coder is free to emit a very long single line.
    const MAX: usize = 200;
    if trimmed.len() <= MAX {
        return Some(trimmed.to_string());
    }
    let mut cut = MAX;
    while cut > 0 && !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    Some(format!("{}...", &trimmed[..cut]))
}

fn parse_dimensions(raw: &str) -> Result<(u32, u32), FrameworkError> {
    let malformed = || {
        FrameworkError::internal(format!(
            "image: could not read dimensions from ImageMagick output {raw:?}"
        ))
    };
    let mut parts = raw.split_whitespace();
    let width = parts.next().ok_or_else(malformed)?;
    let height = parts.next().ok_or_else(malformed)?;
    // More values mean more than one frame's output ran together; the first
    // two would then be a wrong answer rather than an error.
    if parts.next().is_some() {
        return Err(malformed());
    }
    Ok((
        width.parse().map_err(|_| malformed())?,
        height.parse().map_err(|_| malformed())?,
    ))
}

/// Pull the `#RRGGBB` token out of a `txt:` pixel enumeration.
///
/// At depth 8 an opaque pixel renders as `#RRGGBB` and one with an alpha
/// channel as `#RRGGBBAA`; taking the first six digits drops alpha either
/// way.
fn parse_hex_pixel(raw: &str) -> Result<String, FrameworkError> {
    for token in raw.split_whitespace() {
        let Some(hex) = token.strip_prefix('#') else {
            continue;
        };
        if hex.len() >= 6 && hex.as_bytes()[..6].iter().all(u8::is_ascii_hexdigit) {
            return Ok(format!("#{}", hex[..6].to_ascii_lowercase()));
        }
    }
    Err(FrameworkError::internal(format!(
        "image: could not read a pixel colour from ImageMagick output {raw:?}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How a pipeline orients a source whose tag the framework read as
    /// none.
    fn untagged(source: Source) -> Orienting {
        Orienting {
            known: source.read().is_some(),
            tag: None,
        }
    }

    /// A format the framework names but does not read.
    const HEIC: Source = Source::Delegated(sniff::DelegatedFormat::Heic);

    /// The configure path the tests of the process plumbing start their
    /// stand-ins with. They call [`MagickCliDriver::execute`], below the
    /// policy and its check: `sh`, `sleep`, `true` and `cat` are not
    /// ImageMagick and would fail its `-list policy`.
    fn no_configure_path() -> &'static OsStr {
        OsStr::new("")
    }

    fn config() -> ImageConfig {
        ImageConfig {
            max_dimension: 4096,
            max_alloc_bytes: 1024,
            magick_timeout_secs: 30,
            auto_orient: true,
            magick_autodetect: false,
        }
    }

    fn limits() -> Vec<String> {
        vec![
            "-limit", "time", "30", "-limit", "width", "4096", "-limit", "height", "4096",
            "-limit", "area", "1024", "-limit", "memory", "1024", "-limit", "map", "1024",
            "-limit", "disk", "0",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    #[test]
    fn limits_are_derived_from_the_image_config() {
        assert_eq!(limit_args(&config()), limits());
    }

    #[test]
    fn the_pixel_cache_cannot_escape_to_disk() {
        let args = limit_args(&config());
        let disk = args.iter().position(|a| a == "disk").expect("disk limit");
        assert_eq!(
            args[disk + 1],
            "0",
            "a non-zero disk limit lets IM spill past the memory cap"
        );
    }

    #[test]
    fn a_full_pipeline_builds_the_exact_expected_argv() {
        let pipeline = ImagePipeline {
            transformations: vec![
                Transformation::Resize {
                    width: 800,
                    height: 600,
                },
                Transformation::Grayscale,
            ],
            format: Some(OutputFormat::WebP),
            quality: 65,
            settings: TransformationSettings::default(),
        };
        let mut expected = limits();
        expected.extend(
            [
                // The input coder is pinned, not sniffed by ImageMagick, and
                // only the first frame is read.
                "png:-[0]",
                // No orientation to apply: the framework read no tag.
                "-resize",
                "800x600!",
                "-colorspace",
                "Gray",
                // Only the ICC profile survives; the driver strips the rest.
                "+profile",
                "!icc,*",
                "-quality",
                "65",
                // Lossy at every quality: ImageMagick goes lossless at 100.
                "-define",
                "webp:lossless=false",
                "webp:-",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(
            process_args(
                &pipeline,
                &config(),
                Source::Read(sniff::InputFormat::Png),
                OutputFormat::WebP,
                untagged(Source::Read(sniff::InputFormat::Png))
            ),
            expected
        );
    }

    #[test]
    fn webp_lossless_switches_the_coder_to_lossless_mode() {
        let pipeline = ImagePipeline {
            transformations: Vec::new(),
            format: Some(OutputFormat::WebPLossless),
            quality: 65,
            settings: TransformationSettings::default(),
        };
        let lossless = process_args(
            &pipeline,
            &config(),
            Source::Read(sniff::InputFormat::Png),
            OutputFormat::WebPLossless,
            untagged(Source::Read(sniff::InputFormat::Png)),
        );
        assert_eq!(
            lossless[lossless.len() - 5..],
            ["-quality", "65", "-define", "webp:lossless=true", "webp:-"]
        );

        // Lossy WebP hands the quality to the same coder and says that it
        // is lossy, at a quality of 100 too.
        let lossy = process_args(
            &pipeline,
            &config(),
            Source::Read(sniff::InputFormat::Png),
            OutputFormat::WebP,
            untagged(Source::Read(sniff::InputFormat::Png)),
        );
        assert_eq!(
            lossy[lossy.len() - 5..],
            ["-quality", "65", "-define", "webp:lossless=false", "webp:-"]
        );

        // No other format gets the option of the WebP coder.
        let jpeg = process_args(
            &pipeline,
            &config(),
            Source::Read(sniff::InputFormat::Png),
            OutputFormat::Jpeg,
            untagged(Source::Read(sniff::InputFormat::Png)),
        );
        assert!(!jpeg.contains(&"-define".to_string()));
    }

    #[test]
    fn geometry_suffixes_carry_the_resize_semantics() {
        // `!` forces exact dimensions, ignoring aspect ratio.
        assert_eq!(
            transformation_args(
                Transformation::Resize {
                    width: 10,
                    height: 20
                },
                OutputFormat::Png
            ),
            vec!["-resize", "10x20!"]
        );
        // `>` shrinks only - this is what makes `scale` never enlarge.
        assert_eq!(
            transformation_args(
                Transformation::Scale {
                    width: 10,
                    height: 20
                },
                OutputFormat::Png
            ),
            vec!["-resize", "10x20>"]
        );
        assert_eq!(
            transformation_args(Transformation::ScaleWidth(10), OutputFormat::Png),
            vec!["-resize", "10x>"]
        );
        assert_eq!(
            transformation_args(Transformation::ScaleHeight(20), OutputFormat::Png),
            vec!["-resize", "x20>"]
        );
        // A bare geometry fits inside the box, preserving aspect ratio.
        assert_eq!(
            transformation_args(
                Transformation::Contain {
                    width: 10,
                    height: 20
                },
                OutputFormat::Png
            ),
            vec!["-resize", "10x20"]
        );
        // A single dimension lets IM derive the other.
        assert_eq!(
            transformation_args(Transformation::ResizeWidth(10), OutputFormat::Png),
            vec!["-resize", "10x"]
        );
        assert_eq!(
            transformation_args(Transformation::ResizeHeight(20), OutputFormat::Png),
            vec!["-resize", "x20"]
        );
    }

    #[test]
    fn cover_fills_then_crops_from_the_centre() {
        assert_eq!(
            transformation_args(
                Transformation::Cover {
                    width: 64,
                    height: 64
                },
                OutputFormat::Png
            ),
            vec![
                "-resize", "64x64^", "-gravity", "center", "-extent", "64x64", "+repage"
            ]
        );
    }

    #[test]
    fn crop_and_rotate_reset_the_virtual_canvas() {
        let crop = transformation_args(
            Transformation::Crop {
                width: 4,
                height: 3,
                x: 2,
                y: 1,
            },
            OutputFormat::Png,
        );
        assert_eq!(crop, vec!["-crop", "4x3+2+1", "+repage"]);

        let rotate = transformation_args(
            Transformation::Rotate {
                degrees: 45.0,
                background: None,
            },
            OutputFormat::Png,
        );
        assert_eq!(
            rotate,
            vec!["-background", "#00000000", "-rotate", "45", "+repage"],
            "a transparent background keeps rotation from painting corners black"
        );
    }

    #[test]
    fn flips_map_to_their_imagemagick_names() {
        assert_eq!(
            transformation_args(Transformation::FlipVertically, OutputFormat::Png),
            vec!["-flip"]
        );
        assert_eq!(
            transformation_args(Transformation::FlipHorizontally, OutputFormat::Png),
            vec!["-flop"]
        );
    }

    #[test]
    fn zero_strength_blur_and_sharpen_emit_no_arguments() {
        assert!(transformation_args(Transformation::Blur(0), OutputFormat::Png).is_empty());
        assert!(transformation_args(Transformation::Sharpen(0), OutputFormat::Png).is_empty());
    }

    #[test]
    fn blur_and_sharpen_match_the_pure_rust_driver_scale() {
        assert_eq!(blur_sigma(0), None);
        assert_eq!(blur_sigma(100), Some(7.5));
        assert_eq!(sharpen_amount(50), Some(1.0));
        assert_eq!(sharpen_amount(100), Some(2.0));
        assert_eq!(
            transformation_args(Transformation::Blur(100), OutputFormat::Png),
            vec!["-blur", "0x7.5"]
        );
        assert_eq!(
            transformation_args(Transformation::Sharpen(50), OutputFormat::Png),
            vec!["-unsharp", "0x1+1+0"]
        );
    }

    #[test]
    fn every_argument_is_its_own_array_element() {
        // The whole safety argument rests on this: nothing that reaches
        // Command::args may contain a shell metacharacter boundary, because
        // each element is passed as one argv entry and never parsed.
        let pipeline = ImagePipeline {
            transformations: vec![
                Transformation::Crop {
                    width: 1,
                    height: 2,
                    x: 3,
                    y: 4,
                },
                Transformation::Rotate {
                    degrees: -33.5,
                    background: None,
                },
            ],
            format: Some(OutputFormat::Jpeg),
            quality: 70,
            settings: TransformationSettings::default(),
        };
        for arg in process_args(
            &pipeline,
            &config(),
            Source::Read(sniff::InputFormat::Jpeg),
            OutputFormat::Jpeg,
            untagged(Source::Read(sniff::InputFormat::Jpeg)),
        ) {
            assert!(
                !arg.contains(';')
                    && !arg.contains('|')
                    && !arg.contains('&')
                    && !arg.contains(' '),
                "argument {arg:?} would be ambiguous if it ever reached a shell"
            );
        }
    }

    #[test]
    fn dimensions_probe_uses_the_identify_subcommand() {
        let args = dimensions_args(&config(), Source::Read(sniff::InputFormat::Png));
        assert_eq!(args[0], "identify");
        assert_eq!(args[args.len() - 3..], ["-format", "%w %h", "png:-[0]"]);
        assert!(args.contains(&"-limit".to_string()));
    }

    #[test]
    fn a_gif_probe_reports_the_logical_screen() {
        // A first frame can be smaller than the screen it sits on; the
        // built-in driver composes it onto the screen and reports that.
        let args = dimensions_args(&config(), Source::Read(sniff::InputFormat::Gif));
        assert_eq!(args[args.len() - 3..], ["-format", "%W %H", "gif:-[0]"]);
        let heic = dimensions_args(&config(), HEIC);
        assert_eq!(heic[heic.len() - 3..], ["-format", "%w %h", "heic:-[0]"]);
    }

    #[test]
    fn dimensions_probe_reads_only_the_first_frame() {
        // `-format` writes no separator between frames, so an animation probed
        // whole prints `100 100100 100`. `[0]` selects the first frame, for a
        // named coder and for the autodetection opt-in's bare stdin alike.
        let named = dimensions_args(&config(), Source::Read(sniff::InputFormat::Gif));
        assert_eq!(named[named.len() - 1], "gif:-[0]");
        let bare = dimensions_args(&config(), Source::Autodetected);
        assert_eq!(bare[bare.len() - 1], "-[0]");
    }

    #[test]
    fn dominant_color_probe_drops_alpha_before_downscaling() {
        let args = dominant_color_args(&config(), Source::Read(sniff::InputFormat::Bmp));
        let alpha = args.iter().position(|a| a == "-alpha").expect("-alpha");
        let resize = args.iter().position(|a| a == "-resize").expect("-resize");
        assert!(
            alpha < resize,
            "alpha must be off before the 1x1 downscale or it weights the average"
        );
        assert_eq!(args[args.len() - 1], "txt:-");
    }

    #[test]
    fn dimensions_parse_from_identify_output() {
        assert_eq!(parse_dimensions("640 480").expect("dims"), (640, 480));
        assert_eq!(parse_dimensions(" 12 34 \n").expect("dims"), (12, 34));
        assert!(parse_dimensions("").is_err());
        assert!(parse_dimensions("640").is_err());
        assert!(parse_dimensions("wide tall").is_err());
    }

    #[test]
    fn dimensions_output_with_more_than_one_frame_is_an_error() {
        // Two frames' output run together. Reading the first two values would
        // report a height of 100100; refusing makes the probe fail loudly
        // instead.
        assert!(parse_dimensions("100 100100 100").is_err());
        assert!(parse_dimensions("100 100 100 100").is_err());
    }

    #[test]
    fn hex_pixel_parses_from_a_txt_enumeration() {
        let opaque = "# ImageMagick pixel enumeration: 1,1,255,srgb\n\
                      0,0: (255,0,0)  #FF0000  srgb(255,0,0)\n";
        assert_eq!(parse_hex_pixel(opaque).expect("colour"), "#ff0000");

        // With an alpha channel IM writes eight digits; alpha is dropped.
        let with_alpha = "0,0: (18,52,86,128)  #12345680  srgba(18,52,86,0.5)\n";
        assert_eq!(parse_hex_pixel(with_alpha).expect("colour"), "#123456");

        assert!(parse_hex_pixel("no pixels here").is_err());
        assert!(parse_hex_pixel("#xyz").is_err());
    }

    #[test]
    fn a_missing_binary_names_the_env_var_and_the_alternative() {
        let driver = MagickCliDriver::new("suprnova-no-such-binary-exists");
        let err = driver
            .run(&["-".to_string()], b"irrelevant", &config())
            .expect_err("binary is absent");
        let message = err.to_string();
        assert!(message.contains("IMAGE_MAGICK_BINARY"), "got: {message}");
        assert!(message.contains("oxideav"), "got: {message}");
    }

    #[test]
    fn a_recognised_input_pins_the_decoder_instead_of_letting_im_choose() {
        // The ImageTragick shape: a bare `-` lets ImageMagick pick the coder
        // from the bytes, so a file whose magic says MVG or MSL is executed as
        // a script no matter what the app thought it accepted. Naming the
        // coder makes the decode fail instead of becoming something else.
        for (format, expected) in [
            (sniff::InputFormat::Png, "png:-"),
            (sniff::InputFormat::Jpeg, "jpeg:-"),
            (sniff::InputFormat::WebP, "webp:-"),
            (sniff::InputFormat::Gif, "gif:-"),
            (sniff::InputFormat::Bmp, "bmp:-"),
        ] {
            assert_eq!(input_spec(Source::Read(format)), expected);
        }
    }

    #[test]
    fn hardening_a_format_only_imagemagick_reads_pins_its_coder_too() {
        // Reading these is the driver's purpose, and naming them is what
        // keeps the bytes of an upload from choosing the coder.
        for (format, expected) in [
            (sniff::DelegatedFormat::Heic, "heic:-"),
            (sniff::DelegatedFormat::Avif, "avif:-"),
            (sniff::DelegatedFormat::Tiff, "tiff:-"),
            (sniff::DelegatedFormat::Psd, "psd:-"),
            (sniff::DelegatedFormat::Psb, "psb:-"),
        ] {
            assert_eq!(input_spec(Source::Delegated(format)), expected);
        }
    }

    #[test]
    fn hardening_bare_stdin_is_the_autodetection_opt_in_alone() {
        // The one input argument that lets ImageMagick choose the coder,
        // which the framework's policy then has to cover.
        assert_eq!(input_spec(Source::Autodetected), "-");
    }

    #[test]
    fn hardening_the_guard_refuses_unnamed_bytes_unless_opted_in() {
        let mvg = b"push graphic-context\nviewbox 0 0 1 1\npop graphic-context\n";
        let err = MagickCliDriver::guard(mvg, &config()).expect_err("unnamed bytes are refused");
        assert!(
            err.to_string().contains("not one Suprnova names"),
            "got: {err}"
        );
        assert!(
            err.to_string().contains("IMAGE_MAGICK_AUTODETECT"),
            "got: {err}"
        );
        // An empty input stays the empty-input error, opted in or not.
        let opted_in = config().magick_autodetect(true);
        assert!(MagickCliDriver::guard(b"", &opted_in).is_err());
        assert_eq!(
            MagickCliDriver::guard(mvg, &opted_in).expect("opted in"),
            Source::Autodetected
        );
        // A named format keeps its coder with the opt-in on.
        let heic = b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00";
        for config in [config(), opted_in] {
            assert_eq!(MagickCliDriver::guard(heic, &config).expect("named"), HEIC);
        }
        // A format the framework reads still meets its header caps first.
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x20\x00\x00\x00\x20\x00";
        let capped = MagickCliDriver::guard(png, &opted_in).expect_err("8192x8192 is over 4096");
        assert!(capped.to_string().contains("limit"), "got: {capped}");
    }

    #[test]
    fn every_probe_pins_the_coder_when_the_format_is_known() {
        let dims = dimensions_args(&config(), Source::Read(sniff::InputFormat::Png));
        assert_eq!(dims[dims.len() - 1], "png:-[0]");
        let colour = dominant_color_args(&config(), Source::Read(sniff::InputFormat::Jpeg));
        assert!(colour.contains(&"jpeg:-[0]".to_string()));
        let tiff = Source::Delegated(sniff::DelegatedFormat::Tiff);
        let orientation = orientation_args(&config(), tiff);
        assert_eq!(orientation[orientation.len() - 1], "tiff:-[0]");
        let colour = dominant_color_args(&config(), tiff);
        assert!(colour.contains(&"tiff:-[0]".to_string()));
    }

    #[test]
    fn a_gif_is_read_as_its_first_frame_on_its_screen() {
        // The built-in driver decodes the first frame composed onto the
        // logical screen; `-coalesce` composes, `[0]` drops the rest.
        let pipeline = ImagePipeline::default();
        let gif = process_args(
            &pipeline,
            &config(),
            Source::Read(sniff::InputFormat::Gif),
            OutputFormat::Gif,
            untagged(Source::Read(sniff::InputFormat::Gif)),
        );
        let input = gif
            .iter()
            .position(|arg| arg == "gif:-[0]")
            .expect("the input");
        assert_eq!(gif[input + 1], "-coalesce");
        let colour = dominant_color_args(&config(), Source::Read(sniff::InputFormat::Gif));
        assert!(
            colour
                .windows(2)
                .any(|pair| pair == ["gif:-[0]", "-coalesce"])
        );
        // Other formats keep their own pixels: no composing onto a page.
        let png = process_args(
            &pipeline,
            &config(),
            Source::Read(sniff::InputFormat::Png),
            OutputFormat::Png,
            untagged(Source::Read(sniff::InputFormat::Png)),
        );
        assert!(!png.contains(&"-coalesce".to_string()));
        let heic = process_args(
            &pipeline,
            &config(),
            HEIC,
            OutputFormat::Png,
            untagged(HEIC),
        );
        assert!(heic.contains(&"heic:-[0]".to_string()));
        assert!(!heic.contains(&"-coalesce".to_string()));
    }

    #[test]
    fn stderr_is_reduced_to_the_part_a_caller_should_see() {
        // IM appends the source file and line that raised the error, and for a
        // policy rejection the path of the policy.xml. Neither belongs in a
        // 4xx body.
        let coder = "magick: no decode delegate for this image format `HEIC' \
                     @ error/constitute.c/ReadImage/741";
        assert_eq!(
            sanitise_stderr(coder).expect("a message"),
            "magick: no decode delegate for this image format `HEIC'"
        );

        let policy = "magick: attempt to perform an operation not allowed by the security \
                      policy `MVG' @ error/policy.c/IsRightsAuthorized/574";
        let cleaned = sanitise_stderr(policy).expect("a message");
        assert!(!cleaned.contains("policy.c"), "got: {cleaned}");
        assert!(!cleaned.contains('@'), "got: {cleaned}");

        // Only the first line survives a multi-line spew.
        let multi = "first problem @ error/a.c/B/1\nsecond problem @ error/c.c/D/2\n";
        assert_eq!(sanitise_stderr(multi).expect("a message"), "first problem");

        // Blank stderr yields nothing, so the caller falls back to the status.
        assert_eq!(sanitise_stderr("   \n  \n"), None);
        assert_eq!(sanitise_stderr(""), None);

        // A pathological single line is truncated on a char boundary.
        let long = format!("{} @ error/x.c/Y/1", "é".repeat(400));
        let cut = sanitise_stderr(&long).expect("a message");
        assert!(cut.len() <= 204, "length {}", cut.len());
        assert!(cut.ends_with("..."));
    }

    #[test]
    fn the_invocation_is_bounded_in_wall_clock_time() {
        // Without this a stalled delegate holds a blocking worker forever.
        let args = limit_args(&config());
        let time = args.iter().position(|a| a == "time").expect("time limit");
        assert_eq!(args[time - 1], "-limit");
        assert_eq!(args[time + 1], "30");
    }

    #[test]
    #[ignore = "spawns a shell that orphans a grandchild; waits out a real deadline (~3s)"]
    fn an_orphaned_grandchild_cannot_outlive_the_deadline() {
        // The case the deadline was written for, and the one it used to miss.
        // `sh -c 'sleep 120 & wait'` stands in for an ImageMagick delegate: it
        // inherits the pipe write ends, so `read_to_end` could not return even
        // after the shell itself was killed - and joining the readers before
        // the timeout branch meant the call was gated by the grandchild's
        // lifetime, not by the deadline. Two things fix it: the child gets its
        // own process group so the whole tree is signalled, and the timeout
        // path never waits on a reader.
        let config = ImageConfig {
            magick_timeout_secs: 1,
            ..ImageConfig::default()
        };
        let driver = MagickCliDriver::new("sh");
        let started = Instant::now();
        let err = driver
            .execute(
                &["-c".to_string(), "sleep 120 & wait".to_string()],
                b"",
                &config,
                0,
                no_configure_path(),
            )
            .expect_err("an orphaned grandchild must not hold the call open");
        let elapsed = started.elapsed();

        assert!(err.to_string().contains("timed out"), "got: {err}");
        assert!(
            elapsed < Duration::from_secs(10),
            "the call must return on the deadline, not on the grandchild: {elapsed:?}"
        );
        assert!(
            elapsed >= Duration::from_secs(1),
            "it must not fire before the configured timeout: {elapsed:?}"
        );
    }

    #[test]
    #[ignore = "spawns `sleep` and waits out a real deadline (~3s)"]
    fn a_stalled_child_is_killed_at_the_hard_deadline() {
        // `-limit time` is enforced by ImageMagick's own resource monitor, so
        // a child wedged inside a delegate before that monitor engages never
        // trips it. `sleep` stands in for exactly that: it ignores stdin,
        // writes nothing, and would otherwise hold this thread forever.
        let config = ImageConfig {
            magick_timeout_secs: 1,
            ..ImageConfig::default()
        };
        let driver = MagickCliDriver::new("sleep");
        let started = Instant::now();
        let err = driver
            .execute(&["120".to_string()], b"", &config, 0, no_configure_path())
            .expect_err("a stalled child must not hang");
        let elapsed = started.elapsed();

        let message = err.to_string();
        assert!(message.contains("timed out"), "got: {message}");
        assert!(
            message.contains("IMAGE_MAGICK_TIMEOUT_SECS"),
            "the error must name the knob that raises it: {message}"
        );
        assert!(
            elapsed < Duration::from_secs(30),
            "the deadline must fire, not wait out the child: {elapsed:?}"
        );
        assert!(
            elapsed >= Duration::from_secs(1),
            "it must not fire before the configured timeout: {elapsed:?}"
        );
    }

    /// MEM-003: the writer borrows the input, so the call waits for it. A
    /// child that exits without reading any of a 4 MiB input leaves the
    /// write blocked on a full pipe until the driver reads the rest itself.
    #[cfg(unix)]
    #[test]
    fn mem_audit_a_child_that_reads_none_of_its_input_does_not_hold_the_call() {
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let input = vec![b'x'; 4 * 1024 * 1024];
            let result = MagickCliDriver::new("true").execute(
                &[],
                &input,
                &ImageConfig::default(),
                0,
                no_configure_path(),
            );
            let _ = done.send(result);
        });
        let result = finished
            .recv_timeout(Duration::from_secs(20))
            .expect("the unread input held the call");
        let err = result.expect_err("`true` writes nothing");
        assert!(err.to_string().contains("no output"), "got: {err}");
    }

    #[test]
    #[ignore = "spawns `cat` to move real bytes through the pipes"]
    fn a_large_payload_round_trips_without_deadlocking() {
        // The regression guard for the pipe plumbing: `cat` echoes stdin to
        // stdout, so a payload far larger than a pipe buffer only completes if
        // the writer and the stdout reader are genuinely concurrent.
        let payload = vec![b'x'; 4 * 1024 * 1024];
        let driver = MagickCliDriver::new("cat");
        let out = driver
            .execute(
                &[],
                &payload,
                &ImageConfig::default(),
                0,
                no_configure_path(),
            )
            .expect("cat must echo the payload back");
        assert_eq!(out.len(), payload.len(), "every byte must survive");
    }

    #[test]
    fn img_001_orientation_is_applied_after_the_input_or_at_its_step() {
        let jpeg = Source::Read(sniff::InputFormat::Jpeg);
        let six = Orienting {
            known: true,
            tag: Orientation::from_tag(6),
        };
        let pipeline = ImagePipeline::default();
        let args = process_args(&pipeline, &config(), jpeg, OutputFormat::Png, six);
        let input = args.iter().position(|arg| arg == "jpeg:-[0]").unwrap();
        assert_eq!(
            args[input + 1..input + 4],
            ["-rotate", "90", "+repage"],
            "the tag the framework read, applied with a fixed transform"
        );
        assert!(!args.contains(&"-auto-orient".to_string()));

        // Every orientation has its fixed transform; the identity has none.
        for (tag, transform) in [
            (1, &[][..]),
            (2, &["-flop", "+repage"][..]),
            (3, &["-rotate", "180", "+repage"][..]),
            (4, &["-flip", "+repage"][..]),
            (5, &["-transpose", "+repage"][..]),
            (6, &["-rotate", "90", "+repage"][..]),
            (7, &["-transverse", "+repage"][..]),
            (8, &["-rotate", "270", "+repage"][..]),
        ] {
            let orienting = Orienting {
                known: true,
                tag: Orientation::from_tag(tag),
            };
            assert_eq!(orienting.args(), transform, "orientation {tag}");
        }
        // A format the framework cannot read leaves it to ImageMagick.
        let unknown = process_args(
            &pipeline,
            &config(),
            HEIC,
            OutputFormat::Png,
            untagged(HEIC),
        );
        assert!(unknown.contains(&"-auto-orient".to_string()));

        let mut opt_out = config();
        opt_out.auto_orient = false;
        let plain = process_args(&pipeline, &opt_out, jpeg, OutputFormat::Png, six);
        assert!(!plain.contains(&"-rotate".to_string()));
        let oriented = ImagePipeline {
            transformations: vec![Transformation::Grayscale, Transformation::Orient],
            ..ImagePipeline::default()
        };
        let args = process_args(&oriented, &opt_out, jpeg, OutputFormat::Png, six);
        let grey = args.iter().position(|arg| arg == "Gray").unwrap();
        assert_eq!(
            args[grey + 1..grey + 3],
            ["-rotate", "90"],
            "orient() applies at its place"
        );
        // An `orient()` after decode already applied it adds nothing.
        let twice = process_args(&oriented, &config(), jpeg, OutputFormat::Png, six);
        assert_eq!(twice.iter().filter(|arg| *arg == "-rotate").count(), 1);
    }

    #[test]
    fn img_002_the_arguments_that_keep_metadata_are_fixed_strings() {
        let pipeline = ImagePipeline::default();
        let applied = process_args(
            &pipeline,
            &config(),
            HEIC,
            OutputFormat::Jpeg,
            untagged(HEIC),
        );
        let profile = applied.iter().position(|arg| arg == "+profile").unwrap();
        assert_eq!(
            applied[profile + 1],
            "!icc,*",
            "only the ICC profile is kept"
        );
        let mut opt_out = config();
        opt_out.auto_orient = false;
        let kept = process_args(
            &pipeline,
            &opt_out,
            HEIC,
            OutputFormat::Jpeg,
            untagged(HEIC),
        );
        let profile = kept.iter().position(|arg| arg == "+profile").unwrap();
        assert_eq!(
            kept[profile + 1],
            "!icc,!exif,*",
            "the EXIF stays for its tag in a format the framework cannot read"
        );
        let jpeg = Source::Read(sniff::InputFormat::Jpeg);
        let known = process_args(
            &pipeline,
            &opt_out,
            jpeg,
            OutputFormat::Jpeg,
            untagged(jpeg),
        );
        let profile = known.iter().position(|arg| arg == "+profile").unwrap();
        assert_eq!(
            known[profile + 1],
            "!icc,*",
            "the framework read the tag itself and writes it back"
        );
    }

    #[test]
    fn img_004_custom_steps_split_the_pipeline_and_add_no_argument() {
        let custom = Transformation::custom("img-004-split");
        let steps = [
            Transformation::Grayscale,
            custom,
            Transformation::FlipVertically,
        ];
        let (stages, rest) = custom_stages(&steps);
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].0, [Transformation::Grayscale]);
        assert_eq!(stages[0].1, CustomTransformation::new("img-004-split"));
        assert_eq!(rest, [Transformation::FlipVertically]);
        assert!(transformation_args(custom, OutputFormat::Png).is_empty());
        assert_eq!(
            intermediate_args(false),
            ["+profile", "!icc,*", "-depth", "8", "png:-"],
            "between runs the image goes over stdout as a PNG"
        );
    }

    #[test]
    fn img_006_a_colour_reaches_magick_as_the_drivers_formatting() {
        let typed = Color::from_hex("#ABC").unwrap();
        let rotate = transformation_args(
            Transformation::Rotate {
                degrees: 30.0,
                background: Some(typed),
            },
            OutputFormat::Png,
        );
        assert_eq!(
            rotate[1], "#aabbccff",
            "the driver writes the colour from its bytes"
        );
        assert!(!rotate.iter().any(|arg| arg.contains("ABC")));

        // JPEG and GIF default to white and flatten onto the rotation's
        // background; PNG, WebP and BMP default to transparent.
        let default = |target| {
            transformation_args(
                Transformation::Rotate {
                    degrees: 30.0,
                    background: None,
                },
                target,
            )[1]
            .clone()
        };
        assert_eq!(default(OutputFormat::Jpeg), "#ffffffff");
        assert_eq!(default(OutputFormat::Gif), "#ffffffff");
        assert_eq!(default(OutputFormat::Png), "#00000000");
        let pipeline = ImagePipeline {
            transformations: vec![Transformation::Rotate {
                degrees: 30.0,
                background: Some(Color::rgb(0, 0, 255)),
            }],
            ..ImagePipeline::default()
        };
        let jpeg = process_args(
            &pipeline,
            &config(),
            HEIC,
            OutputFormat::Jpeg,
            untagged(HEIC),
        );
        assert!(
            jpeg.windows(6).any(|window| window
                == [
                    "-background",
                    "#0000ffff",
                    "-alpha",
                    "remove",
                    "-alpha",
                    "off"
                ]),
            "JPEG flattens onto the rotation's background: {jpeg:?}"
        );
        let png = process_args(
            &pipeline,
            &config(),
            HEIC,
            OutputFormat::Png,
            untagged(HEIC),
        );
        assert!(!png.contains(&"remove".to_string()), "PNG keeps its alpha");
    }

    #[test]
    fn the_binary_name_comes_from_the_environment_with_a_v7_default() {
        assert_eq!(MagickCliDriver::new("magick").binary(), "magick");
        assert_eq!(DEFAULT_BINARY, "magick", "IM6's `convert` is not accepted");
    }
}
