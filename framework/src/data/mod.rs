//! Unified data-object surface. Implements the `#[derive(Data)]`
//! companion types: a tri-state `Field<T>` for PATCH endpoints, a
//! `RequestIncludeSet` task-local + middleware for `?include=` runtime
//! lazy resolution, and a default-deny allowlist registry.
//!
//! # Validation hooks
//!
//! The derive writes a Data Object's `FormRequest` impl, so its
//! cross-field and database rules live in functions that impl calls:
//! `#[data(after_validation = "fn")]` for `validate!` rules and
//! `#[data(after_validation_async = "fn")]` for `Exists`, `Unique` and
//! other async rules. A generic struct, or one with reference or lazy
//! fields, gets no `FormRequest` impl, so it would never run a hook. It
//! compiles without one:
//!
//! ```
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(suprnova::Data)]
//! pub struct Page<T>
//! where
//!     T: Serialize + for<'de> Deserialize<'de>,
//! {
//!     pub items: Vec<T>,
//! }
//! ```
//!
//! and refuses to compile with one:
//!
//! ```compile_fail
//! use serde::{Deserialize, Serialize};
//! use suprnova::ValidationErrors;
//!
//! #[derive(suprnova::Data)]
//! #[data(after_validation = "page_rules")]
//! pub struct Page<T>
//! where
//!     T: Serialize + for<'de> Deserialize<'de>,
//! {
//!     pub items: Vec<T>,
//! }
//!
//! fn page_rules<T>(_page: &Page<T>) -> Result<(), ValidationErrors>
//! where
//!     T: Serialize + for<'de> Deserialize<'de>,
//! {
//!     Ok(())
//! }
//! ```
//!
//! # Field names
//!
//! The derive writes its own `Serialize` and `Deserialize`, and honors serde's
//! naming attributes in them: `rename_all` on the struct, and `rename`, `skip`,
//! `skip_serializing` and `skip_deserializing` on a field. Validation errors
//! are keyed by the names the input uses, nested ones included. The derive
//! refuses a serde attribute it does not apply, rather than leave it silently
//! unapplied:
//!
//! ```compile_fail
//! #[derive(suprnova::Data, suprnova::Validate)]
//! #[serde(deny_unknown_fields)]
//! pub struct Signup {
//!     pub email: String,
//! }
//! ```
//!
//! and two fields under one key:
//!
//! ```compile_fail
//! #[derive(suprnova::Data, suprnova::Validate)]
//! pub struct Signup {
//!     pub email: String,
//!     #[serde(rename = "email")]
//!     pub email_address: String,
//! }
//! ```
//!
//! A struct that compiles, for contrast:
//!
//! ```
//! #[derive(suprnova::Data, suprnova::Validate)]
//! #[serde(rename_all = "camelCase")]
//! pub struct Signup {
//!     pub email_address: String, // read and written as `emailAddress`
//!     #[serde(skip_serializing)]
//!     pub password: String,
//! }
//! ```

mod error;
mod field;
mod include_set;
#[doc(hidden)]
pub mod input_names;
mod middleware;
pub mod registry;
pub mod route_params;
mod when_loaded;

pub use error::IncludeError;
pub use field::Field;
pub use include_set::{
    REQUEST_INCLUDE_SET, RequestIncludeSet, current_include_set, scope_include_set,
    with_include_overrides,
};
pub use middleware::IncludeMiddleware;
pub use when_loaded::IsRelationLoaded;
