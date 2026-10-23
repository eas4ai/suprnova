//! A model in a package whose `[package.metadata.suprnova.model]` table
//! the framework does not accept.

use suprnova::model;

#[model(table = "stamped")]
pub struct Stamped {
    pub id: i64,
}
