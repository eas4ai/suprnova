//! The `{name}` database seeder.

use suprnova::{FrameworkError, Seeder, async_trait};

/// Inserts the rows `{name}` owns. Register it in `src/bootstrap.rs` with
/// `suprnova::seed::register::<crate::{module}::{name}>();`, and
/// `suprnova db:seed {name}` runs it.
pub struct {name};

#[async_trait]
impl Seeder for {name} {
    /// The name `db:seed` finds this seeder by.
    fn name() -> &'static str {
        "{name}"
    }

    /// Insert the rows, with a model factory or `Model::create`. A seeder
    /// that runs twice inserts twice, so check for the rows first when
    /// that matters.
    async fn run() -> Result<(), FrameworkError> {
        Ok(())
    }
}
