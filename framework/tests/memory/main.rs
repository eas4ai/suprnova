//! Heap tests for the memory-footprint commitment (MEM-001 to MEM-005):
//! each measures what one operation allocates, with dhat counting every
//! allocation in this process. Every test holds `support::exclusive`, so
//! one profiler runs at a time; the mechanism runs each test in a process
//! of its own.

// With `heap-profiling` on, the framework installs this same allocator,
// and a second `#[global_allocator]` would not link.
#[cfg(not(feature = "heap-profiling"))]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

pub mod data;
pub mod files;
pub mod http;
pub mod images;
pub mod jobs;
pub mod media;
pub mod sqs;
pub mod support;
