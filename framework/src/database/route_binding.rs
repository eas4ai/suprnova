//! Route binding: turning a route parameter into a value.
//!
//! A handler argument binds from the route parameter its name names
//! whenever its type implements [`RouteBinding`]:
//!
//! ```rust,ignore
//! use suprnova::{handler, json_response, Response};
//! use crate::models::Post;
//!
//! // GET /posts/{post}
//! #[handler]
//! pub async fn show(post: Post) -> Response {
//!     json_response!({ "title": post.title })
//! }
//! ```
//!
//! `#[model]` implements [`RouteBinding`] for every model it defines. The
//! lookup goes through the model's query, so global scopes, the soft-delete
//! filter and the model's connection apply. The parameter matches the
//! model's route key, its primary key unless `#[model(route_key = "...")]`
//! names another column, or the column a `{post:slug}` segment names. A
//! value that does not parse as the column's type and a value that matches
//! no row both answer 404, with a body that names the model and never
//! repeats the value.
//!
//! Two older forms keep working: [`RouteParam<T>`], which binds as `T`
//! does, and a SeaORM row whose entity implements
//! [`EntityExt`](crate::database::EntityExt) (`post: post::Model`), which
//! binds by primary key.
//!
//! # Security: binding is identity, not authorization
//!
//! Route binding answers **"does this row exist?"** - it does **not**
//! answer **"is the current user allowed to see this row?"**. Authorize
//! against the bound value with `#[authorize("view", post)]` on the handler
//! or with [`Gate::authorize`](crate::authorization::Gate::authorize) in its
//! body. See `framework/tests/authorization/` for working examples.
//!
//! The 404 returned on a missing row does NOT prevent IDOR probing - a 404
//! vs. 403 split discloses existence. If existence-disclosure matters in
//! your threat model, return 404 from the policy too (so unauthorized rows
//! look identical to non-existent ones).

use crate::eloquent::{EagerLoadDispatch, EloquentModel};
use crate::error::FrameworkError;
use async_trait::async_trait;
use sea_orm::{
    EntityTrait, Iterable, ModelTrait as SeaModelTrait, PrimaryKeyToColumn, PrimaryKeyTrait,
};
use std::any::{Any, TypeId};
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::str::FromStr;

/// A boxed future a route lookup returns.
pub type RouteLookup<'a, T> = Pin<Box<dyn Future<Output = Result<T, FrameworkError>> + Send + 'a>>;

/// Wrapper that binds as the value inside it binds.
///
/// `RouteParam<T>` implements [`RouteBinding`] whenever `T` does, and every
/// lookup goes to `T`. Handlers written with it keep working; new code can
/// name the type alone (`user: User`).
///
/// ```rust,ignore
/// use suprnova::{handler, json_response, RouteParam, Response};
/// use crate::models::User;
///
/// #[handler]
/// pub async fn show(RouteParam(user): RouteParam<User>) -> Response {
///     json_response!({ "name": user.name })
/// }
/// ```
///
/// `Deref<Target = T>` lets handlers read fields directly through the
/// wrapper (`route_param.name`) when destructuring isn't convenient.
#[derive(Debug, Clone)]
pub struct RouteParam<M>(pub M);

impl<M> RouteParam<M> {
    /// Move the inner model out of the wrapper.
    pub fn into_inner(self) -> M {
        self.0
    }
}

impl<M> Deref for RouteParam<M> {
    type Target = M;
    fn deref(&self) -> &M {
        &self.0
    }
}

impl<M> DerefMut for RouteParam<M> {
    fn deref_mut(&mut self) -> &mut M {
        &mut self.0
    }
}

/// A value a child lookup found, held without its type.
///
/// [`RouteBinding::resolve_child_route_binding`] returns one because the
/// parent does not know the child's type: the handler argument does, and
/// the framework turns the value back into it. A type that resolves its
/// own children wraps what it found with [`BoundChild::new`].
pub struct BoundChild {
    value: Box<dyn Any + Send + Sync>,
    type_name: &'static str,
}

