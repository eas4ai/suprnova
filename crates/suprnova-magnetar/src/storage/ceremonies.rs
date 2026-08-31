//! Generic ceremony records and single-winner state transitions.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};

use super::{SeaOrmStorage, db_error, in_transaction, random_id};
use crate::schema::{AuthSchema, CeremonyFields, EntityBinding};
use crate::{Error, Result};

/// New ceremony input.
#[derive(Clone, Debug)]
pub struct NewCeremony {
    /// Unique caller selector.
    pub selector: String,
    /// Namespaced ceremony kind.
    pub kind: String,
    /// Initial state.
    pub state: String,
    /// Serialized opaque payload.
    pub payload: Vec<u8>,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
}

/// Generic ceremony row returned by storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CeremonyRecord {
    /// Application-owned row identifier.
    pub id: String,
    /// Unique selector.
    pub selector: String,
    /// Ceremony kind namespace.
    pub kind: String,
    /// Current state.
    pub state: String,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
    /// Opaque payload bytes.
    pub payload: Vec<u8>,
}

/// Generic ceremony storage API.
#[async_trait]
pub trait CeremonyStore: Send + Sync {
    /// Create a ceremony row.
    async fn create(&self, input: NewCeremony) -> Result<CeremonyRecord>;
    /// Consume exactly one unexpired row, selected by selector and kind.
    async fn consume(&self, selector: &str, kind: &str) -> Result<Option<CeremonyRecord>>;
    /// Read an unexpired row without deleting or changing it.
    async fn peek(&self, selector: &str, kind: &str) -> Result<Option<CeremonyRecord>>;
    /// Conditionally transition one unexpired row. Returns whether this caller won.
    async fn transition(
        &self,
        selector: &str,
        kind: &str,
        expected: &str,
        next: &str,
    ) -> Result<bool>;
    /// Atomically transition one live ceremony and consume another live ceremony.
    ///
    /// Drivers must override this method only when they can guarantee both writes
    /// commit or roll back together. The default fails closed so existing external
    /// implementors remain source-compatible without weakening that guarantee.
    async fn transition_and_consume(
        &self,
        _transition_selector: &str,
        _transition_kind: &str,
        _expected: &str,
        _next: &str,
        _consume_selector: &str,
        _consume_kind: &str,
    ) -> Result<Option<CeremonyRecord>> {
        Err(Error::DependencyUnavailable {
            dependency: "ceremony store".to_owned(),
            message: "atomic transition-and-consume is unavailable".to_owned(),
        })
    }
}

fn record<S>(model: &<S::Ceremony as EntityBinding>::Model) -> CeremonyRecord
where
    S: AuthSchema,
    S::Ceremony: CeremonyFields,
{
    CeremonyRecord {
        id: S::Ceremony::read_ceremony_id(model),
        selector: S::Ceremony::read_selector(model),
        kind: S::Ceremony::read_kind(model),
        state: S::Ceremony::read_state(model),
        expires_at: S::Ceremony::read_expires_at(model),
        payload: S::Ceremony::read_payload(model),
    }
}

