//! Foreign key descriptions and the builder `foreign_id` returns.

use sea_orm::sea_query::{Expr, ForeignKeyAction};

use super::blueprint::Blueprint;

/// Column a foreign key references when `references` is not called.
pub(crate) const DEFAULT_REFERENCED_COLUMN: &str = "id";

/// One foreign key as the closure described it. `ref_table` stays `None`
/// until `constrained` or `references` runs; a column declared with
/// `foreign_id` and never constrained is a plain `BIGINT` column.
#[derive(Debug, Clone)]
pub(crate) struct ForeignSpec {
    pub(crate) column: String,
    pub(crate) ref_table: Option<String>,
    pub(crate) ref_column: String,
    pub(crate) on_delete: Option<ForeignKeyAction>,
    pub(crate) on_update: Option<ForeignKeyAction>,
}

impl ForeignSpec {
    pub(crate) fn new(column: &str) -> Self {
        Self {
            column: column.to_owned(),
            ref_table: None,
            ref_column: DEFAULT_REFERENCED_COLUMN.to_owned(),
            on_delete: None,
            on_update: None,
        }
    }

    /// The constraint name, `{table}_{column}_foreign`.
    pub(crate) fn name(&self, table: &str) -> String {
        format!("{table}_{}_foreign", self.column)
    }
}

/// Modifiers for a `foreign_id` column, including the foreign key itself.
///
/// The column has the type of `id()`, `BIGINT`, because MySQL refuses a
/// foreign key between columns of different types. Without `constrained` or
/// `references` no key is created and the column is a plain `BIGINT`.
///
/// ```no_run
/// # use suprnova::schema::Blueprint;
/// # use suprnova::sea_query::ForeignKeyAction;
/// # fn define(t: &mut Blueprint) {
/// t.foreign_id("author_id")
///     .constrained("users")
///     .on_delete(ForeignKeyAction::Cascade);
/// # }
/// ```
pub struct ForeignIdBuilder<'a> {
    blueprint: &'a mut Blueprint,
    column: usize,
    foreign: usize,
}

impl<'a> ForeignIdBuilder<'a> {
    pub(crate) fn new(blueprint: &'a mut Blueprint, column: usize, foreign: usize) -> Self {
        Self {
            blueprint,
            column,
            foreign,
        }
    }

    /// Creates a foreign key to the `id` column of `table`, named
    /// `{table}_{column}_foreign`.
    pub fn constrained(self, table: &str) -> Self {
        self.references(table, DEFAULT_REFERENCED_COLUMN)
    }

    /// Creates a foreign key to `column` of `table`. On MySQL the referenced
    /// column must be a `BIGINT` too, the type of this column.
    pub fn references(self, table: &str, column: &str) -> Self {
        if let Some(spec) = self.blueprint.foreign_mut(self.foreign) {
            spec.ref_table = Some(table.to_owned());
            spec.ref_column = column.to_owned();
        }
        self
    }

    /// Sets what the database does to this row when the referenced row is
    /// deleted. Without a call the database default applies (`NO ACTION`).
    pub fn on_delete(self, action: ForeignKeyAction) -> Self {
        if let Some(spec) = self.blueprint.foreign_mut(self.foreign) {
            spec.on_delete = Some(action);
        }
        self
    }

    /// Sets what the database does to this row when the referenced key is
    /// updated. Without a call the database default applies (`NO ACTION`).
    pub fn on_update(self, action: ForeignKeyAction) -> Self {
        if let Some(spec) = self.blueprint.foreign_mut(self.foreign) {
            spec.on_update = Some(action);
        }
        self
    }

    /// Allows `NULL` in the column. A column is `NOT NULL` without this call.
    pub fn nullable(self) -> Self {
        self.blueprint.set_nullable(self.column);
        self
    }

    /// Sets the value the database stores when an insert leaves the column
    /// out. It accepts a plain Rust value or a SeaQuery `Expr`. On SQLite a
    /// column added to an existing table needs a constant default.
    pub fn default<T>(self, value: T) -> Self
    where
        T: Into<Expr>,
    {
        self.blueprint.set_default(self.column, value.into());
        self
    }

    /// Adds a unique index over this column alone, named
    /// `{table}_{column}_unique`.
    pub fn unique(self) -> Self {
        self.blueprint.add_column_unique(self.column);
        self
    }
}
