//! Model factories - produce randomized model instances for tests and
//! seed data with a Laravel-style fluent builder.
//!
//! ```rust,no_run
//! use suprnova::factory::Factory;
//! use fake::{Fake, Faker};
//! # #[derive(Clone)]
//! # struct User { is_admin: bool, name: String }
//! # impl fake::Dummy<Faker> for User {
//! #     fn dummy_with_rng<R: rand::Rng + ?Sized>(_: &Faker, rng: &mut R) -> Self {
//! #         User { is_admin: false, name: fake::faker::name::en::Name().fake_with_rng(rng) }
//! #     }
//! # }
//!
//! // The minimal hand-written form: pair a marker struct with a
//! // `Factory` impl that knows how to build one instance.
//! struct UserFactory;
//! impl Factory for UserFactory {
//!     type Model = User;
//!     fn definition() -> User {
//!         Faker.fake::<User>()  // assumes `User: fake::Dummy`
//!     }
//! }
//!
//! // Build one
//! let user = UserFactory::new().make();
//!
//! // Build many
//! let users = UserFactory::new().count(10).make_many();
//!
//! // Override per-call
//! let admin = UserFactory::new()
//!     .with(|u| u.is_admin = true)
//!     .make();
//! ```
//!
//! `create` / `create_many` (in `persist`) extend the builder with
//! SeaORM persistence. The fluent surface is intentionally close to
//! Laravel's `User::factory()->count(10)->create()` so the mental model
//! ports without translation.
//!
//! For the typical case where a model derives `fake::Dummy`, see
//! `#[derive(Factory)]` which generates the marker struct + impl from
//! a `#[factory(model = "...")]` attribute.

mod persist;
mod sequence;

pub use persist::{Persistable, persist_via_seaorm};
pub use sequence::Sequence;

/// A factory produces randomized instances of `Model`. Each call to
/// `definition()` returns a fresh, independently-randomized value -
/// the trait carries no per-instance state.
///
/// Implementors are typically zero-sized marker types so callers can
/// reach the factory by name (`UserFactory::new()`) without holding a
/// handle.
pub trait Factory {
    /// The struct this factory builds.
    type Model;

    /// Build one instance with all default-randomized fields. The
    /// builder's `with(...)` overrides run AFTER this returns, so
    /// implementations should populate every field they want
    /// randomized - overrides correct the parts the test cares about.
    fn definition() -> Self::Model
    where
        Self: Sized;

    /// Start a fluent builder. Default `count` is 1; default override
    /// list is empty.
    fn new() -> FactoryBuilder<Self::Model>
    where
        Self: Sized,
    {
        FactoryBuilder {
            count: 1,
            overrides: Vec::new(),
            factory_fn: Self::definition,
        }
    }

    /// Sugar for `Self::new().count(n)`. Matches Laravel's
    /// `Factory::times(int)` API for the "I want N of these" pattern
    /// without the extra method call.
    fn times(n: usize) -> FactoryBuilder<Self::Model, true>
    where
        Self: Sized,
    {
        Self::new().count(n)
    }
}

/// Boxed override closure shape - extracted into a type alias so the
/// builder's field reads clean and clippy's `type_complexity` lint is
/// satisfied at the public API boundary.
pub(crate) type Override<M> = Box<dyn Fn(&mut M) + Send + Sync + 'static>;

/// Fluent builder returned by [`Factory::new`]. Owns the per-instance
/// count and the list of override closures. `MANY` selects the terminal result:
/// `false` returns `M`, and `true` returns `Vec<M>` after `count` or `times`.
///
/// Boxed closures are `Send + Sync + 'static` so the builder itself is
/// `Send` - important for the async `create` / `create_many` paths,
/// which capture the builder across an `.await` point on the SeaORM
/// insert.
pub struct FactoryBuilder<M, const MANY: bool = false> {
    pub(crate) count: usize,
    pub(crate) overrides: Vec<Override<M>>,
    pub(crate) factory_fn: fn() -> M,
}

