//! Pagination - `LengthAwarePaginator` (offset-based, knows total) and
//! `CursorPaginator` (keyset-based, encrypted cursors). The
//! [`Pagination`] facade wraps both over a SeaORM `Select<E>`.

pub mod cursor;
pub mod inertia;
pub mod length_aware;
pub mod links;
pub mod simple;

pub use cursor::{Cursor, CursorDirection, CursorPaginator};
// Internal keyset-scan helpers shared with `eloquent::Builder::cursor_paginate`.
// Imported by function name so the `cursor` parameter in `Pagination::cursor`
// doesn't shadow the module path.
use cursor::{finalize_page, plan_direction};
pub use inertia::IntoInertiaScroll;
pub use length_aware::LengthAwarePaginator;
pub use links::PageLink;
pub use simple::Paginator;

use sea_orm::{ColumnTrait, EntityTrait, ModelTrait, QueryFilter, QueryOrder, QuerySelect, Select};

use crate::FrameworkError;
use crate::database::transaction::{CountOf, ExecutorChoice};

/// Static facade: `Pagination::length_aware` and `Pagination::cursor`.
pub struct Pagination;

impl Pagination {
    /// Run a length-aware (offset/limit + COUNT(*)) paginate.
    ///
    /// Routing matches the Eloquent builder's read path
    /// ([`ExecutorChoice::resolve_read`]): an ambient
    /// [`DB::transaction`](crate::DB::transaction) is honored (the COUNT and
    /// the page query both run on the transaction's connection), and a
    /// registered `__read_replica__` connection is used automatically.
    /// Use [`Self::length_aware_on`] to target a named connection.
    ///
    /// `current_page` is 1-based; values `< 1` are clamped to `1`.
    ///
    /// A limit or an offset already on `query` is dropped, as in
    /// Laravel's `paginate`: the total counts every matching row, and the
    /// page takes its own limit and offset.
    ///
    /// `per_page == 0` returns `FrameworkError::param("per_page")` (HTTP
    /// 400) - the same validation the Eloquent
    /// [`Builder::paginate`](crate::eloquent::Builder::paginate) enforces,
    /// so the two pagination surfaces agree on the zero-page-size contract
    /// instead of one silently emitting a `LIMIT 0` page.
    pub async fn length_aware<E>(
        query: Select<E>,
        per_page: u64,
        current_page: u64,
    ) -> Result<LengthAwarePaginator<E::Model>, FrameworkError>
    where
        E: EntityTrait,
        E::Model: Send + Sync,
    {
        if per_page == 0 {
            return Err(FrameworkError::param("per_page"));
        }
        let exec = ExecutorChoice::resolve_read(None, None, None).await?;
        Self::length_aware_with(exec, query, per_page, current_page).await
    }

    /// Run [`Self::length_aware`] against a specific named connection - the
    /// facade equivalent of [`Builder::on`](crate::eloquent::Builder::on).
    /// Routing matches the builder: an ambient `DB::transaction` still wins
    /// over the named connection, and the `__primary__` sentinel selects
    /// the default pool.
    pub async fn length_aware_on<E>(
        connection: &str,
        query: Select<E>,
        per_page: u64,
        current_page: u64,
    ) -> Result<LengthAwarePaginator<E::Model>, FrameworkError>
    where
        E: EntityTrait,
        E::Model: Send + Sync,
    {
        if per_page == 0 {
            return Err(FrameworkError::param("per_page"));
        }
        let exec = ExecutorChoice::resolve_read(None, Some(connection), None).await?;
        Self::length_aware_with(exec, query, per_page, current_page).await
    }

    async fn length_aware_with<E>(
        exec: ExecutorChoice,
        query: Select<E>,
        per_page: u64,
        current_page: u64,
    ) -> Result<LengthAwarePaginator<E::Model>, FrameworkError>
    where
        E: EntityTrait,
        E::Model: Send + Sync,
    {
        let page = current_page.max(1);
        // The total counts every match, as Laravel's
        // `getCountForPagination()` does: the page below replaces any
        // limit and offset the query carries.
        let total = exec
            .select_count(query.clone(), CountOf::AllMatches)
            .await?;
        let offset = (page - 1).saturating_mul(per_page);
        let data = exec
            .select_all(query.offset(offset).limit(per_page))
            .await?;
        Ok(LengthAwarePaginator::new(data, total, per_page, page))
    }

