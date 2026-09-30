//! The recorder a `Schema::create` or `Schema::table` closure receives.

use sea_orm::sea_query::Expr;

use super::column::{
    ColumnBuilder, ColumnKind, ColumnSpec, MAX_TIME_PRECISION, REMEMBER_TOKEN_LENGTH,
};
use super::foreign::{ForeignBuilder, ForeignIdBuilder, ForeignSpec};

/// An index the closure asked for, before it has a name.
#[derive(Debug, Clone)]
pub(crate) struct IndexSpec {
    pub(crate) columns: Vec<String>,
    pub(crate) unique: bool,
}

impl IndexSpec {
    /// The index name in Laravel's shape: `{table}_{columns}_index` and
    /// `{table}_{columns}_unique`, with the columns joined by `_`.
    pub(crate) fn name(&self, table: &str) -> String {
        let suffix = if self.unique { "unique" } else { "index" };
        format!("{table}_{}_{suffix}", self.columns.join("_"))
    }
}

/// One recorded operation. The list keeps the order of the calls, so a
/// `drop_index` recorded before a `drop_column` runs before it too.
#[derive(Debug, Clone)]
pub(crate) enum Command {
    AddColumn(usize),
    RenameColumn { from: String, to: String },
    DropColumn(String),
    AddIndex(IndexSpec),
    DropIndex(String),
    AddForeign(usize),
    DropForeign(String),
    AddPrimary(Vec<String>),
}

/// Records what a migration wants done to one table.
///
/// `Schema::create` and `Schema::table` hand a `Blueprint` to their closure.
/// The closure only records: nothing touches the database until the closure
/// returns and the schema builder has checked the whole description, so a
/// description the backend cannot run fails before its first statement.
///
/// A column method returns a builder for the modifiers of that column:
///
/// ```no_run
/// # use suprnova::schema::Blueprint;
/// # fn define(t: &mut Blueprint) {
/// t.id();
/// t.string("title");
/// t.text("body").nullable();
/// t.boolean("published").default(false);
/// t.timestamps();
/// # }
/// ```
///
/// A column is `NOT NULL` unless `.nullable()` is called.
#[derive(Debug)]
pub struct Blueprint {
    table: String,
    columns: Vec<ColumnSpec>,
    foreigns: Vec<ForeignSpec>,
    commands: Vec<Command>,
    faults: Vec<String>,
}

impl Blueprint {
    pub(crate) fn new(table: &str) -> Self {
        Self {
            table: table.to_owned(),
            columns: Vec::new(),
            foreigns: Vec::new(),
            commands: Vec::new(),
            faults: Vec::new(),
        }
    }

    pub(crate) fn table(&self) -> &str {
        &self.table
    }

    pub(crate) fn columns(&self) -> &[ColumnSpec] {
        &self.columns
    }

    pub(crate) fn foreigns(&self) -> &[ForeignSpec] {
        &self.foreigns
    }

    pub(crate) fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Descriptions of misuse the recording methods could not report, since
    /// they return no `Result`. The plan turns the first one into an error.
    pub(crate) fn faults(&self) -> &[String] {
        &self.faults
    }

    pub(crate) fn foreign_mut(&mut self, index: usize) -> Option<&mut ForeignSpec> {
        self.foreigns.get_mut(index)
    }

    pub(crate) fn set_nullable(&mut self, column: usize) {
        if let Some(spec) = self.columns.get_mut(column) {
            spec.nullable = true;
        }
    }

    pub(crate) fn set_default(&mut self, column: usize, value: Expr) {
        if let Some(spec) = self.columns.get_mut(column) {
            spec.default = Some(value);
        }
    }

    pub(crate) fn set_length(&mut self, column: usize, length: u32) {
        let Some(spec) = self.columns.get_mut(column) else {
            return;
        };
        if spec.kind == ColumnKind::String {
            spec.length = Some(length);
        } else {
            let fault = format!(
                "schema: cannot set a length on column `{}` of table `{}`: length applies to string columns only, use char(name, length) for a fixed-length column",
                spec.name, self.table
            );
            self.faults.push(fault);
        }
    }

    pub(crate) fn add_column_unique(&mut self, column: usize) {
        self.add_column_index_of(column, true);
    }

    pub(crate) fn add_column_index(&mut self, column: usize) {
        self.add_column_index_of(column, false);
    }

    fn add_column_index_of(&mut self, column: usize, unique: bool) {
        if let Some(spec) = self.columns.get(column) {
            let index = IndexSpec {
                columns: vec![spec.name.clone()],
                unique,
            };
            self.commands.push(Command::AddIndex(index));
        }
    }

