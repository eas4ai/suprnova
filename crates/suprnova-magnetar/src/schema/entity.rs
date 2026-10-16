//! Common descriptor contract shared by every bound application entity.

/// Describes one application-owned SeaORM entity without naming its table.
pub trait EntityBinding: Send + Sync + 'static {
    /// The generated SeaORM entity type.
    type Entity: sea_orm::EntityTrait<Model = Self::Model, ActiveModel = Self::ActiveModel>;
    /// The generated SeaORM column enum.
    type Column: sea_orm::ColumnTrait;
    /// The generated SeaORM primary-key enum.
    type PrimaryKey: sea_orm::PrimaryKeyTrait;
    /// The generated row model.
    type Model: sea_orm::ModelTrait<Entity = Self::Entity>
        + sea_orm::FromQueryResult
        + sea_orm::IntoActiveModel<Self::ActiveModel>
        + Clone
        + Send
        + Sync
        + 'static;
    /// The generated SeaORM active model.
    type ActiveModel: sea_orm::ActiveModelTrait<Entity = Self::Entity> + Default + Send;
}
