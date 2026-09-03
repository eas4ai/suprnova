//! Phase 10C T10 - `DB` facade extensions for model-less queries.
//!
//! This file ships two surfaces:
//!
//! 1. [`DbTableBuilder`] - a chainable query builder returned by
//!    [`DB::table(name)`](crate::DB::table). Mirrors the
//!    `filter`/`order_by`/`limit` shape of `Builder<M>` but materialises
//!    rows as [`DynamicRow`] instead of a typed model. Use it for
//!    tables that aren't worth a full `#[suprnova::model]` (audit
//!    logs, ad-hoc reports, dashboard aggregates).
//!
//! 2. Raw-SQL escapes - [`DB::select`](crate::DB::select),
//!    [`DB::update`](crate::DB::update), [`DB::delete`](crate::DB::delete),
//!    [`DB::statement`](crate::DB::statement),
//!    [`DB::affecting_statement`](crate::DB::affecting_statement). When
//!    the builder isn't enough (window functions, recursive CTEs,
//!    backend-specific DDL), drop to a raw string with placeholder
//!    bindings.
//!
//! ## Trust boundary on identifiers
//!
//! Table names, column names, SQL operators, and ORDER BY directions
//! are interpolated INTO the SQL string verbatim - they are NOT bound
//! as parameters (SQL doesn't allow that). Treat every `impl
//! Into<String>` argument to this builder as a trusted, compile-time
//! literal: do NOT splice user input into table or column names.
//! Values (the right-hand side of `filter` / `filter_op`) ARE bound
//! as parameters and safe to pass through from request data. Explicit
//! null write attributes are emitted as the constant SQL literal `NULL`
//! because their original Rust type is no longer available in [`Attrs`].
//!
//! Backend-aware placeholder generation: `$N` (Postgres) vs `?`
//! (MySQL + SQLite). The counter is monotonic across the SET clause
//! and WHERE clause in UPDATE statements so each binding lines up with
//! its corresponding position.

use crate::FrameworkError;
use crate::database::DB;
use crate::database::dynamic_row::DynamicRow;
use crate::eloquent::Collection;
use crate::eloquent::attrs::Attrs;
use crate::eloquent::builder::Direction;
use sea_orm::{DbBackend, JsonValue, Statement, Value as SeaValue};

/// Materialise a SeaORM `QueryResult` as a [`DynamicRow`]. Mirrors the
/// shape `JsonValue::find_by_statement` produces (a JSON object per
/// row) but goes through the executor's instrumented `query_all` /
/// `query_one` so QueryExecuted observation works. Returns `None`
/// when the row doesn't parse as an object - matching the prior
/// `filter_map` behaviour on `JsonValue`.
fn query_result_to_dynamic_row(qr: &sea_orm::QueryResult) -> Option<DynamicRow> {
    use sea_orm::FromQueryResult;
    let v = JsonValue::from_query_result(qr, "").ok()?;
    match v {
        serde_json::Value::Object(map) => Some(DynamicRow::from_map(map)),
        _ => None,
    }
}

/// True when `sql`, ignoring leading whitespace, starts with `SELECT`
/// (case-insensitive). Used by [`DB::statement`](crate::DB::statement) to
/// decide whether a raw statement is a read (skip the render cache's
/// broad-authority advance) or a write of unknown shape (advance it).
fn is_select_statement(sql: &str) -> bool {
    sql.trim_start()
        .get(.."SELECT".len())
        .is_some_and(|head| head.eq_ignore_ascii_case("SELECT"))
}

fn write_value_expression(
    backend: DbBackend,
    value: &serde_json::Value,
    values: &mut Vec<SeaValue>,
    position: &mut usize,
) -> String {
    if value.is_null() {
        return "NULL".to_owned();
    }

    *position += 1;
    values.push(crate::eloquent::model::json_value_to_sea_value(value));
    if backend == DbBackend::Postgres {
        format!("${position}")
    } else {
        "?".to_owned()
    }
}

/// One WHERE clause captured by [`DbTableBuilder`].
///
/// A named struct rather than a `(col, op, val)` tuple because
/// `where_binary` needs a fourth field and three separate render sites
/// read it - a fourth tuple position would make each of them count.
struct DbWhereTerm {
    column: String,
    op: String,
    value: SeaValue,
    /// `true` for [`DbTableBuilder::where_binary`] /
    /// [`DbTableBuilder::where_not_binary`]: renders MySQL's
    /// `col = binary ?`. Every other backend refuses at render time.
    binary: bool,
}

/// Standalone query builder returned by
/// [`DB::table(name)`](crate::DB::table). Mirrors the where / order /
/// limit / select shape of [`Builder<M>`](crate::eloquent::Builder)
/// but materialises rows as [`DynamicRow`] instead of a typed model.
///
/// See the [module docs](self) for the trust boundary on identifiers.
pub struct DbTableBuilder {
    table: String,
    where_terms: Vec<DbWhereTerm>,
    order: Vec<(String, Direction)>,
    limit_value: Option<u64>,
    offset_value: Option<u64>,
    select_columns: Vec<String>,
    /// Phase 10C T12 - per-builder connection override. Set via
    /// [`Self::on`] or constructed pre-set via
    /// [`DB::table_on`](crate::DB::table_on). Routes terminal methods
    /// through the named connection in the
    /// [`ConnectionRegistry`](crate::database::ConnectionRegistry).
    connection_override: Option<String>,
}

