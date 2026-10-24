//! Scheduled Tasks
//!
//! This module contains all scheduled task definitions.
//! Use `suprnova make:task <name>` to generate new tasks.
//!
//! # Creating Tasks
//!
//! ```bash
//! suprnova make:task CleanupLogs
//! suprnova make:task SendReminders
//! ```
//!
//! # Example Task
//!
//! A task is a type that implements `Task`. The trait has one method,
//! `handle`. The name of a task and the times it runs at are set where the
//! task is registered, in `src/schedule.rs`.
//!
//! ```rust,ignore
//! // In src/tasks/my_task.rs
//! use async_trait::async_trait;
//! use suprnova::{Task, TaskResult};
//!
//! pub struct MyTask;
//!
//! impl MyTask {
//!     pub fn new() -> Self { Self }
//! }
//!
//! #[async_trait]
//! impl Task for MyTask {
//!     async fn handle(&self) -> TaskResult {
//!         println!("Task running!");
//!         Ok(())
//!     }
//! }
//!
//! // In src/schedule.rs
//! schedule.add(
//!     schedule.task(MyTask::new())
//!         .daily()
//!         .at("09:00")
//!         .name("my:task")
//! );
//! ```

// Tasks will be added here by the make:task command