#[async_trait]
impl<S> CeremonyStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::Ceremony: CeremonyFields,
    <S::Ceremony as EntityBinding>::Entity: EntityTrait<
            Model = <S::Ceremony as EntityBinding>::Model,
            ActiveModel = <S::Ceremony as EntityBinding>::ActiveModel,
        >,
    <S::Ceremony as EntityBinding>::Column: ColumnTrait,
{
    async fn create(&self, input: NewCeremony) -> Result<CeremonyRecord> {
        if input.selector.is_empty() || input.kind.is_empty() {
            return Err(Error::InvalidInput {
                field: "selector/kind".to_owned(),
                message: "ceremony selector and kind must be non-empty".to_owned(),
            });
        }
        let id = random_id();
        let mut model: <S::Ceremony as EntityBinding>::ActiveModel = Default::default();
        S::Ceremony::write_ceremony_id(&mut model, &id);
        S::Ceremony::write_selector(&mut model, &input.selector);
        S::Ceremony::write_kind(&mut model, &input.kind);
        S::Ceremony::write_state(&mut model, &input.state);
        S::Ceremony::write_payload(&mut model, &input.payload);
        S::Ceremony::write_expires_at(&mut model, input.expires_at);
        <S::Ceremony as EntityBinding>::Entity::insert(model)
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(CeremonyRecord {
            id,
            selector: input.selector,
            kind: input.kind,
            state: input.state,
            expires_at: input.expires_at,
            payload: input.payload,
        })
    }

    async fn consume(&self, selector: &str, kind: &str) -> Result<Option<CeremonyRecord>> {
        let selector = selector.to_owned();
        let kind = kind.to_owned();
        in_transaction(self.database(), |tx| {
            Box::pin(async move {
                let now = Utc::now();
                let row = <<S::Ceremony as EntityBinding>::Entity as EntityTrait>::find()
                    .filter(S::Ceremony::selector_column().eq(selector.clone()))
                    .filter(S::Ceremony::kind_column().eq(kind.clone()))
                    .filter(S::Ceremony::used_at_column().is_null())
                    .filter(S::Ceremony::expires_at_column().gt(now))
                    .all(tx.connection())
                    .await
                    .map_err(db_error)?
                    .into_iter()
                    .next();
                let Some(row) = row else { return Ok(None) };
                let id = S::Ceremony::read_ceremony_id(&row);
                let deleted =
                    <<S::Ceremony as EntityBinding>::Entity as EntityTrait>::delete_many()
                        .filter(S::Ceremony::ceremony_id_column().eq(id.clone()))
                        .filter(S::Ceremony::kind_column().eq(kind.clone()))
                        .exec(tx.connection())
                        .await
                        .map_err(db_error)?;
                if deleted.rows_affected != 1 {
                    return Err(Error::Conflict {
                        resource: "ceremony".to_owned(),
                        message: "ceremony selector was consumed concurrently".to_owned(),
                    });
                }
                Ok(Some(record::<S>(&row)))
            })
        })
        .await
    }

    async fn peek(&self, selector: &str, kind: &str) -> Result<Option<CeremonyRecord>> {
        let now = Utc::now();
        let rows = <S::Ceremony as EntityBinding>::Entity::find()
            .filter(S::Ceremony::selector_column().eq(selector.to_owned()))
            .filter(S::Ceremony::kind_column().eq(kind.to_owned()))
            .filter(S::Ceremony::used_at_column().is_null())
            .filter(S::Ceremony::expires_at_column().gt(now))
            .all(self.database())
            .await
            .map_err(db_error)?;
        Ok(rows.into_iter().next().map(|row| record::<S>(&row)))
    }

    async fn transition(
        &self,
        selector: &str,
        kind: &str,
        expected: &str,
        next: &str,
    ) -> Result<bool> {
        if selector.is_empty() || kind.is_empty() || expected.is_empty() || next.is_empty() {
            return Err(Error::InvalidInput {
                field: "ceremony state".to_owned(),
                message: "selector, kind, and states must be non-empty".to_owned(),
            });
        }
        let selector = selector.to_owned();
        let kind = kind.to_owned();
        let expected = expected.to_owned();
        let next = next.to_owned();
        in_transaction(self.database(), move |tx| {
            Box::pin(async move {
                let now = Utc::now();
                let row = <<S::Ceremony as EntityBinding>::Entity as EntityTrait>::find()
                    .filter(S::Ceremony::selector_column().eq(selector.clone()))
                    .filter(S::Ceremony::kind_column().eq(kind.clone()))
                    .filter(S::Ceremony::state_column().eq(expected.clone()))
                    .filter(S::Ceremony::used_at_column().is_null())
                    .filter(S::Ceremony::expires_at_column().gt(now))
                    .all(tx.connection())
                    .await
                    .map_err(db_error)?
                    .into_iter()
                    .next();
                let Some(row) = row else { return Ok(false) };
                let id = S::Ceremony::read_ceremony_id(&row);
                let result = <<S::Ceremony as EntityBinding>::Entity as EntityTrait>::update_many()
                    .col_expr(S::Ceremony::state_column(), Expr::value(next.clone()))
                    .filter(S::Ceremony::ceremony_id_column().eq(id))
                    .filter(S::Ceremony::kind_column().eq(kind))
                    .filter(S::Ceremony::state_column().eq(expected))
                    .filter(S::Ceremony::used_at_column().is_null())
                    .filter(S::Ceremony::expires_at_column().gt(now))
                    .exec(tx.connection())
                    .await
                    .map_err(db_error)?;
                Ok(result.rows_affected == 1)
            })
        })
        .await
    }

    async fn transition_and_consume(
        &self,
        transition_selector: &str,
        transition_kind: &str,
        expected: &str,
        next: &str,
        consume_selector: &str,
        consume_kind: &str,
    ) -> Result<Option<CeremonyRecord>> {
        if transition_selector.is_empty()
            || transition_kind.is_empty()
            || expected.is_empty()
            || next.is_empty()
            || consume_selector.is_empty()
            || consume_kind.is_empty()
        {
            return Err(Error::InvalidInput {
                field: "ceremony state".to_owned(),
                message: "selectors, kinds, and states must be non-empty".to_owned(),
            });
        }

        let transition_selector = transition_selector.to_owned();
        let transition_kind = transition_kind.to_owned();
        let expected = expected.to_owned();
        let next = next.to_owned();
        let consume_selector = consume_selector.to_owned();
        let consume_kind = consume_kind.to_owned();
        let transaction = self.database().begin().await.map_err(db_error)?;

        let result = async {
            let now = Utc::now();
            let grant_row = <<S::Ceremony as EntityBinding>::Entity as EntityTrait>::find()
                .filter(S::Ceremony::selector_column().eq(consume_selector.clone()))
                .filter(S::Ceremony::kind_column().eq(consume_kind.clone()))
                .filter(S::Ceremony::used_at_column().is_null())
                .filter(S::Ceremony::expires_at_column().gt(now))
                .one(&transaction)
                .await
                .map_err(db_error)?;
            let Some(grant_row) = grant_row else {
                return Ok(None);
            };
            let grant_id = S::Ceremony::read_ceremony_id(&grant_row);
            let grant_record = record::<S>(&grant_row);

            let transitioned =
                <<S::Ceremony as EntityBinding>::Entity as EntityTrait>::update_many()
                    .col_expr(S::Ceremony::state_column(), Expr::value(next))
                    .filter(S::Ceremony::selector_column().eq(transition_selector))
                    .filter(S::Ceremony::kind_column().eq(transition_kind))
                    .filter(S::Ceremony::state_column().eq(expected))
                    .filter(S::Ceremony::used_at_column().is_null())
                    .filter(S::Ceremony::expires_at_column().gt(now))
                    .exec(&transaction)
                    .await
                    .map_err(db_error)?;
            if transitioned.rows_affected != 1 {
                return Ok(None);
            }

            let deleted = <<S::Ceremony as EntityBinding>::Entity as EntityTrait>::delete_many()
                .filter(S::Ceremony::ceremony_id_column().eq(grant_id))
                .filter(S::Ceremony::selector_column().eq(consume_selector))
                .filter(S::Ceremony::kind_column().eq(consume_kind))
                .filter(S::Ceremony::used_at_column().is_null())
                .filter(S::Ceremony::expires_at_column().gt(now))
                .exec(&transaction)
                .await
                .map_err(db_error)?;
            if deleted.rows_affected != 1 {
                return Ok(None);
            }

            Ok(Some(grant_record))
        }
        .await;

        match result {
            Ok(Some(grant_record)) => transaction
                .commit()
                .await
                .map_err(db_error)
                .map(|()| Some(grant_record)),
            Ok(None) => {
                transaction.rollback().await.map_err(db_error)?;
                Ok(None)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }
}
