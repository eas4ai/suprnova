//! `DocumentQuery<M>`: the query builder a document model's `query()`
//! answers (PAR-184), as laravel-mongodb's query builder.
//!
//! The builder collects conditions, an order, an offset, a limit and a
//! projection, and renders them as the find command's parts
//! ([`DocumentQuery::to_filter`]) or as aggregation stages
//! ([`DocumentQuery::to_pipeline`]), so a test reads the BSON without a
//! server. The model's key field is queried as `_id`, and a soft-deleting
//! model's trashed documents are left out unless the query asks for them.
//!
//! A chain the server cannot run as written is an error that names the
//! call, never a query that silently matches more or less: a write after
//! `skip` or `take` (MongoDB writes every matching document), `upsert`
//! after a condition (it matches by its own fields), or `distinct` sorted
//! by another field. A condition the builder cannot render, such as an
//! unknown operator, is kept and returned by the first call that renders or
//! runs the query.

use std::collections::HashSet;
use std::fmt;
use std::marker::PhantomData;
use std::ops::RangeInclusive;

use ::bson::raw::CString;
use ::bson::{Bson, Document, Regex};
use chrono::{NaiveDate, NaiveTime};
use futures::TryStreamExt;
use serde::Serialize;

use super::document::{DocumentKey, DocumentModel, model_name, negate, number, one};
use super::events;
use crate::eloquent::{Collection as Models, Direction};
use crate::error::FrameworkError;
use crate::pagination::{LengthAwarePaginator, Paginator};

/// Which documents of a soft-deleting model a query reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trashed {
    /// Untrashed documents only, the default.
    Without,
    /// Every document.
    With,
    /// Trashed documents only.
    Only,
}

/// The parts of the find command a [`DocumentQuery`] sends, as
/// [`DocumentQuery::to_filter`] renders them.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedFind {
    /// The query filter, the soft-delete condition included.
    pub filter: Document,
    /// The sort, `None` when the query has no order.
    pub sort: Option<Document>,
    /// The number of documents skipped.
    pub skip: Option<u64>,
    /// The most documents returned.
    pub limit: Option<u64>,
    /// The projection, `None` when the query reads whole documents.
    pub projection: Option<Document>,
}

/// A query over the documents of `M`. Build it with `M::query()`, chain
/// conditions, and finish with a call that runs it.
///
/// ```rust,ignore
/// use suprnova::{Direction, DocumentModel};
///
/// let adults = User::query()
///     .where_("age", ">=", 18)
///     .where_in("role", ["admin", "editor"])
///     .order_by("name", Direction::Asc)
///     .take(20)
///     .get()
///     .await?;
/// ```
///
/// Conditions are joined with AND; `or_where` starts a new group, and the
/// groups are joined with OR, so AND binds tighter, as in SQL.
pub struct DocumentQuery<M> {
    /// Groups of conditions: AND inside a group, OR between groups.
    groups: Vec<Vec<Document>>,
    sort: Document,
    skip: Option<u64>,
    limit: Option<u64>,
    projection: Option<Document>,
    trashed: Trashed,
    /// The first condition that could not be rendered, as its message.
    error: Option<String>,
    model: PhantomData<fn() -> M>,
}

impl<M> Clone for DocumentQuery<M> {
    fn clone(&self) -> Self {
        Self {
            groups: self.groups.clone(),
            sort: self.sort.clone(),
            skip: self.skip,
            limit: self.limit,
            projection: self.projection.clone(),
            trashed: self.trashed,
            error: self.error.clone(),
            model: PhantomData,
        }
    }
}

impl<M> fmt::Debug for DocumentQuery<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentQuery")
            .field("model", &model_name::<M>())
            .field("groups", &self.groups)
            .field("sort", &self.sort)
            .field("skip", &self.skip)
            .field("limit", &self.limit)
            .field("projection", &self.projection)
            .field("trashed", &self.trashed)
            .field("error", &self.error)
            .finish()
    }
}

impl<M: DocumentModel> Default for DocumentQuery<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: DocumentModel> DocumentQuery<M> {
    /// A query over every document of `M`, trashed ones left out.
    pub fn new() -> Self {
        Self {
            groups: vec![Vec::new()],
            sort: Document::new(),
            skip: None,
            limit: None,
            projection: None,
            trashed: Trashed::Without,
            error: None,
            model: PhantomData,
        }
    }

    /// Include trashed documents. Laravel's `withTrashed`.
    pub fn with_trashed(mut self) -> Self {
        self.trashed = Trashed::With;
        self
    }

    /// Read trashed documents only. Laravel's `onlyTrashed`.
    pub fn only_trashed(mut self) -> Self {
        self.trashed = Trashed::Only;
        self
    }

    /// `field op value`, joined to the conditions before with AND. `op` is
    /// one of `=`, `!=` (or `<>`), `<`, `<=`, `>`, `>=`, `like` and
    /// `not like`. `like` takes an SQL pattern, `%` for any run of
    /// characters and `_` for one, and renders a case-insensitive regular
    /// expression anchored at both ends; other characters match
    /// literally.
    ///
    /// The method is `where_` because `where` is a Rust keyword. An
    /// unknown operator, or `like` with a value that is no string, makes
    /// the query an error that names `where_`.
    pub fn where_(self, field: &str, op: &str, value: impl Into<Bson>) -> Self {
        self.condition("where_", field, op, value.into(), false)
    }

