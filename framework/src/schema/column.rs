//! Column descriptions recorded by a [`Blueprint`](super::Blueprint) and the
//! builder its column methods return.

use sea_orm::DbBackend;
use sea_orm::sea_query::{ColumnDef, Expr};

use super::blueprint::Blueprint;
use super::sea_ident;

/// Length of a `string` column when `.length(n)` is not called.
pub(crate) const DEFAULT_STRING_LENGTH: u32 = 255;

/// The column types the builder can declare. It mirrors the "Column types"
/// table of the module documentation, one variant per method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnKind {
    Id,
    ForeignId,
    BigInteger,
    Integer,
    SmallInteger,
    Boolean,
    String,
    Char(u32),
    Text,
    Float,
    Double,
    Decimal(u32, u32),
    Date,
    Time,
    DateTime,
    TimestampTz,
    Json,
    Uuid,
    Ulid,
    Binary,
}

/// One column as the closure described it.
#[derive(Debug, Clone)]
pub(crate) struct ColumnSpec {
    pub(crate) name: String,
    pub(crate) kind: ColumnKind,
    pub(crate) nullable: bool,
    pub(crate) default: Option<Expr>,
    pub(crate) length: Option<u32>,
}

impl ColumnSpec {
    pub(crate) fn new(name: &str, kind: ColumnKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            nullable: false,
            default: None,
            length: None,
        }
    }

    /// Builds the SeaQuery definition for one backend. The backend only
    /// matters for the types the three databases spell differently: `json`
    /// and `uuid`.
    pub(crate) fn to_column_def(&self, backend: DbBackend) -> ColumnDef {
        let mut def = ColumnDef::new(sea_ident(&self.name));
        match self.kind {
            ColumnKind::Id => {
                def.big_integer().auto_increment().primary_key();
            }
            ColumnKind::ForeignId | ColumnKind::BigInteger => {
                def.big_integer();
            }
            ColumnKind::Integer => {
                def.integer();
            }
            ColumnKind::SmallInteger => {
                def.small_integer();
            }
            ColumnKind::Boolean => {
                def.boolean();
            }
            ColumnKind::String => {
                def.string_len(self.length.unwrap_or(DEFAULT_STRING_LENGTH));
            }
            ColumnKind::Char(length) => {
                def.char_len(length);
            }
            ColumnKind::Text => {
                def.text();
            }
            ColumnKind::Float => {
                def.float();
            }
            ColumnKind::Double => {
                def.double();
            }
            ColumnKind::Decimal(precision, scale) => {
                def.decimal_len(precision, scale);
            }
            ColumnKind::Date => {
                def.date();
            }
            ColumnKind::Time => {
                def.time();
            }
            ColumnKind::DateTime => {
                def.date_time();
            }
            ColumnKind::TimestampTz => {
                def.timestamp_with_time_zone();
            }
            ColumnKind::Json => {
                if backend == DbBackend::Postgres {
                    def.json_binary();
                } else {
                    def.json();
                }
            }
            ColumnKind::Uuid => {
                if backend == DbBackend::Postgres {
                    def.uuid();
                } else {
                    def.char_len(36);
                }
            }
            ColumnKind::Ulid => {
                def.char_len(26);
            }
            ColumnKind::Binary => {
                def.blob();
            }
        }
        if self.kind == ColumnKind::Id || !self.nullable {
            def.not_null();
        } else {
            def.null();
        }
        if let Some(default) = &self.default {
            def.default(default.clone());
        }
        def
    }
}

/// Modifiers for the column a [`Blueprint`] method just declared.
///
/// Every method takes and returns the builder, so calls chain:
/// `t.string("slug").length(120).unique();`. The builder borrows the
/// blueprint, so it cannot outlive the closure that received it.
pub struct ColumnBuilder<'a> {
    blueprint: &'a mut Blueprint,
    column: usize,
}

impl<'a> ColumnBuilder<'a> {
    pub(crate) fn new(blueprint: &'a mut Blueprint, column: usize) -> Self {
        Self { blueprint, column }
    }

    /// Allows `NULL` in the column. A column is `NOT NULL` without this call.
    pub fn nullable(self) -> Self {
        self.blueprint.set_nullable(self.column);
        self
    }

    /// Sets the value the database stores when an insert leaves the column
    /// out. It accepts a plain Rust value (`7`, `"draft"`, `true`) or a
    /// SeaQuery `Expr` such as `Expr::current_timestamp()`. On SQLite a
    /// column added to an existing table needs a constant default, because
    /// SQLite refuses `CURRENT_TIMESTAMP` there.
    pub fn default<T>(self, value: T) -> Self
    where
        T: Into<Expr>,
    {
        self.blueprint.set_default(self.column, value.into());
        self
    }

    /// Adds a unique index over this column alone, named
    /// `{table}_{column}_unique`. The index is created after the column, in
    /// its own statement.
    pub fn unique(self) -> Self {
        self.blueprint.add_column_unique(self.column);
        self
    }

    /// Sets the length of a `string` column. Any other column type makes the
    /// migration fail with an error that names the column, because a length
    /// on a `text` or an integer column would be silently ignored.
    pub fn length(self, length: u32) -> Self {
        self.blueprint.set_length(self.column, length);
        self
    }
}
