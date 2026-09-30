//! Column descriptions recorded by a [`Blueprint`](super::Blueprint) and the
//! builder its column methods return.

use sea_orm::DbBackend;
use sea_orm::sea_query::{Alias, ColumnDef, Expr};

use super::blueprint::Blueprint;
use super::{quote_ident, sea_ident};

/// Length of a `string` column when `.length(n)` is not called.
pub(crate) const DEFAULT_STRING_LENGTH: u32 = 255;

/// Length of `remember_token`, as Laravel's `rememberToken()` declares it.
pub(crate) const REMEMBER_TOKEN_LENGTH: u32 = 100;

/// The finest fractional-second precision all three databases keep:
/// microseconds.
pub(crate) const MAX_TIME_PRECISION: u32 = 6;

/// The column types the builder can declare. It mirrors the "Column types"
/// table of the module documentation, one variant per method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnKind {
    Id,
    ForeignId,
    BigInteger,
    Integer,
    SmallInteger,
    TinyInteger,
    Boolean,
    String,
    Char(u32),
    Text,
    MediumText,
    LongText,
    Enum,
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

impl ColumnKind {
    /// Whether `.unsigned()` applies: the integer types.
    pub(crate) fn is_integer(self) -> bool {
        matches!(
            self,
            Self::Id
                | Self::ForeignId
                | Self::BigInteger
                | Self::Integer
                | Self::SmallInteger
                | Self::TinyInteger
        )
    }

    /// Whether `.precision(n)` and `.use_current()` apply: the types that
    /// hold a time of day.
    pub(crate) fn is_temporal(self) -> bool {
        matches!(self, Self::DateTime | Self::TimestampTz | Self::Time)
    }
}

/// One column as the closure described it.
#[derive(Debug, Clone)]
pub(crate) struct ColumnSpec {
    pub(crate) name: String,
    pub(crate) kind: ColumnKind,
    pub(crate) nullable: bool,
    pub(crate) default: Option<Expr>,
    pub(crate) length: Option<u32>,
    /// `UNSIGNED` on MySQL; the other databases have no unsigned integers
    /// and keep the signed type, as Laravel does.
    pub(crate) unsigned: bool,
    /// Fractional-second digits of a temporal column.
    pub(crate) precision: Option<u32>,
    /// The default is the current time. Kept apart from `default` so the
    /// plan can see it: SQLite refuses it on a column added to a table.
    pub(crate) use_current: bool,
    /// The column this one follows, on MySQL only.
    pub(crate) after: Option<String>,
    /// The values an `enumeration` column accepts.
    pub(crate) allowed: Vec<String>,
}

/// `value` as a SQL string literal. A quote doubles on every backend; on
/// MySQL a backslash doubles too, because MySQL reads it as an escape
/// unless `NO_BACKSLASH_ESCAPES` is set.
fn string_literal(backend: DbBackend, value: &str) -> String {
    let mut literal = value.replace('\'', "''");
    if backend == DbBackend::MySql {
        literal = literal.replace('\\', "\\\\");
    }
    format!("'{literal}'")
}