impl BoundChild {
    /// Hold `value` for the handler argument that binds it.
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Self {
            value: Box::new(value),
            type_name: std::any::type_name::<T>(),
        }
    }

    /// Take the value back as a `T`, or get `self` back when it is
    /// another type.
    pub fn downcast<T: Any>(self) -> Result<T, Self> {
        let type_name = self.type_name;
        self.value
            .downcast::<T>()
            .map(|value| *value)
            .map_err(|value| Self { value, type_name })
    }

    /// The name of the type held, for error messages.
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }
}

impl std::fmt::Debug for BoundChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundChild")
            .field("type_name", &self.type_name)
            .finish_non_exhaustive()
    }
}

/// One column a binding can match, with how a path segment parses as the
/// column's type.
///
/// `#[model]` records one for every column of a model. A column whose type
/// cannot be parsed from a path segment, such as a JSON or a byte column,
/// carries no parser, and a route that names it as a binding field is
/// refused at startup.
#[derive(Clone, Copy)]
pub struct RouteColumn {
    name: &'static str,
    parse: Option<fn(&str) -> Option<serde_json::Value>>,
    format: Option<crate::eloquent::unique_id::UniqueIdKind>,
}

impl RouteColumn {
    /// A column named `name`, parsed by `parse`. `None` for a column whose
    /// type cannot be parsed from a path segment.
    pub const fn new(
        name: &'static str,
        parse: Option<fn(&str) -> Option<serde_json::Value>>,
    ) -> Self {
        Self {
            name,
            parse,
            format: None,
        }
    }

    /// The same column, whose values must also be well-formed identifiers
    /// of `kind`: a `unique_id` key, where a malformed value must not
    /// reach the database.
    pub const fn with_format(mut self, kind: crate::eloquent::unique_id::UniqueIdKind) -> Self {
        self.format = Some(kind);
        self
    }

    /// The column's name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Whether a path segment can be parsed as the column's type.
    pub fn parses(&self) -> bool {
        self.parse.is_some()
    }

    /// `value` as the column's type, ready to compare in a query. `None`
    /// when it does not parse, or breaks the column's identifier format.
    pub fn parse(&self, value: &str) -> Option<serde_json::Value> {
        if let Some(kind) = self.format
            && !kind.is_valid(value)
        {
            return None;
        }
        self.parse.and_then(|parse| parse(value))
    }
}

impl std::fmt::Debug for RouteColumn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouteColumn")
            .field("name", &self.name)
            .field("parses", &self.parse.is_some())
            .field("format", &self.format)
            .finish()
    }
}

/// How a type finds a child parameter scoped to it (BIND-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildBindings {
    /// The type declares no relations. A route that scopes a child under
    /// it is refused at startup.
    None,
    /// Children are found through the relations `#[model]` registered for
    /// the type, and every route that scopes a child under it is checked
    /// against them at startup.
    Relations,
    /// The type resolves children itself, in its own
    /// [`RouteBinding::resolve_child_route_binding`]. Nothing is checked.
    Custom,
}

/// What the router needs to know about a type that binds from a route, to
/// check routes at startup and to answer a miss.
#[derive(Debug, Clone)]
pub struct RouteBindingInfo {
    type_id: TypeId,
    name: &'static str,
    columns: Option<Vec<RouteColumn>>,
    children: ChildBindings,
    unit_enum: bool,
}

