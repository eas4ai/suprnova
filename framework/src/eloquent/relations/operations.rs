//! Shared relation writes keep guards, typed values and atomic counters together.

use crate::FrameworkError;
use crate::database::transaction::ExecutorChoice;
use crate::eloquent::{Attrs, Builder, Collection, Model};

/// Keep the creation branch visible so an existing counter alone increments.
pub(super) async fn first_or_create<M>(
    query: Builder<M>,
    attributes: Attrs,
    extra: Attrs,
    owner: Option<(&str, serde_json::Value)>,
) -> Result<(M, bool), FrameworkError>
where
    M: Model + crate::eloquent::EagerLoadDispatch + serde::de::DeserializeOwned,
    M: From<<M::Entity as sea_orm::EntityTrait>::Model>,
    <M::Entity as sea_orm::EntityTrait>::Model: From<M>
        + sea_orm::IntoActiveModel<<M::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <M::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<M::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    let lookup = query.filter_attrs(&attributes);
    if let Some(row) = lookup.clone().first().await? {
        return Ok((row, false));
    }
    let mut values = M::fillable_filter().apply_checked(attributes.merge(extra))?;
    if let Some((column, key)) = owner {
        crate::database::validate_identifier(column)?;
        values.insert(column, key);
    }
    // Filter caller attributes before setting the trusted relation key. The
    // second create filter must not discard a guarded foreign key.
    let savepoint = crate::database::Transaction::current()
        .filter(|tx| tx.backend() == sea_orm::DatabaseBackend::Postgres);
    if let Some(tx) = &savepoint {
        tx.savepoint("relation_create").await?;
    }
    let result = crate::eloquent::unguarded(|| M::create(values)).await;
    match result {
        Ok(row) => Ok((row, true)),
        Err(error @ FrameworkError::Database(_)) => {
            if let Some(tx) = &savepoint {
                tx.rollback_to("relation_create").await?;
            }
            let lookup = if let Some(name) = M::default_connection_name() {
                lookup.on(name)
            } else {
                lookup
            };
            match lookup.first().await? {
                Some(row) => Ok((row, false)),
                None => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

/// Write the increment and extra columns in one statement so a failure changes neither.
pub(super) async fn increment<M>(
    row: M,
    column: &str,
    step: i64,
    extra: Attrs,
) -> Result<M, FrameworkError>
where
    M: Model + crate::eloquent::EagerLoadDispatch + serde::de::DeserializeOwned,
    M: From<<M::Entity as sea_orm::EntityTrait>::Model>,
    <M::Entity as sea_orm::EntityTrait>::Model: From<M>
        + sea_orm::IntoActiveModel<<M::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <M::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<M::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    use super::belongs_to_many::{bind_pivot_extra, pivot_extras_through_casts};
    crate::database::validate_identifier(column)?;
    crate::database::validate_identifier(M::TABLE)?;
    crate::database::validate_identifier(M::PRIMARY_KEY)?;
    let extras = pivot_extras_through_casts::<M>(extra, &[column, M::PRIMARY_KEY])?;
    let key = row.primary_key_value_json();
    crate::render_cache::orm::atomic(M::default_connection_name(), || async {
        let exec = ExecutorChoice::resolve_write(None, None, M::default_connection_name()).await?;
        let backend = exec.backend();
        let mut values: Vec<sea_orm::Value> = vec![step.into()];
        let ph = crate::database::__macro_support::placeholder(backend, 1)?;
        let mut sets = vec![format!("{column} = {column} + {ph}")];
        for (name, value) in extras {
            let value = match &exec {
                ExecutorChoice::Tx(tx, _) => {
                    bind_pivot_extra(tx.as_ref(), M::TABLE, &name, value).await?
                }
                ExecutorChoice::Pool(pool, _) => {
                    bind_pivot_extra(pool.inner(), M::TABLE, &name, value).await?
                }
            };
            let expression = match value {
                Some(value) => {
                    let ph = crate::database::placeholder::typed_placeholder(
                        backend,
                        values.len() + 1,
                        &value,
                    )?;
                    values.push(value);
                    ph
                }
                None => "NULL".to_owned(),
            };
            sets.push(format!("{name} = {expression}"));
        }
        let key_value = M::bind_column(M::PRIMARY_KEY, &key)
            .unwrap_or_else(|| crate::eloquent::model::json_value_to_sea_value(&key));
        let key_ph =
            crate::database::placeholder::typed_placeholder(backend, values.len() + 1, &key_value)?;
        values.push(key_value);
        exec.run(sea_orm::Statement::from_sql_and_values(
            backend,
            format!(
                "UPDATE {} SET {} WHERE {} = {key_ph}",
                M::TABLE,
                sets.join(", "),
                M::PRIMARY_KEY
            ),
            values,
        ))
        .await
        .map_err(|error| FrameworkError::database(error.to_string()))?;
        crate::render_cache::orm::after_model_write(&row).await
    })
    .await?;
    let query = Builder::<M>::new();
    let query = if let Some(name) = M::default_connection_name() {
        query.on(name)
    } else {
        query
    };
    query
        .where_key(key)
        .first()
        .await?
        .ok_or_else(|| FrameworkError::not_found("increment_or_create: record no longer exists"))
}

/// Map bounded reads per record without changing the builder's batch callback contract.
pub(super) async fn chunk_map<M, F, Fut, U>(
    query: Builder<M>,
    size: u64,
    mut closure: F,
) -> Result<Collection<U>, FrameworkError>
where
    M: Model + crate::eloquent::EagerLoadDispatch + serde::de::DeserializeOwned,
    M: From<<M::Entity as sea_orm::EntityTrait>::Model>,
    <M::Entity as sea_orm::EntityTrait>::Model: From<M>
        + sea_orm::IntoActiveModel<<M::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <M::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<M::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
    F: FnMut(M) -> Fut + Send,
    Fut: std::future::Future<Output = Result<U, FrameworkError>> + Send,
    U: Send,
{
    if size == 0 {
        return Err(FrameworkError::param("chunk_map: size must be positive"));
    }
    let query = query.order_by(M::PRIMARY_KEY, crate::eloquent::builder::Direction::Asc);
    let mut offset = 0;
    let mut out = Vec::new();
    loop {
        let batch = query.clone().limit(size).offset(offset).get().await?;
        let count = batch.len() as u64;
        for row in batch {
            out.push(closure(row).await?);
        }
        if count < size {
            break;
        }
        offset = offset
            .checked_add(size)
            .ok_or_else(|| FrameworkError::param("chunk_map: offset overflow"))?;
    }
    Ok(Collection::from_vec(out))
}