    /// [`Self::where_`] joined to the conditions before with OR: it starts
    /// a new group, and the conditions after it join that group.
    pub fn or_where(self, field: &str, op: &str, value: impl Into<Bson>) -> Self {
        self.condition("or_where", field, op, value.into(), true)
    }

    /// `field` equal to one of `values`, with `$in`. No values match no
    /// document.
    pub fn where_in<V: Into<Bson>>(self, field: &str, values: impl IntoIterator<Item = V>) -> Self {
        let values: Vec<Bson> = values.into_iter().map(Into::into).collect();
        self.clause(field, one("$in", values))
    }

    /// `field` equal to none of `values`, with `$nin`.
    pub fn where_not_in<V: Into<Bson>>(
        self,
        field: &str,
        values: impl IntoIterator<Item = V>,
    ) -> Self {
        let values: Vec<Bson> = values.into_iter().map(Into::into).collect();
        self.clause(field, one("$nin", values))
    }

    /// `field` null or missing, as MongoDB matches `{field: null}`.
    pub fn where_null(self, field: &str) -> Self {
        self.clause(field, Bson::Null)
    }

    /// `field` present and not null.
    pub fn where_not_null(self, field: &str) -> Self {
        self.clause(field, one("$ne", Bson::Null))
    }

    /// `field` between the range's ends, both included.
    pub fn where_between<V: Into<Bson>>(self, field: &str, range: RangeInclusive<V>) -> Self {
        let (low, high) = range.into_inner();
        let mut condition = one("$gte", low);
        condition.insert("$lte", high);
        self.clause(field, condition)
    }

    /// The datetime `field` on the day `date`, in UTC: at or after its
    /// midnight and before the next one.
    pub fn where_date(self, field: &str, date: NaiveDate) -> Self {
        let Some(next) = date.succ_opt() else {
            return self.fail(format!(
                "where_date: {date} is the last date chrono represents, so it has no end"
            ));
        };
        let midnight = |day: NaiveDate| {
            Bson::DateTime(::bson::DateTime::from_millis(
                day.and_time(NaiveTime::MIN).and_utc().timestamp_millis(),
            ))
        };
        let mut condition = one("$gte", midnight(date));
        condition.insert("$lt", midnight(next));
        self.clause(field, condition)
    }

    /// `field` present in the document, null or not.
    pub fn where_exists(self, field: &str) -> Self {
        self.clause(field, one("$exists", true))
    }

    /// A filter written in MongoDB's query language, joined with AND as it
    /// is: its field names are not mapped, so the key is `_id` here.
    pub fn where_raw(self, filter: Document) -> Self {
        self.join(filter, false)
    }

    /// The document whose stored key is `key`.
    pub(crate) fn where_stored_key(self, key: Bson) -> Self {
        self.join(one("_id", key), false)
    }

    /// Sort by `field`; later calls sort ties of earlier ones.
    pub fn order_by(mut self, field: &str, direction: Direction) -> Self {
        let order = match direction {
            Direction::Asc => 1,
            Direction::Desc => -1,
        };
        self.sort.insert(storage::<M>(field), order);
        self
    }

    /// Skip the first `count` matching documents.
    pub fn skip(mut self, count: u64) -> Self {
        self.skip = Some(count);
        self
    }

    /// Return at most `count` documents.
    pub fn take(mut self, count: u64) -> Self {
        self.limit = Some(count);
        self
    }

    /// Read only `fields` of each document. A model reads a missing
    /// `Option` field as `None` and a missing `Vec` as empty, so `get`
    /// hydrates a projected model only when its other fields are of those
    /// kinds; [`Self::get_documents`] answers the projected documents as
    /// they are.
    pub fn project<F: AsRef<str>>(mut self, fields: impl IntoIterator<Item = F>) -> Self {
        let mut projection = Document::new();
        for field in fields {
            projection.insert(storage::<M>(field.as_ref()), 1);
        }
        self.projection = Some(projection);
        self
    }

    /// Group the matching documents by `fields`, for the aggregates of
    /// [`DocumentGroup`]. No fields put every matching document in one
    /// group. The query's order, skip, take and projection apply to the
    /// groups.
    pub fn group_by<F: AsRef<str>>(self, fields: impl IntoIterator<Item = F>) -> DocumentGroup<M> {
        let keys = fields
            .into_iter()
            .map(|field| {
                let field = field.as_ref();
                (output_name(field), storage::<M>(field))
            })
            .collect();
        DocumentGroup {
            query: self,
            keys,
            outputs: Vec::new(),
        }
    }

    /// The filter, sort, skip, limit and projection the query sends as a
    /// find command.
    ///
    /// # Errors
    ///
    /// When a condition could not be rendered; the error names the call.
    pub fn to_filter(&self) -> Result<RenderedFind, FrameworkError> {
        self.check()?;
        Ok(RenderedFind {
            filter: self.filter(),
            sort: (!self.sort.is_empty()).then(|| self.sort.clone()),
            skip: self.skip,
            limit: self.limit,
            projection: self.projection.clone(),
        })
    }

