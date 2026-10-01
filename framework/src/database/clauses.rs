//! The `WHERE` and `JOIN` pieces that [`DbTableBuilder`] and the model
//! builder [`Builder<M>`](crate::eloquent::Builder) share.
//!
//! [`DbTableBuilder`] keeps its conditions as [`Condition`]s, and both
//! builders keep their joins as [`JoinClause`]s, so a join renders the same
//! way whichever builder holds it. A subquery - in a join, an `IN (...)`
//! or an `EXISTS (...)` - is always a [`DbTableBuilder`], rendered inline
//! with the outer statement's placeholder counter: Postgres `$N` numbers
//! continue through the subquery, and on SQLite and MySQL the values are
//! pushed in the order their `?` markers appear in the text.
//!
//! Every identifier is checked by
//! [`validate_identifier`](crate::database::validate_identifier) before it
//! renders, then quoted for the backend: backticks on MySQL, double quotes
//! elsewhere. Every value is a bound parameter.

use sea_orm::{DbBackend, Value as SeaValue};

use crate::FrameworkError;
use crate::database::db_facade::DbTableBuilder;
use crate::database::placeholder::placeholder;
use crate::database::{validate_identifier, validate_sql_operator};
use crate::eloquent::builder::{IntoVal, rewrite_raw_placeholders, validate_raw_placeholders};

// ---- Identifiers -------------------------------------------------------------

