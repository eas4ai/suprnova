//! Integration tests for the `Concurrency` facade (PAR-189, PAR-190).
//!
//! The process driver runs each task in a console process. Here that
//! process is this test binary: `support::use_this_binary` points the
//! driver at it, with the `child::entry` test as the libtest filter, and
//! that test dispatches `concurrency:run` when its arguments ask for it.
//! The tasks every test runs are in `tasks`, registered in this binary.
//!
//! Tests that set the process driver's console command, a process-wide
//! setting, are `#[serial]`.

pub mod child;
pub mod defer;
#[cfg(unix)]
pub mod process_driver;
pub mod run;
pub mod support;
pub mod tasks;