    /// Run a cursor-based paginate.
    ///
    /// Routing matches the Eloquent builder's read path
    /// ([`ExecutorChoice::resolve_read`]): an ambient
    /// [`DB::transaction`](crate::DB::transaction) is honored and a
    /// registered `__read_replica__` connection is used automatically. Use
    /// [`Self::cursor_on`] to target a named connection.
    ///
    /// Cursors carry a typed [`sea_orm::Value`] of the `order_col`
    /// boundary plus a direction (`next`/`prev`). The cursor is opaque and
    /// always AES-256-GCM-encrypted via the process key ring; there is no
    /// plaintext base64 fallback - if encryption is not initialized,
    /// encoding returns an error rather than emitting a forgeable cursor.
    ///
    /// `per_page == 0` returns `FrameworkError::param("per_page")` (HTTP
    /// 400), matching the Eloquent
    /// [`Builder::cursor_paginate`](crate::eloquent::Builder::cursor_paginate)
    /// contract.
    ///
    /// # Behavior
    ///
    /// - `cursor == None`: first page. Returns the first `per_page`
    ///   rows ASC by `order_col`. `prev_cursor` is `None`; `next_cursor`
    ///   is set iff more rows remain.
    /// - `cursor == Some(<next>)`: forward step. Returns rows strictly
    ///   greater than the boundary, ASC. `prev_cursor` points back at
    ///   this page's first row; `next_cursor` is set iff more rows
    ///   remain.
    /// - `cursor == Some(<prev>)`: backward step. Returns rows strictly
    ///   less than the boundary, fetched DESC then reversed to ASC.
    ///   `prev_cursor` is set iff more rows lie before; `next_cursor`
    ///   points at this page's last row (back toward the caller's
    ///   origin).
    ///
    /// Multiple ordered entity columns keep their directions and each
    /// contributes a named cursor boundary. Include a unique column to break ties.
    /// Zero or one explicit order uses `order_col` ascending. An `OFFSET` on
    /// `query` positions the first page only, the one requested without
    /// a cursor; every later page starts at its cursor.
    ///
    /// `order_col` should be a column with a total order suitable for
    /// keyset pagination - typically the primary key. Any SeaORM
    /// `Value` variant (`Int`, `BigInt`, `Uuid`, datetimes, decimals,
    /// strings, bytes, …) is supported; the dialect adapter binds the
    /// variant natively so Postgres / MySQL / SQLite all see the
    /// right SQL type.
    pub async fn cursor<E, C>(
        query: Select<E>,
        cursor: Option<&str>,
        per_page: u64,
        order_col: C,
    ) -> Result<CursorPaginator<E::Model>, FrameworkError>
    where
        E: EntityTrait<Column = C>,
        E::Model: Send + Sync,
        C: ColumnTrait + Copy,
    {
        if per_page == 0 {
            return Err(FrameworkError::param("per_page"));
        }
        let exec = ExecutorChoice::resolve_read(None, None, None).await?;
        Self::cursor_with(exec, query, cursor, per_page, order_col).await
    }

    /// Run [`Self::cursor`] against a specific named connection - the facade
    /// equivalent of [`Builder::on`](crate::eloquent::Builder::on). Routing
    /// matches the builder: an ambient `DB::transaction` still wins over the
    /// named connection, and the `__primary__` sentinel selects the default
    /// pool.
    #[allow(clippy::too_many_arguments)]
    pub async fn cursor_on<E, C>(
        connection: &str,
        query: Select<E>,
        cursor: Option<&str>,
        per_page: u64,
        order_col: C,
    ) -> Result<CursorPaginator<E::Model>, FrameworkError>
    where
        E: EntityTrait<Column = C>,
        E::Model: Send + Sync,
        C: ColumnTrait + Copy,
    {
        if per_page == 0 {
            return Err(FrameworkError::param("per_page"));
        }
        let exec = ExecutorChoice::resolve_read(None, Some(connection), None).await?;
        Self::cursor_with(exec, query, cursor, per_page, order_col).await
    }