    pub(crate) fn add_column_primary(&mut self, column: usize) {
        if let Some(spec) = self.columns.get(column) {
            self.commands
                .push(Command::AddPrimary(vec![spec.name.clone()]));
        }
    }

    /// Records a misuse of a modifier on `column`, which the plan returns.
    fn fault_on(&mut self, column: usize, modifier: &str, reason: &str) {
        let name = self
            .columns
            .get(column)
            .map(|spec| spec.name.clone())
            .unwrap_or_default();
        let fault = format!(
            "schema: cannot apply {modifier} to column `{name}` of table `{}`: {reason}",
            self.table
        );
        self.faults.push(fault);
    }

    pub(crate) fn set_unsigned(&mut self, column: usize) {
        match self.columns.get_mut(column) {
            Some(spec) if spec.kind.is_integer() => spec.unsigned = true,
            Some(_) => self.fault_on(column, "unsigned()", "it applies to integer columns only"),
            None => {}
        }
    }

    pub(crate) fn set_precision(&mut self, column: usize, digits: u32) {
        match self.columns.get_mut(column) {
            Some(spec) if spec.kind.is_temporal() && digits <= MAX_TIME_PRECISION => {
                spec.precision = Some(digits);
            }
            Some(spec) if spec.kind.is_temporal() => self.fault_on(
                column,
                "precision()",
                "the precision is the number of fractional-second digits, from 0 to 6",
            ),
            Some(_) => self.fault_on(
                column,
                "precision()",
                "it applies to date_time, timestamp_tz and time columns only",
            ),
            None => {}
        }
    }

    pub(crate) fn set_use_current(&mut self, column: usize) {
        match self.columns.get_mut(column) {
            Some(spec) if matches!(spec.kind, ColumnKind::DateTime | ColumnKind::TimestampTz) => {
                spec.use_current = true;
            }
            Some(_) => self.fault_on(
                column,
                "use_current()",
                "it applies to date_time and timestamp_tz columns only",
            ),
            None => {}
        }
    }

    pub(crate) fn set_after(&mut self, column: usize, after: &str) {
        if after.is_empty() {
            self.fault_on(column, "after()", "the column to follow has an empty name");
        } else if let Some(spec) = self.columns.get_mut(column) {
            spec.after = Some(after.to_owned());
        }
    }

    fn push_column(&mut self, name: &str, kind: ColumnKind) -> usize {
        self.columns.push(ColumnSpec::new(name, kind));
        let index = self.columns.len() - 1;
        self.commands.push(Command::AddColumn(index));
        index
    }