    /// The query as aggregation stages: `$match`, `$sort`, `$skip`,
    /// `$limit` and `$project`, each only when the query has it.
    ///
    /// # Errors
    ///
    /// As [`Self::to_filter`].
    pub fn to_pipeline(&self) -> Result<Vec<Document>, FrameworkError> {
        self.check()?;
        let mut stages = Vec::new();
        let filter = self.filter();
        if !filter.is_empty() {
            stages.push(one("$match", filter));
        }
        self.push_window(&mut stages);
        if let Some(projection) = &self.projection {
            stages.push(one("$project", projection.clone()));
        }
        Ok(stages)
    }

    // --- Reading ------------------------------------------------------------

    /// The matching documents as models. Fires `Retrieving` once, then
    /// `Retrieved` for each model.
    ///
    /// # Errors
    ///
    /// As [`Self::to_filter`]; when the connection is not registered or
    /// the query fails; and when a document does not read as `M`.
    pub async fn get(self) -> Result<Models<M>, FrameworkError> {
        self.check()?;
        events::retrieving::<M>().await?;
        let documents = self.get_documents().await?;
        let mut models = Vec::with_capacity(documents.len());
        for document in documents {
            let model = M::from_document(document)?;
            events::retrieved(&model).await?;
            models.push(model);
        }
        Ok(Models::from(models))
    }

    /// The matching documents as stored, without reading them as models:
    /// for a projected query, or for fields the model does not declare.
    ///
    /// # Errors
    ///
    /// As [`Self::get`], but no document is read as `M`.
    pub async fn get_documents(self) -> Result<Vec<Document>, FrameworkError> {
        let rendered = self.to_filter()?;
        if rendered.limit == Some(0) {
            return Ok(Vec::new());
        }
        let collection = M::collection()?;
        let mut find = collection.find(rendered.filter);
        if let Some(sort) = rendered.sort {
            find = find.sort(sort);
        }
        if let Some(skip) = rendered.skip {
            find = find.skip(skip);
        }
        if let Some(limit) = rendered.limit {
            find = find.limit(signed(limit));
        }
        if let Some(projection) = rendered.projection {
            find = find.projection(projection);
        }
        Ok(find.await?.try_collect().await?)
    }

