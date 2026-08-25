//! Pivot-column filtering shared by the three many-to-many relations.
//!
//! Laravel declares `wherePivot` once on `BelongsToMany` and inherits it
//! into `MorphToMany` and `morphedByMany`. Rust has no relation-type
//! inheritance here: [`BelongsToMany`](super::BelongsToMany),
//! [`MorphToMany`](super::MorphToMany) and
//! [`MorphedByMany`](super::MorphedByMany) are three independent structs
//! with long `where` clauses. So the accumulator lives here and
//! `pivot_filter_methods!` emits the public surface into each relation's
//! own inherent `impl` block - one definition, three expansions, no
//! drift.
//!
//! The terms reach SQL two different ways, because the read paths do:
//!
//! - The hand-built pivot statements (`BelongsToMany::get`'s id scan,
//!   every `count`) take [`PivotFilters::render_and`], which appends
//!   ` AND (...)` fragments and pushes their binds onto the statement's
//!   existing value vector.
//! - The typed pivot-row queries (`P::query()`) take
//!   [`PivotFilters::apply`], which pushes the terms straight onto the
//!   builder so `Builder::validate_inputs` and the normal renderer
//!   handle them.
//!
//! Filters constrain reads only. `attach` / `attach_with` / `detach` /
//! `sync` call [`PivotFilters::reject_mutation`] and fail closed rather
//! than let a read predicate narrow - or silently not narrow - a write.

use sea_orm::{DbBackend, Value as SeaValue};

use crate::eloquent::builder::{Builder, WhereTerm, render_subquery_term, validate_where_term};
use crate::error::FrameworkError;

/// WHERE terms accumulated by the `where_pivot*` family, applied to the
/// pivot side of a many-to-many read.
///
/// Ordering is preserved: the terms AND together in the order they were
/// added, except where an `or_*` call folded a run of them into a
/// [`WhereTerm::Or`] group.
#[derive(Debug, Clone, Default)]
pub(crate) struct PivotFilters {
    terms: Vec<WhereTerm>,
}

impl PivotFilters {
    /// True when no `where_pivot*` call has been made. The read paths
    /// use this to skip rendering entirely so an unfiltered relation
    /// produces byte-for-byte the SQL it produced before this feature
    /// existed.
    pub(crate) fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Append a term, ANDed against everything already present.
    pub(crate) fn push(&mut self, term: WhereTerm) {
        self.terms.push(term);
    }

    /// Fold a term into a disjunction with the previous one.
    ///
    /// Same shape as [`Builder::or_filter`](crate::Builder::or_filter):
    /// an existing `Or` group absorbs the new term, a plain previous
    /// term is popped and re-pushed inside a fresh `Or`, and with no
    /// previous term at all the disjunction reduces to the term itself
    /// (so the renderer never emits a dangling `()`).
    pub(crate) fn push_or(&mut self, term: WhereTerm) {
        match self.terms.last_mut() {
            Some(WhereTerm::Or(group)) => group.push(term),
            Some(_) => {
                let last = self
                    .terms
                    .pop()
                    .expect("checked Some in the match arm above");
                self.terms.push(WhereTerm::Or(vec![last, term]));
            }
            None => self.terms.push(term),
        }
    }

    /// Collapse a closure-constrained pivot builder into one nestable
    /// term.
    ///
    /// Returns `None` when the closure added nothing, so a no-op closure
    /// costs no SQL rather than an empty parenthesis pair.
    pub(crate) fn group_from<M>(inner: Builder<M>) -> Option<WhereTerm> {
        let terms = inner.where_terms;
        if terms.is_empty() {
            None
        } else {
            Some(WhereTerm::Group(terms))
        }
    }

    /// Run the identifier and operator allowlists over every term.
    ///
    /// The typed path gets this from `Builder::validate_inputs`; the
    /// raw-SQL path has no such pass, so [`Self::render_and`] calls this
    /// before it renders anything. A column name that fails
    /// `validate_identifier` must surface as an error, never as SQL.
    pub(crate) fn validate(&self) -> Result<(), FrameworkError> {
        for term in &self.terms {
            validate_where_term(term)?;
        }
        Ok(())
    }

