//! Models for the RBAC tables, in spatie/laravel-permission's layout.
//!
//! `roles` and `permissions` are spatie's tables: `id`, `name`,
//! `guard_name` and nullable timestamps, unique on `(name, guard_name)`.
//! The assignments live in spatie's `model_has_roles`,
//! `model_has_permissions` and `role_has_permissions`, which the functions
//! in [`crate::rbac`] read and write; they have composite keys and no
//! model of their own here.
//!
//! A display name has no column in spatie's tables. An upgrade from the
//! earlier Suprnova layout keeps the ones it finds in
//! `suprnova_role_details` and `suprnova_permission_details`, read through
//! [`RoleDetail`] and [`PermissionDetail`].

use chrono::{DateTime, Utc};

/// Role row, usually named for a coarse application capability such as
/// `"admin"` or `"author"`.
///
/// The timestamps use the naive casts, which read the `TIMESTAMP` columns
/// spatie's migration creates on MySQL, `timestamp` on Postgres and the
/// text SQLite stores.
#[suprnova::model(
    table = "roles",
    timestamps,
    casts = {
        created_at = suprnova::AsOptionalNaiveDateTime,
        updated_at = suprnova::AsOptionalNaiveDateTime,
    },
)]
pub struct Role {
    /// Primary key.
    pub id: u64,
    /// Role name, unique with [`Self::guard_name`].
    pub name: String,
    /// Guard namespace; `"web"` for normal session users.
    pub guard_name: String,
    /// Timestamp at which the row was inserted.
    pub created_at: Option<DateTime<Utc>>,
    /// Timestamp at which the row was last mutated.
    pub updated_at: Option<DateTime<Utc>>,
}

/// Permission row, usually named as a dotted ability such as
/// `"articles.create"`.
///
/// The timestamps use the naive casts for the same reason as [`Role`]'s.
#[suprnova::model(
    table = "permissions",
    timestamps,
    casts = {
        created_at = suprnova::AsOptionalNaiveDateTime,
        updated_at = suprnova::AsOptionalNaiveDateTime,
    },
)]
pub struct Permission {
    /// Primary key.
    pub id: u64,
    /// Permission name, unique with [`Self::guard_name`].
    pub name: String,
    /// Guard namespace; `"web"` for normal session users.
    pub guard_name: String,
    /// Timestamp at which the row was inserted.
    pub created_at: Option<DateTime<Utc>>,
    /// Timestamp at which the row was last mutated.
    pub updated_at: Option<DateTime<Utc>>,
}

/// The display name an earlier Suprnova release kept on a role.
#[suprnova::model(
    table = "suprnova_role_details",
    primary_key = "role_id",
    auto_increment = false,
    timestamps = false
)]
pub struct RoleDetail {
    /// The role's id in `roles`.
    pub role_id: u64,
    /// Human-readable label shown in admin UIs.
    pub display_name: Option<String>,
}

/// The display name an earlier Suprnova release kept on a permission.
#[suprnova::model(
    table = "suprnova_permission_details",
    primary_key = "permission_id",
    auto_increment = false,
    timestamps = false
)]
pub struct PermissionDetail {
    /// The permission's id in `permissions`.
    pub permission_id: u64,
    /// Human-readable label shown in admin UIs.
    pub display_name: Option<String>,
}

pub use permission::{
    ActiveModel as PermissionActiveModel, Column as PermissionColumn, Entity as PermissionEntity,
    Model as PermissionModel,
};
pub use permission_detail::{
    ActiveModel as PermissionDetailActiveModel, Column as PermissionDetailColumn,
    Entity as PermissionDetailEntity, Model as PermissionDetailModel,
};
pub use role::{
    ActiveModel as RoleActiveModel, Column as RoleColumn, Entity as RoleEntity, Model as RoleModel,
};
pub use role_detail::{
    ActiveModel as RoleDetailActiveModel, Column as RoleDetailColumn, Entity as RoleDetailEntity,
    Model as RoleDetailModel,
};