    /// The first matching document, or `None`.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub async fn first(self) -> Result<Option<M>, FrameworkError> {
        Ok(self.take(1).get().await?.into_vec().into_iter().next())
    }

    /// The number of matching documents, after `skip` and within `take`.
    ///
    /// # Errors
    ///
    /// As [`Self::get_documents`].
    pub async fn count(self) -> Result<u64, FrameworkError> {
        self.check()?;
        if self.limit == Some(0) {
            return Ok(0);
        }
        let collection = M::collection()?;
        let mut count = collection.count_documents(self.filter());
        if let Some(skip) = self.skip {
            count = count.skip(skip);
        }
        if let Some(limit) = self.limit {
            count = count.limit(limit);
        }
        Ok(count.await?)
    }

    /// Whether any document matches.
    ///
    /// # Errors
    ///
    /// As [`Self::count`].
    pub async fn exists(self) -> Result<bool, FrameworkError> {
        Ok(self.take(1).count().await? > 0)
    }

    /// The value of `field` in each matching document, in order; null
    /// where a document lacks it. `field` may be a dotted path.
    ///
    /// # Errors
    ///
    /// As [`Self::get_documents`].
    pub async fn pluck(mut self, field: &str) -> Result<Vec<Bson>, FrameworkError> {
        let path = storage::<M>(field);
        self.projection = Some(one(path.clone(), 1));
        let documents = self.get_documents().await?;
        Ok(documents
            .iter()
            .map(|document| value_at(document, &path))
            .collect())
    }

    /// The distinct values of `field` among the matching documents. An
    /// array field gives its elements, as MongoDB's `distinct` does.
    ///
    /// # Errors
    ///
    /// When the query is ordered by another field, which the distinct
    /// values do not carry (the error names `distinct` and the field), and
    /// as [`Self::get_documents`].
    pub async fn distinct(self, field: &str) -> Result<Vec<Bson>, FrameworkError> {
        self.check()?;
        let path = storage::<M>(field);
        if let Some(other) = self.sort.keys().find(|key| **key != path) {
            return Err(FrameworkError::internal(format!(
                "distinct(\"{field}\") answers values of `{field}` only, so the order by \
                 `{other}` cannot sort them; order by `{field}` or drop the order"
            )));
        }
        let filter = self.filter();
        let collection = M::collection()?;
        if self.sort.is_empty() && self.skip.is_none() && self.limit.is_none() {
            return Ok(collection.distinct(&path, filter).await?);
        }
        let mut stages = Vec::new();
        if !filter.is_empty() {
            stages.push(one("$match", filter));
        }
        stages.push(one("$unwind", format!("${path}")));
        stages.push(one("$group", one("_id", format!("${path}"))));
        if let Some(order) = self.sort.get(&path) {
            stages.push(one("$sort", one("_id", order.clone())));
        }
        if let Some(skip) = self.skip {
            stages.push(one("$skip", signed(skip)));
        }
        if let Some(limit) = self.limit {
            stages.push(one("$limit", signed(limit)));
        }
        let documents: Vec<Document> = collection.aggregate(stages).await?.try_collect().await?;
        Ok(documents
            .into_iter()
            .map(|mut document| document.remove("_id").unwrap_or(Bson::Null))
            .collect())
    }

    /// The sum of `field` over the matching documents, `None` when none
    /// match. `order_by`, `skip` and `take` choose the documents first.
    ///
    /// # Errors
    ///
    /// When the query has a projection, which has nothing to select from
    /// one value, and as [`Self::get_documents`].
    pub async fn sum(self, field: &str) -> Result<Option<Bson>, FrameworkError> {
        self.aggregate("sum", "$sum", field).await
    }

    /// The average of `field`; see [`Self::sum`].
    ///
    /// # Errors
    ///
    /// As [`Self::sum`].
    pub async fn avg(self, field: &str) -> Result<Option<Bson>, FrameworkError> {
        self.aggregate("avg", "$avg", field).await
    }

    /// The least value of `field`; see [`Self::sum`].
    ///
    /// # Errors
    ///
    /// As [`Self::sum`].
    pub async fn min(self, field: &str) -> Result<Option<Bson>, FrameworkError> {
        self.aggregate("min", "$min", field).await
    }

    /// The greatest value of `field`; see [`Self::sum`].
    ///
    /// # Errors
    ///
    /// As [`Self::sum`].
    pub async fn max(self, field: &str) -> Result<Option<Bson>, FrameworkError> {
        self.aggregate("max", "$max", field).await
    }

    /// One page of `per_page` matching documents, the `page`th (1-based; 0
    /// reads as 1), with the total that `count` answers. A `skip` or
    /// `take` on the query is replaced, as Laravel's `paginate` does.
    ///
    /// # Errors
    ///
    /// `FrameworkError::param("per_page")` (400) for a `per_page` of 0, and
    /// as [`Self::get`].
    pub async fn paginate(
        mut self,
        per_page: u64,
        page: u64,
    ) -> Result<LengthAwarePaginator<M>, FrameworkError> {
        if per_page == 0 {
            return Err(FrameworkError::param("per_page"));
        }
        self.check()?;
        let page = page.max(1);
        let total = M::collection()?.count_documents(self.filter()).await?;
        self.skip = Some((page - 1).saturating_mul(per_page));
        self.limit = Some(per_page);
        let rows = self.get().await?.into_vec();
        Ok(LengthAwarePaginator::new(rows, total, per_page, page))
    }

    /// One page of `per_page` matching documents without a total: it reads
    /// one document more to know whether another page follows.
    ///
    /// # Errors
    ///
    /// As [`Self::paginate`].
    pub async fn simple_paginate(
        mut self,
        per_page: u64,
        page: u64,
    ) -> Result<Paginator<M>, FrameworkError> {
        if per_page == 0 {
            return Err(FrameworkError::param("per_page"));
        }
        let page = page.max(1);
        self.skip = Some((page - 1).saturating_mul(per_page));
        self.limit = Some(per_page.saturating_add(1));
        let mut rows = self.get().await?.into_vec();
        let has_more = rows.len() as u64 > per_page;
        rows.truncate(usize::try_from(per_page).unwrap_or(usize::MAX));
        Ok(Paginator::new(rows, page, per_page, has_more))
    }

    // --- Writing ------------------------------------------------------------

    /// Update every matching document and answer how many changed. A
    /// document of fields is written with `$set`, leaving other fields as
    /// they are; a document of update operators (`$set`, `$inc`, `$unset`,
    /// ...) is sent as it is. `updated_at` is set when the model manages
    /// timestamps. No model events fire, as for Laravel's query updates.
    ///
    /// # Errors
    ///
    /// After `skip` or `take`; when `update` is empty, mixes operators and
    /// fields, or writes the key; and when the write fails. Each names
    /// `update`.
    pub async fn update(self, update: Document) -> Result<u64, FrameworkError> {
        self.write_guard("update")?;
        let update = self.update_document(update, "update")?;
        let result = M::collection()?.update_many(self.filter(), update).await?;
        Ok(result.modified_count)
    }

    /// Insert each of `values`, or update the document whose `unique_by`
    /// fields equal the value's, and answer how many were inserted or
    /// changed. An update sets the value's fields and leaves the rest;
    /// timestamps are managed. Laravel's `upsert($values, $uniqueBy)`.
    ///
    /// # Errors
    ///
    /// When the query has conditions, an order, `skip` or `take`, which
    /// upsert cannot honour; when `unique_by` is empty or a value lacks one
    /// of its fields; when a value writes the key with other unique fields;
    /// and when a write fails. Each names `upsert`.
    pub async fn upsert(
        self,
        values: Vec<Document>,
        unique_by: &[&str],
    ) -> Result<u64, FrameworkError> {
        self.check()?;
        if self.has_conditions()
            || !self.sort.is_empty()
            || self.skip.is_some()
            || self.limit.is_some()
        {
            return Err(FrameworkError::internal(
                "upsert matches each value by its `unique_by` fields, so it takes no condition, \
                 order, skip or take; call it on Model::query()",
            ));
        }
        if unique_by.is_empty() {
            return Err(FrameworkError::internal(
                "upsert needs at least one `unique_by` field to match values by",
            ));
        }
        let mut writes = Vec::with_capacity(values.len());
        for value in values {
            let mut filter = Document::new();
            for field in unique_by {
                let Some(matched) = value.get(*field) else {
                    return Err(FrameworkError::internal(format!(
                        "upsert: a value lacks its unique field `{field}`"
                    )));
                };
                filter.insert(storage::<M>(field), matched.clone());
            }
            let gives_key = value.contains_key(M::KEY_FIELD) || value.contains_key("_id");
            if !gives_key
                && !filter.contains_key("_id")
                && <M::Key as DocumentKey>::generate().is_none()
            {
                return Err(FrameworkError::internal(format!(
                    "upsert: a value lacks the key `{}`, which {} cannot generate",
                    M::KEY_FIELD,
                    model_name::<M>()
                )));
            }
            let mut set = Document::new();
            let mut on_insert = Document::new();
            for (field, field_value) in value {
                let stored = storage::<M>(&field);
                if stored == "_id" {
                    // `_id` is written once, by the insert; an update
                    // never changes it.
                    if !filter.contains_key("_id") {
                        on_insert.insert(stored, field_value);
                    }
                } else {
                    set.insert(stored, field_value);
                }
            }
            if let Some((created_at, updated_at)) = M::TIMESTAMPS {
                let now = Bson::DateTime(::bson::DateTime::now());
                if !set.contains_key(updated_at) {
                    set.insert(updated_at, now.clone());
                }
                if !set.contains_key(created_at) {
                    on_insert.insert(created_at, now);
                }
            }
            let mut update = Document::new();
            if !set.is_empty() {
                update.insert("$set", set);
            }
            if !on_insert.is_empty() {
                update.insert("$setOnInsert", on_insert);
            }
            if update.is_empty() {
                // Only the unique fields: insert them when absent, which
                // MongoDB does from the filter.
                update.insert("$setOnInsert", filter.clone());
            }
            writes.push((filter, update));
        }
        let collection = M::collection()?;
        let mut written = 0;
        for (filter, update) in writes {
            let result = collection.update_one(filter, update).upsert(true).await?;
            written += result.modified_count + u64::from(result.upserted_id.is_some());
        }
        Ok(written)
    }

    /// Delete every matching document and answer how many. A
    /// soft-deleting model's documents are trashed instead, as Laravel's
    /// query `delete` does. No model events fire.
    ///
    /// # Errors
    ///
    /// After `skip` or `take`, naming `delete`, and when the write fails.
    pub async fn delete(self) -> Result<u64, FrameworkError> {
        self.write_guard("delete")?;
        let collection = M::collection()?;
        match M::SOFT_DELETES {
            Some(column) => {
                let now = Bson::DateTime(::bson::DateTime::now());
                let mut set = one(column, now.clone());
                if let Some((_, updated_at)) = M::TIMESTAMPS {
                    set.insert(updated_at, now);
                }
                let result = collection
                    .update_many(self.filter(), one("$set", set))
                    .await?;
                Ok(result.modified_count)
            }
            None => Ok(collection.delete_many(self.filter()).await?.deleted_count),
        }
    }

    /// Remove every matching document, trashed or not, and answer how
    /// many.
    ///
    /// # Errors
    ///
    /// After `skip` or `take`, naming `force_delete`, and when the write
    /// fails.
    pub async fn force_delete(self) -> Result<u64, FrameworkError> {
        self.write_guard("force_delete")?;
        Ok(M::collection()?
            .delete_many(self.filter())
            .await?
            .deleted_count)
    }

    /// Add `by` to `field` in every matching document, with `$inc`, and
    /// answer how many changed.
    ///
    /// # Errors
    ///
    /// After `skip` or `take`; when `by` is no integer or float or `field`
    /// is the key; and when the write fails. Each names `increment`.
    pub async fn increment(self, field: &str, by: impl Into<Bson>) -> Result<u64, FrameworkError> {
        let amount = number(by.into(), "increment");
        self.operator("increment", "$inc", field, amount).await
    }

    /// Subtract `by` from `field`; [`Self::increment`] with the amount
    /// negated.
    ///
    /// # Errors
    ///
    /// As [`Self::increment`], naming `decrement`.
    pub async fn decrement(self, field: &str, by: impl Into<Bson>) -> Result<u64, FrameworkError> {
        let amount = number(by.into(), "decrement").and_then(|amount| negate(amount, "decrement"));
        self.operator("decrement", "$inc", field, amount).await
    }

    /// Append `value` to the array `field` of every matching document,
    /// with `$push`.
    ///
    /// # Errors
    ///
    /// After `skip` or `take`; when `value` has no BSON form or `field` is
    /// the key; and when the write fails. Each names `push`.
    pub async fn push(self, field: &str, value: impl Serialize) -> Result<u64, FrameworkError> {
        let value = to_bson(&value, "push");
        self.operator("push", "$push", field, value).await
    }

    /// Remove every element equal to `value` from the array `field`, with
    /// `$pull`.
    ///
    /// # Errors
    ///
    /// As [`Self::push`], naming `pull`.
    pub async fn pull(self, field: &str, value: impl Serialize) -> Result<u64, FrameworkError> {
        let value = to_bson(&value, "pull");
        self.operator("pull", "$pull", field, value).await
    }

    /// Remove `fields` from every matching document, with `$unset`.
    ///
    /// # Errors
    ///
    /// After `skip` or `take`, when a field is the key, and when the write
    /// fails. Each names `unset`.
    pub async fn unset<F: AsRef<str>>(
        self,
        fields: impl IntoIterator<Item = F>,
    ) -> Result<u64, FrameworkError> {
        self.write_guard("unset")?;
        let mut unset = Document::new();
        for field in fields {
            let stored = writable::<M>(field.as_ref(), "unset")?;
            unset.insert(stored, "");
        }
        if unset.is_empty() {
            return Err(FrameworkError::internal("unset needs at least one field"));
        }
        let result = M::collection()?
            .update_many(self.filter(), one("$unset", unset))
            .await?;
        Ok(result.modified_count)
    }

    // --- Internals ----------------------------------------------------------

    fn fail(mut self, message: String) -> Self {
        if self.error.is_none() {
            self.error = Some(message);
        }
        self
    }

    fn check(&self) -> Result<(), FrameworkError> {
        match &self.error {
            Some(message) => Err(FrameworkError::internal(message.clone())),
            None => Ok(()),
        }
    }

    fn has_conditions(&self) -> bool {
        self.groups.iter().any(|group| !group.is_empty())
    }

    /// Join `clause` to the last group, or start a new group when `or` and
    /// the last group has conditions. A leading `or` is a plain AND.
    fn join(mut self, clause: Document, or: bool) -> Self {
        let last_has_conditions = self.groups.last().is_some_and(|group| !group.is_empty());
        match self.groups.last_mut() {
            Some(group) if !(or && last_has_conditions) => group.push(clause),
            _ => self.groups.push(vec![clause]),
        }
        self
    }

    fn clause(self, field: &str, condition: impl Into<Bson>) -> Self {
        let field = storage::<M>(field);
        self.join(one(field, condition), false)
    }

    fn condition(self, call: &str, field: &str, op: &str, value: Bson, or: bool) -> Self {
        match render_condition(call, op, value) {
            Ok(condition) => {
                let field = storage::<M>(field);
                self.join(one(field, condition), or)
            }
            Err(message) => self.fail(message),
        }
    }

    /// The filter the query sends: the soft-delete condition and the
    /// groups.
    fn filter(&self) -> Document {
        let groups: Vec<Document> = self
            .groups
            .iter()
            .filter(|group| !group.is_empty())
            .map(|group| and(group.clone()))
            .collect();
        let conditions = match groups.len() {
            0 => Document::new(),
            1 => groups.into_iter().next().unwrap_or_default(),
            _ => one("$or", groups),
        };
        let scope = match (M::SOFT_DELETES, self.trashed) {
            (Some(column), Trashed::Without) => Some(one(column, Bson::Null)),
            (Some(column), Trashed::Only) => Some(one(column, one("$ne", Bson::Null))),
            _ => None,
        };
        match scope {
            None => conditions,
            Some(scope) if conditions.is_empty() => scope,
            Some(scope) => and(vec![scope, conditions]),
        }
    }

    /// The `$sort`, `$skip` and `$limit` stages.
    fn push_window(&self, stages: &mut Vec<Document>) {
        if !self.sort.is_empty() {
            stages.push(one("$sort", self.sort.clone()));
        }
        if let Some(skip) = self.skip {
            stages.push(one("$skip", signed(skip)));
        }
        if let Some(limit) = self.limit {
            stages.push(one("$limit", signed(limit)));
        }
    }

    /// Refuse a write the server would apply to every match.
    fn write_guard(&self, call: &str) -> Result<(), FrameworkError> {
        self.check()?;
        for (set, name) in [
            (self.skip.is_some(), "skip"),
            (self.limit.is_some(), "take"),
        ] {
            if set {
                return Err(FrameworkError::internal(format!(
                    "{call} cannot honour {name}: MongoDB writes every matching document; \
                     select the documents by their keys instead"
                )));
            }
        }
        Ok(())
    }

    /// The update operators `update` sends for `update`.
    fn update_document(&self, update: Document, call: &str) -> Result<Document, FrameworkError> {
        let operators = update.keys().filter(|key| key.starts_with('$')).count();
        let mut update = if update.is_empty() {
            return Err(FrameworkError::internal(format!(
                "{call} needs at least one field or operator"
            )));
        } else if operators == 0 {
            one("$set", map_fields::<M>(update))
        } else if operators == update.len() {
            let mut mapped = Document::new();
            for (operator, fields) in update {
                match fields {
                    Bson::Document(fields) => mapped.insert(operator, map_fields::<M>(fields)),
                    other => mapped.insert(operator, other),
                };
            }
            mapped
        } else {
            return Err(FrameworkError::internal(format!(
                "{call} mixes update operators and fields; put the fields under `$set`"
            )));
        };
        let mut touched = HashSet::new();
        for (_, fields) in update.iter() {
            if let Bson::Document(fields) = fields {
                if fields.contains_key("_id") {
                    return Err(FrameworkError::internal(format!(
                        "{call} cannot write the key `_id`: MongoDB never changes it"
                    )));
                }
                touched.extend(fields.keys().cloned());
            }
        }
        if let Some((_, updated_at)) = M::TIMESTAMPS
            && !touched.contains(updated_at)
        {
            let now = Bson::DateTime(::bson::DateTime::now());
            match update.get_mut("$set") {
                Some(Bson::Document(set)) => {
                    set.insert(updated_at, now);
                }
                Some(_) => {
                    return Err(FrameworkError::internal(format!(
                        "{call}: `$set` must be a document"
                    )));
                }
                None => {
                    update.insert("$set", one(updated_at, now));
                }
            }
        }
        Ok(update)
    }

    /// A one-field update operator over every match.
    async fn operator(
        self,
        call: &str,
        operator: &str,
        field: &str,
        value: Result<Bson, FrameworkError>,
    ) -> Result<u64, FrameworkError> {
        self.write_guard(call)?;
        let value = value?;
        let field = writable::<M>(field, call)?;
        let mut update = one(operator, one(field, value));
        if operator == "$inc"
            && let Some((_, updated_at)) = M::TIMESTAMPS
        {
            update.insert(
                "$set",
                one(updated_at, Bson::DateTime(::bson::DateTime::now())),
            );
        }
        let result = M::collection()?.update_many(self.filter(), update).await?;
        Ok(result.modified_count)
    }

    async fn aggregate(
        self,
        call: &str,
        operator: &str,
        field: &str,
    ) -> Result<Option<Bson>, FrameworkError> {
        self.check()?;
        if self.projection.is_some() {
            return Err(FrameworkError::internal(format!(
                "{call} answers one value, so a projection has nothing to select; drop project"
            )));
        }
        if self.limit == Some(0) {
            return Ok(None);
        }
        let mut stages = Vec::new();
        let filter = self.filter();
        if !filter.is_empty() {
            stages.push(one("$match", filter));
        }
        self.push_window(&mut stages);
        let mut group = one("_id", Bson::Null);
        group.insert("value", one(operator, format!("${}", storage::<M>(field))));
        stages.push(one("$group", group));
        let documents: Vec<Document> = M::collection()?
            .aggregate(stages)
            .await?
            .try_collect()
            .await?;
        Ok(documents
            .into_iter()
            .next()
            .and_then(|mut document| document.remove("value")))
    }
}