    /// Render the terms as ` AND (...)` fragments to append to a
    /// hand-built pivot statement that already has a `WHERE` clause.
    ///
    /// `values` is the statement's bind vector, already holding its
    /// prefix binds; `next_placeholder` is the count of those binds.
    /// PostgreSQL placeholders are positional, so seeding the counter
    /// with the prefix length is what keeps `$2` from colliding with
    /// `$1`. Returns an empty string when no filters are set.
    pub(crate) fn render_and(
        &self,
        backend: DbBackend,
        values: &mut Vec<SeaValue>,
        next_placeholder: &mut usize,
    ) -> Result<String, FrameworkError> {
        if self.is_empty() {
            return Ok(String::new());
        }
        self.validate()?;
        let mut out = String::new();
        for term in &self.terms {
            let sql = render_subquery_term(backend, None, term, values, next_placeholder)?;
            out.push_str(" AND (");
            out.push_str(&sql);
            out.push(')');
        }
        Ok(out)
    }

    /// Push the terms onto a typed pivot query.
    ///
    /// No qualifier is needed: the pivot query's `FROM` is the pivot
    /// table alone, so a bare column is unambiguous.
    pub(crate) fn apply<M>(&self, mut builder: Builder<M>) -> Builder<M> {
        builder.where_terms.extend(self.terms.iter().cloned());
        builder
    }

    /// Refuse a pivot write while filters are set.
    ///
    /// Laravel folds `pivotWheres` into `detach()`. Suprnova builds its
    /// pivot `DELETE` by hand, and quietly ignoring a filter on a write
    /// is a data difference the caller cannot see. Failing closed makes
    /// the caller choose.
    pub(crate) fn reject_mutation(&self) -> Result<(), FrameworkError> {
        if self.is_empty() {
            return Ok(());
        }
        Err(FrameworkError::param(
            "where_pivot filters constrain reads only (get / first / count); \
             drop them before attach / attach_with / detach / sync",
        ))
    }
}