impl RouteBindingInfo {
    /// The defaults for `T`: named after the type, no columns recorded, no
    /// children.
    pub fn of<T: 'static>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            name: short_type_name::<T>(),
            columns: None,
            children: ChildBindings::None,
            unit_enum: false,
        }
    }

    /// The same, under another name in 404 bodies and startup errors.
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// The same, with the columns a binding field may name. Without them a
    /// binding field is passed to the type's lookup unchecked.
    pub fn with_columns(mut self, columns: Vec<RouteColumn>) -> Self {
        self.columns = Some(columns);
        self
    }

    /// The same, finding children as `children` says.
    pub fn with_children(mut self, children: ChildBindings) -> Self {
        self.children = children;
        self
    }

    /// The same, for a unit-only enum: a value that matches no variant
    /// answers 404 without calling the route's `missing()` handler, as
    /// Laravel's enum binding does.
    pub fn unit_enum(mut self) -> Self {
        self.unit_enum = true;
        self
    }

    /// The type a binding produces. A wrapper reports the type it wraps,
    /// so a binder or a relation that returns the inner type fits it.
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// The name 404 bodies and startup errors use.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The columns a binding field may name, when recorded.
    pub fn columns(&self) -> Option<&[RouteColumn]> {
        self.columns.as_deref()
    }

    /// How the type finds a child parameter scoped to it.
    pub fn children(&self) -> ChildBindings {
        self.children
    }

    /// Whether the type is a unit-only enum.
    pub fn is_unit_enum(&self) -> bool {
        self.unit_enum
    }
}

/// The last segment of `T`'s path, without generic arguments: `Post` for
/// `app::models::Post`.
fn short_type_name<T: ?Sized>() -> &'static str {
    let full = std::any::type_name::<T>();
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base)
}