impl<M, const MANY: bool> FactoryBuilder<M, MANY> {
    /// Selects a vector result so `make` and `create` return exactly `n` models.
    /// The result type changes even when `n` is zero or one.
    pub fn count(self, n: usize) -> FactoryBuilder<M, true> {
        FactoryBuilder {
            count: n,
            overrides: self.overrides,
            factory_fn: self.factory_fn,
        }
    }

    /// Selects a vector result when you already hold a factory builder.
    pub fn times(self, n: usize) -> FactoryBuilder<M, true> {
        self.count(n)
    }

    /// Add an override closure that runs against every produced
    /// instance after `definition()`. Multiple `with` calls compose
    /// in registration order, so a later override can clobber an
    /// earlier one.
    pub fn with<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut M) + Send + Sync + 'static,
    {
        self.overrides.push(Box::new(f));
        self
    }

    /// Prepend an override closure to the front of the chain. The
    /// override runs BEFORE any other registered override, so
    /// downstream `with(...)` calls win on the same field.
    ///
    /// Mirrors Laravel's `Factory::prependState($state)` - useful
    /// when a state method wants to set a default that a caller can
    /// still override with a later `with(...)`.
    pub fn prepend<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut M) + Send + Sync + 'static,
    {
        self.overrides.insert(0, Box::new(f));
        self
    }

    /// Conditional builder extension - applies `f` to the builder
    /// only if `cond` is true; otherwise returns the builder
    /// unchanged. Mirrors Laravel's `Conditionable::when($cond, $cb)`
    /// for the "thread a flag through a chain" pattern without
    /// breaking the fluent style.
    ///
    /// ```ignore
    /// UserFactory::times(10)
    ///     .with(|u| u.active = true)
    ///     .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    ///     .create().await?;
    /// ```
    pub fn when<F>(self, cond: bool, f: F) -> Self
    where
        F: FnOnce(Self) -> Self,
    {
        if cond { f(self) } else { self }
    }

    /// Builds exactly one model when you need to discard a counted result.
    pub fn make_one(self) -> M {
        self.build_record()
    }

    fn build_record(&self) -> M {
        let mut model = (self.factory_fn)();
        for override_fn in &self.overrides {
            override_fn(&mut model);
        }
        model
    }

    /// Build `count` instances in memory, applying overrides to each.
    /// Each instance is independently randomized via a fresh call to
    /// `definition()`.
    pub fn make_many(self) -> Vec<M> {
        let FactoryBuilder {
            count,
            overrides,
            factory_fn,
        } = self;
        (0..count)
            .map(|_| {
                let mut model = factory_fn();
                for o in &overrides {
                    o(&mut model);
                }
                model
            })
            .collect()
    }
}

impl<M> FactoryBuilder<M> {
    /// Builds one model so the default factory result stays a single value.
    pub fn make(self) -> M {
        self.make_one()
    }
}

impl<M> FactoryBuilder<M, true> {
    /// Builds the selected number of models with a vector checked by the compiler.
    pub fn make(self) -> Vec<M> {
        self.make_many()
    }
}

impl<M: Persistable + 'static, const MANY: bool> FactoryBuilder<M, MANY> {
    /// Persists exactly one model when you need to discard a counted result.
    pub async fn create_one(self) -> Result<M, crate::FrameworkError> {
        self.make_one().persist().await
    }

    async fn persist_many(models: Vec<M>) -> Result<Vec<M>, crate::FrameworkError> {
        let mut out = Vec::with_capacity(models.len());
        for model in models {
            out.push(model.persist().await?);
        }
        Ok(out)
    }
}