    async fn cursor_with<E, C>(
        exec: ExecutorChoice,
        query: Select<E>,
        cursor: Option<&str>,
        per_page: u64,
        order_col: C,
    ) -> Result<CursorPaginator<E::Model>, FrameworkError>
    where
        E: EntityTrait<Column = C>,
        E::Model: Send + Sync,
        C: ColumnTrait + Copy,
    {
        let decoded = cursor.map(Cursor::decode).transpose()?;
        let mut query = query;
        let orders = cursor_columns::<E, C>(&mut query, order_col)?;
        {
            let statement = sea_orm::QueryTrait::query(&mut query);
            statement.clear_order_by();
            if decoded.is_some() {
                statement.reset_offset();
            }
        }
        let plan = plan_direction(decoded.as_ref().map(Cursor::direction));
        for (column, order) in &orders {
            query = if *order == plan.order_asc {
                query.order_by_asc(*column)
            } else {
                query.order_by_desc(*column)
            };
        }
        if let Some(cursor) = &decoded {
            query = query.filter(cursor_condition(cursor, &orders, plan.order_asc)?);
        }
        let mut rows = exec
            .select_all(query.limit(per_page.saturating_add(1)))
            .await?;
        if !plan.order_asc {
            rows.reverse();
        }
        let (rows, flags) = finalize_page(rows, per_page, &plan);
        let encode = |row: &E::Model, direction| {
            Cursor::new(
                orders
                    .iter()
                    .map(|(column, _)| (column.to_string(), row.get(*column)))
                    .collect(),
                direction,
            )
            .encode()
        };
        let next_cursor = rows
            .last()
            .filter(|_| flags.has_next)
            .map(|row| encode(row, CursorDirection::Next))
            .transpose()?;
        let prev_cursor = rows
            .first()
            .filter(|_| flags.has_prev)
            .map(|row| encode(row, CursorDirection::Prev))
            .transpose()?;

        let paginator = CursorPaginator::new(rows, per_page, next_cursor, prev_cursor);
        // The cursor this page was fetched with is its current page in the
        // Inertia scroll metadata.
        Ok(match cursor {
            Some(cursor) => paginator.with_current_cursor(cursor),
            None => paginator,
        })
    }
}

fn cursor_condition<C: ColumnTrait>(
    cursor: &Cursor,
    orders: &[(C, bool)],
    forward: bool,
) -> Result<sea_orm::sea_query::Condition, FrameworkError> {
    let names = orders
        .iter()
        .map(|(column, ascending)| (column.to_string(), *ascending))
        .collect::<Vec<_>>();
    let mut boundary = sea_orm::sea_query::Condition::any();
    for group in cursor.comparisons(&names, forward)? {
        let mut prefix = sea_orm::sea_query::Condition::all();
        for comparison in group {
            let column = comparison
                .column
                .parse::<C>()
                .map_err(|_| FrameworkError::bad_request("Cursor column is not in the entity"))?;
            prefix = prefix.add(match comparison.operator {
                "=" => column.eq(comparison.value),
                ">" => column.gt(comparison.value),
                _ => column.lt(comparison.value),
            });
        }
        boundary = boundary.add(prefix);
    }
    Ok(boundary)
}

