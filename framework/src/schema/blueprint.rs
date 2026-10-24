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
    /// `{table}_{columns}_unique`, with the columns joined by `_`. A
    /// schema-qualified table's `.` becomes `_`, as Laravel's
    /// `createIndexName` makes it: the index lives in the table's schema,
    /// and its name is one identifier.
    pub(crate) fn name(&self, table: &str) -> String {
        let suffix = if self.unique { "unique" } else { "index" };
        format!(
            "{}_{}_{suffix}",
            table.replace('.', "_"),
            self.columns.join("_")
        )
    }
}

/// A full-text index the closure asked for: a `FULLTEXT` index on MySQL and
/// MariaDB, a `GIN` index over `to_tsvector` on Postgres.
#[derive(Debug, Clone)]
pub(crate) struct FullTextSpec {
    pub(crate) columns: Vec<String>,
    /// The Postgres text search configuration, `english` when unset.
    pub(crate) language: Option<String>,
}

/// The name Laravel's `createIndexName('fulltext', ..)` gives a full-text
/// index over `columns`: `{table}_{columns}_fulltext`, a schema-qualified
/// table's `.` made `_` as for any index. `drop_full_text` finds the index
/// by the same name.
pub(crate) fn full_text_name(table: &str, columns: &[String]) -> String {
    format!("{}_{}_fulltext", table.replace('.', "_"), columns.join("_"))
}

/// One recorded operation. The list keeps the order of the calls, so a
/// `drop_index` recorded before a `drop_column` runs before it too.
#[derive(Debug, Clone)]
pub(crate) enum Command {
    AddColumn(usize),
    RenameColumn {
        from: String,
        to: String,
    },
    DropColumn(String),
    AddIndex(IndexSpec),
    DropIndex(String),
    AddFullText(FullTextSpec),
    /// The columns of the full-text index to drop.
    DropFullText(Vec<String>),
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
    charset: Option<String>,
    collation: Option<String>,
}

impl Blueprint {
    pub(crate) fn new(table: &str) -> Self {
        Self {
            table: table.to_owned(),
            columns: Vec::new(),
            foreigns: Vec::new(),
            commands: Vec::new(),
            faults: Vec::new(),
            charset: None,
            collation: None,
        }
    }

    /// Previews the same validated CREATE statements a migration executes.
    /// You can check backend types without connecting to that database.
    ///
    /// On MySQL the table takes the character set and collation the
    /// blueprint names, or else the configured ones, as
    /// [`Schema::create`](super::Schema::create) gives it.
    pub fn create_sql<F>(
        table: &str,
        backend: sea_orm::DbBackend,
        define: F,
    ) -> Result<Vec<String>, sea_orm::DbErr>
    where
        F: FnOnce(&mut Self),
    {
        let mut blueprint = Self::new(table);
        define(&mut blueprint);
        super::plan::plan_create(&blueprint, backend, &super::table_encoding(backend))
            .map(|steps| steps.into_iter().map(|step| step.sql(backend)).collect())
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

    /// The character set [`Self::charset`] named, if any.
    pub(crate) fn table_charset(&self) -> Option<&str> {
        self.charset.as_deref()
    }

    /// The collation [`Self::collation`] named, if any.
    pub(crate) fn table_collation(&self) -> Option<&str> {
        self.collation.as_deref()
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
        if matches!(spec.kind, ColumnKind::String | ColumnKind::Ulid) {
            spec.length = Some(length);
        } else {
            let fault = format!(
                "schema: cannot set a length on column `{}` of table `{}`: length applies to string and ulid columns only, use char(name, length) for a fixed-length column",
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
            Some(spec) if spec.kind == ColumnKind::Float && (1..=53).contains(&digits) => {
                spec.precision = Some(digits);
            }
            Some(spec) if spec.kind == ColumnKind::Float => self.fault_on(
                column,
                "precision()",
                "float precision is the number of binary digits, from 1 to 53",
            ),
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
                "it applies to float, date_time, timestamp_tz and time columns only",
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

    /// Adds Laravel's auto-increment primary key, unsigned on MySQL by default.
    /// Use [`Schema::set_unsigned_ids`](super::Schema::set_unsigned_ids) for signed IDs.
    pub fn id(&mut self) -> ColumnBuilder<'_> {
        if super::unsigned_ids() {
            return self.unsigned_id();
        }
        self.column("id", ColumnKind::Id)
    }

    /// Adds `id` as Laravel's `id()` creates it: `BIGINT UNSIGNED`,
    /// auto-increment, primary key on MySQL. Postgres and SQLite have no
    /// unsigned integers, so there it is [`id`](Blueprint::id). A model reads
    /// the column into a `u64` key field on every database.
    pub fn unsigned_id(&mut self) -> ColumnBuilder<'_> {
        self.column("id", ColumnKind::Id).unsigned()
    }

    /// Adds a `BIGINT` column meant to hold a foreign key. It has the type of
    /// [`id`](Blueprint::id), unsigned on MySQL by default. Call
    /// `.constrained(table)` on the result to create the key.
    pub fn foreign_id(&mut self, name: &str) -> ForeignIdBuilder<'_> {
        if super::unsigned_ids() {
            return self.unsigned_foreign_id(name);
        }
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

    /// Adds Laravel's boolean storage: `tinyint(1)` on MySQL and SQLite, `boolean` on Postgres.
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

    /// Adds a floating point column with 53 binary digits so values retain double precision.
    /// Call `.precision(n)` to request fewer digits on MySQL or Postgres.
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

    /// Adds a whole-second zoned timestamp so Postgres matches Laravel's default.
    /// Storage is `timestamp(0) with time zone` on Postgres, `timestamp(0)` on
    /// MySQL, and `datetime` on SQLite. Call `.precision(n)` for fractional digits.
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

    /// Adds a `CHAR(26)` column for a standard ULID. `.length(n)` changes the length.
    pub fn ulid(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Ulid)
    }

    /// Adds a ULID column of a chosen length when your schema uses a shorter code.
    /// Rust has no optional arguments, so this is Laravel's `ulid(name, length)`.
    pub fn ulid_with_length(&mut self, name: &str, length: u32) -> ColumnBuilder<'_> {
        self.ulid(name).length(length)
    }