/// A type that binds from a route parameter: Laravel's `UrlRoutable`.
///
/// A handler argument whose type implements it binds from the route
/// parameter its name names. `#[model]` implements it for every model,
/// `#[derive(RouteBinding)]` for a unit-only enum, and any other type can
/// implement it by hand.
///
/// The lookups return `Ok(None)` for "no such value": a value that does not
/// parse, or matches nothing. The router answers that with the route's
/// `missing()` handler, or a 404 naming the type. `Err` is a failure of the
/// lookup itself, such as a database error.
///
/// A route's `with_trashed()` selects the soft-deletable lookups; every
/// other route uses the plain ones. A type with no soft deletes keeps the
/// defaults, which call the plain lookups.
///
/// `AutoRouteBinding` is a second name for this trait.
#[async_trait]
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be bound from a route parameter",
    note = "a handler argument binds from the route when its type implements `suprnova::RouteBinding`: a `#[model]`, a `#[derive(RouteBinding)]` enum, or a type that implements it by hand"
)]
pub trait RouteBinding: Sized + Send + Sync + 'static {
    /// The column a parameter without a binding field matches.
    fn route_key_name() -> &'static str;

    /// The value `route()` fills a parameter with for this value.
    fn route_key(&self) -> String;

    /// The value of the binding column `field`, which `route()` fills a
    /// `{post:slug}` parameter with. `None` when the type has no such
    /// column. The default knows the route key only.
    fn route_field(&self, field: &str) -> Option<String> {
        (field == Self::route_key_name()).then(|| self.route_key())
    }

    /// Look a value up by `value`, matching the column `field` names, or
    /// the route key when `field` is `None`.
    async fn resolve_route_binding(
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError>;

    /// [`Self::resolve_route_binding`], soft-deleted rows included. A
    /// route's `with_trashed()` selects it.
    async fn resolve_soft_deletable_route_binding(
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Self::resolve_route_binding(value, field).await
    }

    /// Look up the child parameter `child`, whose value is `value`, among
    /// the children this value owns, matching `field` or the child's route
    /// key. The default finds no children: it is an error, and a route
    /// that needs it is refused at startup unless
    /// [`Self::route_binding_info`] says the type resolves its own.
    async fn resolve_child_route_binding(
        &self,
        child: &str,
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<BoundChild>, FrameworkError> {
        let _ = (value, field);
        Err(FrameworkError::internal(format!(
            "`{}` does not resolve the scoped child parameter `{child}`",
            Self::route_binding_info().name()
        )))
    }

    /// [`Self::resolve_child_route_binding`], soft-deleted children
    /// included. A route's `with_trashed()` selects it.
    async fn resolve_soft_deletable_child_route_binding(
        &self,
        child: &str,
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<BoundChild>, FrameworkError> {
        self.resolve_child_route_binding(child, value, field).await
    }

    /// Look a value up by its route key, a miss answering
    /// [`FrameworkError::ModelNotFound`]. The form `AutoRouteBinding`
    /// callers used.
    async fn from_route_param(value: &str) -> Result<Self, FrameworkError> {
        match Self::resolve_route_binding(value, None).await? {
            Some(found) => Ok(found),
            None => Err(FrameworkError::model_not_found(
                Self::route_binding_info().name(),
            )),
        }
    }

    /// What the router checks routes against at startup, and names in a
    /// 404. The default names the type and records no columns and no
    /// children.
    fn route_binding_info() -> RouteBindingInfo {
        RouteBindingInfo::of::<Self>()
    }

    /// Turn a value a child lookup or a binder produced into `Self`. The
    /// default takes a `Self`; a wrapper takes what it wraps.
    #[doc(hidden)]
    fn __from_bound(value: BoundChild) -> Result<Self, BoundChild> {
        value.downcast::<Self>()
    }
}

pub use self::RouteBinding as AutoRouteBinding;

#[async_trait]
impl<T: RouteBinding> RouteBinding for RouteParam<T> {
    fn route_key_name() -> &'static str {
        T::route_key_name()
    }

    fn route_key(&self) -> String {
        self.0.route_key()
    }

    fn route_field(&self, field: &str) -> Option<String> {
        self.0.route_field(field)
    }

    async fn resolve_route_binding(
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Ok(T::resolve_route_binding(value, field)
            .await?
            .map(RouteParam))
    }

    async fn resolve_soft_deletable_route_binding(
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Ok(T::resolve_soft_deletable_route_binding(value, field)
            .await?
            .map(RouteParam))
    }

    async fn resolve_child_route_binding(
        &self,
        child: &str,
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<BoundChild>, FrameworkError> {
        self.0
            .resolve_child_route_binding(child, value, field)
            .await
    }

    async fn resolve_soft_deletable_child_route_binding(
        &self,
        child: &str,
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<BoundChild>, FrameworkError> {
        self.0
            .resolve_soft_deletable_child_route_binding(child, value, field)
            .await
    }

    fn route_binding_info() -> RouteBindingInfo {
        T::route_binding_info()
    }

    fn __from_bound(value: BoundChild) -> Result<Self, BoundChild> {
        T::__from_bound(value).map(RouteParam)
    }
}

/// The table and soft-delete column of a `#[model]`, which `#[model]`
/// records for every model. The bare `x::Model` form reads it: a row is
/// soft-deleted when the entity's table is a model's table and that
/// model's soft-delete column is set (BIND-008).
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct SoftDeleteTable {
    /// The model's table.
    pub table: &'static str,
    /// The model's soft-delete column, `""` without soft deletes.
    pub column: &'static str,
}

inventory::collect!(SoftDeleteTable);

/// The soft-delete column of the `#[model]` whose table is `table`, if
/// any declares soft deletes.
fn soft_delete_column_of(table: &str) -> Option<&'static str> {
    inventory::iter::<SoftDeleteTable>()
        .find(|entry| entry.table == table && !entry.column.is_empty())
        .map(|entry| entry.column)
}

/// A `sea_orm::Value` as the text a URL carries.
fn sea_value_text(value: &sea_orm::Value) -> Option<String> {
    json_text(crate::eloquent::model::sea_value_to_json_loose(value))
}

/// A JSON value as the text a URL carries: a string as itself, a number or
/// a boolean as written, `None` for null.
fn json_text(value: serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text),
        other => Some(other.to_string()),
    }
}

