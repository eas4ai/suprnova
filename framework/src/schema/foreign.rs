//! Foreign key descriptions and the builders `foreign_id` and `foreign`
//! return.

use sea_orm::sea_query::{Expr, ForeignKeyAction};

use super::blueprint::Blueprint;

/// Column a foreign key references when `references` is not called.
pub(crate) const DEFAULT_REFERENCED_COLUMN: &str = "id";

/// One foreign key as the closure described it. `ref_table` stays `None`
/// until `constrained` or `references` runs; a column declared with
/// `foreign_id` and never constrained is a plain `BIGINT` column, while a key
/// declared with `foreign` must name its table.
#[derive(Debug, Clone)]
pub(crate) struct ForeignSpec {
    pub(crate) column: String,
    pub(crate) ref_table: Option<String>,
    pub(crate) ref_column: String,
    pub(crate) on_delete: Option<ForeignKeyAction>,
    pub(crate) on_update: Option<ForeignKeyAction>,
    /// The constraint name `.name(..)` gave, instead of the default.
    pub(crate) custom_name: Option<String>,
    /// Declared with `foreign(column)`: the column is not added here, and
    /// the key needs a referenced table.
    pub(crate) explicit: bool,
}

impl ForeignSpec {
    pub(crate) fn new(column: &str) -> Self {
        Self {
            column: column.to_owned(),
            ref_table: None,
            ref_column: DEFAULT_REFERENCED_COLUMN.to_owned(),
            on_delete: None,
            on_update: None,
            custom_name: None,
            explicit: false,
        }
    }

    pub(crate) fn explicit(column: &str) -> Self {
        Self {
            explicit: true,
            ..Self::new(column)
        }
    }

    /// The constraint name: the one `.name(..)` gave, or
    /// `{table}_{column}_foreign`.
    pub(crate) fn name(&self, table: &str) -> String {
        match &self.custom_name {
            Some(name) => name.clone(),
            None => format!("{table}_{}_foreign", self.column),
        }
    }
}

/// The referential-action shorthands both foreign key builders carry,
/// Laravel's `cascadeOnDelete()` and its siblings.
macro_rules! action_shorthands {
    () => {
        /// `on_delete(ForeignKeyAction::Cascade)`: deleting the referenced
        /// row deletes this one.
        pub fn cascade_on_delete(self) -> Self {
            self.on_delete(ForeignKeyAction::Cascade)
        }

        /// `on_delete(ForeignKeyAction::Restrict)`: the referenced row cannot
        /// be deleted while this one points at it.
        pub fn restrict_on_delete(self) -> Self {
            self.on_delete(ForeignKeyAction::Restrict)
        }

        /// `on_delete(ForeignKeyAction::SetNull)`: deleting the referenced
        /// row sets this column to `NULL`, so the column must be nullable.
        pub fn null_on_delete(self) -> Self {
            self.on_delete(ForeignKeyAction::SetNull)
        }

        /// `on_delete(ForeignKeyAction::NoAction)`, the database default,
        /// stated explicitly.
        pub fn no_action_on_delete(self) -> Self {
            self.on_delete(ForeignKeyAction::NoAction)
        }

        /// `on_update(ForeignKeyAction::Cascade)`: a changed referenced key
        /// is copied into this column.
        pub fn cascade_on_update(self) -> Self {
            self.on_update(ForeignKeyAction::Cascade)
        }

        /// `on_update(ForeignKeyAction::Restrict)`: the referenced key cannot
        /// change while this row points at it.
        pub fn restrict_on_update(self) -> Self {
            self.on_update(ForeignKeyAction::Restrict)
        }

        /// `on_update(ForeignKeyAction::SetNull)`: a changed referenced key
        /// sets this column to `NULL`.
        pub fn null_on_update(self) -> Self {
            self.on_update(ForeignKeyAction::SetNull)
        }

        /// `on_update(ForeignKeyAction::NoAction)`, the database default,
        /// stated explicitly.
        pub fn no_action_on_update(self) -> Self {
            self.on_update(ForeignKeyAction::NoAction)
        }
    };
}

/// Modifiers for a `foreign_id` column, including the foreign key itself.
///
/// The column has the type of `id()`, `BIGINT`, because MySQL refuses a
/// foreign key between columns of different types; `unsigned_foreign_id`
/// has the type of `unsigned_id()`. Without `constrained` or `references` no
/// key is created and the column is a plain `BIGINT`.
///
/// ```no_run
/// # use suprnova::schema::Blueprint;
/// # fn define(t: &mut Blueprint) {
/// t.foreign_id("author_id")
///     .constrained("users")
///     .cascade_on_delete();
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
    /// `{table}_{column}_foreign` unless `.name(..)` names it.
    pub fn constrained(self, table: &str) -> Self {
        self.references(table, DEFAULT_REFERENCED_COLUMN)
    }

    /// Creates a foreign key to `column` of `table`. On MySQL the referenced
    /// column must have this column's type, sign included.
    pub fn references(self, table: &str, column: &str) -> Self {
        if let Some(spec) = self.blueprint.foreign_mut(self.foreign) {
            spec.ref_table = Some(table.to_owned());
            spec.ref_column = column.to_owned();
        }
        self
    }

    /// Names the constraint, instead of `{table}_{column}_foreign`.
    pub fn name(self, name: &str) -> Self {
        if let Some(spec) = self.blueprint.foreign_mut(self.foreign) {
            spec.custom_name = Some(name.to_owned());
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

    action_shorthands!();

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

    /// Adds an index over this column alone, named `{table}_{column}_index`.
    /// MySQL indexes a foreign key column by itself; Postgres and SQLite do
    /// not.
    pub fn index(self) -> Self {
        self.blueprint.add_column_index(self.column);
        self
    }

    /// Places the column after `column` when `Schema::table` adds it on
    /// MySQL; ignored on Postgres and SQLite, refused by `Schema::create`.
    pub fn after(self, column: &str) -> Self {
        self.blueprint.set_after(self.column, column);
        self
    }
}

/// The foreign key `Blueprint::foreign` declares on a column that is
/// declared on its own.
pub struct ForeignBuilder<'a> {
    blueprint: &'a mut Blueprint,
    foreign: usize,
}

impl<'a> ForeignBuilder<'a> {
    pub(crate) fn new(blueprint: &'a mut Blueprint, foreign: usize) -> Self {
        Self { blueprint, foreign }
    }

    /// References the `id` column of `table`.
    pub fn constrained(self, table: &str) -> Self {
        self.references(table, DEFAULT_REFERENCED_COLUMN)
    }

    /// References `column` of `table`, Laravel's
    /// `->references($column)->on($table)`. On MySQL the referenced column
    /// must have the type of this one, sign included.
    pub fn references(self, table: &str, column: &str) -> Self {
        if let Some(spec) = self.blueprint.foreign_mut(self.foreign) {
            spec.ref_table = Some(table.to_owned());
            spec.ref_column = column.to_owned();
        }
        self
    }

    /// Names the constraint, instead of `{table}_{column}_foreign`.
    pub fn name(self, name: &str) -> Self {
        if let Some(spec) = self.blueprint.foreign_mut(self.foreign) {
            spec.custom_name = Some(name.to_owned());
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

    action_shorthands!();
}
