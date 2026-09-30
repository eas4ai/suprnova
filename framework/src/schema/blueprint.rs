//! The recorder a `Schema::create` or `Schema::table` closure receives.

use sea_orm::sea_query::Expr;

use super::column::{ColumnBuilder, ColumnKind, ColumnSpec};
use super::foreign::{ForeignIdBuilder, ForeignSpec};

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
        if let Some(spec) = self.columns.get(column) {
            let index = IndexSpec {
                columns: vec![spec.name.clone()],
                unique: true,
            };
            self.commands.push(Command::AddIndex(index));
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

    /// Adds a `BIGINT` column meant to hold a foreign key. It has the type of
    /// [`id`](Blueprint::id). Call `.constrained(table)` on the result to
    /// create the key.
    pub fn foreign_id(&mut self, name: &str) -> ForeignIdBuilder<'_> {
        let column = self.push_column(name, ColumnKind::ForeignId);
        self.foreigns.push(ForeignSpec::new(name));
        let foreign = self.foreigns.len() - 1;
        self.commands.push(Command::AddForeign(foreign));
        ForeignIdBuilder::new(self, column, foreign)
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

    /// Adds an unbounded text column.
    pub fn text(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Text)
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
}