/// The primary-key column of a SeaORM entity.
fn primary_key_column<E: EntityTrait>() -> Option<E::Column> {
    E::PrimaryKey::iter().next().map(|key| key.into_column())
}

/// The bare `x::Model` form, for a SeaORM row whose entity implements
/// [`EntityExt`](crate::database::EntityExt). It binds by primary key
/// alone, reads the default connection, and applies no global scope. A row
/// whose table is a soft-deleting `#[model]`'s binds only through the
/// soft-deletable lookup, which a route's `with_trashed()` selects.
#[async_trait]
impl<M, E> RouteBinding for M
where
    M: SeaModelTrait<Entity = E> + Send + Sync + 'static,
    E: EntityTrait<Model = M> + crate::database::EntityExt + Sync,
    E::PrimaryKey: PrimaryKeyTrait,
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType: FromStr + Send,
{
    fn route_key_name() -> &'static str {
        primary_key_column::<E>()
            .map(|column| sea_orm::IdenStatic::as_str(&column))
            .unwrap_or("id")
    }

    fn route_key(&self) -> String {
        primary_key_column::<E>()
            .and_then(|column| sea_value_text(&self.get(column)))
            .unwrap_or_default()
    }

    fn route_field(&self, field: &str) -> Option<String> {
        let column = <E::Column as FromStr>::from_str(field).ok()?;
        sea_value_text(&self.get(column))
    }

    async fn resolve_route_binding(
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        bare_lookup::<M, E>(value, field, false).await
    }

    async fn resolve_soft_deletable_route_binding(
        value: &str,
        field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        bare_lookup::<M, E>(value, field, true).await
    }

    fn route_binding_info() -> RouteBindingInfo {
        // The body names the entity's module, `post` for `post::Model`.
        let full = std::any::type_name::<M>();
        let module = full.rsplit("::").nth(1).unwrap_or(full);
        // It binds by its primary key alone, so that is the one column a
        // binding field may name.
        let key: RouteColumn = RouteColumn::new(
            <M as RouteBinding>::route_key_name(),
            Some(parse_key::<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>),
        );
        RouteBindingInfo::of::<M>()
            .named(module)
            .with_columns(vec![key])
    }
}

/// Whether `value` parses as the key type `K`. The bare form compares the
/// parsed key itself, so the JSON this returns is never read; it says
/// "parses" to the startup check.
fn parse_key<K: FromStr>(value: &str) -> Option<serde_json::Value> {
    value.parse::<K>().ok().map(|_| serde_json::Value::Null)
}

/// The bare form's lookup: the row by primary key, through
/// [`EntityExt::find_by_pk`](crate::database::EntityExt::find_by_pk), then
/// the soft-delete check of BIND-008.
async fn bare_lookup<M, E>(
    value: &str,
    field: Option<&str>,
    trashed: bool,
) -> Result<Option<M>, FrameworkError>
where
    M: SeaModelTrait<Entity = E> + Send + Sync + 'static,
    E: EntityTrait<Model = M> + crate::database::EntityExt + Sync,
    E::PrimaryKey: PrimaryKeyTrait,
    <E::PrimaryKey as PrimaryKeyTrait>::ValueType: FromStr + Send,
{
    if let Some(field) = field
        && field != <M as RouteBinding>::route_key_name()
    {
        return Err(FrameworkError::internal(format!(
            "the bare `{}` form binds by its primary key only, not by `{field}`; \
             bind the `#[model]` struct to match another column",
            <M as RouteBinding>::route_binding_info().name()
        )));
    }
    let Ok(key) = value.parse::<<E::PrimaryKey as PrimaryKeyTrait>::ValueType>() else {
        return Ok(None);
    };
    let Some(row) = <E as crate::database::EntityExt>::find_by_pk(key).await? else {
        return Ok(None);
    };
    if trashed {
        return Ok(Some(row));
    }
    let table = sea_orm::EntityName::table_name(&E::default());
    if let Some(column) = soft_delete_column_of(table)
        && let Ok(column) = <E::Column as FromStr>::from_str(column)
        && sea_value_text(&row.get(column)).is_some()
    {
        return Ok(None);
    }
    Ok(Some(row))
}

