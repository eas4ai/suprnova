//! SQLite migration entry point.

use sea_orm::DatabaseConnection;

use super::{MigrationReport, create_lookup_indexes};
use crate::{Result, schema::AuthSchema};

/// Add guarded token and ceremony lookup indexes on SQLite.
///
/// SQLite executes each index DDL atomically; no destructive operation is
/// emitted by this function.
pub async fn apply<S: AuthSchema>(db: &DatabaseConnection) -> Result<MigrationReport>
where
    S::Token: crate::schema::TokenFields,
    S::Ceremony: crate::schema::CeremonyFields,
    <S::Token as crate::schema::EntityBinding>::Entity: sea_orm::EntityTrait,
    <S::Token as crate::schema::EntityBinding>::Column: sea_orm::ColumnTrait,
    <S::Ceremony as crate::schema::EntityBinding>::Entity: sea_orm::EntityTrait,
    <S::Ceremony as crate::schema::EntityBinding>::Column: sea_orm::ColumnTrait,
{
    create_lookup_indexes::<S>(db).await
}