impl ColumnSpec {
    pub(crate) fn new(name: &str, kind: ColumnKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            nullable: false,
            default: None,
            length: None,
            unsigned: false,
            precision: None,
            use_current: false,
            after: None,
            allowed: Vec::new(),
        }
    }

    /// The type of a temporal column with a precision, spelled per backend.
    /// SQLite stores these as text and has no precision to set.
    fn temporal_type(&self, backend: DbBackend, precision: u32) -> Option<String> {
        let spelled = match (backend, self.kind) {
            (DbBackend::MySql, ColumnKind::DateTime) => format!("datetime({precision})"),
            (DbBackend::MySql, ColumnKind::TimestampTz) => format!("timestamp({precision})"),
            (DbBackend::MySql, ColumnKind::Time) => format!("time({precision})"),
            (DbBackend::Postgres, ColumnKind::DateTime) => {
                format!("timestamp({precision}) without time zone")
            }
            (DbBackend::Postgres, ColumnKind::TimestampTz) => {
                format!("timestamp({precision}) with time zone")
            }
            (DbBackend::Postgres, ColumnKind::Time) => {
                format!("time({precision}) without time zone")
            }
            _ => return None,
        };
        Some(spelled)
    }

    /// Builds the SeaQuery definition for one backend. Where the databases
    /// differ the builder follows Laravel: `unsigned` and `after` apply on
    /// MySQL only, a tiny integer is a `smallint` on Postgres and an
    /// `integer` on SQLite, medium and long text are `text` off MySQL, and
    /// an enumeration off MySQL is a string with a `CHECK` on its values.
    pub(crate) fn to_column_def(&self, backend: DbBackend) -> ColumnDef {
        let mut def = ColumnDef::new(sea_ident(&self.name));
        let mysql = backend == DbBackend::MySql;
        let unsigned = mysql && self.unsigned;
        match self.kind {
            ColumnKind::Id => {
                if unsigned {
                    def.big_unsigned();
                } else {
                    def.big_integer();
                }
                def.auto_increment().primary_key();
            }
            ColumnKind::ForeignId | ColumnKind::BigInteger => {
                if unsigned {
                    def.big_unsigned();
                } else {
                    def.big_integer();
                }
            }
            ColumnKind::Integer => {
                if unsigned {
                    def.unsigned();
                } else {
                    def.integer();
                }
            }
            ColumnKind::SmallInteger => {
                if unsigned {
                    def.small_unsigned();
                } else {
                    def.small_integer();
                }
            }
            ColumnKind::TinyInteger => match backend {
                DbBackend::MySql if unsigned => {
                    def.tiny_unsigned();
                }
                DbBackend::MySql => {
                    def.tiny_integer();
                }
                DbBackend::Postgres => {
                    def.small_integer();
                }
                _ => {
                    def.integer();
                }
            },
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
            ColumnKind::MediumText if mysql => {
                def.custom(Alias::new("mediumtext"));
            }
            ColumnKind::LongText if mysql => {
                def.custom(Alias::new("longtext"));
            }
            ColumnKind::MediumText | ColumnKind::LongText => {
                def.text();
            }
            ColumnKind::Enum => {
                let values = self
                    .allowed
                    .iter()
                    .map(|value| string_literal(backend, value))
                    .collect::<Vec<_>>()
                    .join(", ");
                match backend {
                    DbBackend::MySql => {
                        def.custom(Alias::new(format!("enum({values})")));
                    }
                    other => {
                        if other == DbBackend::Postgres {
                            def.string_len(DEFAULT_STRING_LENGTH);
                        } else {
                            def.custom(Alias::new("varchar"));
                        }
                        // Spelled here, not through sea-query's value
                        // rendering, so the values carry the same escaping
                        // as the MySQL `ENUM` list.
                        let column = quote_ident(backend, &self.name);
                        def.check(Expr::cust(format!("{column} IN ({values})")));
                    }
                }
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
            ColumnKind::Time | ColumnKind::DateTime | ColumnKind::TimestampTz => {
                match self
                    .precision
                    .and_then(|precision| self.temporal_type(backend, precision))
                {
                    Some(spelled) => {
                        def.custom(Alias::new(spelled));
                    }
                    None if self.kind == ColumnKind::Time => {
                        def.time();
                    }
                    None if self.kind == ColumnKind::DateTime => {
                        def.date_time();
                    }
                    None => {
                        def.timestamp_with_time_zone();
                    }
                }
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
        if self.use_current {
            // MySQL wants the default's precision to match the column's.
            match self.precision.filter(|_| mysql) {
                Some(precision) => {
                    def.default(Expr::cust(format!("CURRENT_TIMESTAMP({precision})")));
                }
                None => {
                    def.default(Expr::current_timestamp());
                }
            }
        } else if let Some(default) = &self.default {
            def.default(default.clone());
        }
        if let Some(after) = self.after.as_deref().filter(|_| mysql) {
            def.extra(format!("AFTER {}", quote_ident(backend, after)));
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

    /// Adds a (non-unique) index over this column alone, named
    /// `{table}_{column}_index`, created after the column in its own
    /// statement.
    pub fn index(self) -> Self {
        self.blueprint.add_column_index(self.column);
        self
    }

    /// Makes this column the table's primary key. A table has one: combined
    /// with `id()` or another `primary`, the migration fails. For a key over
    /// several columns use [`Blueprint::primary`].
    pub fn primary(self) -> Self {
        self.blueprint.add_column_primary(self.column);
        self
    }

    /// Declares an integer column `UNSIGNED` on MySQL, the type Laravel's
    /// `unsignedBigInteger` and `id()` create there. Postgres and SQLite have
    /// no unsigned integers and keep the signed type, as Laravel does. On a
    /// column that is not an integer the migration fails.
    ///
    /// On MySQL a model reads an unsigned column into an unsigned field:
    /// `u64` for a `BIGINT UNSIGNED`, with `key_type = "u64"` on the model
    /// when it is the primary key.
    pub fn unsigned(self) -> Self {
        self.blueprint.set_unsigned(self.column);
        self
    }

    /// Sets the fractional-second digits of a `date_time`, `timestamp_tz` or
    /// `time` column, from 0 to 6. MySQL keeps whole seconds without it;
    /// Postgres keeps microseconds. SQLite stores these columns as text and
    /// has no precision to set. On any other column type, or above 6, the
    /// migration fails.
    pub fn precision(self, digits: u32) -> Self {
        self.blueprint.set_precision(self.column, digits);
        self
    }

    /// Defaults the column to the current time, Laravel's `useCurrent()`,
    /// for a `date_time` or `timestamp_tz` column. SQLite refuses it on a
    /// column added to an existing table, and `Schema::table` refuses it
    /// there before any statement runs.
    pub fn use_current(self) -> Self {
        self.blueprint.set_use_current(self.column);
        self
    }

    /// Places the column after `column` when `Schema::table` adds it on
    /// MySQL. Postgres and SQLite always add a column last, and ignore it,
    /// as Laravel does; `Schema::create` refuses it, because MySQL does.
    pub fn after(self, column: &str) -> Self {
        self.blueprint.set_after(self.column, column);
        self
    }
}