fn cursor_columns<E, C>(
    query: &mut Select<E>,
    order_col: C,
) -> Result<Vec<(C, bool)>, FrameworkError>
where
    E: EntityTrait<Column = C>,
    C: ColumnTrait + Copy,
{
    // SeaQuery exposes orders through its renderer, rather than a getter.
    let inspector = OrderInspector::default();
    sea_orm::QueryTrait::query(query).build_any(&inspector);
    let recorded = inspector.orders.into_inner();
    let orders: Vec<(C, bool)> = if recorded.len() > 1 {
        recorded
            .into_iter()
            .map(|order| {
                let (name, ascending) = parse_cursor_order(&order)?;
                let column = name.parse::<C>().map_err(|_| {
                    FrameworkError::bad_request("Cursor column is not in the entity")
                })?;
                Ok((column, ascending))
            })
            .collect::<Result<_, FrameworkError>>()?
    } else {
        vec![(order_col, true)]
    };
    Ok(orders)
}

fn parse_cursor_order(order: &str) -> Result<(String, bool), FrameworkError> {
    let invalid =
        || FrameworkError::bad_request("Cursor pagination needs ascending or descending columns");
    let (expression, ascending) = order
        .strip_suffix(" ASC")
        .map(|expr| (expr, true))
        .or_else(|| order.strip_suffix(" DESC").map(|expr| (expr, false)))
        .ok_or_else(invalid)?;
    // The inspector uses SQLite's quoted identifiers. Accept only a column,
    // with optional table qualification, rather than interpreting SQL expressions.
    let mut remainder = expression;
    let mut column = String::new();
    loop {
        remainder = remainder.strip_prefix('"').ok_or_else(invalid)?;
        let mut identifier = String::new();
        loop {
            let end = remainder.find('"').ok_or_else(invalid)?;
            identifier.push_str(&remainder[..end]);
            remainder = &remainder[end + 1..];
            if let Some(rest) = remainder.strip_prefix('"') {
                identifier.push('"');
                remainder = rest;
            } else {
                break;
            }
        }
        column.clear();
        column.push_str(&identifier);
        if remainder.is_empty() {
            break;
        }
        remainder = remainder.strip_prefix('.').ok_or_else(invalid)?;
    }
    Ok((column, ascending))
}

// Record only the outer SELECT's orders. Nested expressions, FROM queries,
// CTEs, windows and union arms render without visiting this hook.
#[derive(Default)]
struct OrderInspector {
    orders: std::cell::RefCell<Vec<String>>,
}