/// The groups of a query and their aggregates, as `group_by` answers them.
/// Each aggregate adds one field to every group's result: `count`, and
/// `sum_<field>`, `avg_<field>`, `min_<field>` and `max_<field>`, a dotted
/// path's dots written as underscores.
///
/// ```rust,ignore
/// let per_role = User::query()
///     .where_("active", "=", true)
///     .group_by(["role"])
///     .count()
///     .avg("age")
///     .get()
///     .await?;
/// // [{ "role": "admin", "count": 2, "avg_age": 41.5 }, ...]
/// ```
pub struct DocumentGroup<M> {
    query: DocumentQuery<M>,
    /// The group keys: the output name and the stored path.
    keys: Vec<(String, String)>,
    /// The aggregates: the output name and the accumulator.
    outputs: Vec<(String, Document)>,
}

impl<M> fmt::Debug for DocumentGroup<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentGroup")
            .field("query", &self.query)
            .field("keys", &self.keys)
            .field("outputs", &self.outputs)
            .finish()
    }
}

impl<M: DocumentModel> DocumentGroup<M> {
    /// The number of documents in each group, as `count`.
    pub fn count(self) -> Self {
        self.output("count".to_owned(), one("$sum", 1))
    }

    /// The sum of `field` in each group, as `sum_<field>`.
    pub fn sum(self, field: &str) -> Self {
        self.accumulate("sum", field)
    }

