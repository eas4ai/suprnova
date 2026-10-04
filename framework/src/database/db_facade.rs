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
//! Table names, column names, aliases, SQL operators, and ORDER BY
//! directions are written INTO the SQL string - they are NOT bound as
//! parameters (SQL doesn't allow that). Every one of them is checked
//! against [`validate_identifier`](crate::database::validate_identifier)
//! or the operator allowlist before the statement renders, and then
//! quoted for the backend (backticks on MySQL, double quotes elsewhere).
//! That is a safety net, not a licence: treat every `impl Into<String>`
//! identifier argument as a trusted, compile-time literal and do NOT
//! splice user input into table or column names. The raw fragments
//! ([`DbTableBuilder::select_raw`], [`DbTableBuilder::where_raw`]) are
//! written verbatim and are never checked. Values (the right-hand side of
//! `filter` / `filter_op`, `where_in` lists, join conditions, raw-fragment
//! bindings) ARE bound as parameters and safe to pass through from request
//! data. Explicit null write attributes are emitted as the constant SQL
//! literal `NULL` because their original Rust type is no longer available
//! in [`Attrs`].
//!
//! Backend-aware placeholder generation: `$N` (Postgres) vs `?`
//! (MySQL + SQLite). One counter runs through the whole statement - the
//! SET clause, joined subqueries, join conditions, the WHERE clause and
//! every subquery inside it - so each binding lines up with its position.

use crate::FrameworkError;
use crate::database::DB;
use crate::database::clauses::{
    Condition, Grouping, IntoWhereIn, JoinClause, JoinKind, JoinTarget, ReadSet, condition_tables,
    grouped, in_condition, join_tables, push_or, quote_identifier, raw_select_may_read,
    render_conditions, render_join, render_select_column, validate_condition, validate_join,
    validate_select_column,
};
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
///
/// SeaORM's decoder picks each column's Rust type from the type the
/// driver reports for the column. SQLite reports none for a computed
/// column - an aggregate, `COALESCE(...)`, any `select_raw` expression -
/// and SeaORM then drops the column whenever its value is not text. On
/// SQLite every column the decoder dropped is read again by the value's
/// own runtime type, so the row carries it.
fn query_result_to_dynamic_row(
    backend: DbBackend,
    qr: &sea_orm::QueryResult,
) -> Option<DynamicRow> {
    use sea_orm::FromQueryResult;
    let v = JsonValue::from_query_result(qr, "").ok()?;
    let serde_json::Value::Object(mut map) = v else {
        return None;
    };
    if backend == DbBackend::Sqlite {
        for column in qr.column_names() {
            if !map.contains_key(&column)
                && let Some(value) = sqlite_value_by_runtime_type(qr, &column)
            {
                map.insert(column, value);
            }
        }
    }
    Some(DynamicRow::from_map(map))
}