/// `ident` quoted for `backend`, one segment at a time, so `posts.title`
/// becomes `"posts"."title"` and `posts.*` becomes `"posts".*`. The quote
/// character is doubled inside a segment, though a validated identifier
/// never contains one.
pub(crate) fn quote_identifier(backend: DbBackend, ident: &str) -> String {
    let quote = if backend == DbBackend::MySql {
        '`'
    } else {
        '"'
    };
    ident
        .split('.')
        .map(|segment| {
            if segment == "*" {
                "*".to_owned()
            } else {
                let doubled = segment.replace(quote, &format!("{quote}{quote}"));
                format!("{quote}{doubled}{quote}")
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Split `name` or `name as alias` (any case, any run of spaces) into
/// its two parts. Anything else is an error, so a stray word can never
/// reach the SQL text.
fn split_alias(item: &str) -> Result<(&str, Option<&str>), FrameworkError> {
    let parts: Vec<&str> = item.split_whitespace().collect();
    match parts.as_slice() {
        [name] => Ok((name, None)),
        [name, keyword, alias] if keyword.eq_ignore_ascii_case("as") => Ok((name, Some(alias))),
        _ => Err(FrameworkError::param(format!(
            "SQL identifier '{item}' is not a name or `name as alias`"
        ))),
    }
}

/// An alias is one identifier segment: no schema, no table prefix.
fn validate_alias(alias: &str) -> Result<(), FrameworkError> {
    validate_identifier(alias)?;
    if alias.contains('.') {
        return Err(FrameworkError::param(format!(
            "alias `{alias}` must be a single name without a `.`"
        )));
    }
    Ok(())
}

/// A select-list entry: a column, `table.column`, `*` or `table.*`,
/// optionally followed by `as alias`.
pub(crate) fn validate_select_column(item: &str) -> Result<(), FrameworkError> {
    let (column, alias) = split_alias(item)?;
    if column != "*" {
        match column.strip_suffix(".*") {
            Some(table) => {
                validate_identifier(table)?;
            }
            None => {
                validate_identifier(column)?;
            }
        }
    }
    if let Some(alias) = alias {
        validate_alias(alias)?;
    }
    Ok(())
}

/// A validated select-list entry, quoted for `backend`.
pub(crate) fn render_select_column(
    backend: DbBackend,
    item: &str,
) -> Result<String, FrameworkError> {
    let (column, alias) = split_alias(item)?;
    let column = quote_identifier(backend, column);
    Ok(match alias {
        Some(alias) => format!("{column} AS {}", quote_identifier(backend, alias)),
        None => column,
    })
}

/// A table, optionally followed by `as alias`.
fn validate_aliased_table(item: &str) -> Result<(), FrameworkError> {
    let (table, alias) = split_alias(item)?;
    validate_identifier(table)?;
    if let Some(alias) = alias {
        validate_alias(alias)?;
    }
    Ok(())
}

fn render_aliased_table(backend: DbBackend, item: &str) -> Result<String, FrameworkError> {
    let (table, alias) = split_alias(item)?;
    let table = quote_identifier(backend, table);
    Ok(match alias {
        Some(alias) => format!("{table} AS {}", quote_identifier(backend, alias)),
        None => table,
    })
}

/// The table an `item` names, without its alias: the name the render
/// cache records a read of. `None` when the entry does not parse, which
/// validation reports before any read happens.
fn bare_table(item: &str) -> Option<&str> {
    split_alias(item).ok().map(|(table, _)| table)
}

// ---- Conditions -------------------------------------------------------------

/// One condition in a [`DbTableBuilder`]'s `WHERE` list or in a join's
/// `ON` list. The list joins with `AND`; an `or_*` call folds the new
/// condition into the one before it as an [`Condition::Any`] group, so an
/// `OR` only ever widens that one condition.
#[derive(Debug, Clone)]
pub(crate) enum Condition {
    /// `column op ?`. `binary` renders MySQL's `column op binary ?`,
    /// which every other backend refuses.
    Compare {
        column: String,
        op: String,
        value: SeaValue,
        binary: bool,
    },
    /// `first op second`, two columns and no value.
    Columns {
        first: String,
        op: String,
        second: String,
    },
    /// `column [NOT] IN (?, ?, ...)`. An empty list renders the constant
    /// `1 = 0` (or `1 = 1` when negated) so the SQL stays valid.
    In {
        column: String,
        values: Vec<SeaValue>,
        negated: bool,
    },
    /// `column [NOT] IN (<subquery>)`.
    InQuery {
        column: String,
        query: Box<DbTableBuilder>,
        negated: bool,
    },
    /// `column IS [NOT] NULL`.
    Null { column: String, negated: bool },
    /// A caller-written fragment with portable `?` markers.
    Raw {
        sql: String,
        bindings: Vec<SeaValue>,
    },
    /// `[NOT] EXISTS (<subquery>)`.
    Exists {
        query: Box<DbTableBuilder>,
        negated: bool,
    },
    /// `(c1 OR c2 ...)`, one atom wherever it sits.
    Any(Vec<Condition>),
    /// `(c1 AND c2 ...)`, one atom wherever it sits.
    All(Vec<Condition>),
    /// `NOT (c)`.
    Not(Box<Condition>),
}

/// How [`grouped`] combines one comparison across several columns.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Grouping {
    /// At least one column matches: `where_any`.
    Any,
    /// Every column matches: `where_all`.
    All,
    /// No column matches: `where_none`.
    None,
}

/// The condition `where_any` / `where_all` / `where_none` add: one
/// comparison per column, grouped in parentheses so an `OR` inside never
/// reaches the conditions around it. `None` for an empty column list,
/// which adds no condition at all, as in Laravel.
pub(crate) fn grouped(
    columns: Vec<String>,
    op: &str,
    value: &SeaValue,
    grouping: Grouping,
) -> Option<Condition> {
    if columns.is_empty() {
        return None;
    }
    let comparisons = columns
        .into_iter()
        .map(|column| Condition::Compare {
            column,
            op: op.to_owned(),
            value: value.clone(),
            binary: false,
        })
        .collect();
    Some(match grouping {
        Grouping::Any => Condition::Any(comparisons),
        Grouping::All => Condition::All(comparisons),
        Grouping::None => Condition::Not(Box::new(Condition::Any(comparisons))),
    })
}

/// Fold `condition` into the last one in `conditions` as a disjunction:
/// into a trailing [`Condition::Any`] group so a run of `or_*` calls stays
/// flat, around a plain condition otherwise, and as a plain condition when
/// the list is empty.
pub(crate) fn push_or(conditions: &mut Vec<Condition>, condition: Condition) {
    match conditions.pop() {
        Some(Condition::Any(mut group)) => {
            group.push(condition);
            conditions.push(Condition::Any(group));
        }
        Some(last) => conditions.push(Condition::Any(vec![last, condition])),
        None => conditions.push(condition),
    }
}

/// Check every identifier and operator in `condition`, recursing into
/// groups and subqueries.
pub(crate) fn validate_condition(condition: &Condition) -> Result<(), FrameworkError> {
    match condition {
        Condition::Compare { column, op, .. } => {
            validate_identifier(column)?;
            validate_sql_operator(op)?;
        }
        Condition::Columns { first, op, second } => {
            validate_identifier(first)?;
            validate_sql_operator(op)?;
            validate_identifier(second)?;
        }
        Condition::In { column, .. } | Condition::Null { column, .. } => {
            validate_identifier(column)?;
        }
        Condition::InQuery { column, query, .. } => {
            validate_identifier(column)?;
            query.validate_inputs()?;
        }
        Condition::Raw { sql, bindings } => validate_raw_placeholders(sql, bindings.len())?,
        Condition::Exists { query, .. } => query.validate_inputs()?,
        Condition::Any(conditions) | Condition::All(conditions) => {
            for condition in conditions {
                validate_condition(condition)?;
            }
        }
        Condition::Not(condition) => validate_condition(condition)?,
    }
    Ok(())
}

/// Bind `value` and return its placeholder.
fn bind(
    backend: DbBackend,
    value: &SeaValue,
    values: &mut Vec<SeaValue>,
    n: &mut usize,
) -> Result<String, FrameworkError> {
    *n += 1;
    values.push(value.clone());
    placeholder(backend, *n)
}

/// Render `conditions` joined with `AND`, binding their values in order.
pub(crate) fn render_conditions(
    conditions: &[Condition],
    backend: DbBackend,
    values: &mut Vec<SeaValue>,
    n: &mut usize,
) -> Result<String, FrameworkError> {
    let parts = conditions
        .iter()
        .map(|condition| render_condition(condition, backend, values, n))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join(" AND "))
}

fn render_group(
    conditions: &[Condition],
    joiner: &str,
    empty: &str,
    backend: DbBackend,
    values: &mut Vec<SeaValue>,
    n: &mut usize,
) -> Result<String, FrameworkError> {
    if conditions.is_empty() {
        return Ok(empty.to_owned());
    }
    let parts = conditions
        .iter()
        .map(|condition| render_condition(condition, backend, values, n))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!("({})", parts.join(joiner)))
}

fn render_condition(
    condition: &Condition,
    backend: DbBackend,
    values: &mut Vec<SeaValue>,
    n: &mut usize,
) -> Result<String, FrameworkError> {
    Ok(match condition {
        Condition::Compare {
            column,
            op,
            value,
            binary,
        } => {
            let ph = bind(backend, value, values, n)?;
            let op = if *binary {
                match backend {
                    DbBackend::MySql => format!("{op} binary"),
                    _ => return Err(crate::database::binary_comparison_unsupported(backend)),
                }
            } else {
                op.clone()
            };
            format!("{} {op} {ph}", quote_identifier(backend, column))
        }
        Condition::Columns { first, op, second } => format!(
            "{} {op} {}",
            quote_identifier(backend, first),
            quote_identifier(backend, second)
        ),
        Condition::In {
            column,
            values: list,
            negated,
        } => {
            if list.is_empty() {
                return Ok(if *negated { "1 = 1" } else { "1 = 0" }.to_owned());
            }
            let placeholders = list
                .iter()
                .map(|value| bind(backend, value, values, n))
                .collect::<Result<Vec<_>, _>>()?;
            let not = if *negated { "NOT " } else { "" };
            format!(
                "{} {not}IN ({})",
                quote_identifier(backend, column),
                placeholders.join(", ")
            )
        }
        Condition::InQuery {
            column,
            query,
            negated,
        } => {
            let column = quote_identifier(backend, column);
            let not = if *negated { "NOT " } else { "" };
            let subquery = query.render_select_into(backend, values, n)?;
            format!("{column} {not}IN ({subquery})")
        }
        Condition::Null { column, negated } => {
            let not = if *negated { "NOT " } else { "" };
            format!("{} IS {not}NULL", quote_identifier(backend, column))
        }
        Condition::Raw { sql, bindings } => {
            let rendered = rewrite_raw_placeholders(backend, sql, bindings.len(), *n)?;
            for value in bindings {
                *n += 1;
                values.push(value.clone());
            }
            rendered
        }
        Condition::Exists { query, negated } => {
            let not = if *negated { "NOT " } else { "" };
            let subquery = query.render_select_into(backend, values, n)?;
            format!("{not}EXISTS ({subquery})")
        }
        // `OR` of nothing is false; `AND` of nothing is true. The public
        // surface never builds an empty group - `grouped` drops an empty
        // column list - but `()` is a syntax error, so these stay valid.
        Condition::Any(conditions) => {
            render_group(conditions, " OR ", "1 = 0", backend, values, n)?
        }
        Condition::All(conditions) => {
            render_group(conditions, " AND ", "1 = 1", backend, values, n)?
        }
        Condition::Not(condition) => {
            let inner = render_condition(condition, backend, values, n)?;
            match condition.as_ref() {
                // A non-empty group already renders inside parentheses.
                Condition::Any(group) | Condition::All(group) if !group.is_empty() => {
                    format!("NOT {inner}")
                }
                _ => format!("NOT ({inner})"),
            }
        }
    })
}

/// Every table `condition` reads through a subquery.
pub(crate) fn condition_tables(condition: &Condition, out: &mut Vec<String>) {
    match condition {
        Condition::InQuery { query, .. } | Condition::Exists { query, .. } => {
            query.collect_tables(out);
        }
        Condition::Any(conditions) | Condition::All(conditions) => {
            for condition in conditions {
                condition_tables(condition, out);
            }
        }
        Condition::Not(condition) => condition_tables(condition, out),
        Condition::Compare { .. }
        | Condition::Columns { .. }
        | Condition::In { .. }
        | Condition::Null { .. }
        | Condition::Raw { .. } => {}
    }
}

// ---- where_in right-hand side -------------------------------------------------

/// The right-hand side of `where_in` and its siblings: a list of values,
/// or a subquery whose one column supplies them. Built by
/// [`IntoWhereIn`]; callers pass a list or a [`DbTableBuilder`] and never
/// name this type.
pub enum WhereIn<T> {
    /// A list of values, each bound as its own parameter.
    Values(Vec<T>),
    /// A subquery, rendered inside the `IN (...)` with its own values
    /// bound in place.
    Query(Box<DbTableBuilder>),
}

/// What `where_in`, `where_not_in`, `or_where_in` and `or_where_not_in`
/// accept: any list of values (`[1, 2]`, a `Vec`, an iterator), or a
/// [`DbTableBuilder`] used as a subquery, as Laravel's `whereIn` accepts
/// an array or a query.
///
/// `T` is the value type of the builder taking it: `sea_orm::Value` for
/// [`DbTableBuilder`], `serde_json::Value` for the model builder.
///
/// ```rust,no_run
/// # use suprnova::DB;
/// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
/// let booked = DB::table("rooms")
///     .where_in("id", DB::table("slots").select(["room_id"]).filter("day", "mon"))
///     .or_where_in("id", [7i64, 9])
///     .get()
///     .await?;
/// # Ok(()) }
/// ```
pub trait IntoWhereIn<T> {
    /// Convert `self` into a value list or a subquery.
    fn into_where_in(self) -> WhereIn<T>;
}

impl<I, V> IntoWhereIn<SeaValue> for I
where
    I: IntoIterator<Item = V>,
    V: Into<SeaValue>,
{
    fn into_where_in(self) -> WhereIn<SeaValue> {
        WhereIn::Values(self.into_iter().map(Into::into).collect())
    }
}

impl<I, V> IntoWhereIn<serde_json::Value> for I
where
    I: IntoIterator<Item = V>,
    V: IntoVal,
{
    fn into_where_in(self) -> WhereIn<serde_json::Value> {
        WhereIn::Values(self.into_iter().map(IntoVal::into_val).collect())
    }
}

impl<T> IntoWhereIn<T> for DbTableBuilder {
    fn into_where_in(self) -> WhereIn<T> {
        WhereIn::Query(Box::new(self))
    }
}

/// The [`DbTableBuilder`] condition for `column [NOT] IN <source>`.
pub(crate) fn in_condition(column: String, source: WhereIn<SeaValue>, negated: bool) -> Condition {
    match source {
        WhereIn::Values(values) => Condition::In {
            column,
            values,
            negated,
        },
        WhereIn::Query(query) => Condition::InQuery {
            column,
            query,
            negated,
        },
    }
}

// ---- Joins ---------------------------------------------------------------------

/// Which rows a join keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JoinKind {
    Inner,
    Left,
    Right,
    Cross,
}