    /// The average of `field` in each group, as `avg_<field>`.
    pub fn avg(self, field: &str) -> Self {
        self.accumulate("avg", field)
    }

    /// The least value of `field` in each group, as `min_<field>`.
    pub fn min(self, field: &str) -> Self {
        self.accumulate("min", field)
    }

    /// The greatest value of `field` in each group, as `max_<field>`.
    pub fn max(self, field: &str) -> Self {
        self.accumulate("max", field)
    }

    /// Sort the groups by `field`, a group key's output name or an
    /// aggregate's name, as the result documents spell it.
    pub fn order_by(mut self, field: &str, direction: Direction) -> Self {
        let order = match direction {
            Direction::Asc => 1,
            Direction::Desc => -1,
        };
        self.query.sort.insert(field, order);
        self
    }

    /// Skip the first `count` groups.
    pub fn skip(mut self, count: u64) -> Self {
        self.query = self.query.skip(count);
        self
    }

    /// Return at most `count` groups.
    pub fn take(mut self, count: u64) -> Self {
        self.query = self.query.take(count);
        self
    }

    /// The aggregation stages: `$match`, `$group`, a `$project` that
    /// writes each group key as a field, then the order, skip, take and
    /// projection of the query.
    ///
    /// # Errors
    ///
    /// When a condition of the query could not be rendered.
    pub fn to_pipeline(&self) -> Result<Vec<Document>, FrameworkError> {
        self.query.check()?;
        let mut stages = Vec::new();
        let filter = self.query.filter();
        if !filter.is_empty() {
            stages.push(one("$match", filter));
        }
        let id = if self.keys.is_empty() {
            Bson::Null
        } else {
            let mut id = Document::new();
            for (name, path) in &self.keys {
                id.insert(name.clone(), format!("${path}"));
            }
            Bson::Document(id)
        };
        let mut group = one("_id", id);
        let mut project = one("_id", 0);
        for (name, _) in &self.keys {
            project.insert(name.clone(), format!("$_id.{name}"));
        }
        for (name, accumulator) in &self.outputs {
            group.insert(name.clone(), accumulator.clone());
            project.insert(name.clone(), 1);
        }
        stages.push(one("$group", group));
        stages.push(one("$project", project));
        self.query.push_window(&mut stages);
        if let Some(projection) = &self.query.projection {
            stages.push(one("$project", projection.clone()));
        }
        Ok(stages)
    }