impl sea_orm::sea_query::QuotedBuilder for OrderInspector {
    fn quote(&self) -> sea_orm::sea_query::Quote {
        sea_orm::sea_query::QuotedBuilder::quote(&sea_orm::sea_query::SqliteQueryBuilder)
    }
}
impl sea_orm::sea_query::EscapeBuilder for OrderInspector {}
impl sea_orm::sea_query::TableRefBuilder for OrderInspector {}
impl sea_orm::sea_query::OperLeftAssocDecider for OrderInspector {
    fn well_known_left_associative(&self, op: &sea_orm::sea_query::BinOper) -> bool {
        sea_orm::sea_query::OperLeftAssocDecider::well_known_left_associative(
            &sea_orm::sea_query::SqliteQueryBuilder,
            op,
        )
    }
}
impl sea_orm::sea_query::PrecedenceDecider for OrderInspector {
    fn inner_expr_well_known_greater_precedence(
        &self,
        inner: &sea_orm::sea_query::Expr,
        outer: &sea_orm::sea_query::Oper,
    ) -> bool {
        sea_orm::sea_query::PrecedenceDecider::inner_expr_well_known_greater_precedence(
            &sea_orm::sea_query::SqliteQueryBuilder,
            inner,
            outer,
        )
    }
}
impl sea_orm::sea_query::QueryBuilder for OrderInspector {
    fn prepare_query_statement(
        &self,
        query: &sea_orm::sea_query::SubQueryStatement,
        sql: &mut impl sea_orm::sea_query::SqlWriter,
    ) {
        sea_orm::sea_query::SqliteQueryBuilder.prepare_query_statement(query, sql);
    }
    fn prepare_select_into(
        &self,
        into: &sea_orm::sea_query::SelectInto,
        sql: &mut impl sea_orm::sea_query::SqlWriter,
    ) {
        sea_orm::sea_query::SqliteQueryBuilder.prepare_select_into(into, sql);
    }
    fn prepare_explain_statement(
        &self,
        explain: &sea_orm::sea_query::ExplainStatement,
        sql: &mut impl sea_orm::sea_query::SqlWriter,
    ) {
        sea_orm::sea_query::SqliteQueryBuilder.prepare_explain_statement(explain, sql);
    }
    fn prepare_value(&self, value: sea_orm::Value, sql: &mut impl sea_orm::sea_query::SqlWriter) {
        sea_orm::sea_query::SqliteQueryBuilder.prepare_value(value, sql);
    }
    fn prepare_order_expr(
        &self,
        order: &sea_orm::sea_query::OrderExpr,
        sql: &mut impl sea_orm::sea_query::SqlWriter,
    ) {
        let mut rendered = String::new();
        sea_orm::sea_query::SqliteQueryBuilder.prepare_order_expr(order, &mut rendered);
        self.orders.borrow_mut().push(rendered);
        sea_orm::sea_query::SqliteQueryBuilder.prepare_order_expr(order, sql);
    }
    fn prepare_expr(
        &self,
        expr: &sea_orm::sea_query::Expr,
        sql: &mut impl sea_orm::sea_query::SqlWriter,
    ) {
        sea_orm::sea_query::SqliteQueryBuilder.prepare_expr(expr, sql);
    }
    fn prepare_table_ref(
        &self,
        table: &sea_orm::sea_query::TableRef,
        sql: &mut impl sea_orm::sea_query::SqlWriter,
    ) {
        sea_orm::sea_query::SqliteQueryBuilder.prepare_table_ref(table, sql);
    }
    fn prepare_with_clause(
        &self,
        clause: &sea_orm::sea_query::WithClause,
        sql: &mut impl sea_orm::sea_query::SqlWriter,
    ) {
        sea_orm::sea_query::SqliteQueryBuilder.prepare_with_clause(clause, sql);
    }
    fn prepare_window_statement(
        &self,
        window: &sea_orm::sea_query::WindowStatement,
        sql: &mut impl sea_orm::sea_query::SqlWriter,
    ) {
        sea_orm::sea_query::SqliteQueryBuilder.prepare_window_statement(window, sql);
    }
    fn prepare_union_statement(
        &self,
        kind: sea_orm::sea_query::UnionType,
        select: &sea_orm::sea_query::SelectStatement,
        sql: &mut impl sea_orm::sea_query::SqlWriter,
    ) {
        sea_orm::sea_query::SqliteQueryBuilder.prepare_union_statement(kind, select, sql);
    }
}

/// The URL of one page: the base path with the pair `key=value` in its
/// query string, percent-encoded. Shared by the three paginators and by
/// the JSON:API link builder, so all of them build a URL the same way.
///
/// The base is used as it is written. What it has in its query string
/// stays, in its order, so the filters of a listing are given with the
/// path. A pair of the same key is taken out first: a base that is the
/// URL of the current request has the current page in it, and the URL of
/// the next page has one page and not two. A fragment stays at the end,
/// behind the query string, where a browser reads it. With no base the
/// URL is the bare `?key=value`.
///
/// A root-relative base gets the public root unless it already has it
/// (PFX-006, PFX-010); a base that is only a query, or a relative path,
/// stays relative.
pub(crate) fn build_query_url(path: Option<&str>, key: &str, value: &str) -> String {
    let base = crate::routing::root::rooted(path.unwrap_or(""));
    let base = base.as_ref();
    let (base, fragment) = match base.split_once('#') {
        Some((base, fragment)) => (base, Some(fragment)),
        None => (base, None),
    };
    let (location, query) = base.split_once('?').unwrap_or((base, ""));

    let mut url = String::with_capacity(base.len() + key.len() + value.len() + 4);
    url.push_str(location);
    url.push('?');
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let same_key = url::form_urlencoded::parse(pair.as_bytes())
            .next()
            .is_some_and(|(name, _)| name == key);
        if !same_key {
            url.push_str(pair);
            url.push('&');
        }
    }
    url.push_str(
        &url::form_urlencoded::Serializer::new(String::new())
            .append_pair(key, value)
            .finish(),
    );
    if let Some(fragment) = fragment {
        url.push('#');
        url.push_str(fragment);
    }
    url
}