/// Read `column` as an integer, a real, text or a blob, whichever the
/// SQLite value is; a `NULL` reads as JSON `null`. The driver checks each
/// attempt against the value's runtime type, so only the matching one
/// succeeds. `None` when none does.
fn sqlite_value_by_runtime_type(
    qr: &sea_orm::QueryResult,
    column: &str,
) -> Option<serde_json::Value> {
    if let Ok(value) = qr.try_get::<Option<i64>>("", column) {
        return Some(serde_json::json!(value));
    }
    if let Ok(value) = qr.try_get::<Option<f64>>("", column) {
        return Some(serde_json::json!(value));
    }
    if let Ok(value) = qr.try_get::<Option<String>>("", column) {
        return Some(serde_json::json!(value));
    }
    if let Ok(value) = qr.try_get::<Option<Vec<u8>>>("", column) {
        return Some(serde_json::json!(value));
    }
    None
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

/// Bind `value`, written to `table.column`, and return its placeholder.
///
/// A JSON `u64` above `i64::MAX` binds as an unsigned number, which a
/// MySQL unsigned column stores exactly. Postgres and SQLite have no
/// integer column that holds it - as text, Postgres refused the statement
/// and SQLite stored a rounded real - so there the write is refused before
/// anything is sent, as a model's write is: a database error that names
/// the column, which a client sees as the generic 500 body.
fn write_value_expression(
    backend: DbBackend,
    (table, column): (&str, &str),
    value: &serde_json::Value,
    values: &mut Vec<SeaValue>,
    position: &mut usize,
) -> Result<String, FrameworkError> {
    if value.is_null() {
        return Ok("NULL".to_owned());
    }

    let bound = match value.as_u64() {
        Some(n) if n > i64::MAX as u64 => {
            let bound = SeaValue::BigUnsigned(Some(n));
            crate::eloquent::casts::unsigned::refuse_unsigned_overflow(
                backend, table, column, &bound,
            )
            .map_err(FrameworkError::database)?;
            bound
        }
        _ => crate::eloquent::model::json_value_to_sea_value(value),
    };
    *position += 1;
    values.push(bound);
    Ok(if backend == DbBackend::Postgres {
        format!("${position}")
    } else {
        "?".to_owned()
    })
}

/// One entry in a [`DbTableBuilder`]'s select list.
#[derive(Debug, Clone)]
enum SelectItem {
    /// A column, `table.column`, `*` or `table.*`, optionally `as alias`;
    /// validated and quoted.
    Column(String),
    /// A caller-written expression from [`DbTableBuilder::select_raw`],
    /// written verbatim.
    Raw(String),
}

/// Standalone query builder returned by
/// [`DB::table(name)`](crate::DB::table). Mirrors the where / order /
/// limit / select shape of [`Builder<M>`](crate::eloquent::Builder)
/// but materialises rows as [`DynamicRow`] instead of a typed model.
///
/// It is also the subquery type of both builders: pass one to
/// `join_sub`, `where_in` or `where_exists` and it renders inside the
/// outer statement with its values bound in place.
///
/// See the [module docs](self) for the trust boundary on identifiers.
#[derive(Debug, Clone)]
pub struct DbTableBuilder {
    table: String,
    joins: Vec<JoinClause>,
    conditions: Vec<Condition>,
    groups: Vec<String>,
    order: Vec<(String, Direction)>,
    limit_value: Option<u64>,
    offset_value: Option<u64>,
    select_items: Vec<SelectItem>,
    /// Phase 10C T12 - per-builder connection override. Set via
    /// [`Self::on`] or constructed pre-set via
    /// [`DB::table_on`](crate::DB::table_on). Routes terminal methods
    /// through the named connection in the
    /// [`ConnectionRegistry`](crate::database::ConnectionRegistry).
    /// A subquery runs on the outer statement's connection, so its own
    /// override is ignored.
    connection_override: Option<String>,
}

impl DbTableBuilder {
    /// Construct a builder for the given table. Prefer
    /// [`DB::table(name)`](crate::DB::table) - this is the underlying
    /// constructor.
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            joins: Vec::new(),
            conditions: Vec::new(),
            groups: Vec::new(),
            order: Vec::new(),
            limit_value: None,
            offset_value: None,
            select_items: Vec::new(),
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

    /// Restrict the SELECT to a specific column list, replacing any list
    /// set before. Empty means `*`. Each entry is a column, a
    /// `table.column`, `*` or `table.*`, optionally followed by
    /// `as alias` - the shape a join needs to tell two `name` columns
    /// apart.
    ///
    /// ```rust,no_run
    /// # use suprnova::DB;
    /// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// DB::table("audit_log").select(["id", "event"]).get().await?;
    /// DB::table("posts")
    ///     .join("users", "users.id", "=", "posts.author_id")
    ///     .select(["posts.title", "users.name as author"])
    ///     .get()
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn select<I, S>(mut self, cols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.select_items = cols
            .into_iter()
            .map(|s| SelectItem::Column(s.into()))
            .collect();
        self
    }

    /// Append a raw expression to the SELECT list (`COUNT(*) AS total`,
    /// `COALESCE(t.total, 0) AS total`), after any columns from
    /// [`Self::select`]. Like Laravel's `selectRaw`, and unlike the model
    /// builder's `select_raw`, it adds to the list rather than replacing
    /// it, so `select(["status_id"]).select_raw("COUNT(*) AS total")`
    /// selects both.
    ///
    /// # Security
    ///
    /// `raw` is written into the query verbatim and never checked.
    /// **Never pass user input here.**
    pub fn select_raw(mut self, raw: impl Into<String>) -> Self {
        self.select_items.push(SelectItem::Raw(raw.into()));
        self
    }

    /// Add a `WHERE col = ?` clause. Multiple `filter` calls AND together.
    pub fn filter(self, col: impl Into<String>, val: impl Into<SeaValue>) -> Self {
        self.filter_op(col, "=", val)
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
        self.conditions.push(Condition::Compare {
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
        self.conditions.push(Condition::Compare {
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
        self.conditions.push(Condition::Compare {
            column: col.into(),
            op: "!=".into(),
            value: SeaValue::String(Some(val.into())),
            binary: true,
        });
        self
    }

    /// Add a `WHERE first = second` clause comparing two columns, with
    /// no value. Inside a subquery it correlates with the outer query:
    /// `DB::table("posts").where_column("posts.author_id", "users.id")`
    /// under `DB::table("users").where_exists(...)`.
    pub fn where_column(mut self, first: impl Into<String>, second: impl Into<String>) -> Self {
        self.conditions.push(Condition::Columns {
            first: first.into(),
            op: "=".into(),
            second: second.into(),
        });
        self
    }

    /// Add a `WHERE col IN (...)` clause. `values` is a list of values,
    /// each bound as a parameter, or a [`DbTableBuilder`] used as a
    /// subquery whose own values bind in place. An empty list matches no
    /// row.
    pub fn where_in(mut self, col: impl Into<String>, values: impl IntoWhereIn<SeaValue>) -> Self {
        self.conditions
            .push(in_condition(col.into(), values.into_where_in(), false));
        self
    }

    /// Add a `WHERE col NOT IN (...)` clause; see [`Self::where_in`]. An
    /// empty list excludes no row.
    pub fn where_not_in(
        mut self,
        col: impl Into<String>,
        values: impl IntoWhereIn<SeaValue>,
    ) -> Self {
        self.conditions
            .push(in_condition(col.into(), values.into_where_in(), true));
        self
    }

    /// `OR col IN (...)`, folded into the condition before it; see
    /// [`Self::where_in`] for what `values` takes.
    pub fn or_where_in(
        mut self,
        col: impl Into<String>,
        values: impl IntoWhereIn<SeaValue>,
    ) -> Self {
        push_or(
            &mut self.conditions,
            in_condition(col.into(), values.into_where_in(), false),
        );
        self
    }

    /// `OR col NOT IN (...)`, folded into the condition before it.
    pub fn or_where_not_in(
        mut self,
        col: impl Into<String>,
        values: impl IntoWhereIn<SeaValue>,
    ) -> Self {
        push_or(
            &mut self.conditions,
            in_condition(col.into(), values.into_where_in(), true),
        );
        self
    }

    /// Add a `WHERE col IS NULL` clause.
    pub fn where_null(mut self, col: impl Into<String>) -> Self {
        self.conditions.push(Condition::Null {
            column: col.into(),
            negated: false,
        });
        self
    }

    /// Add a `WHERE col IS NOT NULL` clause.
    pub fn where_not_null(mut self, col: impl Into<String>) -> Self {
        self.conditions.push(Condition::Null {
            column: col.into(),
            negated: true,
        });
        self
    }

    /// `OR col IS NULL`, folded into the condition before it.
    pub fn or_where_null(mut self, col: impl Into<String>) -> Self {
        push_or(
            &mut self.conditions,
            Condition::Null {
                column: col.into(),
                negated: false,
            },
        );
        self
    }

    /// `OR col IS NOT NULL`, folded into the condition before it.
    pub fn or_where_not_null(mut self, col: impl Into<String>) -> Self {
        push_or(
            &mut self.conditions,
            Condition::Null {
                column: col.into(),
                negated: true,
            },
        );
        self
    }

    /// Add a raw `WHERE` fragment. Write each value as a portable `?`
    /// marker and pass it in `bindings`; on Postgres the markers are
    /// renumbered to their place in the statement. Use `??` for a
    /// literal question mark. A marker count that does not match
    /// `bindings` is an error before any I/O.
    ///
    /// The fragment is written as given, without parentheses, the same
    /// as Laravel's `whereRaw`: wrap a fragment that contains `OR` in
    /// parentheses yourself.
    ///
    /// # Security
    ///
    /// `sql` is written into the query verbatim. **Never pass user input
    /// as the fragment** - put it in `bindings`.
    pub fn where_raw(mut self, sql: impl Into<String>, bindings: Vec<SeaValue>) -> Self {
        self.conditions.push(Condition::Raw {
            sql: sql.into(),
            bindings,
        });
        self
    }

    /// `OR <sql>`, folded into the condition before it; see
    /// [`Self::where_raw`].
    pub fn or_where_raw(mut self, sql: impl Into<String>, bindings: Vec<SeaValue>) -> Self {
        push_or(
            &mut self.conditions,
            Condition::Raw {
                sql: sql.into(),
                bindings,
            },
        );
        self
    }

    fn push_grouped<I, S>(
        mut self,
        cols: I,
        op: impl Into<String>,
        val: impl Into<SeaValue>,
        grouping: Grouping,
        or: bool,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let cols: Vec<String> = cols.into_iter().map(Into::into).collect();
        let op: String = op.into();
        let value: SeaValue = val.into();
        if let Some(condition) = grouped(cols, &op, &value, grouping) {
            if or {
                push_or(&mut self.conditions, condition);
            } else {
                self.conditions.push(condition);
            }
        }
        self
    }

    /// Add `WHERE (c1 op ? OR c2 op ? ...)`: one comparison across
    /// several columns, true when any column matches. The parentheses
    /// keep the `OR` inside, so `filter("a", 1).where_any(["b", "c"], "=",
    /// 2)` never returns a row whose `a` is not 1. An empty column list
    /// adds no condition, as in Laravel.
    ///
    /// ```rust,no_run
    /// # use suprnova::DB;
    /// # async fn ex(search: &str) -> Result<(), Box<dyn std::error::Error>> {
    /// let rows = DB::table("products")
    ///     .where_any(["code", "description"], "like", format!("%{search}%"))
    ///     .get()
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn where_any<I, S>(self, cols: I, op: impl Into<String>, val: impl Into<SeaValue>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.push_grouped(cols, op, val, Grouping::Any, false)
    }

    /// `OR (c1 op ? OR c2 op ? ...)`, folded into the condition before
    /// it; see [`Self::where_any`].
    pub fn or_where_any<I, S>(
        self,
        cols: I,
        op: impl Into<String>,
        val: impl Into<SeaValue>,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.push_grouped(cols, op, val, Grouping::Any, true)
    }

    /// Add `WHERE (c1 op ? AND c2 op ? ...)`: true when every column
    /// matches. An empty column list adds no condition.
    pub fn where_all<I, S>(self, cols: I, op: impl Into<String>, val: impl Into<SeaValue>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.push_grouped(cols, op, val, Grouping::All, false)
    }

    /// `OR (c1 op ? AND c2 op ? ...)`, folded into the condition before
    /// it.
    pub fn or_where_all<I, S>(
        self,
        cols: I,
        op: impl Into<String>,
        val: impl Into<SeaValue>,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.push_grouped(cols, op, val, Grouping::All, true)
    }

    /// Add `WHERE NOT (c1 op ? OR c2 op ? ...)`: true when no column
    /// matches. An empty column list adds no condition.
    pub fn where_none<I, S>(self, cols: I, op: impl Into<String>, val: impl Into<SeaValue>) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.push_grouped(cols, op, val, Grouping::None, false)
    }

    /// `OR NOT (c1 op ? OR c2 op ? ...)`, folded into the condition
    /// before it.
    pub fn or_where_none<I, S>(
        self,
        cols: I,
        op: impl Into<String>,
        val: impl Into<SeaValue>,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.push_grouped(cols, op, val, Grouping::None, true)
    }

    /// Add `WHERE EXISTS (<query>)`. The subquery may correlate with this
    /// one through [`Self::where_column`]; its own values bind in place.
    ///
    /// ```rust,no_run
    /// # use suprnova::DB;
    /// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// let authors = DB::table("users")
    ///     .where_exists(
    ///         DB::table("posts")
    ///             .select_raw("1")
    ///             .where_column("posts.author_id", "users.id"),
    ///     )
    ///     .get()
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn where_exists(mut self, query: DbTableBuilder) -> Self {
        self.conditions.push(Condition::Exists {
            query: Box::new(query),
            negated: false,
        });
        self
    }

    /// Add `WHERE NOT EXISTS (<query>)`; see [`Self::where_exists`].
    pub fn where_not_exists(mut self, query: DbTableBuilder) -> Self {
        self.conditions.push(Condition::Exists {
            query: Box::new(query),
            negated: true,
        });
        self
    }

    /// Add a `GROUP BY col` term. Multiple calls chain in order.
    pub fn group_by(mut self, col: impl Into<String>) -> Self {
        self.groups.push(col.into());
        self
    }

    // ---- Joins ---------------------------------------------------------

    /// `INNER JOIN table ON first op second`. Name the table
    /// `"users as authors"` to join it under an alias.
    ///
    /// ```rust,no_run
    /// # use suprnova::DB;
    /// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// let rows = DB::table("posts")
    ///     .left_join("categories", "categories.id", "=", "posts.category_id")
    ///     .left_join("users as authors", "authors.id", "=", "posts.author_id")
    ///     .select(["posts.title", "categories.name as category", "authors.name as author"])
    ///     .order_by_asc("posts.title")
    ///     .get()
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn join(
        mut self,
        table: impl Into<String>,
        first: impl Into<String>,
        op: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        self.joins.push(JoinClause::table_on(
            JoinKind::Inner,
            table.into(),
            first.into(),
            op.into(),
            second.into(),
        ));
        self
    }

    /// `LEFT JOIN table ON first op second`; see [`Self::join`].
    pub fn left_join(
        mut self,
        table: impl Into<String>,
        first: impl Into<String>,
        op: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        self.joins.push(JoinClause::table_on(
            JoinKind::Left,
            table.into(),
            first.into(),
            op.into(),
            second.into(),
        ));
        self
    }

    /// `RIGHT JOIN table ON first op second`; see [`Self::join`]. SQLite
    /// supports right joins from version 3.39.
    pub fn right_join(
        mut self,
        table: impl Into<String>,
        first: impl Into<String>,
        op: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        self.joins.push(JoinClause::table_on(
            JoinKind::Right,
            table.into(),
            first.into(),
            op.into(),
            second.into(),
        ));
        self
    }

    /// `CROSS JOIN table`: every row paired with every row of `table`.
    pub fn cross_join(mut self, table: impl Into<String>) -> Self {
        self.joins.push(JoinClause::new(
            JoinKind::Cross,
            JoinTarget::Table(table.into()),
        ));
        self
    }

    /// `INNER JOIN table ON ...` with the conditions the closure adds to
    /// the [`JoinClause`] it receives: `on` / `or_on` for two columns,
    /// `filter` / `db_where` / `or_where` for a column and a bound value.
    /// A join the closure leaves without a condition is an error when the
    /// query runs.
    pub fn join_with(
        mut self,
        table: impl Into<String>,
        build: impl FnOnce(JoinClause) -> JoinClause,
    ) -> Self {
        self.joins.push(JoinClause::built_with(
            JoinKind::Inner,
            JoinTarget::Table(table.into()),
            build,
        ));
        self
    }

    /// `LEFT JOIN table ON ...`, conditions from the closure; see
    /// [`Self::join_with`].
    pub fn left_join_with(
        mut self,
        table: impl Into<String>,
        build: impl FnOnce(JoinClause) -> JoinClause,
    ) -> Self {
        self.joins.push(JoinClause::built_with(
            JoinKind::Left,
            JoinTarget::Table(table.into()),
            build,
        ));
        self
    }

    /// `RIGHT JOIN table ON ...`, conditions from the closure; see
    /// [`Self::join_with`].
    pub fn right_join_with(
        mut self,
        table: impl Into<String>,
        build: impl FnOnce(JoinClause) -> JoinClause,
    ) -> Self {
        self.joins.push(JoinClause::built_with(
            JoinKind::Right,
            JoinTarget::Table(table.into()),
            build,
        ));
        self
    }

    /// `INNER JOIN (<query>) AS alias ON first op second`: join a
    /// subquery under an alias. The subquery's values bind in place,
    /// ahead of the values of the conditions and `WHERE` clauses after it.
    pub fn join_sub(
        mut self,
        query: DbTableBuilder,
        alias: impl Into<String>,
        first: impl Into<String>,
        op: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        self.joins.push(JoinClause::query_on(
            JoinKind::Inner,
            query,
            alias.into(),
            first.into(),
            op.into(),
            second.into(),
        ));
        self
    }

    /// `LEFT JOIN (<query>) AS alias ON first op second`; see
    /// [`Self::join_sub`].
    ///
    /// ```rust,no_run
    /// # use suprnova::DB;
    /// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// let totals = DB::table("orders")
    ///     .select(["status_id"])
    ///     .select_raw("COUNT(*) AS total")
    ///     .group_by("status_id");
    /// let rows = DB::table("statuses")
    ///     .left_join_sub(totals, "order_totals", "order_totals.status_id", "=", "statuses.id")
    ///     .select(["statuses.name"])
    ///     .select_raw("COALESCE(order_totals.total, 0) AS total")
    ///     .get()
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn left_join_sub(
        mut self,
        query: DbTableBuilder,
        alias: impl Into<String>,
        first: impl Into<String>,
        op: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        self.joins.push(JoinClause::query_on(
            JoinKind::Left,
            query,
            alias.into(),
            first.into(),
            op.into(),
            second.into(),
        ));
        self
    }

    /// `INNER JOIN (<query>) AS alias ON ...`, conditions from the
    /// closure; see [`Self::join_with`] and [`Self::join_sub`].
    pub fn join_sub_with(
        mut self,
        query: DbTableBuilder,
        alias: impl Into<String>,
        build: impl FnOnce(JoinClause) -> JoinClause,
    ) -> Self {
        self.joins.push(JoinClause::built_with(
            JoinKind::Inner,
            JoinTarget::Query {
                query: Box::new(query),
                alias: alias.into(),
            },
            build,
        ));
        self
    }

    /// `LEFT JOIN (<query>) AS alias ON ...`, conditions from the
    /// closure; see [`Self::join_with`] and [`Self::join_sub`].
    pub fn left_join_sub_with(
        mut self,
        query: DbTableBuilder,
        alias: impl Into<String>,
        build: impl FnOnce(JoinClause) -> JoinClause,
    ) -> Self {
        self.joins.push(JoinClause::built_with(
            JoinKind::Left,
            JoinTarget::Query {
                query: Box::new(query),
                alias: alias.into(),
            },
            build,
        ));
        self
    }

    // ---- Ordering and windowing --------------------------------------

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

    /// Drop every ordering set so far. Laravel's `reorder()`: use it on
    /// a base query before reusing it as a subquery, or before ordering
    /// it another way.
    pub fn reorder(mut self) -> Self {
        self.order.clear();
        self
    }

    /// Drop every ordering set so far and order by `col` instead.
    /// Laravel's `reorder($column, $direction)`.
    pub fn reorder_by(mut self, col: impl Into<String>, direction: Direction) -> Self {
        self.order.clear();
        self.order.push((col.into(), direction));
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
    /// in this builder, its joins and its subqueries. Called by every
    /// terminal method before the SQL is rendered, and by an outer
    /// builder for the subqueries it holds. See
    /// [`identifier`](crate::database::identifier) for the contract.
    pub(crate) fn validate_inputs(&self) -> Result<(), FrameworkError> {
        crate::database::validate_identifier(&self.table)?;
        for item in &self.select_items {
            if let SelectItem::Column(col) = item {
                validate_select_column(col)?;
            }
        }
        for join in &self.joins {
            validate_join(join)?;
        }
        for condition in &self.conditions {
            validate_condition(condition)?;
        }
        for col in &self.groups {
            crate::database::validate_identifier(col)?;
        }
        for (col, _dir) in &self.order {
            crate::database::validate_identifier(col)?;
        }
        Ok(())
    }

    /// Every table this query reads: its own, each joined table, and
    /// every table a subquery reads, plus whether a `select_raw`,
    /// `where_raw` or `or_where_raw` fragment may read more. The render
    /// cache records each table, so a write to a joined table invalidates a
    /// cached page too; see [`ReadSet`] for what a raw fragment does.
    pub(crate) fn collect_tables(&self, out: &mut ReadSet) {
        out.tables.push(self.table.clone());
        if self
            .select_items
            .iter()
            .any(|item| matches!(item, SelectItem::Raw(raw) if raw_select_may_read(raw)))
        {
            out.raw_fragment = true;
        }
        for join in &self.joins {
            join_tables(join, out);
        }
        for condition in &self.conditions {
            condition_tables(condition, out);
        }
    }

    /// Record what [`Self::collect_tables`] finds on the render-cache
    /// collector.
    fn observe_reads(&self) {
        if !crate::render_cache::collector::is_active() {
            return;
        }
        let mut reads = ReadSet::default();
        self.collect_tables(&mut reads);
        reads.observe();
    }

    /// Refuse a write on a builder that carries joins. The `UPDATE` and
    /// `DELETE` this builder renders name one table and would ignore the
    /// joins, so they would touch rows the joins were there to exclude.
    fn refuse_joins(&self, operation: &str) -> Result<(), FrameworkError> {
        if self.joins.is_empty() {
            return Ok(());
        }
        Err(FrameworkError::database(format!(
            "DB::table(\"{}\")::{operation} does not support joins: the statement would \
             ignore them; filter with where_in or where_exists on a subquery instead",
            self.table
        )))
    }

    /// Execute the SELECT and return every matching row as a
    /// [`Collection<DynamicRow>`]. Materialises rows through the
    /// instrumented executor helpers - emits
    /// [`QueryExecuted`](crate::database::events::QueryExecuted) on
    /// every call.
    ///
    /// Inside a RenderCache render this records a read of every table
    /// the query reads - its own, each joined table and each table a
    /// subquery reads (`collector::observe_table_read`) - so an ORM or
    /// query-builder write to any of them later invalidates the cached
    /// representation. The tables are known here, which is what makes the
    /// observation precise; the raw `DB::select` family cannot name its
    /// tables and marks the render unstorable instead. So does a query
    /// carrying a [`Self::select_raw`], [`Self::where_raw`] or
    /// [`Self::or_where_raw`] fragment, anywhere in it, subqueries included:
    /// the fragment may read a table the builder does not name. A
    /// `select_raw` that is a bare integer, as in `select_raw("1")`, reads
    /// nothing and is the one exception. `first` funnels through this
    /// method.
    pub async fn get(self) -> Result<Collection<DynamicRow>, FrameworkError> {
        self.validate_inputs()?;
        self.observe_reads();
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
            .filter_map(|qr| query_result_to_dynamic_row(backend, qr))
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
    /// count. Ignores `select` / `order` / `limit` / `offset` - count
    /// semantics don't care about those. A grouped query counts its
    /// groups: the grouped SELECT runs as a subquery and the outer query
    /// counts its rows, because `COUNT(*)` beside a `GROUP BY` counts each
    /// group's rows instead.
    ///
    /// Uses `query_one` + `try_get` directly instead of
    /// `JsonValue::find_by_statement` because aggregate columns
    /// (`COUNT(*)`) don't always carry a type tag through sqlx's
    /// per-column type detection that backs `JsonValue`'s
    /// `FromQueryResult` impl - on SQLite the typed accessor is the
    /// reliable path.
    pub async fn count(self) -> Result<u64, FrameworkError> {
        self.validate_inputs()?;
        // Same observation as `get`; see its doc.
        self.observe_reads();
        // T11/T12: route through resolve_read.
        let exec = crate::database::transaction::ExecutorChoice::resolve_read(
            None,
            self.connection_override.as_deref(),
            None,
        )
        .await?;
        let backend = exec.backend();
        let mut copy = self;
        copy.order.clear();
        copy.limit_value = None;
        copy.offset_value = None;
        let (sql, values) = if copy.groups.is_empty() {
            copy.select_items = vec![SelectItem::Raw("COUNT(*) AS count".into())];
            copy.render_select(backend)?
        } else {
            copy.select_items = vec![SelectItem::Raw("1 AS __suprnova_group".into())];
            let (grouped, values) = copy.render_select(backend)?;
            (
                format!("SELECT COUNT(*) AS count FROM ({grouped}) AS __suprnova_groups"),
                values,
            )
        };
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
        let connection = self.connection_override.clone();
        crate::render_cache::orm::atomic(connection.as_deref(), || self.insert_inner(attrs)).await
    }

    async fn insert_inner(self, attrs: Attrs) -> Result<i64, FrameworkError> {
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
                write_value_expression(backend, (&self.table, c), v, &mut values, &mut position)
            })
            .collect::<Result<_, _>>()?;

        let base = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_identifier(backend, &self.table),
            cols.iter()
                .map(|c| quote_identifier(backend, c))
                .collect::<Vec<_>>()
                .join(", "),
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
    /// A builder with a join is refused with an error: the `UPDATE` names
    /// one table and would ignore the join. Narrow the rows with
    /// `where_in` or `where_exists` on a subquery instead.
    ///
    /// Dual-API: this is the Laravel-faithful name; the
    /// `Builder<M>`-style alias is [`Self::update_all`]. Both call into
    /// the same implementation. Prefer the `_all` name when the
    /// table-wide intent is the point of the call site - it makes the
    /// missing `filter` visible to reviewers. Non-null attributes are bound;
    /// explicit nulls use the constant SQL literal `NULL` to retain the target
    /// column type on PostgreSQL.
    pub async fn update(self, attrs: Attrs) -> Result<u64, FrameworkError> {
        let connection = self.connection_override.clone();
        crate::render_cache::orm::atomic(connection.as_deref(), || self.update_inner(attrs)).await
    }

    async fn update_inner(self, attrs: Attrs) -> Result<u64, FrameworkError> {
        if attrs.is_empty() {
            return Err(FrameworkError::database(format!(
                "DB::table(\"{}\")::update called with empty attrs",
                self.table
            )));
        }
        self.refuse_joins("update")?;
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
    /// A builder with a join is refused with an error, for the reason
    /// [`Self::update`] gives.
    ///
    /// Dual-API: this is the Laravel-faithful name; the
    /// `Builder<M>`-style alias is [`Self::delete_all`]. Both call into
    /// the same implementation. Prefer the `_all` name when the
    /// table-wide intent is the point of the call site.
    pub async fn delete(self) -> Result<u64, FrameworkError> {
        let connection = self.connection_override.clone();
        crate::render_cache::orm::atomic(connection.as_deref(), || self.delete_inner()).await
    }

    async fn delete_inner(self) -> Result<u64, FrameworkError> {
        self.refuse_joins("delete")?;
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

    /// Render the WHERE clause, including the leading ` WHERE `. Returns
    /// an empty string when the builder carries no conditions, so callers
    /// push the result unconditionally.
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
        if self.conditions.is_empty() {
            return Ok(String::new());
        }
        Ok(format!(
            " WHERE {}",
            render_conditions(&self.conditions, backend, values, counter)?
        ))
    }

    fn render_select(&self, backend: DbBackend) -> Result<(String, Vec<SeaValue>), FrameworkError> {
        let mut values: Vec<SeaValue> = Vec::new();
        let mut counter = 0usize;
        let sql = self.render_select_into(backend, &mut values, &mut counter)?;
        Ok((sql, values))
    }

    /// Render this SELECT into a statement that may already hold values:
    /// the top-level query starts the counter at zero, and a subquery
    /// continues the outer statement's counter, so its placeholders and
    /// values fall in place. Every part renders in text order - select
    /// list, joins, `WHERE`, `GROUP BY`, `ORDER BY` - so the values are
    /// pushed in the order their markers appear.
    pub(crate) fn render_select_into(
        &self,
        backend: DbBackend,
        values: &mut Vec<SeaValue>,
        counter: &mut usize,
    ) -> Result<String, FrameworkError> {
        let mut sql = String::new();
        sql.push_str("SELECT ");
        if self.select_items.is_empty() {
            sql.push('*');
        } else {
            let items = self
                .select_items
                .iter()
                .map(|item| match item {
                    SelectItem::Column(col) => render_select_column(backend, col),
                    SelectItem::Raw(raw) => Ok(raw.clone()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            sql.push_str(&items.join(", "));
        }
        sql.push_str(" FROM ");
        sql.push_str(&quote_identifier(backend, &self.table));

        for join in &self.joins {
            sql.push_str(&render_join(join, backend, values, counter)?);
        }

        sql.push_str(&self.render_where_clauses(backend, values, counter)?);

        if !self.groups.is_empty() {
            sql.push_str(" GROUP BY ");
            let groups: Vec<String> = self
                .groups
                .iter()
                .map(|col| quote_identifier(backend, col))
                .collect();
            sql.push_str(&groups.join(", "));
        }

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
                    format!("{} {dir_sql}", quote_identifier(backend, col))
                })
                .collect();
            sql.push_str(&order.join(", "));
        }

        sql.push_str(&super::clauses::render_limit_offset(
            backend,
            self.limit_value,
            self.offset_value,
        ));

        Ok(sql)
    }

    fn render_update(
        &self,
        attrs: &Attrs,
        backend: DbBackend,
    ) -> Result<(String, Vec<SeaValue>), FrameworkError> {
        let mut values: Vec<SeaValue> = Vec::new();
        let mut counter = 0usize;

        let mut sql = format!("UPDATE {} SET ", quote_identifier(backend, &self.table));
        let sets: Vec<String> = attrs
            .keys()
            .map(|col| {
                let v = attrs
                    .get(col)
                    .expect("key present in iter must be present in get");
                let expression = write_value_expression(
                    backend,
                    (&self.table, col),
                    v,
                    &mut values,
                    &mut counter,
                )?;
                Ok(format!("{} = {expression}", quote_identifier(backend, col)))
            })
            .collect::<Result<_, FrameworkError>>()?;
        sql.push_str(&sets.join(", "));

        sql.push_str(&self.render_where_clauses(backend, &mut values, &mut counter)?);

        Ok((sql, values))
    }

    fn render_delete(&self, backend: DbBackend) -> Result<(String, Vec<SeaValue>), FrameworkError> {
        let mut values: Vec<SeaValue> = Vec::new();
        let mut counter = 0usize;
        let mut sql = format!("DELETE FROM {}", quote_identifier(backend, &self.table));
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
    /// Inside a RenderCache render this marks the render unstorable
    /// (`collector::observe_unobservable_read`): the statement text is
    /// opaque to the framework, so the tables it read cannot be recorded and
    /// no later write could be relied on to invalidate the entry. The
    /// response is still served; it is just never cached. `select_one`,
    /// `scalar`, and `select_on` behave the same way; `DB::table(..).get()`
    /// knows its table and is observed precisely instead.
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
        crate::render_cache::collector::observe_unobservable_read();
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
            .filter_map(|qr| query_result_to_dynamic_row(backend, &qr))
            .collect())
    }

    /// Run a raw SELECT and return the FIRST row (or `None`).
    /// Mirrors Laravel's `DB::selectOne($sql, $bindings)`. Inside a
    /// RenderCache render this marks the render unstorable; see
    /// [`DB::select`].
    pub async fn select_one(
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<Option<DynamicRow>, FrameworkError> {
        crate::render_cache::collector::observe_unobservable_read();
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_read(None, None, None).await?;
        let backend = exec.backend();
        let stmt =
            Statement::from_sql_and_values(backend, sql, values.into_iter().collect::<Vec<_>>());
        let row = exec
            .query_one(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(row
            .as_ref()
            .and_then(|qr| query_result_to_dynamic_row(backend, qr)))
    }

    /// [`DB::select_one`] for a statement whose tables the caller knows:
    /// observes each named table precisely instead of marking the render
    /// unobservable.
    ///
    /// `pub(crate)` on purpose. The honest boundary for application raw SQL
    /// is unchanged - a caller who writes the statement is also the only
    /// one who could get its table list wrong, and a wrong list is worse
    /// than no list: it lets an entry be stored on a dependency set that
    /// nothing will ever advance. Only statements this crate owns, whose
    /// table lists are literals beside their SQL and are asserted against
    /// that SQL by a test, may use it.
    pub(crate) async fn select_one_observing(
        sql: &str,
        values: Vec<SeaValue>,
        tables: &[&str],
    ) -> Result<Option<DynamicRow>, FrameworkError> {
        for table in tables {
            crate::render_cache::collector::observe_table_read(table);
        }
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_read(None, None, None).await?;
        let backend = exec.backend();
        let stmt = Statement::from_sql_and_values(backend, sql, values);
        let row = exec
            .query_one(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(row
            .as_ref()
            .and_then(|qr| query_result_to_dynamic_row(backend, qr)))
    }

    /// Run a raw SELECT, return the FIRST column of the FIRST row.
    /// Mirrors Laravel's `DB::scalar($sql, $bindings)`.
    ///
    /// `T` is any [`ColumnValue`](crate::ColumnValue): a SeaORM
    /// `TryGetable` type, with `u64` read on every database. Inside a
    /// RenderCache render this marks the render unstorable; see
    /// [`DB::select`].
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
        T: crate::ColumnValue,
    {
        crate::render_cache::collector::observe_unobservable_read();
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
        T::from_column(&row, 0_usize).map_err(|e| {
            FrameworkError::database(format!("DB::scalar: {}", sea_orm::DbErr::from(e)))
        })
    }

    /// [`DB::scalar`] for a statement whose tables the caller knows. See
    /// [`DB::select_one_observing`] for why this is `pub(crate)`.
    pub(crate) async fn scalar_observing<T>(
        sql: &str,
        values: Vec<SeaValue>,
        tables: &[&str],
    ) -> Result<T, FrameworkError>
    where
        T: crate::ColumnValue,
    {
        for table in tables {
            crate::render_cache::collector::observe_table_read(table);
        }
        let exec =
            crate::database::transaction::ExecutorChoice::resolve_read(None, None, None).await?;
        let backend = exec.backend();
        let stmt = Statement::from_sql_and_values(backend, sql, values);
        let row = exec
            .query_one(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?
            .ok_or_else(|| {
                FrameworkError::database("DB::scalar_observing: query returned no rows")
            })?;
        T::from_column(&row, 0_usize).map_err(|e| {
            FrameworkError::database(format!("DB::scalar_observing: {}", sea_orm::DbErr::from(e)))
        })
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
        let values: Vec<SeaValue> = values.into_iter().collect();
        crate::render_cache::orm::atomic(None, || async move {
            let exec =
                crate::database::transaction::ExecutorChoice::resolve_write(None, None, None)
                    .await?;
            let backend = exec.backend();
            let stmt = Statement::from_sql_and_values(backend, sql, values);
            exec.run(stmt)
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            if !is_select_statement(sql) {
                crate::render_cache::orm::after_unknown_write().await?;
            }
            Ok(true)
        })
        .await
    }

    /// Run a raw, unprepared SQL statement (no placeholder binding).
    /// Mirrors Laravel's `DB::unprepared($sql)`. The string is
    /// executed VERBATIM - never splice user input into it.
    ///
    /// Necessary for DDL on backends that reject parameter-bound
    /// statements: `CREATE INDEX`, `ALTER TABLE`, `VACUUM`, etc.
    pub async fn unprepared(sql: &str) -> Result<bool, FrameworkError> {
        crate::render_cache::orm::atomic(None, || async move {
            use sea_orm::ConnectionTrait;
            let exec =
                crate::database::transaction::ExecutorChoice::resolve_write(None, None, None)
                    .await?;
            // Emit QueryExecuted for unprepared statements as well - they
            // are still queries from the observer's perspective.
            if super::events::is_dispatching() || !super::events::query_observation_active() {
                match &exec {
                    crate::database::transaction::ExecutorChoice::Tx(t, _) => {
                        t.execute_unprepared(sql).await
                    }
                    crate::database::transaction::ExecutorChoice::Pool(c, _) => {
                        c.inner().execute_unprepared(sql).await
                    }
                }
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            } else {
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
                res.map_err(|e| FrameworkError::database(e.to_string()))?;
            }
            if !is_select_statement(sql) {
                crate::render_cache::orm::after_unknown_write().await?;
            }
            Ok(true)
        })
        .await
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
        let values: Vec<SeaValue> = values.into_iter().collect();
        crate::render_cache::orm::atomic(None, || async move {
            let exec =
                crate::database::transaction::ExecutorChoice::resolve_write(None, None, None)
                    .await?;
            let backend = exec.backend();
            let stmt = Statement::from_sql_and_values(backend, sql, values);
            let result = exec
                .run(stmt)
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            crate::render_cache::orm::after_unknown_write().await?;
            Ok(result.rows_affected())
        })
        .await
    }

    /// [`DB::affecting_statement`] for a write whose single table the caller
    /// knows: advances that table rather than the broad authority every
    /// representation observes. See [`DB::select_one_observing`] for why
    /// this is `pub(crate)`.
    pub(crate) async fn affecting_statement_on_table(
        sql: &str,
        values: Vec<SeaValue>,
        table: &str,
    ) -> Result<u64, FrameworkError> {
        crate::render_cache::orm::atomic(None, || async move {
            let exec =
                crate::database::transaction::ExecutorChoice::resolve_write(None, None, None)
                    .await?;
            let backend = exec.backend();
            let stmt = Statement::from_sql_and_values(backend, sql, values);
            let result = exec
                .run(stmt)
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            crate::render_cache::orm::after_table_write(table).await?;
            Ok(result.rows_affected())
        })
        .await
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
    /// chain. Errors if `conn_name` isn't registered. Inside a RenderCache
    /// render this marks the render unstorable; see [`DB::select`]. The
    /// render cache's own generation ledger does not use this method for
    /// that reason: its reads go through `ExecutorChoice` directly.
    pub async fn select_on(
        conn_name: &str,
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<Vec<DynamicRow>, FrameworkError> {
        crate::render_cache::collector::observe_unobservable_read();
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
            .filter_map(|qr| query_result_to_dynamic_row(backend, &qr))
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
        let values: Vec<SeaValue> = values.into_iter().collect();
        crate::render_cache::orm::atomic(Some(conn_name), || async move {
            let exec = crate::database::transaction::ExecutorChoice::resolve_write(
                None,
                Some(conn_name),
                None,
            )
            .await?;
            let backend = exec.backend();
            let stmt = Statement::from_sql_and_values(backend, sql, values);
            exec.run(stmt)
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            if !is_select_statement(sql) {
                crate::render_cache::orm::after_unknown_write().await?;
            }
            Ok(true)
        })
        .await
    }

    /// Phase 10C T12 - `DB::affecting_statement` variant pinned to the
    /// named connection. INSERT / UPDATE / DELETE / UPSERT on a
    /// non-primary write target.
    pub async fn affecting_statement_on(
        conn_name: &str,
        sql: &str,
        values: impl IntoIterator<Item = SeaValue>,
    ) -> Result<u64, FrameworkError> {
        let values: Vec<SeaValue> = values.into_iter().collect();
        crate::render_cache::orm::atomic(Some(conn_name), || async move {
            let exec = crate::database::transaction::ExecutorChoice::resolve_write(
                None,
                Some(conn_name),
                None,
            )
            .await?;
            let backend = exec.backend();
            let stmt = Statement::from_sql_and_values(backend, sql, values);
            let result = exec
                .run(stmt)
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            crate::render_cache::orm::after_unknown_write().await?;
            Ok(result.rows_affected())
        })
        .await
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
    ///
    /// Inside a test container
    /// ([`TestContainer::fake`](crate::container::testing::TestContainer::fake) /
    /// [`TestContainer::scope`](crate::container::testing::TestContainer::scope))
    /// the listener belongs to that container: it fires for the queries
    /// run inside it and ends with it, so a test that counts queries
    /// counts its own even while other tests run in the same process. A
    /// listener registered outside any test container fires for every
    /// query of the process.
    pub fn listen<F>(callback: F) -> Result<(), FrameworkError>
    where
        F: Fn(&crate::database::events::QueryExecuted) + Send + Sync + 'static,
    {
        let observation = crate::database::events::current_observation();
        let mut reg = crate::lock::write(&observation.listeners, "db event listeners")?;
        reg.listeners.push(std::sync::Arc::new(callback));
        Ok(())
    }

    /// Remove every `DB::listen` callback of the current scope: the
    /// active test container's, or the application's outside one. Does
    /// NOT touch `EventFacade::listen` listeners - those go through
    /// [`EventFacade::forget`](crate::EventFacade) (the dispatcher's
    /// per-event forget surface).
    pub fn flush_listeners() -> Result<(), FrameworkError> {
        let observation = crate::database::events::current_observation();
        let mut reg = crate::lock::write(&observation.listeners, "db event listeners")?;
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
    ///
    /// Inside a test container the log is that container's, as a
    /// [`Self::listen`] callback is: it holds the queries run inside the
    /// container only. Every query-log method reads and changes the log
    /// of the scope it is called in.
    pub fn enable_query_log() -> Result<(), FrameworkError> {
        let observation = crate::database::events::current_observation();
        let mut log = crate::lock::lock(&observation.log, "query_log")?;
        log.enabled = true;
        Ok(())
    }

    /// Disable the in-memory query log. Existing buffered entries are
    /// retained; call [`Self::flush_query_log`] to drop them. Mirrors
    /// Laravel's `DB::disableQueryLog`.
    pub fn disable_query_log() -> Result<(), FrameworkError> {
        let observation = crate::database::events::current_observation();
        let mut log = crate::lock::lock(&observation.log, "query_log")?;
        log.enabled = false;
        Ok(())
    }

    /// True when the query log is currently active. Mirrors Laravel's
    /// `DB::logging()`.
    pub fn logging() -> bool {
        crate::database::events::current_observation()
            .log
            .lock()
            .map(|l| l.enabled)
            .unwrap_or(false)
    }

    /// Snapshot the captured query log. Returns a `Vec` of every
    /// `QueryExecuted` event since the log was enabled (or last
    /// flushed). Does NOT drain the buffer - call
    /// [`Self::flush_query_log`] to clear it.
    pub fn get_query_log() -> Result<Vec<crate::database::events::QueryExecuted>, FrameworkError> {
        let observation = crate::database::events::current_observation();
        let log = crate::lock::lock(&observation.log, "query_log")?;
        Ok(log.entries.clone())
    }

    /// Drop every captured entry from the query log. The log stays
    /// enabled - new queries will still be appended. Mirrors Laravel's
    /// `DB::flushQueryLog`.
    ///
    /// The log's buffer is released too, not only emptied: a long run of
    /// queries grows it, and a cleared `Vec` keeps that capacity for the
    /// life of the process.
    pub fn flush_query_log() -> Result<(), FrameworkError> {
        let observation = crate::database::events::current_observation();
        let mut log = crate::lock::lock(&observation.log, "query_log")?;
        log.entries = Vec::new();
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
            crate::database::DatabaseType::Mysql if cfg.names_mariadb() => "MariaDB",
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
        assert_eq!(sql, "SELECT * FROM `users` WHERE `email` = binary ?");
        assert_eq!(values.len(), 1, "the value stays bound; got: {values:?}");
    }

    #[test]
    fn mysql_renders_the_negated_binary_operator_modifier() {
        let (sql, _values) = DbTableBuilder::new("users")
            .where_not_binary("email", "Alice@example.com")
            .render_select(DbBackend::MySql)
            .expect("MySQL supports binary comparison");
        assert_eq!(sql, "SELECT * FROM `users` WHERE `email` != binary ?");
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
            "SELECT * FROM `users` WHERE `active` = ? AND `email` = binary ?"
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
        assert_eq!(sql, r#"UPDATE "users" SET "name" = $1 WHERE "id" = $2"#);
        assert_eq!(values.len(), 2, "got: {values:?}");
    }

    #[test]
    fn an_empty_builder_renders_no_where_clause() {
        let (sql, values) = DbTableBuilder::new("users")
            .render_select(DbBackend::Sqlite)
            .expect("no filters, nothing to refuse");
        assert_eq!(sql, r#"SELECT * FROM "users""#);
        assert!(values.is_empty(), "got: {values:?}");
    }

    /// Postgres numbers its placeholders, so a subquery's values must
    /// take the numbers of the positions they hold in the text: a joined
    /// subquery and the join's own condition come before the outer
    /// `WHERE`, and a `where_in` or `EXISTS` subquery sits between the
    /// outer conditions around it.
    #[test]
    fn postgres_numbers_subquery_values_in_text_order() {
        let (sql, values) = DbTableBuilder::new("statuses")
            .left_join_sub_with(
                DbTableBuilder::new("orders")
                    .select(["status_id"])
                    .filter_op("total", ">=", 15)
                    .group_by("status_id"),
                "big",
                |join| {
                    join.on("big.status_id", "=", "statuses.id")
                        .filter("big.region", "eu")
                },
            )
            .filter("statuses.kind", "open")
            .where_in(
                "statuses.id",
                DbTableBuilder::new("flags")
                    .select(["status_id"])
                    .filter("flag", "hot"),
            )
            .where_exists(DbTableBuilder::new("audits").filter("audits.level", 3))
            .filter("statuses.region", "us")
            .render_select(DbBackend::Postgres)
            .expect("renders for Postgres");
        assert_eq!(
            sql,
            concat!(
                r#"SELECT * FROM "statuses" LEFT JOIN (SELECT "status_id" FROM "orders" "#,
                r#"WHERE "total" >= $1 GROUP BY "status_id") AS "big" "#,
                r#"ON "big"."status_id" = "statuses"."id" AND "big"."region" = $2 "#,
                r#"WHERE "statuses"."kind" = $3 AND "statuses"."id" IN "#,
                r#"(SELECT "status_id" FROM "flags" WHERE "flag" = $4) "#,
                r#"AND EXISTS (SELECT * FROM "audits" WHERE "audits"."level" = $5) "#,
                r#"AND "statuses"."region" = $6"#,
            )
        );
        assert_eq!(
            values,
            vec![
                SeaValue::from(15),
                SeaValue::from("eu"),
                SeaValue::from("open"),
                SeaValue::from("hot"),
                SeaValue::from(3),
                SeaValue::from("us"),
            ]
        );
    }

    #[test]
    fn grouped_helpers_and_or_forms_render_their_parentheses() {
        let (sql, values) = DbTableBuilder::new("items")
            .filter("a", 1)
            .where_any(["b", "c"], "=", 2)
            .or_where_null("label")
            .where_none(["d", "e"], "like", "x%")
            .or_where_raw("f + g = ?", vec![SeaValue::from(4)])
            .reorder_by("id", Direction::Desc)
            .render_select(DbBackend::Postgres)
            .expect("renders for Postgres");
        assert_eq!(
            sql,
            concat!(
                r#"SELECT * FROM "items" WHERE "a" = $1 "#,
                r#"AND ("b" = $2 OR "c" = $3 OR "label" IS NULL) "#,
                r#"AND (NOT ("d" like $4 OR "e" like $5) OR f + g = $6) "#,
                r#"ORDER BY "id" DESC"#,
            )
        );
        assert_eq!(values.len(), 6, "got: {values:?}");
    }

    #[test]
    fn a_select_list_quotes_columns_and_aliases_and_keeps_raw_entries() {
        let (sql, _) = DbTableBuilder::new("posts")
            .join("users as authors", "authors.id", "=", "posts.author_id")
            .cross_join("tags")
            .select(["posts.*", "authors.name as author"])
            .select_raw("COUNT(*) AS total")
            .group_by("posts.id")
            .render_select(DbBackend::MySql)
            .expect("renders for MySQL");
        assert_eq!(
            sql,
            concat!(
                "SELECT `posts`.*, `authors`.`name` AS `author`, COUNT(*) AS total ",
                "FROM `posts` INNER JOIN `users` AS `authors` ON `authors`.`id` = `posts`.`author_id` ",
                "CROSS JOIN `tags` GROUP BY `posts`.`id`",
            )
        );
    }

    #[test]
    fn every_table_a_query_reads_is_collected() {
        let builder = DbTableBuilder::new("posts")
            .join("users as authors", "authors.id", "=", "posts.author_id")
            .left_join_sub(
                DbTableBuilder::new("orders").where_exists(DbTableBuilder::new("audits")),
                "o",
                "o.id",
                "=",
                "posts.id",
            )
            .or_where_in("posts.id", DbTableBuilder::new("pins").select(["post_id"]));
        let mut reads = ReadSet::default();
        builder.collect_tables(&mut reads);
        assert_eq!(
            reads.tables,
            vec!["posts", "users", "orders", "audits", "pins"]
        );
        assert!(!reads.raw_fragment);
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

#[cfg(test)]
mod mem_audit {
    use super::*;
    use crate::container::testing::TestContainer;

    /// MEM-001: flushing the query log releases its buffer.
    #[tokio::test]
    async fn mem_audit_flushing_the_query_log_releases_its_buffer() {
        let _container = TestContainer::fake();
        DB::enable_query_log().unwrap();
        {
            let observation = crate::database::events::current_observation();
            let mut log = observation.log.lock().unwrap();
            log.entries.reserve(10_000);
        }
        DB::flush_query_log().unwrap();
        assert!(DB::get_query_log().unwrap().is_empty());
        let observation = crate::database::events::current_observation();
        assert_eq!(observation.log.lock().unwrap().entries.capacity(), 0);
        DB::disable_query_log().unwrap();
    }
}