    /// Run the aggregation and answer one document per group: the group
    /// keys and the aggregates.
    ///
    /// # Errors
    ///
    /// As [`Self::to_pipeline`], and when the connection is not registered
    /// or the aggregation fails.
    pub async fn get(self) -> Result<Vec<Document>, FrameworkError> {
        let stages = self.to_pipeline()?;
        if self.query.limit == Some(0) {
            return Ok(Vec::new());
        }
        Ok(M::collection()?
            .aggregate(stages)
            .await?
            .try_collect()
            .await?)
    }

    fn accumulate(self, function: &str, field: &str) -> Self {
        let name = format!("{function}_{}", output_name(field));
        let path = storage::<M>(field);
        self.output(name, one(format!("${function}"), format!("${path}")))
    }

    fn output(mut self, name: String, accumulator: Document) -> Self {
        self.outputs.retain(|(existing, _)| *existing != name);
        self.outputs.push((name, accumulator));
        self
    }
}

/// The stored path of `field`: the key's field name is `_id`.
fn storage<M: DocumentModel>(field: &str) -> String {
    match field.split_once('.') {
        Some((root, rest)) if root == M::KEY_FIELD => format!("_id.{rest}"),
        None if field == M::KEY_FIELD => "_id".to_owned(),
        _ => field.to_owned(),
    }
}