impl JoinKind {
    fn keyword(self) -> &'static str {
        match self {
            JoinKind::Inner => "INNER JOIN",
            JoinKind::Left => "LEFT JOIN",
            JoinKind::Right => "RIGHT JOIN",
            JoinKind::Cross => "CROSS JOIN",
        }
    }
}

/// What a join reads: a table, written `table` or `table as alias`, or a
/// subquery under an alias.
#[derive(Debug, Clone)]
pub(crate) enum JoinTarget {
    Table(String),
    Query {
        query: Box<DbTableBuilder>,
        alias: String,
    },
}

/// One join and its `ON` conditions. The closure forms of the join
/// methods - [`DbTableBuilder::join_with`] and its siblings on both
/// builders - hand one of these over to add conditions to:
///
/// ```rust,no_run
/// # use suprnova::DB;
/// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
/// let rows = DB::table("posts")
///     .join_with("users", |join| {
///         join.on("users.id", "=", "posts.author_id")
///             .filter("users.active", true)
///             .or_where("users.role", "admin")
///     })
///     .get()
///     .await?;
/// # Ok(()) }
/// ```
///
/// `on` compares two columns; `filter` / `db_where` compare a column with
/// a value, which is always bound as a parameter. The conditions join
/// with `AND`. Each `or_*` method folds its condition into the one before
/// it, so `on(a).filter(b).or_where(c)` reads `a AND (b OR c)`: an `OR`
/// widens one condition and never the whole `ON` clause. That is the same
/// rule the builders' own `or_where` follows.
#[derive(Debug, Clone)]
pub struct JoinClause {
    pub(crate) kind: JoinKind,
    pub(crate) target: JoinTarget,
    pub(crate) conditions: Vec<Condition>,
}

