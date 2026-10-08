//! SeaORM connection facade bound to one [`AuthSchema`](crate::schema::AuthSchema).

use std::marker::PhantomData;

use sea_orm::DatabaseConnection;

use super::Store;
use crate::schema::AuthSchema;

/// Generic storage facade for an application-owned schema.
#[derive(Clone)]
pub struct SeaOrmStorage<S: AuthSchema> {
    pub(crate) inner: Store<S>,
    pub(crate) marker: PhantomData<S>,
}

impl<S: AuthSchema> SeaOrmStorage<S> {
    /// Bind all storage primitives to the application's SeaORM connection.
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            inner: Store::new(db),
            marker: PhantomData,
        }
    }

    /// Borrow the application-owned connection.
    pub fn database(&self) -> &DatabaseConnection {
        self.inner.database()
    }

    /// Consume the facade and return its connection.
    pub fn into_database(self) -> DatabaseConnection {
        self.inner.db
    }
}