/// What `#[model]` records for route binding, for every model: its columns,
/// its route key, and how to find a child through one of its relations.
///
/// The default lookups, [`resolve_model_route_binding`] and
/// [`resolve_model_child_route_binding`], read it. A model with
/// `#[model(custom_route_binding)]` still implements it, so its own
/// [`RouteBinding`] can call them.
pub trait ModelRouteBinding: EloquentModel {
    /// Every column of the model, with how a path segment parses as it.
    fn route_columns() -> Vec<RouteColumn>;

    /// The model's route key: `#[model(route_key = "...")]`, else its
    /// primary key.
    fn model_route_key_name() -> &'static str;

    /// Find the child `value` among the rows the relation `relation`
    /// returns, matching `field` or the child's route key. `#[model]`
    /// writes one arm per relation; a name it does not declare is an
    /// error.
    #[doc(hidden)]
    fn __route_child<'a>(
        &'a self,
        relation: &'a str,
        value: &'a str,
        field: Option<&'a str>,
        trashed: bool,
    ) -> RouteLookup<'a, Option<BoundChild>>;
}

/// The column of `M` a binding matches: `field`, else `route_key`, with
/// its parser. `None` when the model has no such column.
fn binding_column<M: ModelRouteBinding>(
    field: Option<&str>,
    route_key: &str,
) -> Option<RouteColumn> {
    let name = field.unwrap_or(route_key);
    M::route_columns()
        .into_iter()
        .find(|column| column.name() == name)
}

/// The default lookup `#[model]` generates: the row whose column `field`,
/// else the route key, holds `value`, read through the model's query, so
/// global scopes, the soft-delete filter and the model's connection apply.
/// `trashed` lifts the soft-delete filter alone.
///
/// A value that does not parse as the column's type, or breaks a
/// `unique_id` key's format, matches nothing: `Ok(None)`, as a missing row
/// is. A model that replaces its binding with
/// `#[model(custom_route_binding)]` can still call this.
pub async fn resolve_model_route_binding<M>(
    value: &str,
    field: Option<&str>,
    trashed: bool,
) -> Result<Option<M>, FrameworkError>
where
    M: ModelRouteBinding + crate::eloquent::Model + Send + Sync,
    M: From<<<M as EloquentModel>::Entity as EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + EagerLoadDispatch,
    <<M as EloquentModel>::Entity as EntityTrait>::Model: From<M>
        + sea_orm::IntoActiveModel<<<M as EloquentModel>::Entity as EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <<M as EloquentModel>::Entity as EntityTrait>::ActiveModel: Send,
    <<<M as EloquentModel>::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    let Some(column) = binding_column::<M>(field, M::model_route_key_name()) else {
        return Err(FrameworkError::internal(format!(
            "`{}` has no column `{}` to bind by",
            short_type_name::<M>(),
            field.unwrap_or(M::model_route_key_name())
        )));
    };
    let Some(parsed) = column.parse(value) else {
        return Ok(None);
    };
    let mut query = M::query();
    if trashed {
        query = query.lift_soft_deletes();
    }
    query
        .filter(format!("{}.{}", M::TABLE, column.name()), parsed)
        .first()
        .await
}

/// The default child lookup `#[model]` generates: the child parameter
/// `child` is looked up through the relation named by its plural as
/// `Str::plural` forms it (`posts` for `post`).
pub async fn resolve_model_child_route_binding<M: ModelRouteBinding>(
    parent: &M,
    child: &str,
    value: &str,
    field: Option<&str>,
    trashed: bool,
) -> Result<Option<BoundChild>, FrameworkError> {
    let relation = child_relation_name(child);
    parent.__route_child(&relation, value, field, trashed).await
}