impl<M: Persistable + 'static> FactoryBuilder<M> {
    /// Persists one model so the default factory result stays a single value.
    pub async fn create(self) -> Result<M, crate::FrameworkError> {
        self.create_one().await
    }

    /// Persists a count or per-record attribute maps over the factory definition.
    /// Attributes require a model that implements serde serialization and deserialization.
    /// Inserts run in order and stop on error. Use a transaction for atomicity.
    pub async fn create_many<R: FactoryRecords<M>>(
        self,
        records: R,
    ) -> Result<Vec<M>, crate::FrameworkError> {
        Self::persist_many(records.into_models(self)?).await
    }

    /// Persists one model without waking its lifecycle event listeners.
    pub async fn create_quietly(self) -> Result<M, crate::FrameworkError> {
        crate::seed::without_events(self.create()).await
    }

    /// Persists explicit records without waking their lifecycle event listeners.
    pub async fn create_many_quietly<R: FactoryRecords<M>>(
        self,
        records: R,
    ) -> Result<Vec<M>, crate::FrameworkError> {
        crate::seed::without_events(self.create_many(records)).await
    }
}

impl<M: Persistable + 'static> FactoryBuilder<M, true> {
    /// Persists the selected number of models and returns a typed vector.
    /// Inserts stop on error. Use a transaction if all rows must roll back together.
    pub async fn create(self) -> Result<Vec<M>, crate::FrameworkError> {
        Self::persist_many(self.make()).await
    }

    /// Keeps counted builders compatible with the explicit many terminal.
    pub async fn create_many(self) -> Result<Vec<M>, crate::FrameworkError> {
        self.create().await
    }

    /// Persists the selected models without waking lifecycle event listeners.
    pub async fn create_quietly(self) -> Result<Vec<M>, crate::FrameworkError> {
        crate::seed::without_events(self.create()).await
    }

    /// Keeps counted quiet inserts compatible with the explicit many terminal.
    pub async fn create_many_quietly(self) -> Result<Vec<M>, crate::FrameworkError> {
        self.create_quietly().await
    }
}

/// Supplies a count or attribute maps so one factory can create different records.
/// A count works for every model. Attribute maps need serde to overlay typed fields.
pub trait FactoryRecords<M> {
    /// Builds all records before persistence so invalid attributes insert no rows.
    fn into_models(self, builder: FactoryBuilder<M>) -> Result<Vec<M>, crate::FrameworkError>;
}

impl<M> FactoryRecords<M> for usize {
    fn into_models(self, builder: FactoryBuilder<M>) -> Result<Vec<M>, crate::FrameworkError> {
        Ok(builder.count(self).make())
    }
}

impl<M> FactoryRecords<M> for Vec<crate::Attrs>
where
    M: serde::Serialize + serde::de::DeserializeOwned,
{
    fn into_models(self, builder: FactoryBuilder<M>) -> Result<Vec<M>, crate::FrameworkError> {
        self.into_iter()
            .map(|attrs| {
                let model = builder.build_record();
                let mut value = serde_json::to_value(model).map_err(|error| {
                    crate::FrameworkError::internal(format!(
                        "factory definition serialization: {error}"
                    ))
                })?;
                let fields = value.as_object_mut().ok_or_else(|| {
                    crate::FrameworkError::bad_request("factory attributes require an object model")
                })?;
                for (name, value) in attrs.0 {
                    if !fields.contains_key(&name) {
                        return Err(crate::FrameworkError::bad_request(format!(
                            "factory attribute `{name}` is not a serialized model field"
                        )));
                    }
                    fields.insert(name, value);
                }
                serde_json::from_value(value).map_err(|error| {
                    crate::FrameworkError::bad_request(format!("factory attributes: {error}"))
                })
            })
            .collect()
    }
}

impl<M, const N: usize> FactoryRecords<M> for [crate::Attrs; N]
where
    M: serde::Serialize + serde::de::DeserializeOwned,
{
    fn into_models(self, builder: FactoryBuilder<M>) -> Result<Vec<M>, crate::FrameworkError> {
        Vec::from(self).into_models(builder)
    }
}
