//! SeaORM tables and store for the authorization server on SQLite,
//! PostgreSQL and MySQL/MariaDB.
//!
//! The four tables belong to the plugin, the way `auth_two_factor` belongs
//! to the second factor: `auth_oauth_clients` holds dynamically registered
//! clients, `auth_oauth_authorizations` pending authorizations and code
//! digests, `auth_oauth_grants` one row per approval and
//! `auth_oauth_tokens` the digest of every access and refresh token.
//! [`migrate`] creates them, and [`crate::default_schema::migrate`] calls it
//! when the `authorization-server` feature is on.
//!
//! Pending authorizations and codes live here rather than in the host's
//! ceremony table: their JSON outgrows the 255-byte `varbinary` that
//! column gets on MySQL.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Alias, Expr, Index, Query};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, DbBackend, EntityTrait, ExprTrait, QueryFilter, QueryOrder, QuerySelect,
    Schema, TransactionTrait,
};

use super::store::{
    AuthorizationRecord, AuthorizationStore, GrantRecord, NewTokenRecord, REFRESH_TOKEN_KIND,
    RegisteredClient, TokenRecord,
};
use crate::storage::db_error;
use crate::storage::migrations::has_index;
use crate::{Error, Result};

/// SeaORM entity for the `auth_oauth_clients` table.
#[allow(missing_docs)]
pub mod clients {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_oauth_clients")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub client_id: String,
        #[sea_orm(column_type = "Text")]
        pub client_name: Option<String>,
        #[sea_orm(column_type = "Text")]
        pub redirect_uris: String,
        #[sea_orm(column_type = "Text")]
        pub grant_types: String,
        #[sea_orm(column_type = "Text")]
        pub scope: Option<String>,
        #[sea_orm(column_type = "Text")]
        pub client_uri: Option<String>,
        #[sea_orm(column_type = "Text")]
        pub logo_uri: Option<String>,
        pub created_at: DateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// SeaORM entity for the `auth_oauth_authorizations` table.
#[allow(missing_docs)]
pub mod authorizations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_oauth_authorizations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub kind: String,
        pub state: String,
        pub user_id: String,
        #[sea_orm(column_type = "Text")]
        pub payload: String,
        pub expires_at: DateTimeUtc,
        pub created_at: DateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// SeaORM entity for the `auth_oauth_grants` table.
#[allow(missing_docs)]
pub mod grants {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_oauth_grants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub kind: String,
        pub user_id: Option<String>,
        #[sea_orm(column_type = "Text")]
        pub client_id: String,
        #[sea_orm(column_type = "Text")]
        pub client_name: String,
        #[sea_orm(column_type = "Text")]
        pub client_host: String,
        #[sea_orm(column_type = "Text")]
        pub scopes: String,
        #[sea_orm(column_type = "Text")]
        pub resource: String,
        pub auth_epoch: Option<i64>,
        pub access_kind: String,
        pub access_lifetime: Option<i64>,
        pub created_at: DateTimeUtc,
        pub last_used_at: Option<DateTimeUtc>,
        pub revoked_at: Option<DateTimeUtc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// SeaORM entity for the `auth_oauth_tokens` table.
#[allow(missing_docs)]
pub mod tokens {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_oauth_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub grant_id: String,
        pub kind: String,
        pub token_hash: String,
        #[sea_orm(column_type = "Text")]
        pub scopes: String,
        pub expires_at: DateTimeUtc,
        pub used_at: Option<DateTimeUtc>,
        pub revoked_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Create the authorization server's tables and indexes when they are
/// missing. Running it again changes nothing, and two processes running it
/// at once both succeed.
///
/// # Errors
///
/// Returns an error for a backend other than SQLite, PostgreSQL or MySQL,
/// or when the database refuses a statement.
pub async fn migrate(db: &DatabaseConnection) -> Result<()> {
    let backend = db.get_database_backend();
    if !matches!(
        backend,
        DbBackend::Sqlite | DbBackend::Postgres | DbBackend::MySql
    ) {
        return Err(Error::DependencyUnavailable {
            dependency: "database backend".to_owned(),
            message: format!("unsupported SeaORM database backend: {backend:?}"),
        });
    }
    let schema = Schema::new(backend);
    for statement in [
        schema
            .create_table_from_entity(clients::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(authorizations::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(grants::Entity)
            .if_not_exists()
            .to_owned(),
        schema
            .create_table_from_entity(tokens::Entity)
            .if_not_exists()
            .to_owned(),
    ] {
        db.execute(&statement)
            .await
            .map_err(|error| Error::Internal {
                message: format!("create authorization server table: {error}"),
            })?;
    }
    // The digest lookup must be unique: two tokens with one digest would let
    // a verification read either grant.
    ensure_index(
        db,
        "auth_oauth_tokens",
        "auth_oauth_tokens_token_hash_unique",
        &["token_hash"],
        true,
    )
    .await?;
    ensure_index(
        db,
        "auth_oauth_tokens",
        "auth_oauth_tokens_grant_id",
        &["grant_id"],
        false,
    )
    .await?;
    ensure_index(
        db,
        "auth_oauth_tokens",
        "auth_oauth_tokens_expires_at",
        &["expires_at"],
        false,
    )
    .await?;
    ensure_index(
        db,
        "auth_oauth_grants",
        "auth_oauth_grants_user_id",
        &["user_id"],
        false,
    )
    .await?;
    // Pruning selects grants by these two; without an index each batch
    // would scan, and lock, the whole table.
    ensure_index(
        db,
        "auth_oauth_grants",
        "auth_oauth_grants_revoked_at",
        &["revoked_at"],
        false,
    )
    .await?;
    ensure_index(
        db,
        "auth_oauth_grants",
        "auth_oauth_grants_created_at",
        &["created_at"],
        false,
    )
    .await?;
    ensure_index(
        db,
        "auth_oauth_authorizations",
        "auth_oauth_authorizations_user_id",
        &["user_id", "kind"],
        false,
    )
    .await?;
    ensure_index(
        db,
        "auth_oauth_authorizations",
        "auth_oauth_authorizations_expires_at",
        &["expires_at"],
        false,
    )
    .await
}

/// Create an index unless it exists. When the create fails because another
/// process created the index between the check and the create, the index
/// is there and the failure is not one.
async fn ensure_index(
    db: &DatabaseConnection,
    table: &str,
    name: &str,
    columns: &[&'static str],
    unique: bool,
) -> Result<()> {
    if has_index(db, None, table, name).await? {
        return Ok(());
    }
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    for column in columns {
        index.col(Alias::new(*column));
    }
    if unique {
        index.unique();
    }
    match db.execute(&index).await {
        Ok(_) => Ok(()),
        Err(error) => {
            if has_index(db, None, table, name).await? {
                Ok(())
            } else {
                Err(Error::Internal {
                    message: format!("create authorization server index {name}: {error}"),
                })
            }
        }
    }
}

/// The SeaORM implementation of [`AuthorizationStore`] over the tables
/// [`migrate`] creates.
#[derive(Clone)]
pub struct SqlAuthorizationStore {
    db: DatabaseConnection,
}

impl SqlAuthorizationStore {
    /// Bind the store to the application's database.
    #[must_use]
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

/// The live tokens of the grant row in the outer query: unrevoked, unspent
/// and unexpired.
fn live_tokens_of_grant(now: DateTime<Utc>) -> sea_orm::sea_query::SelectStatement {
    Query::select()
        .expr(Expr::val(1))
        .from(tokens::Entity)
        .and_where(
            Expr::col((tokens::Entity, tokens::Column::GrantId))
                .equals((grants::Entity, grants::Column::Id)),
        )
        .and_where(Expr::col((tokens::Entity, tokens::Column::RevokedAt)).is_null())
        .and_where(Expr::col((tokens::Entity, tokens::Column::UsedAt)).is_null())
        .and_where(Expr::col((tokens::Entity, tokens::Column::ExpiresAt)).gt(now))
        .to_owned()
}

fn join_scopes(scopes: &[String]) -> String {
    scopes.join(" ")
}

fn split_scopes(scopes: &str) -> Vec<String> {
    scopes.split_whitespace().map(str::to_owned).collect()
}

fn epoch_to_db(epoch: Option<u64>) -> Result<Option<i64>> {
    epoch
        .map(|value| {
            i64::try_from(value).map_err(|_| Error::InvalidInput {
                field: "auth_epoch".to_owned(),
                message: "exceeds the database integer range".to_owned(),
            })
        })
        .transpose()
}

fn grant_record(model: grants::Model) -> GrantRecord {
    GrantRecord {
        id: model.id,
        kind: model.kind,
        user_id: model.user_id,
        client_id: model.client_id,
        client_name: model.client_name,
        client_host: model.client_host,
        scopes: split_scopes(&model.scopes),
        resource: model.resource,
        // A negative epoch was never written by this store; reading it as
        // absent would skip the epoch check, so it reads as a value no
        // user holds instead.
        auth_epoch: model
            .auth_epoch
            .map(|value| u64::try_from(value).unwrap_or(u64::MAX)),
        access_kind: model.access_kind,
        // A negative lifetime was never written either; it reads as zero,
        // which mints tokens that are already expired.
        access_lifetime: model
            .access_lifetime
            .map(|seconds| std::time::Duration::from_secs(u64::try_from(seconds).unwrap_or(0))),
        created_at: model.created_at,
        last_used_at: model.last_used_at,
        revoked_at: model.revoked_at,
    }
}

fn token_record(model: tokens::Model) -> TokenRecord {
    TokenRecord {
        id: model.id,
        grant_id: model.grant_id,
        kind: model.kind,
        token_hash: model.token_hash,
        scopes: split_scopes(&model.scopes),
        expires_at: model.expires_at,
        used_at: model.used_at,
        revoked_at: model.revoked_at,
        created_at: model.created_at,
    }
}

fn token_model(grant_id: &str, token: NewTokenRecord, at: DateTime<Utc>) -> tokens::ActiveModel {
    tokens::ActiveModel {
        id: Set(token.id),
        grant_id: Set(grant_id.to_owned()),
        kind: Set(token.kind),
        token_hash: Set(token.token_hash),
        scopes: Set(join_scopes(&token.scopes)),
        expires_at: Set(token.expires_at),
        used_at: Set(None),
        revoked_at: Set(None),
        created_at: Set(at),
    }
}

async fn insert_tokens(
    transaction: &DatabaseTransaction,
    grant_id: &str,
    tokens: Vec<NewTokenRecord>,
    at: DateTime<Utc>,
) -> Result<()> {
    if tokens.is_empty() {
        return Ok(());
    }
    tokens::Entity::insert_many(
        tokens
            .into_iter()
            .map(|token| token_model(grant_id, token, at))
            .collect::<Vec<_>>(),
    )
    .exec(transaction)
    .await
    .map_err(db_error)?;
    Ok(())
}

/// Finish a transaction: commit what the operation decided to keep, roll
/// back everything else.
async fn finish<T>(
    transaction: DatabaseTransaction,
    outcome: Result<T>,
    keep: impl FnOnce(&T) -> bool,
) -> Result<T> {
    match outcome {
        Ok(value) if keep(&value) => {
            transaction.commit().await.map_err(db_error)?;
            Ok(value)
        }
        Ok(value) => {
            transaction.rollback().await.map_err(db_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = transaction.rollback().await;
            Err(error)
        }
    }
}

#[async_trait]
impl AuthorizationStore for SqlAuthorizationStore {
    async fn insert_client(&self, client: RegisteredClient) -> Result<()> {
        let redirect_uris =
            serde_json::to_string(&client.redirect_uris).map_err(|error| Error::Internal {
                message: format!("encode redirect URIs: {error}"),
            })?;
        clients::Entity::insert(clients::ActiveModel {
            client_id: Set(client.client_id),
            client_name: Set(client.client_name),
            redirect_uris: Set(redirect_uris),
            grant_types: Set(client.grant_types.join(" ")),
            scope: Set(client.scope.map(|scope| join_scopes(&scope))),
            client_uri: Set(client.client_uri),
            logo_uri: Set(client.logo_uri),
            created_at: Set(client.created_at),
        })
        .exec(&self.db)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn find_client(&self, client_id: &str) -> Result<Option<RegisteredClient>> {
        let Some(model) = clients::Entity::find_by_id(client_id.to_owned())
            .one(&self.db)
            .await
            .map_err(db_error)?
        else {
            return Ok(None);
        };
        let redirect_uris: Vec<String> =
            serde_json::from_str(&model.redirect_uris).map_err(|error| Error::Internal {
                message: format!(
                    "stored redirect URIs {}",
                    crate::crypto::decode_failure(&error)
                ),
            })?;
        Ok(Some(RegisteredClient {
            client_id: model.client_id,
            client_name: model.client_name,
            redirect_uris,
            grant_types: split_scopes(&model.grant_types),
            scope: model.scope.as_deref().map(split_scopes),
            client_uri: model.client_uri,
            logo_uri: model.logo_uri,
            created_at: model.created_at,
        }))
    }

    async fn create_grant(&self, grant: GrantRecord, tokens: Vec<NewTokenRecord>) -> Result<()> {
        let auth_epoch = epoch_to_db(grant.auth_epoch)?;
        let access_lifetime = grant
            .access_lifetime
            .map(|lifetime| {
                i64::try_from(lifetime.as_secs()).map_err(|_| Error::InvalidInput {
                    field: "access_lifetime".to_owned(),
                    message: "exceeds the database integer range".to_owned(),
                })
            })
            .transpose()?;
        let transaction = self.db.begin().await.map_err(db_error)?;
        let outcome = async {
            let at = grant.created_at;
            let grant_id = grant.id.clone();
            grants::Entity::insert(grants::ActiveModel {
                id: Set(grant.id),
                kind: Set(grant.kind),
                user_id: Set(grant.user_id),
                client_id: Set(grant.client_id),
                client_name: Set(grant.client_name),
                client_host: Set(grant.client_host),
                scopes: Set(join_scopes(&grant.scopes)),
                resource: Set(grant.resource),
                auth_epoch: Set(auth_epoch),
                access_kind: Set(grant.access_kind),
                access_lifetime: Set(access_lifetime),
                created_at: Set(at),
                last_used_at: Set(None),
                revoked_at: Set(None),
            })
            .exec(&transaction)
            .await
            .map_err(db_error)?;
            insert_tokens(&transaction, &grant_id, tokens, at).await
        }
        .await;
        finish(transaction, outcome, |_| true).await
    }

    async fn find_token(&self, token_hash: &str) -> Result<Option<TokenRecord>> {
        Ok(tokens::Entity::find()
            .filter(tokens::Column::TokenHash.eq(token_hash.to_owned()))
            .one(&self.db)
            .await
            .map_err(db_error)?
            .map(token_record))
    }

    async fn find_grant(&self, grant_id: &str) -> Result<Option<GrantRecord>> {
        Ok(grants::Entity::find_by_id(grant_id.to_owned())
            .one(&self.db)
            .await
            .map_err(db_error)?
            .map(grant_record))
    }

    async fn rotate_refresh_token(
        &self,
        refresh_token_id: &str,
        grant_id: &str,
        at: DateTime<Utc>,
        replacements: Vec<NewTokenRecord>,
    ) -> Result<bool> {
        let transaction = self.db.begin().await.map_err(db_error)?;
        let outcome = async {
            // Every write path locks the grant row before any of its token
            // rows, as revoke_grant does, so a rotation racing a revocation
            // waits instead of deadlocking. A revoked grant affects no row
            // here and the rotation loses.
            let touched = grants::Entity::update_many()
                .col_expr(grants::Column::LastUsedAt, Expr::value(at))
                .filter(grants::Column::Id.eq(grant_id.to_owned()))
                .filter(grants::Column::RevokedAt.is_null())
                .exec(&transaction)
                .await
                .map_err(db_error)?;
            if touched.rows_affected != 1 {
                return Ok(false);
            }
            // The conditional update is the single-winner guard: the
            // database lets exactly one statement move `used_at` off NULL,
            // and every concurrent one affects no row.
            let claimed = tokens::Entity::update_many()
                .col_expr(tokens::Column::UsedAt, Expr::value(at))
                .filter(tokens::Column::Id.eq(refresh_token_id.to_owned()))
                .filter(tokens::Column::GrantId.eq(grant_id.to_owned()))
                .filter(tokens::Column::Kind.eq(REFRESH_TOKEN_KIND))
                .filter(tokens::Column::UsedAt.is_null())
                .filter(tokens::Column::RevokedAt.is_null())
                .filter(tokens::Column::ExpiresAt.gt(at))
                .exec(&transaction)
                .await
                .map_err(db_error)?;
            if claimed.rows_affected != 1 {
                return Ok(false);
            }
            insert_tokens(&transaction, grant_id, replacements, at).await?;
            Ok(true)
        }
        .await;
        finish(transaction, outcome, |won| *won).await
    }

    async fn revoke_token(&self, token_id: &str, at: DateTime<Utc>) -> Result<bool> {
        let revoked = tokens::Entity::update_many()
            .col_expr(tokens::Column::RevokedAt, Expr::value(at))
            .filter(tokens::Column::Id.eq(token_id.to_owned()))
            .filter(tokens::Column::RevokedAt.is_null())
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(revoked.rows_affected == 1)
    }

    async fn revoke_grant(&self, grant_id: &str, at: DateTime<Utc>) -> Result<bool> {
        let transaction = self.db.begin().await.map_err(db_error)?;
        let outcome = async {
            let grant = grants::Entity::update_many()
                .col_expr(grants::Column::RevokedAt, Expr::value(at))
                .filter(grants::Column::Id.eq(grant_id.to_owned()))
                .filter(grants::Column::RevokedAt.is_null())
                .exec(&transaction)
                .await
                .map_err(db_error)?;
            tokens::Entity::update_many()
                .col_expr(tokens::Column::RevokedAt, Expr::value(at))
                .filter(tokens::Column::GrantId.eq(grant_id.to_owned()))
                .filter(tokens::Column::RevokedAt.is_null())
                .exec(&transaction)
                .await
                .map_err(db_error)?;
            Ok(grant.rows_affected == 1)
        }
        .await;
        finish(transaction, outcome, |_| true).await
    }

    async fn revoke_user_grants(&self, user_id: &str, at: DateTime<Utc>) -> Result<u64> {
        let transaction = self.db.begin().await.map_err(db_error)?;
        let outcome = async {
            // Grant rows first, then token rows: the order every write path
            // takes its locks in.
            let revoked = grants::Entity::update_many()
                .col_expr(grants::Column::RevokedAt, Expr::value(at))
                .filter(grants::Column::UserId.eq(user_id.to_owned()))
                .filter(grants::Column::RevokedAt.is_null())
                .exec(&transaction)
                .await
                .map_err(db_error)?;
            let user_grants = Query::select()
                .column(grants::Column::Id)
                .from(grants::Entity)
                .and_where(Expr::col(grants::Column::UserId).eq(user_id.to_owned()))
                .to_owned();
            tokens::Entity::update_many()
                .col_expr(tokens::Column::RevokedAt, Expr::value(at))
                .filter(tokens::Column::GrantId.in_subquery(user_grants))
                .filter(tokens::Column::RevokedAt.is_null())
                .exec(&transaction)
                .await
                .map_err(db_error)?;
            Ok(revoked.rows_affected)
        }
        .await;
        finish(transaction, outcome, |_| true).await
    }

    async fn touch_grant(&self, grant_id: &str, at: DateTime<Utc>) -> Result<()> {
        grants::Entity::update_many()
            .col_expr(grants::Column::LastUsedAt, Expr::value(at))
            .filter(grants::Column::Id.eq(grant_id.to_owned()))
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn list_user_grants(
        &self,
        user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<GrantRecord>> {
        // A grant whose tokens all expired, were spent or were revoked is
        // over; the database drops it, so the answer holds only live grants
        // however many dead ones the user has.
        Ok(grants::Entity::find()
            .filter(grants::Column::UserId.eq(user_id.to_owned()))
            .filter(grants::Column::RevokedAt.is_null())
            .filter(Expr::exists(live_tokens_of_grant(now)))
            .order_by_desc(grants::Column::CreatedAt)
            .all(&self.db)
            .await
            .map_err(db_error)?
            .into_iter()
            .map(grant_record)
            .collect())
    }

    async fn create_authorization(&self, record: AuthorizationRecord) -> Result<()> {
        // Steps live minutes; dropping the lapsed ones here keeps the table
        // at the size of the flows in progress without a scheduled job.
        authorizations::Entity::delete_many()
            .filter(authorizations::Column::ExpiresAt.lte(Utc::now()))
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        authorizations::Entity::insert(authorizations::ActiveModel {
            id: Set(record.id),
            kind: Set(record.kind),
            state: Set(record.state),
            user_id: Set(record.user_id),
            payload: Set(record.payload),
            expires_at: Set(record.expires_at),
            created_at: Set(record.created_at),
        })
        .exec(&self.db)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn find_authorization(
        &self,
        id: &str,
        kind: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<AuthorizationRecord>> {
        Ok(authorizations::Entity::find()
            .filter(authorizations::Column::Id.eq(id.to_owned()))
            .filter(authorizations::Column::Kind.eq(kind.to_owned()))
            .filter(authorizations::Column::ExpiresAt.gt(now))
            .one(&self.db)
            .await
            .map_err(db_error)?
            .map(|model| AuthorizationRecord {
                id: model.id,
                kind: model.kind,
                state: model.state,
                user_id: model.user_id,
                payload: model.payload,
                expires_at: model.expires_at,
                created_at: model.created_at,
            }))
    }

    async fn transition_authorization(
        &self,
        id: &str,
        kind: &str,
        expected: &str,
        next: &str,
        live_at: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        let mut update = authorizations::Entity::update_many()
            .col_expr(authorizations::Column::State, Expr::value(next.to_owned()))
            .filter(authorizations::Column::Id.eq(id.to_owned()))
            .filter(authorizations::Column::Kind.eq(kind.to_owned()))
            .filter(authorizations::Column::State.eq(expected.to_owned()));
        if let Some(live_at) = live_at {
            update = update.filter(authorizations::Column::ExpiresAt.gt(live_at));
        }
        let moved = update.exec(&self.db).await.map_err(db_error)?;
        Ok(moved.rows_affected == 1)
    }

    async fn consume_authorization(
        &self,
        id: &str,
        kind: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let deleted = authorizations::Entity::delete_many()
            .filter(authorizations::Column::Id.eq(id.to_owned()))
            .filter(authorizations::Column::Kind.eq(kind.to_owned()))
            .filter(authorizations::Column::ExpiresAt.gt(now))
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(deleted.rows_affected == 1)
    }

    async fn trim_authorizations(
        &self,
        user_id: &str,
        kind: &str,
        keep: u64,
        newest: &str,
    ) -> Result<u64> {
        // Steps are trimmed on every create, so at most a handful lie past
        // `keep`; the bound only stops one call from reading without end.
        // `newest` is set aside first: MySQL keeps whole seconds, so steps
        // created in one second tie on `created_at`, and the identifier
        // that breaks the tie is random.
        let surplus: Vec<String> = authorizations::Entity::find()
            .select_only()
            .column(authorizations::Column::Id)
            .filter(authorizations::Column::UserId.eq(user_id.to_owned()))
            .filter(authorizations::Column::Kind.eq(kind.to_owned()))
            .filter(authorizations::Column::Id.ne(newest.to_owned()))
            .order_by_desc(authorizations::Column::CreatedAt)
            .order_by_desc(authorizations::Column::Id)
            .offset(keep.saturating_sub(1))
            .limit(1000)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(db_error)?;
        if surplus.is_empty() {
            return Ok(0);
        }
        let deleted = authorizations::Entity::delete_many()
            .filter(authorizations::Column::Id.is_in(surplus))
            .filter(authorizations::Column::Kind.eq(kind.to_owned()))
            .exec(&self.db)
            .await
            .map_err(db_error)?;
        Ok(deleted.rows_affected)
    }

    async fn prune_expired(&self, now: DateTime<Utc>, grant_cutoff: DateTime<Utc>) -> Result<u64> {
        let steps = self
            .delete_steps_in_batches(
                Condition::all().add(authorizations::Column::ExpiresAt.lte(now)),
            )
            .await?;
        let expired_tokens = self
            .delete_tokens_in_batches(Condition::all().add(tokens::Column::ExpiresAt.lte(now)))
            .await?;
        let long_revoked = Query::select()
            .column(grants::Column::Id)
            .from(grants::Entity)
            .and_where(Expr::col(grants::Column::RevokedAt).lt(grant_cutoff))
            .to_owned();
        let revoked_tokens = self
            .delete_tokens_in_batches(
                Condition::all().add(tokens::Column::GrantId.in_subquery(long_revoked)),
            )
            .await?;
        let revoked_grants = self
            .delete_grants_in_batches(
                Condition::all().add(grants::Column::RevokedAt.lt(grant_cutoff)),
            )
            .await?;
        // A grant with no token left can never be used again and is no
        // longer listed; it stays in the table for the retention as a
        // record of what the user approved.
        let any_token = Query::select()
            .expr(Expr::val(1))
            .from(tokens::Entity)
            .and_where(
                Expr::col((tokens::Entity, tokens::Column::GrantId))
                    .equals((grants::Entity, grants::Column::Id)),
            )
            .to_owned();
        let idle_grants = self
            .delete_grants_in_batches(
                // A revoked grant ended when it was revoked and follows
                // `revoked_at` above, whatever its tokens.
                Condition::all()
                    .add(grants::Column::RevokedAt.is_null())
                    .add(Expr::exists(any_token).not())
                    .add(grants::Column::CreatedAt.lt(grant_cutoff))
                    .add(
                        Condition::any()
                            .add(grants::Column::LastUsedAt.is_null())
                            .add(grants::Column::LastUsedAt.lt(grant_cutoff)),
                    ),
            )
            .await?;
        Ok(steps + expired_tokens + revoked_tokens + revoked_grants + idle_grants)
    }
}

/// How many rows one pruning statement deletes at most, so no statement
/// holds locks over a large part of a table while requests wait.
const PRUNE_BATCH: u64 = 500;

impl SqlAuthorizationStore {
    async fn delete_steps_in_batches(&self, condition: Condition) -> Result<u64> {
        let mut deleted = 0;
        loop {
            let ids: Vec<String> = authorizations::Entity::find()
                .select_only()
                .column(authorizations::Column::Id)
                .filter(condition.clone())
                .limit(PRUNE_BATCH)
                .into_tuple::<String>()
                .all(&self.db)
                .await
                .map_err(db_error)?;
            let selected = ids.len() as u64;
            if selected == 0 {
                return Ok(deleted);
            }
            let batch = authorizations::Entity::delete_many()
                .filter(authorizations::Column::Id.is_in(ids))
                .filter(condition.clone())
                .exec(&self.db)
                .await
                .map_err(db_error)?
                .rows_affected;
            deleted += batch;
            if selected < PRUNE_BATCH || batch == 0 {
                return Ok(deleted);
            }
        }
    }

    async fn delete_tokens_in_batches(&self, condition: Condition) -> Result<u64> {
        let mut deleted = 0;
        loop {
            let ids: Vec<String> = tokens::Entity::find()
                .select_only()
                .column(tokens::Column::Id)
                .filter(condition.clone())
                .limit(PRUNE_BATCH)
                .into_tuple::<String>()
                .all(&self.db)
                .await
                .map_err(db_error)?;
            let selected = ids.len() as u64;
            if selected == 0 {
                return Ok(deleted);
            }
            let batch = tokens::Entity::delete_many()
                .filter(tokens::Column::Id.is_in(ids))
                .filter(condition.clone())
                .exec(&self.db)
                .await
                .map_err(db_error)?
                .rows_affected;
            deleted += batch;
            if selected < PRUNE_BATCH || batch == 0 {
                return Ok(deleted);
            }
        }
    }

    async fn delete_grants_in_batches(&self, condition: Condition) -> Result<u64> {
        let mut deleted = 0;
        loop {
            let ids: Vec<String> = grants::Entity::find()
                .select_only()
                .column(grants::Column::Id)
                .filter(condition.clone())
                .limit(PRUNE_BATCH)
                .into_tuple::<String>()
                .all(&self.db)
                .await
                .map_err(db_error)?;
            let selected = ids.len() as u64;
            if selected == 0 {
                return Ok(deleted);
            }
            let batch = grants::Entity::delete_many()
                .filter(grants::Column::Id.is_in(ids))
                .filter(condition.clone())
                .exec(&self.db)
                .await
                .map_err(db_error)?
                .rows_affected;
            deleted += batch;
            if selected < PRUNE_BATCH || batch == 0 {
                return Ok(deleted);
            }
        }
    }
}