/// The relation a scoped child parameter is found through: its name in the
/// plural, as `Str::plural` forms it, `-` read as `_`.
pub(crate) fn child_relation_name(child: &str) -> String {
    crate::strings::Str::plural(&child.replace('-', "_"), 2)
}

/// [`RouteBindingInfo`] for a `#[model]` with the generated binding: its
/// columns, and children through its relations.
pub fn model_route_binding_info<M: ModelRouteBinding + 'static>() -> RouteBindingInfo {
    RouteBindingInfo::of::<M>()
        .with_columns(M::route_columns())
        .with_children(ChildBindings::Relations)
}

/// The value of the column `field` of a `#[model]` row, as the text a URL
/// carries. `None` for an unknown column or a null value.
pub fn model_route_field<M>(model: &M, field: &str) -> Option<String>
where
    M: crate::eloquent::Model,
    M: From<<<M as EloquentModel>::Entity as EntityTrait>::Model>,
    <<M as EloquentModel>::Entity as EntityTrait>::Model: From<M>
        + sea_orm::IntoActiveModel<<<M as EloquentModel>::Entity as EntityTrait>::ActiveModel>
        + serde::Serialize
        + Send
        + Sync,
    <<M as EloquentModel>::Entity as EntityTrait>::ActiveModel: Send,
    <<<M as EloquentModel>::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    model.field_value(field).and_then(json_text)
}

/// The child lookup one relation arm runs: the rows `relation` returns,
/// read through the child's own query, narrowed to the one whose column
/// `field`, else the child's route key, holds `value`. The column is
/// qualified with the child's table, so a relation that joins a pivot or an
/// intermediate table reads it unambiguously.
#[doc(hidden)]
pub async fn __route_child_lookup<C, R>(
    relation: R,
    value: &str,
    field: Option<&str>,
    trashed: bool,
) -> Result<Option<BoundChild>, FrameworkError>
where
    R: crate::eloquent::relations::RouteChildRelation<C>,
    C: ModelRouteBinding + RouteBinding + crate::eloquent::Model + Send + Sync,
    C: From<<<C as EloquentModel>::Entity as EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + EagerLoadDispatch,
    <<C as EloquentModel>::Entity as EntityTrait>::Model: From<C>
        + sea_orm::IntoActiveModel<<<C as EloquentModel>::Entity as EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <<C as EloquentModel>::Entity as EntityTrait>::ActiveModel: Send,
    <<<C as EloquentModel>::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    let Some(column) = binding_column::<C>(field, <C as RouteBinding>::route_key_name()) else {
        return Err(FrameworkError::internal(format!(
            "`{}` has no column `{}` to bind by",
            short_type_name::<C>(),
            field.unwrap_or(<C as RouteBinding>::route_key_name())
        )));
    };
    let Some(parsed) = column.parse(value) else {
        return Ok(None);
    };
    let _ = relation.__route_child_query()?;
    let mut query = C::query();
    if trashed {
        query = query.lift_soft_deletes();
    }
    let found = query
        .filter(format!("{}.{}", C::TABLE, column.name()), parsed)
        .first()
        .await?;
    Ok(found.map(BoundChild::new))
}

/// Autoref-specialised probe for a column's path-segment parser, used by
/// the code `#[model]` generates. A type that is `FromStr` and `Serialize`
/// parses, and so does an `Option` of one (a nullable column); a JSON
/// value and any other type do not.
#[doc(hidden)]
pub struct __ColumnProbe<T>(std::marker::PhantomData<fn() -> T>);