impl JoinClause {
    pub(crate) fn new(kind: JoinKind, target: JoinTarget) -> Self {
        Self {
            kind,
            target,
            conditions: Vec::new(),
        }
    }

    /// The join a `join(table, first, op, second)` call adds.
    pub(crate) fn table_on(
        kind: JoinKind,
        table: String,
        first: String,
        op: String,
        second: String,
    ) -> Self {
        Self::new(kind, JoinTarget::Table(table)).on(first, op, second)
    }

    /// The join a `join_sub(query, alias, first, op, second)` call adds.
    pub(crate) fn query_on(
        kind: JoinKind,
        query: DbTableBuilder,
        alias: String,
        first: String,
        op: String,
        second: String,
    ) -> Self {
        Self::new(
            kind,
            JoinTarget::Query {
                query: Box::new(query),
                alias,
            },
        )
        .on(first, op, second)
    }

    /// The join a closure form adds: the closure receives the empty join
    /// and returns it with its conditions.
    pub(crate) fn built_with(
        kind: JoinKind,
        target: JoinTarget,
        build: impl FnOnce(JoinClause) -> JoinClause,
    ) -> Self {
        build(Self::new(kind, target))
    }

    fn push(mut self, condition: Condition) -> Self {
        self.conditions.push(condition);
        self
    }