#[cfg(test)]
mod url_tests {
    use super::build_query_url;

    #[test]
    fn the_pair_is_the_query_string_of_a_base_that_has_none() {
        assert_eq!(
            build_query_url(Some("/users"), "page", "2"),
            "/users?page=2"
        );
        assert_eq!(build_query_url(None, "page", "2"), "?page=2");
        assert_eq!(
            build_query_url(Some("/users/"), "page", "2"),
            "/users/?page=2",
            "the base is used as it is written"
        );
    }

    #[test]
    fn the_query_string_of_the_base_stays_in_its_order() {
        assert_eq!(
            build_query_url(Some("/users?role=admin&sort=-name"), "page", "2"),
            "/users?role=admin&sort=-name&page=2"
        );
        assert_eq!(
            build_query_url(Some("/users?q=a%20b&tag=x+y"), "page", "2"),
            "/users?q=a%20b&tag=x+y&page=2",
            "a pair of the base is not encoded a second time"
        );
        assert_eq!(
            build_query_url(Some("?role=admin"), "page", "2"),
            "?role=admin&page=2"
        );
    }

    /// The base is often the URL of the current request, which has the
    /// current page in it.
    #[test]
    fn a_pair_of_the_same_key_is_replaced() {
        assert_eq!(
            build_query_url(Some("/users?role=admin&page=3"), "page", "4"),
            "/users?role=admin&page=4"
        );
        assert_eq!(
            build_query_url(Some("/users?page=3&role=admin&page=9"), "page", "4"),
            "/users?role=admin&page=4"
        );
        assert_eq!(
            build_query_url(Some("/users?users%5Fpage=3"), "users_page", "4"),
            "/users?users_page=4",
            "the key is compared as it is read, not as it is written"
        );
        assert_eq!(
            build_query_url(Some("/users?pages=3&page"), "page", "4"),
            "/users?pages=3&page=4",
            "a key that only begins the same stays, and a key with no value goes"
        );
    }

    #[test]
    fn a_fragment_stays_behind_the_query_string() {
        assert_eq!(
            build_query_url(Some("/users#list"), "page", "2"),
            "/users?page=2#list"
        );
        assert_eq!(
            build_query_url(Some("/users?role=admin#list"), "page", "2"),
            "/users?role=admin&page=2#list"
        );
    }

    #[test]
    fn the_key_and_the_value_are_encoded() {
        assert_eq!(
            build_query_url(Some("/users"), "weird key", "a&b=c"),
            "/users?weird+key=a%26b%3Dc"
        );
    }
}

// ── Paginated<T> trait ────────────────────────────────────────────────────

/// Common surface consumed by `Resource::paginated` for building
/// JSON:API pagination links and meta. Implemented by
/// `LengthAwarePaginator<T>` and `CursorPaginator<T>`.
pub trait Paginated<T> {
    /// The items on the current page.
    fn items(&self) -> &[T];

    /// `meta.pagination` payload - conventionally placed under
    /// `meta.pagination` in JSON:API responses.
    fn meta_value(&self) -> serde_json::Value;

    /// Yield `(rel, href)` pairs for pagination links
    /// (`first`, `last`, `prev`, `next`, `self`).
    fn links_iter(&self) -> Box<dyn Iterator<Item = (&'static str, String)> + '_>;
}

impl<T> Paginated<T> for LengthAwarePaginator<T> {
    fn items(&self) -> &[T] {
        &self.data
    }

    fn meta_value(&self) -> serde_json::Value {
        serde_json::json!({
            "total": self.total,
            "per_page": self.per_page,
            "current_page": self.current_page,
            "last_page": self.last_page,
        })
    }