impl<T> __ColumnProbe<T> {
    /// A probe for `T`.
    #[doc(hidden)]
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for __ColumnProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse `value` as `T` and hand it to a query as JSON.
fn parse_as<T: FromStr + serde::Serialize>(value: &str) -> Option<serde_json::Value> {
    value
        .parse::<T>()
        .ok()
        .and_then(|parsed| serde_json::to_value(parsed).ok())
}

/// A column's path-segment parser, picked by the column's type. The
/// generated code calls it on `&&&&probe`, and method lookup tries the
/// impls from the most references down: a JSON value, or an `Option` of
/// one, never parses (a path segment is never compared with one); then a
/// type that is `FromStr` and `Serialize` parses; then an `Option` of such
/// a type (a nullable column) parses as its inner type; and any other type
/// does not parse. One trait for every kind means the generated code
/// imports one name, and uses it wherever it imports it.
#[doc(hidden)]
pub trait __ColumnParser {
    /// The column's parser, or `None` when a path segment never matches it.
    fn __column_parser(&self) -> Option<fn(&str) -> Option<serde_json::Value>>;
}
impl __ColumnParser for &&&__ColumnProbe<serde_json::Value> {
    fn __column_parser(&self) -> Option<fn(&str) -> Option<serde_json::Value>> {
        None
    }
}
impl __ColumnParser for &&&__ColumnProbe<Option<serde_json::Value>> {
    fn __column_parser(&self) -> Option<fn(&str) -> Option<serde_json::Value>> {
        None
    }
}
impl<T: FromStr + serde::Serialize> __ColumnParser for &&__ColumnProbe<T> {
    fn __column_parser(&self) -> Option<fn(&str) -> Option<serde_json::Value>> {
        Some(parse_as::<T>)
    }
}
impl<T: FromStr + serde::Serialize> __ColumnParser for &__ColumnProbe<Option<T>> {
    fn __column_parser(&self) -> Option<fn(&str) -> Option<serde_json::Value>> {
        Some(parse_as::<T>)
    }
}
impl<T> __ColumnParser for __ColumnProbe<T> {
    fn __column_parser(&self) -> Option<fn(&str) -> Option<serde_json::Value>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eloquent::unique_id::UniqueIdKind;

    /// The parser the generated code picks for a column of type `$ty`,
    /// through the same chain of references.
    macro_rules! parser_of {
        ($ty:ty) => {
            (&&&&__ColumnProbe::<$ty>::new()).__column_parser()
        };
    }

    #[test]
    fn column_probe_picks_the_parser_by_type() {
        let int = parser_of!(i64);
        assert_eq!(int.and_then(|p| p("7")), Some(serde_json::json!(7)));
        assert_eq!(int.and_then(|p| p("7abc")), None);

        let optional = parser_of!(Option<String>);
        assert_eq!(
            optional.and_then(|p| p("slug")),
            Some(serde_json::json!("slug"))
        );

        assert!(parser_of!(serde_json::Value).is_none());
        assert!(parser_of!(Vec<u8>).is_none());
    }

    #[test]
    fn a_unique_id_column_refuses_a_malformed_value() {
        let parser = parser_of!(String);
        let column = RouteColumn::new("id", parser).with_format(UniqueIdKind::UuidV4);
        assert_eq!(column.parse("not-a-uuid"), None);
        let id = "67e55044-10b1-426f-9247-bb680e5fe0c8";
        assert_eq!(column.parse(id), Some(serde_json::json!(id)));
    }

    #[test]
    fn bound_child_round_trips_its_type() {
        let child = BoundChild::new(42_u32);
        let back = child.downcast::<String>().expect_err("not a String");
        assert_eq!(back.downcast::<u32>().ok(), Some(42));
    }

    #[test]
    fn short_type_name_strips_path_and_generics() {
        assert_eq!(short_type_name::<String>(), "String");
        assert_eq!(short_type_name::<RouteParam<u8>>(), "RouteParam");
    }

    #[test]
    fn child_relation_is_the_plural_of_the_parameter() {
        assert_eq!(child_relation_name("post"), "posts");
        assert_eq!(child_relation_name("blog_post"), "blog_posts");
        assert_eq!(child_relation_name("category"), "categories");
    }
}