/// Emit the `where_pivot*` family into a relation's inherent `impl`
/// block. `$pivot` is that relation's pivot type parameter.
///
/// Invoked once per relation struct rather than written out three
/// times - see the module docs. The type parameter is threaded in
/// explicitly because `macro_rules!` hygiene does not cover generics,
/// so a bare `P` here would bind to whatever the expansion site happens
/// to call its pivot.
macro_rules! pivot_filter_methods {
    ($pivot:ident) => {
        /// `WHERE <col> = <val>` on the pivot table. Mirrors Laravel's
        /// `wherePivot($column, $value)`.
        ///
        /// Pivot filters constrain the read path only - `get`, `first`
        /// and `count`. `attach` / `attach_with` / `detach` / `sync`
        /// return an error while a filter is set.
        ///
        /// # Security
        ///
        /// `col` interpolates into the pivot statement as a raw SQL
        /// identifier (same contract as
        /// [`Builder::filter`](crate::Builder::filter)) and is checked
        /// against the identifier allowlist when the read executes.
        /// Never take it from untrusted input. `val` binds as a
        /// parameter and is safe to.
        pub fn where_pivot(mut self, col: impl IntoColumn, val: impl IntoVal) -> Self {
            self.pivot_filters
                .push(WhereTerm::Eq(col.col_name(), val.into_val()));
            self
        }

        /// `OR <col> = <val>` on the pivot table, folded into a
        /// disjunction with the preceding pivot filter. Mirrors
        /// Laravel's `orWherePivot($column, $value)`.
        pub fn or_where_pivot(mut self, col: impl IntoColumn, val: impl IntoVal) -> Self {
            self.pivot_filters
                .push_or(WhereTerm::Eq(col.col_name(), val.into_val()));
            self
        }

        /// `WHERE <col> <op> <val>` on the pivot table, for operators
        /// beyond equality. The operator is checked against the SQL
        /// operator allowlist when the read executes, so an operator
        /// outside it surfaces as an error rather than reaching SQL.
        pub fn where_pivot_op(mut self, col: impl IntoColumn, op: &str, val: impl IntoVal) -> Self {
            self.pivot_filters.push(WhereTerm::Op(
                col.col_name(),
                op.to_string(),
                val.into_val(),
            ));
            self
        }

        /// `OR <col> <op> <val>` on the pivot table. Disjunctive form of
        /// [`Self::where_pivot_op`].
        pub fn or_where_pivot_op(
            mut self,
            col: impl IntoColumn,
            op: &str,
            val: impl IntoVal,
        ) -> Self {
            self.pivot_filters.push_or(WhereTerm::Op(
                col.col_name(),
                op.to_string(),
                val.into_val(),
            ));
            self
        }

        /// `WHERE <col> IN (...)` on the pivot table. An empty list
        /// matches nothing, matching the builder's `filter_in`.
        pub fn where_pivot_in<V, I>(mut self, col: impl IntoColumn, vals: I) -> Self
        where
            I: IntoIterator<Item = V>,
            V: IntoVal,
        {
            let v = vals.into_iter().map(|x| x.into_val()).collect();
            self.pivot_filters.push(WhereTerm::In(col.col_name(), v));
            self
        }

        /// `OR <col> IN (...)` on the pivot table.
        pub fn or_where_pivot_in<V, I>(mut self, col: impl IntoColumn, vals: I) -> Self
        where
            I: IntoIterator<Item = V>,
            V: IntoVal,
        {
            let v = vals.into_iter().map(|x| x.into_val()).collect();
            self.pivot_filters.push_or(WhereTerm::In(col.col_name(), v));
            self
        }

        /// `WHERE <col> NOT IN (...)` on the pivot table. An empty list
        /// matches everything, matching the builder's `filter_not_in`.
        pub fn where_pivot_not_in<V, I>(mut self, col: impl IntoColumn, vals: I) -> Self
        where
            I: IntoIterator<Item = V>,
            V: IntoVal,
        {
            let v = vals.into_iter().map(|x| x.into_val()).collect();
            self.pivot_filters.push(WhereTerm::NotIn(col.col_name(), v));
            self
        }

        /// `OR <col> NOT IN (...)` on the pivot table.
        pub fn or_where_pivot_not_in<V, I>(mut self, col: impl IntoColumn, vals: I) -> Self
        where
            I: IntoIterator<Item = V>,
            V: IntoVal,
        {
            let v = vals.into_iter().map(|x| x.into_val()).collect();
            self.pivot_filters
                .push_or(WhereTerm::NotIn(col.col_name(), v));
            self
        }

        /// `WHERE <col> IS NULL` on the pivot table.
        pub fn where_pivot_null(mut self, col: impl IntoColumn) -> Self {
            self.pivot_filters.push(WhereTerm::Null(col.col_name()));
            self
        }

        /// `OR <col> IS NULL` on the pivot table.
        pub fn or_where_pivot_null(mut self, col: impl IntoColumn) -> Self {
            self.pivot_filters.push_or(WhereTerm::Null(col.col_name()));
            self
        }

        /// `WHERE <col> IS NOT NULL` on the pivot table.
        pub fn where_pivot_not_null(mut self, col: impl IntoColumn) -> Self {
            self.pivot_filters.push(WhereTerm::NotNull(col.col_name()));
            self
        }

        /// `OR <col> IS NOT NULL` on the pivot table.
        pub fn or_where_pivot_not_null(mut self, col: impl IntoColumn) -> Self {
            self.pivot_filters
                .push_or(WhereTerm::NotNull(col.col_name()));
            self
        }

        /// `WHERE <col> BETWEEN low AND high` on the pivot table.
        /// Bounds are inclusive, matching SQL.
        pub fn where_pivot_between<V: IntoVal + Clone>(
            mut self,
            col: impl IntoColumn,
            range: ::std::ops::RangeInclusive<V>,
        ) -> Self {
            let (a, b) = (
                range.start().clone().into_val(),
                range.end().clone().into_val(),
            );
            self.pivot_filters
                .push(WhereTerm::Between(col.col_name(), a, b));
            self
        }

        /// `OR <col> BETWEEN low AND high` on the pivot table.
        pub fn or_where_pivot_between<V: IntoVal + Clone>(
            mut self,
            col: impl IntoColumn,
            range: ::std::ops::RangeInclusive<V>,
        ) -> Self {
            let (a, b) = (
                range.start().clone().into_val(),
                range.end().clone().into_val(),
            );
            self.pivot_filters
                .push_or(WhereTerm::Between(col.col_name(), a, b));
            self
        }

        /// `WHERE <col> NOT BETWEEN low AND high` on the pivot table.
        pub fn where_pivot_not_between<V: IntoVal + Clone>(
            mut self,
            col: impl IntoColumn,
            range: ::std::ops::RangeInclusive<V>,
        ) -> Self {
            let (a, b) = (
                range.start().clone().into_val(),
                range.end().clone().into_val(),
            );
            self.pivot_filters
                .push(WhereTerm::NotBetween(col.col_name(), a, b));
            self
        }

        /// `OR <col> NOT BETWEEN low AND high` on the pivot table.
        pub fn or_where_pivot_not_between<V: IntoVal + Clone>(
            mut self,
            col: impl IntoColumn,
            range: ::std::ops::RangeInclusive<V>,
        ) -> Self {
            let (a, b) = (
                range.start().clone().into_val(),
                range.end().clone().into_val(),
            );
            self.pivot_filters
                .push_or(WhereTerm::NotBetween(col.col_name(), a, b));
            self
        }

        /// `WHERE (<closure's terms ANDed>)` on the pivot table -
        /// Laravel's closure form of `wherePivot`.
        ///
        /// The closure receives a fresh pivot builder and the WHERE
        /// terms it leaves become one parenthesised group, so a
        /// following [`Self::or_where_pivot`] disjoins against the whole
        /// group rather than only its last term. A closure that adds
        /// nothing adds no SQL.
        ///
        /// Only the WHERE terms are carried over, matching Laravel's
        /// nested-where splice. Anything else the closure sets on the
        /// builder - ordering, limits, grouping, eager loads - has no
        /// meaning inside a predicate group and is discarded.
        ///
        /// ```ignore
        /// // (active = 1 AND note IS NOT NULL) OR pinned = 1
        /// user.roles()
        ///     .where_pivot_group(|q| q.filter("active", 1i64).filter_not_null("note"))
        ///     .or_where_pivot("pinned", 1i64)
        /// ```
        pub fn where_pivot_group<F>(mut self, predicate: F) -> Self
        where
            F: FnOnce(Builder<$pivot>) -> Builder<$pivot>,
        {
            if let Some(group) = PivotFilters::group_from(predicate(Builder::<$pivot>::new())) {
                self.pivot_filters.push(group);
            }
            self
        }

        /// `OR (<closure's terms ANDed>)` on the pivot table -
        /// Laravel's closure form of `orWherePivot`. Disjunctive form of
        /// [`Self::where_pivot_group`].
        pub fn or_where_pivot_group<F>(mut self, predicate: F) -> Self
        where
            F: FnOnce(Builder<$pivot>) -> Builder<$pivot>,
        {
            if let Some(group) = PivotFilters::group_from(predicate(Builder::<$pivot>::new())) {
                self.pivot_filters.push_or(group);
            }
            self
        }
    };
}

