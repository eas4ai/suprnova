//! Backend-specific, non-destructive migration scaffolding.
//!
//! Applications own table creation and migration ordering. These helpers only
//! add guarded lookup indexes through the application [`AuthSchema`].

use crate::schema::{AuthSchema, CeremonyFields, EntityBinding, TokenFields};
use crate::{Error, Result};
use sea_orm::sea_query::{Alias, Index, IntoIden, TableRef};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, EntityName, EntityTrait, Statement};

pub mod mysql;
pub mod postgres;
pub mod sqlite;

/// Migration execution summary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// Number of guarded statements submitted.
    pub statements: usize,
}

pub(crate) async fn create_lookup_indexes<S>(db: &DatabaseConnection) -> Result<MigrationReport>
where
    S: AuthSchema,
    S::Token: TokenFields,
    S::Ceremony: CeremonyFields,
    <S::Token as EntityBinding>::Entity: EntityTrait + Default,
    <S::Token as EntityBinding>::Column: sea_orm::ColumnTrait,
    <S::Ceremony as EntityBinding>::Entity: EntityTrait + Default,
    <S::Ceremony as EntityBinding>::Column: sea_orm::ColumnTrait,
{
    let backend = db.get_database_backend();
    let token_entity = <<S::Token as EntityBinding>::Entity as Default>::default();
    let ceremony_entity = <<S::Ceremony as EntityBinding>::Entity as Default>::default();
    let token_schema = token_entity.schema_name();
    let ceremony_schema = ceremony_entity.schema_name();
    validate_schema_refs(backend, token_schema, ceremony_schema)?;
    let token_table = token_entity.table_name().to_owned();
    let ceremony_table = ceremony_entity.table_name().to_owned();
    let token_name = format!("magnetar_{token_table}_token_lookup");
    let ceremony_name = format!("magnetar_{ceremony_table}_ceremony_lookup");
    let token_ref = if backend == DbBackend::Postgres {
        token_entity.table_ref()
    } else {
        TableRef::Table(Alias::new(token_table.as_str()).into_iden())
    };
    let ceremony_ref = if backend == DbBackend::Postgres {
        ceremony_entity.table_ref()
    } else {
        TableRef::Table(Alias::new(ceremony_table.as_str()).into_iden())
    };
    let mut statements = 0;
    if !has_index(db, token_schema, &token_table, &token_name).await?
        && has_columns(
            db,
            token_schema,
            &token_table,
            &[
                S::Token::digest_column_name(),
                S::Token::purpose_column_name(),
                S::Token::used_at_column_name(),
            ],
        )
        .await?
    {
        let token = Index::create()
            .name(token_name)
            .table(token_ref.clone())
            .col(S::Token::digest_column())
            .col(S::Token::purpose_column())
            .col(S::Token::used_at_column())
            .to_owned();
        db.execute(backend.build(&token)).await.map_err(db_error)?;
        statements += 1;
    }
    if !has_index(db, ceremony_schema, &ceremony_table, &ceremony_name).await?
        && has_columns(
            db,
            ceremony_schema,
            &ceremony_table,
            &[
                S::Ceremony::selector_column_name(),
                S::Ceremony::kind_column_name(),
                S::Ceremony::state_column_name(),
            ],
        )
        .await?
    {
        let ceremony = Index::create()
            .name(ceremony_name)
            .table(ceremony_ref)
            .col(S::Ceremony::selector_column())
            .col(S::Ceremony::kind_column())
            .col(S::Ceremony::state_column())
            .to_owned();
        db.execute(backend.build(&ceremony))
            .await
            .map_err(db_error)?;
        statements += 1;
    }
    Ok(MigrationReport { statements })
}

fn validate_schema_refs(
    backend: DbBackend,
    token_schema: Option<&str>,
    ceremony_schema: Option<&str>,
) -> Result<()> {
    if backend == DbBackend::MySql && (token_schema.is_some() || ceremony_schema.is_some()) {
        return Err(Error::InvalidInput {
            field: "mysql schema-qualified index".to_owned(),
            message: "MySQL index emission fails closed for schema-qualified bindings".to_owned(),
        });
    }
    Ok(())
}

pub(crate) async fn has_index<C: ConnectionTrait + ?Sized>(
    db: &C,
    schema: Option<&str>,
    table: &str,
    index: &str,
) -> Result<bool> {
    let backend = db.get_database_backend();
    let (sql, values) = match backend {
        DbBackend::Sqlite => (
            "SELECT 1 FROM sqlite_master WHERE type = 'index' AND tbl_name = ? AND name = ? LIMIT 1",
            vec![table.to_owned().into(), index.to_owned().into()],
        ),
        DbBackend::Postgres => (
            "SELECT 1 FROM pg_indexes WHERE schemaname = COALESCE($1, current_schema()) AND tablename = $2 AND indexname = $3 LIMIT 1",
            vec![
                schema.map(str::to_owned).into(),
                table.to_owned().into(),
                index.to_owned().into(),
            ],
        ),
        DbBackend::MySql => (
            "SELECT 1 FROM information_schema.statistics WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? AND index_name = ? LIMIT 1",
            vec![
                schema.map(str::to_owned).into(),
                table.to_owned().into(),
                index.to_owned().into(),
            ],
        ),
    };
    Ok(db
        .query_one(Statement::from_sql_and_values(backend, sql, values))
        .await
        .map_err(db_error)?
        .is_some())
}

pub(crate) async fn has_columns<C: ConnectionTrait + ?Sized>(
    db: &C,
    schema: Option<&str>,

    table: &str,
    columns: &[&str],
) -> Result<bool> {
    for column in columns {
        let backend = db.get_database_backend();
        let (sql, values) = match backend {
            DbBackend::Sqlite => (
                "SELECT 1 FROM pragma_table_info(?) WHERE name = ? LIMIT 1",
                vec![table.to_owned().into(), (*column).to_owned().into()],
            ),
            DbBackend::Postgres => (
                "SELECT 1 FROM information_schema.columns WHERE table_schema = COALESCE($1, current_schema()) AND table_name = $2 AND column_name = $3 LIMIT 1",
                vec![
                    schema.map(str::to_owned).into(),
                    table.to_owned().into(),
                    (*column).to_owned().into(),
                ],
            ),
            DbBackend::MySql => (
                "SELECT 1 FROM information_schema.columns WHERE table_schema = COALESCE(?, DATABASE()) AND table_name = ? AND column_name = ? LIMIT 1",
                vec![
                    schema.map(str::to_owned).into(),
                    table.to_owned().into(),
                    (*column).to_owned().into(),
                ],
            ),
        };
        if db
            .query_one(Statement::from_sql_and_values(backend, sql, values))
            .await
            .map_err(db_error)?
            .is_none()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn db_error(error: sea_orm::DbErr) -> Error {
    Error::Internal {
        message: format!("migration database error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_schema_refs;
    use sea_orm::DbBackend;

    #[test]
    fn mysql_schema_qualification_fails_closed() {
        assert!(validate_schema_refs(DbBackend::MySql, Some("tenant"), None).is_err());
        assert!(validate_schema_refs(DbBackend::Sqlite, Some("tenant"), None).is_ok());
    }
}