    fn push_or(mut self, condition: Condition) -> Self {
        push_or(&mut self.conditions, condition);
        self
    }

    /// `ON first op second`: compare two columns.
    pub fn on(
        self,
        first: impl Into<String>,
        op: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        self.push(Condition::Columns {
            first: first.into(),
            op: op.into(),
            second: second.into(),
        })
    }

    /// `OR first op second`, folded into the condition before it.
    pub fn or_on(
        self,
        first: impl Into<String>,
        op: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        self.push_or(Condition::Columns {
            first: first.into(),
            op: op.into(),
            second: second.into(),
        })
    }

    /// `AND column = ?`, with the value bound.
    pub fn filter(self, column: impl Into<String>, value: impl Into<SeaValue>) -> Self {
        self.filter_op(column, "=", value)
    }

    /// `AND column op ?`, with the value bound. The operator comes from
    /// the same allowlist as [`DbTableBuilder::filter_op`].
    pub fn filter_op(
        self,
        column: impl Into<String>,
        op: impl Into<String>,
        value: impl Into<SeaValue>,
    ) -> Self {
        self.push(Condition::Compare {
            column: column.into(),
            op: op.into(),
            value: value.into(),
            binary: false,
        })
    }

    /// `OR column = ?`, folded into the condition before it.
    pub fn or_filter(self, column: impl Into<String>, value: impl Into<SeaValue>) -> Self {
        self.or_filter_op(column, "=", value)
    }