pub(crate) use pivot_filter_methods;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as JsonValue;

    fn eq(col: &str, v: i64) -> WhereTerm {
        WhereTerm::Eq(col.to_string(), JsonValue::from(v))
    }

    #[test]
    fn render_and_is_empty_when_no_filter_is_set() {
        let filters = PivotFilters::default();
        assert!(filters.is_empty());
        let mut values: Vec<SeaValue> = vec![SeaValue::from(7i64)];
        let mut n = 1usize;
        let sql = filters
            .render_and(DbBackend::Postgres, &mut values, &mut n)
            .expect("empty renders");
        assert_eq!(sql, "");
        assert_eq!(values.len(), 1, "no binds added");
        assert_eq!(n, 1, "counter untouched");
    }

    #[test]
    fn render_and_numbers_postgres_placeholders_after_the_prefix_binds() {
        // The statement already bound the parent key as $1, so the
        // first filter placeholder must be $2.
        let mut filters = PivotFilters::default();
        filters.push(eq("active", 1));
        filters.push(WhereTerm::In(
            "tier".to_string(),
            vec![JsonValue::from(1), JsonValue::from(2)],
        ));

        let mut values: Vec<SeaValue> = vec![SeaValue::from(7i64)];
        let mut n = 1usize;
        let sql = filters
            .render_and(DbBackend::Postgres, &mut values, &mut n)
            .expect("renders");

        assert_eq!(sql, " AND (active = $2) AND (tier IN ($3, $4))");
        assert_eq!(values.len(), 4);
        assert_eq!(n, 4);
    }

    #[test]
    fn render_and_uses_portable_markers_on_sqlite() {
        let mut filters = PivotFilters::default();
        filters.push(eq("active", 1));
        let mut values: Vec<SeaValue> = vec![SeaValue::from(7i64)];
        let mut n = 1usize;
        let sql = filters
            .render_and(DbBackend::Sqlite, &mut values, &mut n)
            .expect("renders");
        assert_eq!(sql, " AND (active = ?)");
    }

    #[test]
    fn push_or_folds_the_previous_term_into_a_group() {
        let mut filters = PivotFilters::default();
        filters.push(eq("a", 1));
        filters.push_or(eq("b", 2));
        filters.push_or(eq("c", 3));

        let mut values: Vec<SeaValue> = Vec::new();
        let mut n = 0usize;
        let sql = filters
            .render_and(DbBackend::Sqlite, &mut values, &mut n)
            .expect("renders");
        assert_eq!(sql, " AND ((a = ? OR b = ? OR c = ?))");
    }

    #[test]
    fn push_or_with_no_previous_term_pushes_it_plain() {
        let mut filters = PivotFilters::default();
        filters.push_or(eq("a", 1));

        let mut values: Vec<SeaValue> = Vec::new();
        let mut n = 0usize;
        let sql = filters
            .render_and(DbBackend::Sqlite, &mut values, &mut n)
            .expect("renders");
        assert_eq!(sql, " AND (a = ?)", "no dangling disjunction wrapper");
    }

    #[test]
    fn a_group_renders_as_one_conjunctive_atom_inside_a_disjunction() {
        let mut filters = PivotFilters::default();
        filters.push(eq("pinned", 1));
        filters.push_or(WhereTerm::Group(vec![eq("active", 1), eq("tier", 2)]));

        let mut values: Vec<SeaValue> = Vec::new();
        let mut n = 0usize;
        let sql = filters
            .render_and(DbBackend::Sqlite, &mut values, &mut n)
            .expect("renders");
        assert_eq!(sql, " AND ((pinned = ? OR (active = ? AND tier = ?)))");
    }

    #[test]
    fn group_from_drops_a_no_op_closure_and_keeps_a_constrained_one() {
        assert!(
            PivotFilters::group_from(Builder::<()>::new()).is_none(),
            "an empty inner builder must not become a term"
        );
        let group = PivotFilters::group_from(Builder::<()>::new().filter("active", 1i64))
            .expect("a constrained builder becomes one group");
        assert!(matches!(group, WhereTerm::Group(ref t) if t.len() == 1));
    }

    #[test]
    fn an_empty_group_renders_as_the_always_true_identity() {
        // Unreachable through the public surface (`group_from` drops
        // it), but `()` would be a syntax error, so pin the guard.
        let mut filters = PivotFilters::default();
        filters.push(WhereTerm::Group(Vec::new()));
        let mut values: Vec<SeaValue> = Vec::new();
        let mut n = 0usize;
        let sql = filters
            .render_and(DbBackend::Sqlite, &mut values, &mut n)
            .expect("renders");
        assert_eq!(sql, " AND (1 = 1)");
    }

    #[test]
    fn render_and_rejects_an_invalid_identifier_before_rendering() {
        let mut filters = PivotFilters::default();
        filters.push(WhereTerm::Eq(
            "active; DROP TABLE role_user".to_string(),
            JsonValue::from(1),
        ));
        let mut values: Vec<SeaValue> = Vec::new();
        let mut n = 0usize;
        let err = filters
            .render_and(DbBackend::Sqlite, &mut values, &mut n)
            .expect_err("an illegal column name must not reach SQL");
        assert!(
            values.is_empty(),
            "validation runs before any bind is pushed, got: {values:?}"
        );
        let _ = err;
    }

    #[test]
    fn validation_recurses_through_a_group() {
        let mut filters = PivotFilters::default();
        filters.push(WhereTerm::Group(vec![
            eq("active", 1),
            WhereTerm::Eq("note; DROP TABLE role_user".to_string(), JsonValue::from(1)),
        ]));
        let mut values: Vec<SeaValue> = Vec::new();
        let mut n = 0usize;
        filters
            .render_and(DbBackend::Sqlite, &mut values, &mut n)
            .expect_err("a bad identifier nested in a group must be caught too");
        assert!(values.is_empty());
    }

    #[test]
    fn apply_pushes_every_term_onto_a_typed_builder() {
        let mut filters = PivotFilters::default();
        filters.push(eq("active", 1));
        filters.push_or(eq("pinned", 1));

        let builder = filters.apply(Builder::<()>::new());
        assert_eq!(builder.where_terms.len(), 1, "the or_ fold collapsed both");
        assert!(matches!(
            builder.where_terms.first(),
            Some(WhereTerm::Or(group)) if group.len() == 2
        ));
    }

    #[test]
    fn reject_mutation_passes_when_empty_and_fails_once_a_filter_is_set() {
        let mut filters = PivotFilters::default();
        filters.reject_mutation().expect("no filters, no objection");

        filters.push(eq("active", 1));
        let err = filters
            .reject_mutation()
            .expect_err("a filtered write must fail closed");
        assert!(
            format!("{err}").contains("reads only"),
            "message must say why, got: {err}"
        );
    }
}