    /// Adds a column of raw bytes: `bytea` on Postgres, `BLOB` elsewhere.
    pub fn binary(&mut self, name: &str) -> ColumnBuilder<'_> {
        self.column(name, ColumnKind::Binary)
    }

    /// Adds nullable `created_at` and `updated_at`, the columns a
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
        self.column("created_at", ColumnKind::String).nullable();
        self.column("updated_at", ColumnKind::String).nullable();
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

    /// Creates a full-text index over `columns`, named
    /// `{table}_{columns}_fulltext`, as Laravel's `fullText`: a `FULLTEXT`
    /// index on MySQL and MariaDB, and on Postgres a `GIN` index over
    /// `to_tsvector('english', c1) || to_tsvector('english', c2)`, the
    /// expression `where_full_text` searches, so Postgres answers the search
    /// from the index. `.language(name)` names another Postgres text search
    /// configuration.
    ///
    /// SQLite has no full-text index: the migration fails there before it
    /// runs any statement, with an error that names SQLite and `full_text`.
    ///
    /// ```no_run
    /// # use suprnova::schema::Blueprint;
    /// # fn define(t: &mut Blueprint) {
    /// t.string("title");
    /// t.text("body");
    /// t.full_text(&["title", "body"]).language("english");
    /// # }
    /// ```
    pub fn full_text(&mut self, columns: &[&str]) -> FullTextIndexBuilder<'_> {
        self.commands.push(Command::AddFullText(FullTextSpec {
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            language: None,
        }));
        let command = self.commands.len() - 1;
        FullTextIndexBuilder {
            blueprint: self,
            command,
        }
    }

    /// Drops the full-text index over `columns`, the one
    /// [`full_text`](Blueprint::full_text) named `{table}_{columns}_fulltext`,
    /// as Laravel's `dropFullText` given the columns. Drop an index of
    /// another name with [`drop_index`](Blueprint::drop_index). Only
    /// `Schema::table` accepts it, and SQLite refuses it.
    pub fn drop_full_text(&mut self, columns: &[&str]) {
        self.commands.push(Command::DropFullText(
            columns.iter().map(|c| (*c).to_owned()).collect(),
        ));
    }

    /// Sets the language of the full-text index `command` recorded.
    fn set_full_text_language(&mut self, command: usize, language: &str) {
        if let Some(Command::AddFullText(spec)) = self.commands.get_mut(command) {
            spec.language = Some(language.to_owned());
        }
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

    /// Gives the table the character set `name` on MySQL and MariaDB, over
    /// `DB_CHARSET`, as Laravel's `$table->charset`. Postgres and SQLite
    /// ignore it. It applies to [`Schema::create`](super::Schema::create);
    /// [`Schema::table`](super::Schema::table) refuses it, since an existing
    /// table keeps the character set it was created with.
    ///
    /// The name reaches the SQL unquoted, so one with anything but letters,
    /// digits and underscores makes the migration fail.
    pub fn charset(&mut self, name: &str) {
        self.charset = Some(name.to_owned());
    }

    /// Gives the table the collation `name` on MySQL and MariaDB, over
    /// `DB_COLLATION`, as Laravel's `$table->collation`: `utf8mb4_bin`
    /// compares text byte by byte, for a column of tokens or case-sensitive
    /// codes. Otherwise as [`Self::charset`].
    ///
    /// ```no_run
    /// # use suprnova::schema::Blueprint;
    /// # fn define(t: &mut Blueprint) {
    /// t.id();
    /// t.string("token").unique();
    /// t.collation("utf8mb4_bin");
    /// # }
    /// ```
    pub fn collation(&mut self, name: &str) {
        self.collation = Some(name.to_owned());
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

/// The modifier of the full-text index [`Blueprint::full_text`] just
/// declared. It borrows the blueprint, so it cannot outlive the closure.
pub struct FullTextIndexBuilder<'a> {
    blueprint: &'a mut Blueprint,
    command: usize,
}

impl FullTextIndexBuilder<'_> {
    /// Builds the Postgres index with the text search configuration
    /// `language`, such as `simple` or `french`, instead of `english`, as
    /// Laravel's `->language(..)`. A search uses the index only when it
    /// names the same language, through
    /// [`FullTextOptions::language`](crate::FullTextOptions::language).
    /// MySQL and MariaDB have no language and ignore it.
    ///
    /// The name is written into the SQL, so one with anything but letters,
    /// digits and `_` (and an optional `schema.` prefix) makes the migration
    /// fail on every backend.
    pub fn language(self, language: &str) -> Self {
        self.blueprint
            .set_full_text_language(self.command, language);
        self
    }
}