    /// `OR column op ?`, folded into the condition before it.
    pub fn or_filter_op(
        self,
        column: impl Into<String>,
        op: impl Into<String>,
        value: impl Into<SeaValue>,
    ) -> Self {
        self.push_or(Condition::Compare {
            column: column.into(),
            op: op.into(),
            value: value.into(),
            binary: false,
        })
    }

    /// Laravel-shape name for [`Self::filter`] (`where` is a Rust keyword).
    #[doc(alias = "where")]
    pub fn db_where(self, column: impl Into<String>, value: impl Into<SeaValue>) -> Self {
        self.filter(column, value)
    }

    /// Laravel-shape name for [`Self::filter_op`].
    pub fn db_where_op(
        self,
        column: impl Into<String>,
        op: impl Into<String>,
        value: impl Into<SeaValue>,
    ) -> Self {
        self.filter_op(column, op, value)
    }

    /// Laravel-shape name for [`Self::or_filter`].
    pub fn or_where(self, column: impl Into<String>, value: impl Into<SeaValue>) -> Self {
        self.or_filter(column, value)
    }

    /// Laravel-shape name for [`Self::or_filter_op`].
    pub fn or_where_op(
        self,
        column: impl Into<String>,
        op: impl Into<String>,
        value: impl Into<SeaValue>,
    ) -> Self {
        self.or_filter_op(column, op, value)
    }
}

/// A name for the join in error messages.
fn join_name(join: &JoinClause) -> String {
    match &join.target {
        JoinTarget::Table(table) => table.clone(),
        JoinTarget::Query { alias, .. } => alias.clone(),
    }
}

/// Check the join's table or subquery, its alias, and every condition.
/// A join other than a cross join must have at least one condition: an
/// `INNER JOIN t` with no `ON` is a syntax error on most backends and a
/// silent cross join on others.
pub(crate) fn validate_join(join: &JoinClause) -> Result<(), FrameworkError> {
    match &join.target {
        JoinTarget::Table(table) => validate_aliased_table(table)?,
        JoinTarget::Query { query, alias } => {
            query.validate_inputs()?;
            validate_alias(alias)?;
        }
    }
    if join.kind != JoinKind::Cross && join.conditions.is_empty() {
        return Err(FrameworkError::param(format!(
            "the join to `{}` has no condition; add one with `on`",
            join_name(join)
        )));
    }
    for condition in &join.conditions {
        validate_condition(condition)?;
    }
    Ok(())
}