impl DbTableBuilder {
    /// Construct a builder for the given table. Prefer
    /// [`DB::table(name)`](crate::DB::table) - this is the underlying
    /// constructor.
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            where_terms: Vec::new(),
            order: Vec::new(),
            limit_value: None,
            offset_value: None,
            select_columns: Vec::new(),
            connection_override: None,
        }
    }

    /// Phase 10C T12 - route every terminal method on this builder
    /// through the connection registered under `name`. Inside an
    /// active transaction (`DB::transaction` closure) the override is
    /// silently ignored - every op runs through the tx connection.
    pub fn on(mut self, name: impl Into<String>) -> Self {
        self.connection_override = Some(name.into());
        self
    }

    /// Restrict the SELECT to a specific column list. Empty means `*`.
    ///
    /// ```rust,no_run
    /// # use suprnova::DB;
    /// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// DB::table("audit_log").select(["id", "event"]).get().await?;
    /// # Ok(()) }
    /// ```
    pub fn select<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.select_columns = cols.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Add a `WHERE col = ?` clause. Multiple `filter` calls AND together.
    pub fn filter(mut self, col: impl Into<String>, val: impl Into<SeaValue>) -> Self {
        self.where_terms.push(DbWhereTerm {
            column: col.into(),
            op: "=".into(),
            value: val.into(),
            binary: false,
        });
        self
    }

    /// Add a `WHERE col <op> ?` clause with an explicit operator
    /// (`>`, `>=`, `<`, `<=`, `<>`, `LIKE`, etc.). Multiple calls AND
    /// together.
    pub fn filter_op(
        mut self,
        col: impl Into<String>,
        op: impl Into<String>,
        val: impl Into<SeaValue>,
    ) -> Self {
        self.where_terms.push(DbWhereTerm {
            column: col.into(),
            op: op.into(),
            value: val.into(),
            binary: false,
        });
        self
    }

    /// Add a `WHERE col = binary ?` clause - compare the raw bytes
    /// instead of the column's collation, so the match is case- and
    /// accent-sensitive.
    ///
    /// **MySQL and MariaDB only.** On Postgres and SQLite every
    /// terminal returns `Err` when the statement renders, before any
    /// I/O: a plain `=` fallback would compare under the column's
    /// collation and return rows you asked to exclude.
    pub fn where_binary(mut self, col: impl Into<String>, val: impl Into<String>) -> Self {
        self.where_terms.push(DbWhereTerm {
            column: col.into(),
            op: "=".into(),
            value: SeaValue::String(Some(val.into())),
            binary: true,
        });
        self
    }

    /// Add a `WHERE col != binary ?` clause - the negated form of
    /// [`Self::where_binary`]. Same backend split.
    pub fn where_not_binary(mut self, col: impl Into<String>, val: impl Into<String>) -> Self {
        self.where_terms.push(DbWhereTerm {
            column: col.into(),
            op: "!=".into(),
            value: SeaValue::String(Some(val.into())),
            binary: true,
        });
        self
    }

    /// Add an `ORDER BY col DESC` term. Multiple `order_by_*` calls
    /// chain in insertion order.
    pub fn order_by_desc(mut self, col: impl Into<String>) -> Self {
        self.order.push((col.into(), Direction::Desc));
        self
    }

    /// Add an `ORDER BY col ASC` term.
    pub fn order_by_asc(mut self, col: impl Into<String>) -> Self {
        self.order.push((col.into(), Direction::Asc));
        self
    }

    /// Set the LIMIT.
    pub fn limit(mut self, n: u64) -> Self {
        self.limit_value = Some(n);
        self
    }

    /// Set the OFFSET.
    pub fn offset(mut self, n: u64) -> Self {
        self.offset_value = Some(n);
        self
    }

    /// Validate every user-supplied identifier and operator captured
    /// in this builder. Called by every terminal method before the
    /// SQL is rendered. See [`identifier`](crate::database::identifier)
    /// for the contract.
    fn validate_inputs(&self) -> Result<(), FrameworkError> {
        crate::database::validate_identifier(&self.table)?;
        for col in &self.select_columns {
            crate::database::validate_identifier(col)?;
        }
        for term in &self.where_terms {
            crate::database::validate_identifier(&term.column)?;
            crate::database::validate_sql_operator(&term.op)?;
        }
        for (col, _dir) in &self.order {
            crate::database::validate_identifier(col)?;
        }
        Ok(())
    }

    /// Execute the SELECT and return every matching row as a
    /// [`Collection<DynamicRow>`]. Materialises rows through the
    /// instrumented executor helpers - emits
    /// [`QueryExecuted`](crate::database::events::QueryExecuted) on
    /// every call.
    pub async fn get(self) -> Result<Collection<DynamicRow>, FrameworkError> {
        self.validate_inputs()?;
        let exec = crate::database::transaction::ExecutorChoice::resolve_read(
            None,
            self.connection_override.as_deref(),
            None,
        )
        .await?;
        let backend = exec.backend();
        let (sql, values) = self.render_select(backend)?;
        let stmt = Statement::from_sql_and_values(backend, &sql, values);

        let rows = exec
            .query_all(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        let dyn_rows: Vec<DynamicRow> = rows
            .iter()
            .filter_map(query_result_to_dynamic_row)
            .collect();

        Ok(Collection::from_vec(dyn_rows))
    }

    /// Execute the SELECT with `LIMIT 1` and return the single row
    /// (or `None` when the result set is empty).
    pub async fn first(self) -> Result<Option<DynamicRow>, FrameworkError> {
        let rows = self.limit(1).get().await?.into_vec();
        Ok(rows.into_iter().next())
    }

    /// Execute `SELECT COUNT(*) FROM ... WHERE ...` and return the
    /// count. Clears `select_columns` / `order` / `limit` / `offset`
    /// before rendering - count semantics don't care about those.
    ///
    /// Uses `query_one` + `try_get` directly instead of
    /// `JsonValue::find_by_statement` because aggregate columns
    /// (`COUNT(*)`) don't always carry a type tag through sqlx's
    /// per-column type detection that backs `JsonValue`'s
    /// `FromQueryResult` impl - on SQLite the typed accessor is the
    /// reliable path.
    pub async fn count(self) -> Result<u64, FrameworkError> {
        // Validate user inputs BEFORE the COUNT(*) override stomps
        // `select_columns` - the override is framework-controlled
        // literal SQL and would otherwise fail the identifier
        // validator on the parenthesised aggregate.
        self.validate_inputs()?;
        // T11/T12: route through resolve_read.
        let exec = crate::database::transaction::ExecutorChoice::resolve_read(
            None,
            self.connection_override.as_deref(),
            None,
        )
        .await?;
        let backend = exec.backend();
        let mut copy = self;
        copy.select_columns = vec!["COUNT(*) as count".into()];
        copy.order.clear();
        copy.limit_value = None;
        copy.offset_value = None;
        let (sql, values) = copy.render_select(backend)?;
        let stmt = Statement::from_sql_and_values(backend, &sql, values);

        let row = exec
            .query_one(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        let count: i64 = row
            .as_ref()
            .and_then(|r| r.try_get::<i64>("", "count").ok())
            .unwrap_or(0);
        Ok(count.max(0) as u64)
    }

    /// Insert one row and return the newly-assigned **auto-increment
    /// integer primary key** named `id`.
    ///
    /// This mirrors Laravel's `DB::table(...)->insertGetId(...)`. The
    /// model-less builder assumes the standard `id BIGINT PRIMARY KEY
    /// AUTO_INCREMENT` / `SERIAL` / `INTEGER PRIMARY KEY AUTOINCREMENT`
    /// convention because there is no entity definition to consult. If
    /// the target table:
    ///
    /// - has no column named `id`, or
    /// - uses a UUID, composite, renamed, or non-integer primary key,
    ///
    /// the call returns a [`FrameworkError::Database`] instead of
    /// silently producing a wrong id. Use the typed Eloquent
    /// [`Model`](crate::eloquent::Model) surface for tables that don't
    /// match the convention - it consults the model definition for
    /// primary-key shape and type.
    ///
    /// Backend split: Postgres + SQLite use `RETURNING id`; MySQL runs
    /// the INSERT then surfaces the driver's per-connection
    /// `last_insert_id()` from the `ExecResult`. Non-null attributes remain
    /// parameter-bound; explicit nulls are emitted as the constant SQL literal
    /// `NULL` so PostgreSQL can infer each target column's type.
    pub async fn insert(self, attrs: Attrs) -> Result<i64, FrameworkError> {
        // Audit HIGH `database` #2 - validate identifiers and operators
        // captured in the builder state, plus the attrs keys which are
        // themselves identifiers being interpolated into SQL.
        self.validate_inputs()?;
        for col in attrs.keys() {
            crate::database::validate_identifier(col)?;
        }
        // T11/T12: route through resolve_write - writes never go to
        // `__read_replica__`.
        let exec = crate::database::transaction::ExecutorChoice::resolve_write(
            None,
            self.connection_override.as_deref(),
            None,
        )
        .await?;
        let backend = exec.backend();

        let cols: Vec<String> = attrs.keys().map(String::from).collect();
        if cols.is_empty() {
            return Err(FrameworkError::database(format!(
                "DB::table(\"{}\")::insert called with empty attrs",
                self.table
            )));
        }

        let mut values: Vec<SeaValue> = Vec::new();
        let mut position = 0usize;
        let placeholders: Vec<String> = cols
            .iter()
            .map(|c| {
                let v = attrs
                    .get(c)
                    .expect("key present in iter must be present in get");
                write_value_expression(backend, v, &mut values, &mut position)
            })
            .collect();

        let base = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.table,
            cols.join(", "),
            placeholders.join(", "),
        );

        let id: i64 = match backend {
            DbBackend::Postgres | DbBackend::Sqlite => {
                let sql = format!("{base} RETURNING id");
                let stmt = Statement::from_sql_and_values(backend, &sql, values);
                let row = exec.query_one(stmt)
                    .await
                    .map_err(|e| FrameworkError::database(e.to_string()))?
                    .ok_or_else(|| {
                        FrameworkError::database(format!(
                            "DB::table(\"{}\")::insert: backend returned no row from `RETURNING id`",
                            self.table,
                        ))
                    })?;
                row.try_get::<i64>("", "id").map_err(|e| {
                    FrameworkError::database(format!(
                        "DB::table(\"{}\")::insert: cannot read auto-increment `id` as i64 ({e}). \
                         The model-less builder assumes an `id BIGINT` (or compatible) \
                         primary key - for UUID, composite, renamed, or non-integer \
                         primary keys use the typed Eloquent Model surface instead.",
                        self.table,
                    ))
                })?
            }
            DbBackend::MySql => {
                // MySQL doesn't support `RETURNING`. Use the driver's
                // own per-connection `last_insert_id()` exposed through
                // `ExecResult` - running a separate `SELECT
                // LAST_INSERT_ID()` against a pooled connection would
                // be unsafe because the SELECT might land on a
                // different physical connection than the INSERT.
                let stmt = Statement::from_sql_and_values(backend, &base, values);
                let result = exec
                    .run(stmt)
                    .await
                    .map_err(|e| FrameworkError::database(e.to_string()))?;
                let raw = result.last_insert_id();
                if raw == 0 {
                    // MySQL returns 0 for tables without an
                    // AUTO_INCREMENT column (e.g. UUID PK or composite
                    // PK). Surfacing `0` would silently lie - fail
                    // loudly with the same actionable guidance as the
                    // Postgres / SQLite branch.
                    return Err(FrameworkError::database(format!(
                        "DB::table(\"{}\")::insert: MySQL returned last_insert_id() = 0; \
                         the model-less builder assumes an AUTO_INCREMENT integer `id` \
                         primary key. For UUID, composite, renamed, or non-integer \
                         primary keys use the typed Eloquent Model surface instead.",
                        self.table,
                    )));
                }
                raw as i64
            }
            _ => return Err(super::unsupported_database_backend(backend)),
        };
        crate::render_cache::orm::after_table_write(&self.table).await?;
        Ok(id)
    }

    /// Update every row matched by the WHERE clauses. Returns the
    /// number of rows affected.
    ///
    /// **Empty WHERE updates every row in the table.** That's a
    /// supported but rarely-correct operation - callers should add at
    /// least one `filter` unless they really mean "all rows."
    ///
    /// Dual-API: this is the Laravel-faithful name; the
    /// `Builder<M>`-style alias is [`Self::update_all`]. Both call into
    /// the same implementation. Prefer the `_all` name when the
    /// table-wide intent is the point of the call site - it makes the
    /// missing `filter` visible to reviewers. Non-null attributes are bound;
    /// explicit nulls use the constant SQL literal `NULL` to retain the target
    /// column type on PostgreSQL.
    pub async fn update(self, attrs: Attrs) -> Result<u64, FrameworkError> {
        if attrs.is_empty() {
            return Err(FrameworkError::database(format!(
                "DB::table(\"{}\")::update called with empty attrs",
                self.table
            )));
        }
        // Audit HIGH `database` #2 - same validation as insert; the
        // attrs keys land in `SET col = ?` so they must be safe
        // identifiers.
        self.validate_inputs()?;
        for col in attrs.keys() {
            crate::database::validate_identifier(col)?;
        }
        // T11/T12: route through resolve_write.
        let exec = crate::database::transaction::ExecutorChoice::resolve_write(
            None,
            self.connection_override.as_deref(),
            None,
        )
        .await?;
        let backend = exec.backend();
        let (sql, values) = self.render_update(&attrs, backend)?;
        let stmt = Statement::from_sql_and_values(backend, &sql, values);
        let result = exec
            .run(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        crate::render_cache::orm::after_table_write(&self.table).await?;
        Ok(result.rows_affected())
    }

    /// Alias for [`Self::update`] that matches the [`Builder<M>`]
    /// typed-Eloquent surface, where the bulk mutator is named
    /// [`update_all`](crate::eloquent::Builder::update_all) to make the
    /// table-wide intent explicit at the call site. Same semantics: an
    /// empty WHERE updates every row.
    ///
    /// [`Builder<M>`]: crate::eloquent::Builder
    #[doc(alias = "update")]
    pub async fn update_all(self, attrs: Attrs) -> Result<u64, FrameworkError> {
        self.update(attrs).await
    }

    /// Delete every row matched by the WHERE clauses. Returns the
    /// number of rows affected.
    ///
    /// **Empty WHERE truncates the table.** `DB::table("x").delete()`
    /// removes every row by design - add a `filter` if you don't mean
    /// that.
    ///
    /// Dual-API: this is the Laravel-faithful name; the
    /// `Builder<M>`-style alias is [`Self::delete_all`]. Both call into
    /// the same implementation. Prefer the `_all` name when the
    /// table-wide intent is the point of the call site.
    pub async fn delete(self) -> Result<u64, FrameworkError> {
        // Audit HIGH `database` #2 - identifier + operator validation.
        self.validate_inputs()?;
        // T11/T12: route through resolve_write.
        let exec = crate::database::transaction::ExecutorChoice::resolve_write(
            None,
            self.connection_override.as_deref(),
            None,
        )
        .await?;
        let backend = exec.backend();
        let (sql, values) = self.render_delete(backend)?;
        let stmt = Statement::from_sql_and_values(backend, &sql, values);
        let result = exec
            .run(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        crate::render_cache::orm::after_table_write(&self.table).await?;
        Ok(result.rows_affected())
    }

    /// Alias for [`Self::delete`] that matches the [`Builder<M>`]
    /// typed-Eloquent surface, where the bulk mutator is named
    /// [`delete_all`](crate::eloquent::Builder::delete_all) to make the
    /// table-wide intent explicit at the call site. Same semantics: an
    /// empty WHERE removes every row.
    ///
    /// [`Builder<M>`]: crate::eloquent::Builder
    #[doc(alias = "delete")]
    pub async fn delete_all(self) -> Result<u64, FrameworkError> {
        self.delete().await
    }

    // ---- SQL rendering ---------------------------------------------------

    /// Render the shared WHERE clause list, including the leading
    /// ` WHERE `. Returns an empty string when the builder carries no
    /// filters, so callers push the result unconditionally.
    ///
    /// Fallible because `where_binary` is a MySQL/MariaDB-only
    /// comparison: on any other backend this returns `Err` before a
    /// statement leaves the process, rather than degrading to a
    /// collation-dependent `=`.
    fn render_where_clauses(
        &self,
        backend: DbBackend,
        values: &mut Vec<SeaValue>,
        counter: &mut usize,
    ) -> Result<String, FrameworkError> {
        if self.where_terms.is_empty() {
            return Ok(String::new());
        }
        let mut clauses: Vec<String> = Vec::with_capacity(self.where_terms.len());
        for term in &self.where_terms {
            *counter += 1;
            values.push(term.value.clone());
            let placeholder = if backend == DbBackend::Postgres {
                format!("${counter}")
            } else {
                "?".to_owned()
            };
            let op = if term.binary {
                match backend {
                    DbBackend::MySql => format!("{} binary", term.op),
                    _ => return Err(crate::database::binary_comparison_unsupported(backend)),
                }
            } else {
                term.op.clone()
            };
            clauses.push(format!("{} {} {}", term.column, op, placeholder));
        }
        Ok(format!(" WHERE {}", clauses.join(" AND ")))
    }

    fn render_select(&self, backend: DbBackend) -> Result<(String, Vec<SeaValue>), FrameworkError> {
        let mut values: Vec<SeaValue> = Vec::new();
        let mut counter = 0usize;
        let mut sql = String::new();
        sql.push_str("SELECT ");
        if self.select_columns.is_empty() {
            sql.push('*');
        } else {
            sql.push_str(&self.select_columns.join(", "));
        }
        sql.push_str(" FROM ");
        sql.push_str(&self.table);

        sql.push_str(&self.render_where_clauses(backend, &mut values, &mut counter)?);

        if !self.order.is_empty() {
            sql.push_str(" ORDER BY ");
            let order: Vec<String> = self
                .order
                .iter()
                .map(|(col, dir)| {
                    let dir_sql = match dir {
                        Direction::Asc => "ASC",
                        Direction::Desc => "DESC",
                    };
                    format!("{col} {dir_sql}")
                })
                .collect();
            sql.push_str(&order.join(", "));
        }

        if let Some(n) = self.limit_value {
            sql.push_str(&format!(" LIMIT {n}"));
        }
        if let Some(n) = self.offset_value {
            sql.push_str(&format!(" OFFSET {n}"));
        }

        Ok((sql, values))
    }

    fn render_update(
        &self,
        attrs: &Attrs,
        backend: DbBackend,
    ) -> Result<(String, Vec<SeaValue>), FrameworkError> {
        let mut values: Vec<SeaValue> = Vec::new();
        let mut counter = 0usize;

        let mut sql = format!("UPDATE {} SET ", self.table);
        let sets: Vec<String> = attrs
            .keys()
            .map(|col| {
                let v = attrs
                    .get(col)
                    .expect("key present in iter must be present in get");
                let expression = write_value_expression(backend, v, &mut values, &mut counter);
                format!("{col} = {expression}")
            })
            .collect();
        sql.push_str(&sets.join(", "));

        sql.push_str(&self.render_where_clauses(backend, &mut values, &mut counter)?);

        Ok((sql, values))
    }

    fn render_delete(&self, backend: DbBackend) -> Result<(String, Vec<SeaValue>), FrameworkError> {
        let mut values: Vec<SeaValue> = Vec::new();
        let mut counter = 0usize;
        let mut sql = format!("DELETE FROM {}", self.table);
        sql.push_str(&self.render_where_clauses(backend, &mut values, &mut counter)?);
        Ok((sql, values))
    }
}

// ---- DB facade extensions -----------------------------------------------

impl DB {
    /// Open a model-less query builder for `name`. See
    /// [`DbTableBuilder`] for the chainable surface.
    ///
    /// ```rust,no_run
    /// # use suprnova::DB;
    /// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// let rows = DB::table("audit_log")
    ///     .filter("actor_id", 42)
    ///     .order_by_desc("id")
    ///     .limit(50)
    ///     .get()
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn table(name: impl Into<String>) -> DbTableBuilder {
        DbTableBuilder::new(name)
    }

    /// Run a raw SELECT and return every row as a [`DynamicRow`].
    /// Placeholders must match the active backend (`$1, $2, ...` for
    /// Postgres, `?` for MySQL + SQLite).
    ///
    /// ```rust,no_run
    /// # use suprnova::DB;
    /// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// let rows = DB::select(
    ///     "SELECT * FROM audit_log WHERE actor_id = ?",
    ///     vec![42i64.into()],
    /// ).await?;
    /// # Ok(()) }
    /// ```
    pub async fn select(
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<Vec<DynamicRow>, FrameworkError> {
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_read(None, None, None).await?;
        let backend = exec.backend();
        let stmt =
            Statement::from_sql_and_values(backend, sql, values.into_iter().collect::<Vec<_>>());
        let rows = exec
            .query_all(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter_map(|qr| query_result_to_dynamic_row(&qr))
            .collect())
    }

    /// Run a raw SELECT and return the FIRST row (or `None`).
    /// Mirrors Laravel's `DB::selectOne($sql, $bindings)`.
    pub async fn select_one(
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<Option<DynamicRow>, FrameworkError> {
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_read(None, None, None).await?;
        let backend = exec.backend();
        let stmt =
            Statement::from_sql_and_values(backend, sql, values.into_iter().collect::<Vec<_>>());
        let row = exec
            .query_one(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(row.as_ref().and_then(query_result_to_dynamic_row))
    }

    /// Run a raw SELECT, return the FIRST column of the FIRST row.
    /// Mirrors Laravel's `DB::scalar($sql, $bindings)`.
    ///
    /// `T` must implement `sea_orm::TryGetable` - the framework
    /// re-exports the trait at the crate root.
    ///
    /// ```rust,no_run
    /// # use suprnova::DB;
    /// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// let count: i64 = DB::scalar("SELECT COUNT(*) FROM users", vec![]).await?;
    /// let name: String = DB::scalar("SELECT name FROM users LIMIT 1", vec![]).await?;
    /// # Ok(()) }
    /// ```
    pub async fn scalar<T>(
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<T, FrameworkError>
    where
        T: sea_orm::TryGetable,
    {
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_read(None, None, None).await?;
        let backend = exec.backend();
        let stmt =
            Statement::from_sql_and_values(backend, sql, values.into_iter().collect::<Vec<_>>());
        let row = exec
            .query_one(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?
            .ok_or_else(|| FrameworkError::database("DB::scalar: query returned no rows"))?;
        row.try_get_by_index::<T>(0)
            .map_err(|e| FrameworkError::database(format!("DB::scalar: {e}")))
    }

    /// Run a raw INSERT statement. Returns `true` when at least one row
    /// was affected, `false` otherwise. Mirrors Laravel's
    /// `DB::insert($sql, $bindings)`.
    ///
    /// For builder-style inserts that return the inserted PK use
    /// [`DB::table`] + [`DbTableBuilder::insert`] instead.
    pub async fn insert(
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<bool, FrameworkError> {
        let rows = DB::affecting_statement(sql, values).await?;
        Ok(rows > 0)
    }

    /// Run a raw UPDATE and return the number of rows affected.
    /// Convenience alias over [`DB::affecting_statement`].
    pub async fn update(
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<u64, FrameworkError> {
        DB::affecting_statement(sql, values).await
    }

    /// Run a raw DELETE and return the number of rows affected.
    /// Convenience alias over [`DB::affecting_statement`].
    pub async fn delete(
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<u64, FrameworkError> {
        DB::affecting_statement(sql, values).await
    }

    /// Run a SQL statement with bindings. Returns `true` when the
    /// statement was accepted by the driver, `false` otherwise. The
    /// `bindings` close the `?` / `$N` placeholders in `sql`.
    ///
    /// Use for DDL with bindings, or any statement that doesn't fit
    /// the `select` / `insert` / `update` / `delete` shape. For DDL
    /// with no bindings, [`Self::unprepared`] is the explicit form.
    ///
    /// This is the raw escape hatch, so the render cache cannot know
    /// which tables or rows a given statement touches: every call whose
    /// `sql` does not start with `SELECT` (case-insensitively, leading
    /// whitespace ignored) advances the broad authority every
    /// representation observes, on the assumption that it changed
    /// something. A statement beginning with `SELECT` never does -
    /// over-invalidating a read is merely wasted cache, but treating a
    /// write as a read would serve stale content forever.
    pub async fn statement(
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<bool, FrameworkError> {
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_write(None, None, None).await?;
        let backend = exec.backend();
        let stmt =
            Statement::from_sql_and_values(backend, sql, values.into_iter().collect::<Vec<_>>());
        let ok = exec
            .run(stmt)
            .await
            .map(|_| true)
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        if !is_select_statement(sql) {
            crate::render_cache::orm::after_unknown_write().await?;
        }
        Ok(ok)
    }

    /// Run a raw, unprepared SQL statement (no placeholder binding).
    /// Mirrors Laravel's `DB::unprepared($sql)`. The string is
    /// executed VERBATIM - never splice user input into it.
    ///
    /// Necessary for DDL on backends that reject parameter-bound
    /// statements: `CREATE INDEX`, `ALTER TABLE`, `VACUUM`, etc.
    pub async fn unprepared(sql: &str) -> Result<bool, FrameworkError> {
        use sea_orm::ConnectionTrait;
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_write(None, None, None).await?;
        // Emit QueryExecuted for unprepared statements as well - they
        // are still queries from the observer's perspective.
        if super::events::is_dispatching() || !super::events::query_observation_active() {
            return match &exec {
                crate::database::transaction::ExecutorChoice::Tx(t, _) => {
                    t.execute_unprepared(sql).await
                }
                crate::database::transaction::ExecutorChoice::Pool(c, _) => {
                    c.inner().execute_unprepared(sql).await
                }
            }
            .map(|_| true)
            .map_err(|e| FrameworkError::database(e.to_string()));
        }
        let conn_name = exec.connection_name().to_string();
        let start = std::time::Instant::now();
        let res = match &exec {
            crate::database::transaction::ExecutorChoice::Tx(t, _) => {
                t.execute_unprepared(sql).await
            }
            crate::database::transaction::ExecutorChoice::Pool(c, _) => {
                c.inner().execute_unprepared(sql).await
            }
        };
        let elapsed = start.elapsed();
        let result_for_event: Result<(), String> = match &res {
            Ok(_) => Ok(()),
            Err(e) => Err(e.to_string()),
        };
        let event = super::events::QueryExecuted {
            sql: sql.to_string(),
            bindings: vec![],
            time: elapsed,
            connection_name: conn_name,
            read_write_type: Some(super::events::ReadWriteType::Write),
            result: result_for_event,
        };
        super::transaction::emit_query_executed(event).await;
        res.map(|_| true)
            .map_err(|e| FrameworkError::database(e.to_string()))
    }

    /// Run a raw statement that produces a `rows_affected` result.
    /// Used by [`DB::update`] and [`DB::delete`] under the hood;
    /// exposed directly for cases where the operation doesn't fit
    /// either name (e.g. `INSERT ... ON CONFLICT DO UPDATE`).
    pub async fn affecting_statement(
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<u64, FrameworkError> {
        // T11/T12: route through resolve_write - affecting statements
        // (INSERT/UPDATE/DELETE/UPSERT) never go to the replica.
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_write(None, None, None).await?;
        let backend = exec.backend();
        let stmt =
            Statement::from_sql_and_values(backend, sql, values.into_iter().collect::<Vec<_>>());
        let result = exec
            .run(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(result.rows_affected())
    }

    // ---- Phase 10C T12 - connection-pinned raw escapes ------------------

    /// Phase 10C T12 - `DB::table(name)` variant that pins the returned
    /// builder to the connection registered under `conn_name`. Equivalent
    /// to `DB::table(table).on(conn_name)`. Inside a `DB::transaction`
    /// the override is silently ignored - every op runs through the tx.
    pub fn table_on(conn_name: impl Into<String>, table: impl Into<String>) -> DbTableBuilder {
        DbTableBuilder::new(table).on(conn_name)
    }

    /// Phase 10C T12 - `DB::select` variant that runs against the
    /// named connection instead of consulting the default routing
    /// chain. Errors if `conn_name` isn't registered.
    pub async fn select_on(
        conn_name: &str,
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<Vec<DynamicRow>, FrameworkError> {
        // Inside a transaction the tx connection wins absolutely -
        // even an explicit `_on` call cannot route around it because
        // it would split the atomicity contract. Resolve_read with the
        // override expresses exactly that precedence.
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_read(None, Some(conn_name), None)
                .await?;
        let backend = exec.backend();
        let stmt =
            Statement::from_sql_and_values(backend, sql, values.into_iter().collect::<Vec<_>>());
        let rows = exec
            .query_all(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter_map(|qr| query_result_to_dynamic_row(&qr))
            .collect())
    }

    /// Phase 10C T12 - `DB::statement` variant that runs against the
    /// named connection. Useful for backend-specific DDL on a read
    /// replica (e.g. `CREATE INDEX` on a follower that's been promoted
    /// to standalone).
    pub async fn statement_on(
        conn_name: &str,
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<bool, FrameworkError> {
        let exec = crate::database::transaction::ExecutorChoice::resolve_write(
            None,
            Some(conn_name),
            None,
        )
        .await?;
        let backend = exec.backend();
        let stmt =
            Statement::from_sql_and_values(backend, sql, values.into_iter().collect::<Vec<_>>());
        exec.run(stmt)
            .await
            .map(|_| true)
            .map_err(|e| FrameworkError::database(e.to_string()))
    }

    /// Phase 10C T12 - `DB::affecting_statement` variant pinned to the
    /// named connection. INSERT / UPDATE / DELETE / UPSERT on a
    /// non-primary write target.
    pub async fn affecting_statement_on(
        conn_name: &str,
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<u64, FrameworkError> {
        let exec = crate::database::transaction::ExecutorChoice::resolve_write(
            None,
            Some(conn_name),
            None,
        )
        .await?;
        let backend = exec.backend();
        let stmt =
            Statement::from_sql_and_values(backend, sql, values.into_iter().collect::<Vec<_>>());
        let result = exec
            .run(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(result.rows_affected())
    }
}

// ---- Observability ------------------------------------------------------

impl DB {
    /// Register a `Fn(&QueryExecuted)` listener that fires after every
    /// query routed through the instrumented executor helpers. Mirrors
    /// Laravel's `DB::listen(function (QueryExecuted $event) { ... })`.
    ///
    /// Coverage today: every raw helper on this facade
    /// (`select`/`select_one`/`scalar`/`insert`/`update`/`delete`/
    /// `statement`/`affecting_statement`/`unprepared`) and every
    /// terminal method on [`DbTableBuilder`]. The Eloquent ORM
    /// execution path matches the executor's `Tx`/`Pool` arms directly
    /// today; adopting the helpers (and therefore observation) is
    /// tracked on the Eloquent module.
    ///
    /// Listeners run synchronously inside the executor helper. A
    /// failing or slow listener WILL slow the query - keep them light
    /// and non-blocking. The complementary
    /// [`EventFacade::listen::<QueryExecuted, _>(...)`](crate::EventFacade::listen)
    /// path runs through `dispatch_best_effort` and tolerates errors;
    /// prefer it for anything that can fail.
    ///
    /// Re-entrancy: a listener that itself issues a database query
    /// will NOT re-fire `QueryExecuted` for that nested query - the
    /// inner call short-circuits to skip emission.
    pub fn listen<F>(callback: F) -> Result<(), FrameworkError>
    where
        F: Fn(&crate::database::events::QueryExecuted) + Send + Sync + 'static,
    {
        let mut reg =
            crate::lock::write(crate::database::events::listeners(), "db event listeners")?;
        reg.listeners.push(std::sync::Arc::new(callback));
        Ok(())
    }

    /// Remove every `DB::listen` callback. Does NOT touch
    /// `EventFacade::listen` listeners - those go through
    /// [`EventFacade::forget`](crate::EventFacade) (the dispatcher's
    /// per-event forget surface).
    pub fn flush_listeners() -> Result<(), FrameworkError> {
        let mut reg =
            crate::lock::write(crate::database::events::listeners(), "db event listeners")?;
        reg.listeners.clear();
        Ok(())
    }

    /// Enable the in-memory query log. Every query that fires
    /// [`QueryExecuted`](crate::database::events::QueryExecuted) will
    /// be appended to a buffer drainable via [`Self::get_query_log`].
    ///
    /// **The buffer is unbounded**: every captured query grows it.
    /// Use [`Self::flush_query_log`] periodically - or
    /// [`Self::disable_query_log`] when done - to release memory.
    pub fn enable_query_log() -> Result<(), FrameworkError> {
        let mut log = crate::database::events::query_log()
            .lock()
            .map_err(|e| FrameworkError::internal(format!("query_log lock poisoned: {e}")))?;
        log.enabled = true;
        Ok(())
    }

    /// Disable the in-memory query log. Existing buffered entries are
    /// retained; call [`Self::flush_query_log`] to drop them. Mirrors
    /// Laravel's `DB::disableQueryLog`.
    pub fn disable_query_log() -> Result<(), FrameworkError> {
        let mut log = crate::database::events::query_log()
            .lock()
            .map_err(|e| FrameworkError::internal(format!("query_log lock poisoned: {e}")))?;
        log.enabled = false;
        Ok(())
    }

    /// True when the query log is currently active. Mirrors Laravel's
    /// `DB::logging()`.
    pub fn logging() -> bool {
        crate::database::events::query_log()
            .lock()
            .map(|l| l.enabled)
            .unwrap_or(false)
    }

    /// Snapshot the captured query log. Returns a `Vec` of every
    /// `QueryExecuted` event since the log was enabled (or last
    /// flushed). Does NOT drain the buffer - call
    /// [`Self::flush_query_log`] to clear it.
    pub fn get_query_log() -> Result<Vec<crate::database::events::QueryExecuted>, FrameworkError> {
        let log = crate::database::events::query_log()
            .lock()
            .map_err(|e| FrameworkError::internal(format!("query_log lock poisoned: {e}")))?;
        Ok(log.entries.clone())
    }

    /// Drop every captured entry from the query log. The log stays
    /// enabled - new queries will still be appended. Mirrors Laravel's
    /// `DB::flushQueryLog`.
    pub fn flush_query_log() -> Result<(), FrameworkError> {
        let mut log = crate::database::events::query_log()
            .lock()
            .map_err(|e| FrameworkError::internal(format!("query_log lock poisoned: {e}")))?;
        log.entries.clear();
        Ok(())
    }
}

// ---- Connection metadata ------------------------------------------------

impl DB {
    /// Return the database name extracted from the configured URL.
    /// Mirrors Laravel's `DB::connection()->getDatabaseName()` -
    /// returns the path component of the URL ("forge" for
    /// `postgres://u:p@host/forge"`, the file name for SQLite paths).
    ///
    /// Errors when [`DB::init`] has not been called.
    pub fn database_name() -> Result<String, FrameworkError> {
        let cfg = crate::Config::get::<crate::database::DatabaseConfig>().ok_or_else(|| {
            FrameworkError::internal(
                "DatabaseConfig not registered; call Config::register(DatabaseConfig::from_env()) first",
            )
        })?;
        Ok(parse_database_name(&cfg.url))
    }

    /// Return the driver name as a lower-case kebab-style string:
    /// `"postgres"`, `"mysql"`, `"sqlite"`. Mirrors Laravel's
    /// `DB::connection()->getDriverName()`.
    pub fn driver_name() -> Result<&'static str, FrameworkError> {
        let cfg = crate::Config::get::<crate::database::DatabaseConfig>().ok_or_else(|| {
            FrameworkError::internal(
                "DatabaseConfig not registered; call Config::register(DatabaseConfig::from_env()) first",
            )
        })?;
        Ok(match cfg.database_type() {
            crate::database::DatabaseType::Postgres => "postgres",
            crate::database::DatabaseType::Mysql => "mysql",
            crate::database::DatabaseType::Sqlite => "sqlite",
            crate::database::DatabaseType::Unknown => "unknown",
        })
    }

    /// Return the human-readable driver title - `"Postgres"`, `"MySQL"`,
    /// `"SQLite"`, `"Unknown"`. Mirrors Laravel's
    /// `DB::connection()->getDriverTitle()`.
    pub fn driver_title() -> Result<&'static str, FrameworkError> {
        let cfg = crate::Config::get::<crate::database::DatabaseConfig>().ok_or_else(|| {
            FrameworkError::internal(
                "DatabaseConfig not registered; call Config::register(DatabaseConfig::from_env()) first",
            )
        })?;
        Ok(match cfg.database_type() {
            crate::database::DatabaseType::Postgres => "Postgres",
            crate::database::DatabaseType::Mysql => "MySQL",
            crate::database::DatabaseType::Sqlite => "SQLite",
            crate::database::DatabaseType::Unknown => "Unknown",
        })
    }

    /// Query the live database for its server version string. Issues
    /// a backend-specific introspection query:
    ///
    /// - Postgres / MySQL: `SELECT VERSION()`.
    /// - SQLite: `SELECT sqlite_version()`.
    ///
    /// Mirrors Laravel's `DB::connection()->getServerVersion()`.
    pub async fn server_version() -> Result<String, FrameworkError> {
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_read(None, None, None).await?;
        let backend = exec.backend();
        let sql = match backend {
            DbBackend::Postgres | DbBackend::MySql => "SELECT VERSION() AS v",
            DbBackend::Sqlite => "SELECT sqlite_version() AS v",
            _ => return Err(super::unsupported_database_backend(backend)),
        };
        let stmt = Statement::from_sql_and_values(backend, sql, Vec::<SeaValue>::new());
        let row = exec
            .query_one(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?
            .ok_or_else(|| FrameworkError::database("DB::server_version: query returned no row"))?;
        row.try_get::<String>("", "v")
            .map_err(|e| FrameworkError::database(format!("DB::server_version: {e}")))
    }
}

/// Extract the database name from a SeaORM-style connection URL.
/// Used by [`DB::database_name`]; pulled out as a free function for
/// the unit test.
///
/// - `postgres://u:p@host/forge?sslmode=require` → `"forge"`
/// - `mysql://u:p@host:3306/laravel` → `"laravel"`
/// - `sqlite://./database.db` → `"./database.db"`
/// - `sqlite::memory:` → `":memory:"`
fn parse_database_name(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("sqlite://") {
        return rest.split('?').next().unwrap_or(rest).to_string();
    }
    if let Some(rest) = url.strip_prefix("sqlite:") {
        return rest.split('?').next().unwrap_or(rest).to_string();
    }
    // For postgres / mysql, the path component starts at the FIRST
    // single-slash AFTER the host segment. Skip past the scheme + the
    // authority `//user:pass@host:port/`, then take everything up to
    // the query string.
    if let Some(after_scheme) = url.split_once("://") {
        let after = after_scheme.1;
        if let Some((_, after_host)) = after.split_once('/') {
            return after_host
                .split('?')
                .next()
                .unwrap_or(after_host)
                .to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod where_clause_render_tests {
    //! The three `DbTableBuilder` renderers are private, so the MySQL
    //! `= binary` shape and the shared placeholder counter can only be
    //! pinned from inside the crate. The integration test
    //! (`framework/tests/query_binary_comparison.rs`) covers the
    //! SQLite refusal end to end through a live terminal.

    use super::*;

    #[test]
    fn mysql_renders_the_binary_operator_modifier() {
        let (sql, values) = DbTableBuilder::new("users")
            .where_binary("email", "Alice@example.com")
            .render_select(DbBackend::MySql)
            .expect("MySQL supports binary comparison");
        assert_eq!(sql, "SELECT * FROM users WHERE email = binary ?");
        assert_eq!(values.len(), 1, "the value stays bound; got: {values:?}");
    }

    #[test]
    fn mysql_renders_the_negated_binary_operator_modifier() {
        let (sql, _values) = DbTableBuilder::new("users")
            .where_not_binary("email", "Alice@example.com")
            .render_select(DbBackend::MySql)
            .expect("MySQL supports binary comparison");
        assert_eq!(sql, "SELECT * FROM users WHERE email != binary ?");
    }

    #[test]
    fn mysql_mixes_binary_and_plain_terms_in_one_clause() {
        let (sql, values) = DbTableBuilder::new("users")
            .filter("active", true)
            .where_binary("email", "Alice@example.com")
            .render_select(DbBackend::MySql)
            .expect("MySQL supports binary comparison");
        assert_eq!(
            sql,
            "SELECT * FROM users WHERE active = ? AND email = binary ?"
        );
        assert_eq!(values.len(), 2, "got: {values:?}");
    }

    #[test]
    fn postgres_and_sqlite_refuse_the_binary_term() {
        for backend in [DbBackend::Postgres, DbBackend::Sqlite] {
            let err = DbTableBuilder::new("users")
                .where_binary("email", "Alice@example.com")
                .render_select(backend)
                .expect_err("no binary comparison operator on this backend");
            assert!(
                format!("{err}").contains("where_binary is not supported"),
                "got: {err}"
            );
        }
    }

    #[test]
    fn delete_and_update_refuse_the_binary_term_too() {
        let err = DbTableBuilder::new("users")
            .where_binary("email", "Alice@example.com")
            .render_delete(DbBackend::Sqlite)
            .expect_err("DELETE renders through the same clause builder");
        assert!(
            format!("{err}").contains("where_binary is not supported"),
            "got: {err}"
        );

        let mut attrs = Attrs::new();
        attrs.insert("active", false);
        let err = DbTableBuilder::new("users")
            .where_binary("email", "Alice@example.com")
            .render_update(&attrs, DbBackend::Sqlite)
            .expect_err("UPDATE renders through the same clause builder");
        assert!(
            format!("{err}").contains("where_binary is not supported"),
            "got: {err}"
        );
    }

    #[test]
    fn postgres_placeholder_numbering_stays_monotonic_across_set_and_where() {
        // Regression guard for the render refactor: `render_update`
        // shares one counter between the SET list and the WHERE list,
        // so extracting the WHERE loop must not reset it.
        let mut attrs = Attrs::new();
        attrs.insert("name", "Bob");
        let (sql, values) = DbTableBuilder::new("users")
            .filter("id", 7i64)
            .render_update(&attrs, DbBackend::Postgres)
            .expect("no binary term, so Postgres renders");
        assert_eq!(sql, "UPDATE users SET name = $1 WHERE id = $2");
        assert_eq!(values.len(), 2, "got: {values:?}");
    }

    #[test]
    fn an_empty_builder_renders_no_where_clause() {
        let (sql, values) = DbTableBuilder::new("users")
            .render_select(DbBackend::Sqlite)
            .expect("no filters, nothing to refuse");
        assert_eq!(sql, "SELECT * FROM users");
        assert!(values.is_empty(), "got: {values:?}");
    }
}

#[cfg(test)]
mod parse_database_name_tests {
    use super::parse_database_name;

    #[test]
    fn postgres_url() {
        assert_eq!(
            parse_database_name("postgres://user:pass@localhost:5432/myapp"),
            "myapp"
        );
        assert_eq!(
            parse_database_name("postgres://localhost/db?sslmode=require"),
            "db"
        );
    }

    #[test]
    fn mysql_url() {
        assert_eq!(
            parse_database_name("mysql://root:secret@127.0.0.1:3306/laravel"),
            "laravel"
        );
    }

    #[test]
    fn sqlite_file() {
        assert_eq!(
            parse_database_name("sqlite://./database.db"),
            "./database.db"
        );
        assert_eq!(parse_database_name("sqlite::memory:"), ":memory:");
        assert_eq!(parse_database_name("sqlite://./db?mode=rwc"), "./db");
    }

    #[test]
    fn unknown_url() {
        assert_eq!(parse_database_name(""), "");
        assert_eq!(parse_database_name("not a url"), "");
    }
}
