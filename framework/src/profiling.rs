//! Heap profiling an administrator switches on with the `heap-profiling`
//! feature.
//!
//! With the feature, the framework installs dhat's allocator for the whole
//! process, and [`crate::main`] starts its profiler right after it loads
//! `.env`, before the runtime exists. When the process ends normally, a
//! command that finishes or a server that shuts down gracefully, the
//! profiler writes `dhat-heap.json`, or the file [`PROFILE_PATH_VAR`]
//! names, for DHAT's viewer, and prints the totals to stderr: the bytes
//! allocated, the heap at its peak and the heap still live at the end. A
//! process that calls `std::process::exit` or dies on a signal writes
//! nothing, because no destructor runs.
//!
//! Without the feature none of this compiles: [`HeapProfile`] is empty and
//! [`start`] does nothing, so an ordinary build carries no dhat code.
//!
//! The allocator is the whole process's, so an application with its own
//! `#[global_allocator]` cannot also turn this feature on: the link fails
//! with two global allocators.

/// dhat counts every allocation through this allocator; with no profiler
/// running it passes each call straight to the system allocator.
#[cfg(feature = "heap-profiling")]
#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

/// The environment variable naming the file the profile is written to.
/// Unset, the profile goes to `dhat-heap.json` in the working directory.
pub const PROFILE_PATH_VAR: &str = "SUPRNOVA_HEAP_PROFILE";

/// The running heap profile: dropping it writes the profile.
///
/// [`crate::main`] holds it for the whole of `main` and declares it before
/// the runtime, so the runtime's teardown is measured too.
#[must_use = "the profile is written when this guard drops"]
pub struct HeapProfile {
    #[cfg(feature = "heap-profiling")]
    _profiler: Option<dhat::Profiler>,
}

impl std::fmt::Debug for HeapProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeapProfile").finish_non_exhaustive()
    }
}

/// Starts the heap profiler when the `heap-profiling` feature is on.
///
/// dhat allows one profiler per process and panics on a second, so only
/// the first call starts one; any later call returns a guard that writes
/// nothing.
pub fn start() -> HeapProfile {
    #[cfg(feature = "heap-profiling")]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static STARTED: AtomicBool = AtomicBool::new(false);
        if STARTED.swap(true, Ordering::SeqCst) {
            return HeapProfile { _profiler: None };
        }
        let path = std::env::var_os(PROFILE_PATH_VAR)
            .filter(|path| !path.is_empty())
            .map_or_else(
                || std::path::PathBuf::from("dhat-heap.json"),
                std::path::PathBuf::from,
            );
        HeapProfile {
            _profiler: Some(dhat::Profiler::builder().file_name(path).build()),
        }
    }
    #[cfg(not(feature = "heap-profiling"))]
    HeapProfile {}
}