/// Render ` <KIND> JOIN <target> [ON <conditions>]`, with a leading space,
/// binding the subquery's values and then the conditions' values.
pub(crate) fn render_join(
    join: &JoinClause,
    backend: DbBackend,
    values: &mut Vec<SeaValue>,
    n: &mut usize,
) -> Result<String, FrameworkError> {
    let target = match &join.target {
        JoinTarget::Table(table) => render_aliased_table(backend, table)?,
        JoinTarget::Query { query, alias } => format!(
            "({}) AS {}",
            query.render_select_into(backend, values, n)?,
            quote_identifier(backend, alias)
        ),
    };
    let mut sql = format!(" {} {target}", join.kind.keyword());
    if !join.conditions.is_empty() {
        sql.push_str(" ON ");
        sql.push_str(&render_conditions(&join.conditions, backend, values, n)?);
    }
    Ok(sql)
}

/// Every table the join reads: its own, or every table its subquery reads.
pub(crate) fn join_tables(join: &JoinClause, out: &mut Vec<String>) {
    match &join.target {
        JoinTarget::Table(table) => {
            if let Some(table) = bare_table(table) {
                out.push(table.to_owned());
            }
        }
        JoinTarget::Query { query, .. } => query.collect_tables(out),
    }
    for condition in &join.conditions {
        condition_tables(condition, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_quote_per_backend_segment_by_segment() {
        assert_eq!(
            quote_identifier(DbBackend::Postgres, "posts.title"),
            "\"posts\".\"title\""
        );
        assert_eq!(quote_identifier(DbBackend::MySql, "posts.*"), "`posts`.*");
        assert_eq!(quote_identifier(DbBackend::Sqlite, "id"), "\"id\"");
    }

    #[test]
    fn select_columns_take_an_alias_and_a_star() {
        for ok in [
            "id",
            "posts.id",
            "*",
            "posts.*",
            "posts.title AS t",
            "c  as  alias",
        ] {
            assert!(validate_select_column(ok).is_ok(), "{ok}");
        }
        for bad in [
            "id alias",
            "posts.title as a.b",
            "id as",
            "id; --",
            "posts.**",
            "a as b as c",
        ] {
            assert!(validate_select_column(bad).is_err(), "{bad}");
        }
        assert_eq!(
            render_select_column(DbBackend::MySql, "posts.title as t").unwrap(),
            "`posts`.`title` AS `t`"
        );
    }

    #[test]
    fn or_folds_into_the_previous_condition_only() {
        let compare = |column: &str| Condition::Compare {
            column: column.into(),
            op: "=".into(),
            value: 1.into(),
            binary: false,
        };
        let mut conditions = vec![compare("a"), compare("b")];
        push_or(&mut conditions, compare("c"));
        push_or(&mut conditions, compare("d"));
        let mut values = Vec::new();
        let mut n = 0;
        let sql = render_conditions(&conditions, DbBackend::Postgres, &mut values, &mut n).unwrap();
        assert_eq!(
            sql,
            "\"a\" = $1 AND (\"b\" = $2 OR \"c\" = $3 OR \"d\" = $4)"
        );
        assert_eq!(values.len(), 4);
    }

    #[test]
    fn an_empty_column_list_groups_to_nothing() {
        assert!(grouped(Vec::new(), "=", &1.into(), Grouping::Any).is_none());
    }

    #[test]
    fn a_join_without_a_condition_is_refused_unless_it_is_a_cross_join() {
        let inner = JoinClause::new(JoinKind::Inner, JoinTarget::Table("users".into()));
        assert!(validate_join(&inner).is_err());
        let cross = JoinClause::new(JoinKind::Cross, JoinTarget::Table("users".into()));
        assert!(validate_join(&cross).is_ok());
    }
}