    fn column(&mut self, name: &str, kind: ColumnKind) -> ColumnBuilder<'_> {
        let index = self.push_column(name, kind);
        ColumnBuilder::new(self, index)
    }

    /// Adds `id`: `BIGINT`, auto-increment, primary key.
    pub fn id(&mut self) -> ColumnBuilder<'_> {
        self.column("id", ColumnKind::Id)
    }

    /// Adds `id` as Laravel's `id()` creates it: `BIGINT UNSIGNED`,
    /// auto-increment, primary key on MySQL. Postgres and SQLite have no
    /// unsigned integers, so there it is [`id`](Blueprint::id). A model reads
    /// the MySQL column into a `u64` key: `key_type = "u64"`.
    pub fn unsigned_id(&mut self) -> ColumnBuilder<'_> {
        self.column("id", ColumnKind::Id).unsigned()
    }

    /// Adds a `BIGINT` column meant to hold a foreign key. It has the type of
    /// [`id`](Blueprint::id). Call `.constrained(table)` on the result to
    /// create the key.
    pub fn foreign_id(&mut self, name: &str) -> ForeignIdBuilder<'_> {
        let column = self.push_column(name, ColumnKind::ForeignId);
        self.push_foreign_id(name, column)
    }

    fn push_foreign_id(&mut self, name: &str, column: usize) -> ForeignIdBuilder<'_> {
        self.foreigns.push(ForeignSpec::new(name));
        let foreign = self.foreigns.len() - 1;
        self.commands.push(Command::AddForeign(foreign));
        ForeignIdBuilder::new(self, column, foreign)
    }

    /// Adds a `BIGINT` column for a foreign key to an [`unsigned_id`]
    /// column: `BIGINT UNSIGNED` on MySQL, where a foreign key's type must
    /// match the referenced column's sign, and `BIGINT` elsewhere. It is
    /// Laravel's `foreignId`.
    ///
    /// [`unsigned_id`]: Blueprint::unsigned_id
    pub fn unsigned_foreign_id(&mut self, name: &str) -> ForeignIdBuilder<'_> {
        let column = self.push_column(name, ColumnKind::ForeignId);
        self.set_unsigned(column);
        self.push_foreign_id(name, column)
    }

    /// Adds a 64-bit integer column.
    pub fn big_integer(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::BigInteger)
    }

    /// Adds a 32-bit integer column.
    pub fn integer(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Integer)
    }

    /// Adds a 16-bit integer column.
    pub fn small_integer(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::SmallInteger)
    }

    /// Adds an 8-bit integer column: `TINYINT` on MySQL, `smallint` on
    /// Postgres and `integer` on SQLite, the types Laravel's `tinyInteger`
    /// creates.
    pub fn tiny_integer(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::TinyInteger)
    }

    /// Adds `big_integer(name).unsigned()`: `BIGINT UNSIGNED` on MySQL.
    pub fn unsigned_big_integer(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.big_integer(name).unsigned()
    }

    /// Adds `integer(name).unsigned()`: `INT UNSIGNED` on MySQL.
    pub fn unsigned_integer(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.integer(name).unsigned()
    }

    /// Adds `small_integer(name).unsigned()`: `SMALLINT UNSIGNED` on MySQL.
    pub fn unsigned_small_integer(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.small_integer(name).unsigned()
    }

    /// Adds `tiny_integer(name).unsigned()`: `TINYINT UNSIGNED` on MySQL.
    pub fn unsigned_tiny_integer(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.tiny_integer(name).unsigned()
    }

    /// Adds a boolean column.
    pub fn boolean(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Boolean)
    }

    /// Adds a `VARCHAR(255)` column. `.length(n)` changes the length.
    pub fn string(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::String)
    }

    /// Adds a fixed-length `CHAR(length)` column.
    pub fn char(&mut self, name: &str, length: u32) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Char(length))
    }

    /// Adds an unbounded text column. On MySQL `TEXT` holds 64 KB; use
    /// [`medium_text`](Blueprint::medium_text) or
    /// [`long_text`](Blueprint::long_text) for more.
    pub fn text(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Text)
    }

    /// Adds a text column that holds 16 MB on MySQL (`MEDIUMTEXT`). It is
    /// `text` on Postgres and SQLite, which have no smaller limit.
    pub fn medium_text(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::MediumText)
    }

    /// Adds a text column that holds 4 GB on MySQL (`LONGTEXT`). It is
    /// `text` on Postgres and SQLite.
    pub fn long_text(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::LongText)
    }

    /// Adds a column that accepts only `values`, Laravel's `enum`: `ENUM`
    /// on MySQL, and on Postgres and SQLite a string column with a `CHECK`
    /// that the value is one of them. The builder quotes each value, so it
    /// may contain a quote. The migration fails on an empty list, a value
    /// listed twice (MySQL refuses it), and a value with a backslash, whose
    /// meaning depends on the server's settings.
    ///
    /// ```no_run
    /// # use suprnova::schema::Blueprint;
    /// # fn define(t: &mut Blueprint) {
    /// t.enumeration("status", &["draft", "published"]).default("draft");
    /// # }
    /// ```
    pub fn enumeration(&mut self, name: &str, values: &[&str]) -> ColumnBuilder<'_> {
        let index = self.push_column(name, ColumnKind::Enum);
        // MySQL compares enumeration values without case and without
        // trailing spaces, so `a`, `A` and `a ` are one value to it; the
        // builder holds every backend to that, so the migration runs on all.
        let comparable = |value: &str| value.trim_end_matches(' ').to_lowercase();
        let repeated = values.iter().enumerate().find(|(position, value)| {
            values[..*position]
                .iter()
                .any(|earlier| comparable(earlier) == comparable(value))
        });
        if values.is_empty() {
            self.fault_on(index, "enumeration()", "give it at least one value");
        } else if let Some(value) = values.iter().find(|value| value.contains('\\')) {
            let reason = format!(
                "the value `{value}` holds a backslash, which MySQL and Postgres read differently depending on server settings"
            );
            self.fault_on(index, "enumeration()", &reason);
        } else if let Some((_, value)) = repeated {
            let reason = format!(
                "the value `{value}` is listed twice (MySQL ignores case and trailing spaces)"
            );
            self.fault_on(index, "enumeration()", &reason);
        } else if let Some(spec) = self.columns.get_mut(index) {
            spec.allowed = values.iter().map(|value| (*value).to_owned()).collect();
        }
        ColumnBuilder::new(self, index)
    }

    /// Adds a 32-bit floating point column.
    pub fn float(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Float)
    }

    /// Adds a 64-bit floating point column.
    pub fn double(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Double)
    }

    /// Adds an exact decimal column with `precision` digits in total, `scale`
    /// of them after the decimal point.
    pub fn decimal(&mut self, name: &str, precision: u32, scale: u32) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Decimal(precision, scale))
    }

    /// Adds a calendar date column.
    pub fn date(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Date)
    }

    /// Adds a time of day column.
    pub fn time(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Time)
    }

    /// Adds a date and time column without a time zone.
    pub fn date_time(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::DateTime)
    }

    /// Adds a date and time column with a time zone: `timestamp with time
    /// zone` on Postgres, `timestamp` on MySQL, text on SQLite.
    pub fn timestamp_tz(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::TimestampTz)
    }

    /// Adds a JSON column: `jsonb` on Postgres, `JSON` on MySQL, text on
    /// SQLite.
    pub fn json(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Json)
    }

    /// Adds a UUID column: `uuid` on Postgres, `CHAR(36)` on MySQL and
    /// SQLite.
    pub fn uuid(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Uuid)
    }

    /// Adds a `CHAR(26)` column, the length of a ULID.
    pub fn ulid(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Ulid)
    }

    /// Adds a column of raw bytes: `bytea` on Postgres, `BLOB` elsewhere.
    pub fn binary(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Binary)
    }

    /// Adds `created_at` and `updated_at`, both `NOT NULL`, the columns a
    /// `#[suprnova::model]` writes on every insert.
    ///
    /// They are `VARCHAR(255)` columns on every backend. A model field of
    /// type `DateTime<Utc>` with no declared cast uses the `AsDateTime` cast,
    /// which stores RFC 3339 text, and Postgres refuses a text parameter for a
    /// `timestamp` column. A string column is the one type that round-trips
    /// on all three backends. For native columns use
    /// [`timestamps_tz`](Blueprint::timestamps_tz) with the
    /// `AsNativeDateTime` cast, or [`datetimes`](Blueprint::datetimes)
    /// with `AsNaiveDateTime`.
    pub fn timestamps(&mut self) {
        self.column("created_at", ColumnKind::String);
        self.column("updated_at", ColumnKind::String);
    }

    /// Adds a nullable `deleted_at`, the column a soft-deleting
    /// `#[suprnova::model]` fills when it deletes a row.
    ///
    /// It is a `VARCHAR(255)` column on every backend, for the reason given
    /// at [`timestamps`](Blueprint::timestamps): the default model cast
    /// stores RFC 3339 text. For a native column use
    /// [`soft_deletes_tz`](Blueprint::soft_deletes_tz) or
    /// [`soft_deletes_datetime`](Blueprint::soft_deletes_datetime).
    pub fn soft_deletes(&mut self) {
        self.column("deleted_at", ColumnKind::String).nullable();
    }

    /// Adds a nullable `remember_token`, `VARCHAR(100)`, the column Laravel's
    /// `rememberToken()` creates for "remember me" sign-ins.
    pub fn remember_token(&mut self) {
        self.column("remember_token", ColumnKind::String)
            .length(REMEMBER_TOKEN_LENGTH)
            .nullable();
    }

    /// Laravel's `timestampsTz`: nullable `created_at` and `updated_at`
    /// columns that keep the zone - `timestamp with time zone` on Postgres,
    /// `timestamp` on MySQL, text on SQLite.
    ///
    /// The model's fields need the native cast, because a `DateTime<Utc>`
    /// field defaults to RFC 3339 text, which Postgres refuses for these
    /// columns: `casts = { created_at = AsNativeDateTime, updated_at =
    /// AsNativeDateTime }`.
    pub fn timestamps_tz(&mut self) {
        self.column("created_at", ColumnKind::TimestampTz)
            .nullable();
        self.column("updated_at", ColumnKind::TimestampTz)
            .nullable();
    }

    /// Laravel's `datetimes`: nullable `created_at` and `updated_at`
    /// columns without a zone - `timestamp` on Postgres, `DATETIME` on
    /// MySQL, text on SQLite - holding the UTC wall clock through the
    /// `AsNaiveDateTime` cast.
    pub fn datetimes(&mut self) {
        self.column("created_at", ColumnKind::DateTime).nullable();
        self.column("updated_at", ColumnKind::DateTime).nullable();
    }

    /// Laravel's `softDeletesTz`: a nullable `deleted_at` that keeps the
    /// zone, for a model field cast with `AsOptionalNativeDateTime`.
    pub fn soft_deletes_tz(&mut self) {
        self.column("deleted_at", ColumnKind::TimestampTz)
            .nullable();
    }

    /// Laravel's `softDeletesDatetime`: a nullable `deleted_at` without a
    /// zone, for a model field cast with `AsOptionalNaiveDateTime`.
    pub fn soft_deletes_datetime(&mut self) {
        self.column("deleted_at", ColumnKind::DateTime).nullable();
    }

    /// Creates an index over `columns`, named `{table}_{columns}_index`. The
    /// columns are joined with `_` in the name.
    pub fn index(&mut self, columns: &[&str]) {
        self.push_index(columns, false);
    }

    /// Creates a unique index over `columns`, named
    /// `{table}_{columns}_unique`.
    pub fn unique(&mut self, columns: &[&str]) {
        self.push_index(columns, true);
    }

    /// Makes `columns` the table's primary key, Laravel's `primary`: a
    /// composite key for a pivot table, or a key on a column other than
    /// `id`. A table has one primary key, so combined with `id()` or another
    /// `primary` the migration fails, and so does a nullable column in it.
    ///
    /// In `Schema::create` the key is part of `CREATE TABLE`. In
    /// `Schema::table` it is added to the existing table on Postgres and
    /// MySQL, which make the key's columns `NOT NULL` and fail if a row
    /// holds `NULL` in one; SQLite cannot add a primary key to a table, and
    /// the call fails there before any statement runs.
    pub fn primary(&mut self, columns: &[&str]) {
        self.commands.push(Command::AddPrimary(
            columns.iter().map(|column| (*column).to_owned()).collect(),
        ));
    }

    /// Creates a foreign key on `column`, a column the table already has or
    /// that this closure declares, Laravel's `foreign`. Name the referenced
    /// table and column with `.references(table, column)`, or
    /// `.constrained(table)` for its `id`; without either the migration
    /// fails.
    ///
    /// ```no_run
    /// # use suprnova::schema::Blueprint;
    /// # fn define(t: &mut Blueprint) {
    /// t.foreign("state_id")
    ///     .references("states", "id")
    ///     .name("orders_state_fk")
    ///     .cascade_on_delete();
    /// # }
    /// ```
    pub fn foreign(&mut self, column: &str) -> ForeignBuilder<'_> {
        self.foreigns.push(ForeignSpec::explicit(column));
        let foreign = self.foreigns.len() - 1;
        self.commands.push(Command::AddForeign(foreign));
        ForeignBuilder::new(self, foreign)
    }

    fn push_index(&mut self, columns: &[&str], unique: bool) {
        let spec = IndexSpec {
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            unique,
        };
        self.commands.push(Command::AddIndex(spec));
    }

    /// Renames a column. Only `Schema::table` accepts it.
    pub fn rename_column(&mut self, from: &str, to: &str) {
        self.commands.push(Command::RenameColumn {
            from: from.to_owned(),
            to: to.to_owned(),
        });
    }

    /// Drops a column. Only `Schema::table` accepts it. SQLite refuses to drop
    /// a column that an index, a unique constraint or a foreign key covers:
    /// drop those first, an index earlier in the same closure.
    pub fn drop_column(&mut self, name: &str) {
        self.commands.push(Command::DropColumn(name.to_owned()));
    }

    /// Drops the index called `name`, including a unique index. Only
    /// `Schema::table` accepts it.
    pub fn drop_index(&mut self, name: &str) {
        self.commands.push(Command::DropIndex(name.to_owned()));
    }

    /// Drops the foreign key called `name`. Only `Schema::table` accepts it,
    /// and SQLite refuses it.
    pub fn drop_foreign(&mut self, name: &str) {
        self.commands.push(Command::DropForeign(name.to_owned()));
    }

    /// Drops the foreign key `{table}_{column}_foreign`, then the column,
    /// Laravel's `dropConstrainedForeignId`. A key that `.name(..)` named
    /// has another name: drop it with [`drop_foreign`](Blueprint::drop_foreign)
    /// and the column with [`drop_column`](Blueprint::drop_column). Only
    /// `Schema::table` accepts it, and SQLite refuses it, as it refuses
    /// `drop_foreign`.
    pub fn drop_constrained_foreign_id(&mut self, column: &str) {
        let name = ForeignSpec::new(column).name(&self.table);
        self.commands.push(Command::DropForeign(name));
        self.commands.push(Command::DropColumn(column.to_owned()));
    }
}