/// A field a write may name: any but the key.
fn writable<M: DocumentModel>(field: &str, call: &str) -> Result<String, FrameworkError> {
    let stored = storage::<M>(field);
    if stored == "_id" || stored.starts_with("_id.") {
        return Err(FrameworkError::internal(format!(
            "{call} cannot write the key `{field}`: MongoDB never changes `_id`"
        )));
    }
    Ok(stored)
}

/// `fields` with each field name mapped to its stored path.
fn map_fields<M: DocumentModel>(fields: Document) -> Document {
    fields
        .into_iter()
        .map(|(field, value)| (storage::<M>(&field), value))
        .collect()
}

/// An output field name: a dotted path's dots as underscores, since a
/// `$group` field name holds no dot.
fn output_name(field: &str) -> String {
    field.replace('.', "_")
}

/// Join `clauses` with AND: one document when no two name the same field
/// or operator, else `$and`, so no condition overwrites another.
fn and(clauses: Vec<Document>) -> Document {
    let mut seen = HashSet::new();
    let collides = clauses
        .iter()
        .flat_map(|clause| clause.keys())
        .any(|key| !seen.insert(key.clone()));
    if collides {
        return one("$and", clauses);
    }
    clauses.into_iter().flatten().collect()
}

/// The condition `op value` renders.
fn render_condition(call: &str, op: &str, value: Bson) -> Result<Bson, String> {
    let operator = match op.trim().to_ascii_lowercase().as_str() {
        "=" | "==" => return Ok(value),
        "!=" | "<>" => "$ne",
        "<" => "$lt",
        "<=" => "$lte",
        ">" => "$gt",
        ">=" => "$gte",
        "like" => return like(call, "like", value).map(|regex| one("$regex", regex).into()),
        "not like" => return like(call, "not like", value).map(|regex| one("$not", regex).into()),
        other => {
            return Err(format!(
                "{call} does not know the operator `{other}`; use =, !=, <>, <, <=, >, >=, like \
                 or not like"
            ));
        }
    };
    Ok(one(operator, value).into())
}

/// The regular expression an SQL `like` pattern renders: anchored at both
/// ends, case-insensitive, `%` any run of characters, `_` one character,
/// and every other character literal.
fn like(call: &str, op: &str, value: Bson) -> Result<Regex, String> {
    let pattern = match value {
        Bson::String(pattern) => pattern,
        other => {
            return Err(format!(
                "{call} with `{op}` needs a string pattern, not {other}"
            ));
        }
    };
    let mut regex = String::from("^");
    let mut literal = String::new();
    for character in pattern.chars() {
        match character {
            '%' | '_' => {
                regex.push_str(&regex::escape(&literal));
                literal.clear();
                regex.push_str(if character == '%' { ".*" } else { "." });
            }
            other => literal.push(other),
        }
    }
    regex.push_str(&regex::escape(&literal));
    regex.push('$');
    let pattern = CString::try_from(regex)
        .map_err(|error| format!("{call}: the `{op}` pattern holds a NUL character: {error}"))?;
    let options =
        CString::try_from("i").map_err(|error| format!("{call}: the `{op}` options: {error}"))?;
    Ok(Regex { pattern, options })
}

/// The value at the dotted `path` of `document`, or null.
fn value_at(document: &Document, path: &str) -> Bson {
    let mut current = document;
    let mut segments = path.split('.').peekable();
    while let Some(segment) = segments.next() {
        match (current.get(segment), segments.peek()) {
            (Some(value), None) => return value.clone(),
            (Some(Bson::Document(inner)), Some(_)) => current = inner,
            _ => return Bson::Null,
        }
    }
    Bson::Null
}

fn to_bson(value: &impl Serialize, call: &str) -> Result<Bson, FrameworkError> {
    ::bson::serialize_to_bson(value).map_err(|error| {
        FrameworkError::internal(format!("{call}: the value has no BSON form: {error}"))
    })
}

/// A count for a stage or an option that takes `i64`.
fn signed(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}