    fn links_iter(&self) -> Box<dyn Iterator<Item = (&'static str, String)> + '_> {
        let mut links: Vec<(&'static str, String)> = Vec::new();
        links.push(("self", self.url_for_page(self.current_page)));
        links.push(("first", self.url_for_page(1)));
        if self.last_page > 0 {
            links.push(("last", self.url_for_page(self.last_page)));
        }
        if self.current_page > 1 {
            links.push(("prev", self.url_for_page(self.current_page - 1)));
        }
        if self.current_page < self.last_page {
            links.push(("next", self.url_for_page(self.current_page + 1)));
        }
        Box::new(links.into_iter())
    }
}

impl<T> Paginated<T> for CursorPaginator<T> {
    fn items(&self) -> &[T] {
        &self.data
    }

    fn meta_value(&self) -> serde_json::Value {
        serde_json::json!({
            "next_cursor": self.next_cursor,
            "prev_cursor": self.prev_cursor,
        })
    }

    fn links_iter(&self) -> Box<dyn Iterator<Item = (&'static str, String)> + '_> {
        // Emit `next`/`prev` links from the stored cursor values, keyed by
        // `cursor_name` (defaulting to "cursor" - the query key
        // `Builder::cursor_paginate` reads). Mirrors the length-aware
        // paginator: links are produced whenever the corresponding cursor
        // exists, with or without a base path (no path → relative
        // `?cursor=<opaque>`).
        let key = self.cursor_name.as_deref().unwrap_or("cursor");
        let mut links: Vec<(&'static str, String)> = Vec::new();
        if let Some(next) = &self.next_cursor {
            links.push(("next", build_query_url(self.path.as_deref(), key, next)));
        }
        if let Some(prev) = &self.prev_cursor {
            links.push(("prev", build_query_url(self.path.as_deref(), key, prev)));
        }
        Box::new(links.into_iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cursor_with(next: Option<&str>, prev: Option<&str>) -> CursorPaginator<i32> {
        CursorPaginator::new(
            vec![1, 2],
            10,
            next.map(|s| s.to_string()),
            prev.map(|s| s.to_string()),
        )
    }

    fn links_of<T>(p: &impl Paginated<T>) -> HashMap<&'static str, String> {
        p.links_iter().collect()
    }

    #[test]
    fn cursor_links_emit_next_and_prev_with_path() {
        let p = cursor_with(Some("NEXTCUR"), Some("PREVCUR")).with_path("/api/items");
        let links = links_of(&p);
        assert_eq!(
            links.get("next").map(String::as_str),
            Some("/api/items?cursor=NEXTCUR")
        );
        assert_eq!(
            links.get("prev").map(String::as_str),
            Some("/api/items?cursor=PREVCUR")
        );
    }

    #[test]
    fn cursor_links_omit_absent_cursors() {
        // First page: prev_cursor is None → only a `next` link is emitted.
        let p = cursor_with(Some("NEXTCUR"), None).with_path("/api/items");
        let links = links_of(&p);
        assert!(links.contains_key("next"));
        assert!(!links.contains_key("prev"));
    }

    #[test]
    fn cursor_links_use_custom_cursor_name() {
        let p = cursor_with(Some("NEXTCUR"), None)
            .with_path("/api/items")
            .with_cursor_name("after");
        let links = links_of(&p);
        assert_eq!(
            links.get("next").map(String::as_str),
            Some("/api/items?after=NEXTCUR")
        );
    }

    #[test]
    fn cursor_links_append_to_existing_query_string() {
        // A base that already carries a query string must get `&cursor=`,
        // never a malformed second `?`.
        let p = cursor_with(Some("NEXTCUR"), None).with_path("/api/items?sort=name");
        let links = links_of(&p);
        assert_eq!(
            links.get("next").map(String::as_str),
            Some("/api/items?sort=name&cursor=NEXTCUR")
        );
    }

    #[test]
    fn cursor_links_without_path_are_relative() {
        // Parity with LengthAwarePaginator, which emits `?page=N` when no
        // base path is set.
        let p = cursor_with(Some("NEXTCUR"), None);
        let links = links_of(&p);
        assert_eq!(
            links.get("next").map(String::as_str),
            Some("?cursor=NEXTCUR")
        );
    }
}
